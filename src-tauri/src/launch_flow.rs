//! (02/11) Etapa 6 — «Secuencia de arranque» (como el Launch Flow de Bob
//! Dispatcher). Cada secuencia abre en orden apps, webs, carpetas y el
//! simulador; antes de cada paso se esperan unos segundos o un momento del
//! simulador (MSFS abierto / avión cargado). El front elige la secuencia por el
//! tipo de avión del OFP. Al cerrar el simulador se cierran las apps marcadas
//! «se cierra al salir», sólo las que abrió la secuencia: nunca las que ya
//! estaban abiertas.
//!
//! Entorno de pruebas: el simulador NUNCA se abre de verdad. Con
//! `SIMFLEET_LAUNCH_SIM` (un .exe inofensivo) y `SIMFLEET_LAUNCH_SIM_PROCESS`
//! (su proceso) hace de simulador: «abierto» a los 3 s y «avión cargado» a
//! los 8 s de verlo vivo.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI8, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use sha2::Digest as _;
use sqlx::{Row, SqlitePool};
use tauri::{AppHandle, Emitter, Manager};

use crate::dispatch::DispatchChanges;

// ---------------------------------------------------------------------------
// Secuencias
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LaunchStep {
    #[serde(default)]
    pub id: String,
    /// `app` | `web` | `folder` | `sim` | `script`.
    pub kind: String,
    #[serde(default)]
    pub name: String,
    /// Ruta del .exe, dirección web o carpeta (vacío en el simulador).
    #[serde(default)]
    pub target: String,
    #[serde(default)]
    pub args: String,
    /// Procesos de la app cuando se abre con un lanzador (SayIntentions,
    /// VAS ACARS, Discord…). Vacío = el propio .exe.
    #[serde(default)]
    pub process: Vec<String>,
    /// `normal` | `minimized` | `admin` (las webs van al navegador).
    #[serde(default = "normal")]
    pub open: String,
    /// Espera ANTES de este paso: `seconds` | `menu` | `loaded`.
    #[serde(default = "seconds")]
    pub wait: String,
    #[serde(default)]
    pub wait_seconds: u32,
    /// `stay` | `exit` (se cierra al salir del simulador).
    #[serde(default = "stay")]
    pub close: String,
}

fn normal() -> String {
    "normal".into()
}
fn seconds() -> String {
    "seconds".into()
}
fn stay() -> String {
    "stay".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchSequence {
    /// 0 = nueva.
    #[serde(default)]
    pub id: i64,
    pub name: String,
    /// Tipos ICAO de avión del OFP que la usan (A20N, B738…).
    #[serde(default)]
    pub aircraft: Vec<String>,
    #[serde(default)]
    pub is_default: bool,
    #[serde(default)]
    pub steps: Vec<LaunchStep>,
    /// Pasos cuyo .exe o carpeta ya no existe (sólo lectura).
    #[serde(default, skip_deserializing)]
    pub missing: Vec<String>,
}

const EXAMPLES_KEY: &str = "launch_examples_done";

/// (06/11) TechLog 1.2 reemplaza a las dos apps: un paso que abría una de
/// ellas pasa a abrirlo a él. Conserva la forma de abrir, la espera y el cierre.
fn swap_techlog(steps: &mut [LaunchStep], exe: &str) -> bool {
    let mut changed = false;
    for step in steps.iter_mut().filter(|s| s.kind == "app") {
        let target = step.target.replace('/', r"\").to_ascii_lowercase();
        if target.ends_with(r"\fenix techlog\app\techlog.exe") || target.ends_with(r"\pmdg techlog\app\pmdgtechlog.exe") {
            step.target = exe.to_string();
            if matches!(step.name.as_str(), "Fenix TechLog" | "PMDG TechLog") {
                step.name = "TechLog".into();
            }
            changed = true;
        }
    }
    changed
}

/// Las secuencias guardadas que aún abren una app TechLog antigua.
async fn migrate_techlog(pool: &SqlitePool) {
    let Some(exe) = crate::techlog::unified_exe() else { return };
    let exe = exe.to_string_lossy();
    let Ok(rows) = sqlx::query("SELECT id, steps FROM launch_sequence").fetch_all(pool).await else { return };
    for r in rows {
        let Ok(mut steps) = serde_json::from_str::<Vec<LaunchStep>>(&r.get::<String, _>("steps")) else { continue };
        if !swap_techlog(&mut steps, &exe) {
            continue;
        }
        let Ok(json) = serde_json::to_string(&steps) else { continue };
        let id: i64 = r.get("id");
        match sqlx::query("UPDATE launch_sequence SET steps = ?1 WHERE id = ?2").bind(json).bind(id).execute(pool).await {
            Ok(_) => tracing::info!(target: "launch", "secuencia {id}: ahora abre el TechLog unificado"),
            Err(e) => tracing::warn!(target: "launch", "secuencia {id}: {e:#}"),
        }
    }
}

/// Las secuencias, en orden. La primera vez crea las de ejemplo.
pub async fn list(pool: &SqlitePool) -> Result<Vec<LaunchSequence>> {
    ensure_examples(pool).await;
    migrate_techlog(pool).await;
    let rows = sqlx::query("SELECT id, name, aircraft, is_default, steps FROM launch_sequence ORDER BY sort, id")
        .fetch_all(pool)
        .await?;
    Ok(rows
        .iter()
        .map(|r| {
            let steps: Vec<LaunchStep> = serde_json::from_str(&r.get::<String, _>("steps")).unwrap_or_default();
            let missing = steps
                .iter()
                .filter(|s| match s.kind.as_str() {
                    "app" | "script" => !Path::new(&s.target).is_file(),
                    "folder" => !Path::new(&s.target).is_dir(),
                    _ => false,
                })
                .map(|s| s.id.clone())
                .collect();
            LaunchSequence {
                id: r.get("id"),
                name: r.get("name"),
                aircraft: serde_json::from_str(&r.get::<String, _>("aircraft")).unwrap_or_default(),
                is_default: r.get::<i64, _>("is_default") != 0,
                steps,
                missing,
            }
        })
        .collect())
}

async fn get(pool: &SqlitePool, id: i64) -> Result<LaunchSequence> {
    list(pool)
        .await?
        .into_iter()
        .find(|s| s.id == id)
        .ok_or_else(|| anyhow::anyhow!("not_found"))
}

fn is_exe(p: &str) -> bool {
    Path::new(p).extension().is_some_and(|e| e.eq_ignore_ascii_case("exe"))
}

/// (07/11) Etapa F: lo que puede ser un paso de script.
fn is_script(p: &str) -> bool {
    Path::new(p)
        .extension()
        .is_some_and(|e| ["bat", "cmd", "ps1"].iter().any(|x| e.eq_ignore_ascii_case(x)))
}

fn is_web(u: &str) -> bool {
    url::Url::parse(u).is_ok_and(|u| matches!(u.scheme(), "http" | "https"))
}

/// Nombre por defecto de un paso: el archivo, la carpeta o el dominio.
fn default_name(s: &LaunchStep) -> String {
    match s.kind.as_str() {
        "web" => url::Url::parse(&s.target)
            .ok()
            .and_then(|u| u.host_str().map(|h| h.trim_start_matches("www.").to_string()))
            .unwrap_or_else(|| s.target.clone()),
        "sim" => "MSFS".into(),
        _ => Path::new(&s.target)
            .file_stem()
            .or_else(|| Path::new(&s.target).file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| s.target.clone()),
    }
}

/// Valida y normaliza lo que manda el editor.
fn clean(mut seq: LaunchSequence) -> Result<LaunchSequence> {
    seq.name = seq.name.trim().chars().take(60).collect();
    if seq.name.is_empty() {
        bail!("name_required");
    }
    let mut seen = HashSet::new();
    seq.aircraft = seq
        .aircraft
        .iter()
        .map(|a| a.trim().to_uppercase())
        .filter(|a| (2..=4).contains(&a.len()) && a.chars().all(|c| c.is_ascii_alphanumeric()))
        .filter(|a| seen.insert(a.clone()))
        .collect();
    if seq.steps.len() > 30 {
        bail!("too_many_steps");
    }
    if seq.steps.iter().filter(|s| s.kind == "sim").count() > 1 {
        bail!("one_sim");
    }
    let mut ids = HashSet::new();
    for s in &mut seq.steps {
        s.target = s.target.trim().to_string();
        s.args = s.args.trim().to_string();
        if s.id.trim().is_empty() || !ids.insert(s.id.clone()) {
            s.id = uuid::Uuid::new_v4().to_string();
            ids.insert(s.id.clone());
        }
        match s.kind.as_str() {
            "app" if is_exe(&s.target) => {}
            "app" => bail!("bad_app"),
            "web" if is_web(&s.target) => {}
            "web" => bail!("bad_web"),
            "folder" if !s.target.is_empty() => {}
            "folder" => bail!("bad_folder"),
            "script" if is_script(&s.target) => {}
            "script" => bail!("bad_script"),
            "sim" => {
                s.target.clear();
                s.args.clear();
            }
            _ => bail!("bad_kind"),
        }
        s.name = s.name.trim().chars().take(80).collect();
        if s.name.is_empty() {
            s.name = default_name(s);
        }
        let app = s.kind == "app";
        if !app || !matches!(s.open.as_str(), "normal" | "minimized" | "admin") {
            s.open = "normal".into();
        }
        if !app || s.close != "exit" {
            s.close = "stay".into();
        }
        if !app {
            s.process.clear();
            if s.kind != "script" {
                s.args.clear();
            }
        }
        s.process = s
            .process
            .iter()
            .map(|p| p.trim().to_string())
            .filter(|p| is_exe(p) && !p.contains(['\\', '/']))
            .take(5)
            .collect();
        if !matches!(s.wait.as_str(), "seconds" | "menu" | "loaded") {
            s.wait = "seconds".into();
        }
        s.wait_seconds = s.wait_seconds.min(600);
    }
    Ok(seq)
}

/// Guarda (crea o cambia) una secuencia. Un tipo de avión sólo puede estar en
/// una secuencia: si estaba en otra, se le quita allí.
pub async fn save(pool: &SqlitePool, seq: LaunchSequence) -> Result<i64> {
    let seq = clean(seq)?;
    let aircraft = serde_json::to_string(&seq.aircraft)?;
    let steps = serde_json::to_string(&seq.steps)?;
    let mut tx = pool.begin().await?;
    let id = if seq.id > 0 {
        let n = sqlx::query(
            "UPDATE launch_sequence SET name = ?1, aircraft = ?2, is_default = ?3, steps = ?4, updated_at = datetime('now') WHERE id = ?5",
        )
        .bind(&seq.name)
        .bind(&aircraft)
        .bind(seq.is_default as i64)
        .bind(&steps)
        .bind(seq.id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if n == 0 {
            bail!("not_found");
        }
        seq.id
    } else {
        sqlx::query(
            "INSERT INTO launch_sequence (name, aircraft, is_default, steps, sort) \
             VALUES (?1, ?2, ?3, ?4, COALESCE((SELECT MAX(sort) + 1 FROM launch_sequence), 0))",
        )
        .bind(&seq.name)
        .bind(&aircraft)
        .bind(seq.is_default as i64)
        .bind(&steps)
        .execute(&mut *tx)
        .await?
        .last_insert_rowid()
    };
    if seq.is_default {
        sqlx::query("UPDATE launch_sequence SET is_default = 0 WHERE id <> ?1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    if !seq.aircraft.is_empty() {
        let others = sqlx::query("SELECT id, aircraft FROM launch_sequence WHERE id <> ?1")
            .bind(id)
            .fetch_all(&mut *tx)
            .await?;
        for r in others {
            let theirs: Vec<String> = serde_json::from_str(&r.get::<String, _>("aircraft")).unwrap_or_default();
            let kept: Vec<String> = theirs.iter().filter(|a| !seq.aircraft.contains(a)).cloned().collect();
            if kept.len() != theirs.len() {
                sqlx::query("UPDATE launch_sequence SET aircraft = ?1 WHERE id = ?2")
                    .bind(serde_json::to_string(&kept)?)
                    .bind(r.get::<i64, _>("id"))
                    .execute(&mut *tx)
                    .await?;
            }
        }
    }
    tx.commit().await?;
    Ok(id)
}

pub async fn delete(pool: &SqlitePool, id: i64) -> Result<()> {
    sqlx::query("DELETE FROM launch_sequence WHERE id = ?1").bind(id).execute(pool).await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Apps encontradas en el PC
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectedApp {
    pub key: String,
    pub name: String,
    pub target: String,
    pub args: String,
    pub process: Vec<String>,
    pub open: String,
    pub wait: String,
    pub wait_seconds: u32,
    pub close: String,
}

impl DetectedApp {
    pub fn step(&self) -> LaunchStep {
        LaunchStep {
            id: uuid::Uuid::new_v4().to_string(),
            kind: "app".into(),
            name: self.name.clone(),
            target: self.target.clone(),
            args: self.args.clone(),
            process: self.process.clone(),
            open: self.open.clone(),
            wait: self.wait.clone(),
            wait_seconds: self.wait_seconds,
            close: self.close.clone(),
        }
    }
}

/// Apps de simulación conocidas: cómo encontrarlas en «Programas instalados»
/// y cómo abrirlas por defecto.
struct Known {
    key: &'static str,
    name: &'static str,
    /// DisplayName del desinstalador (empieza por…).
    uninstall: &'static str,
    /// Ejecutable en su carpeta ("" = el del icono del desinstalador).
    exe: &'static str,
    args: &'static str,
    process: &'static [&'static str],
    open: &'static str,
    wait: &'static str,
    wait_seconds: u32,
    close: &'static str,
}

const fn known(key: &'static str, name: &'static str, uninstall: &'static str, exe: &'static str) -> Known {
    Known { key, name, uninstall, exe, args: "", process: &[], open: "normal", wait: "seconds", wait_seconds: 2, close: "exit" }
}

const KNOWN: &[Known] = &[
    known("activesky", "Active Sky FS", "Active Sky FS", "AS_FS.exe"),
    Known { open: "minimized", close: "stay", ..known("navigraph-simlink", "Navigraph Simlink", "Navigraph Simlink", "NavigraphSimlink.exe") },
    known("navigraph-charts", "Navigraph Charts", "Navigraph Charts", ""),
    Known { open: "minimized", close: "stay", ..known("navigraph-hub", "Navigraph Hub", "Navigraph Hub", "") },
    Known {
        process: &["SayIntentionsAI.exe"],
        wait: "loaded",
        wait_seconds: 0,
        ..known("sayintentions", "SayIntentions.AI", "SayIntentions.AI", "SayIntentionsUpdater3.exe")
    },
    Known { wait: "loaded", wait_seconds: 0, ..known("vpilot", "vPilot", "vPilot", "") },
    Known { args: "launch", process: &["VAS-ACARS.exe"], ..known("vas-acars", "VAS ACARS", "VAS ACARS", "vas-acars-updater.exe") },
    Known { open: "minimized", ..known("techlog", "TechLog", "TechLog", r"app\TechLog.exe") },
    Known { open: "minimized", ..known("fenix-techlog", "Fenix TechLog", "Fenix TechLog", r"app\TechLog.exe") },
    Known { open: "minimized", ..known("pmdg-techlog", "PMDG TechLog", "PMDG TechLog", r"app\PMDGTechLog.exe") },
    known("rex-atmos", "REX Atmos Core", "REX Atmos Core", "weathercreator.exe"),
    known("fs2crew-cc", "FS2Crew Command Center", "FS2Crew Command Center", "FS2CrewCommandCenter.exe"),
    known("little-navmap", "Little Navmap", "Little Navmap", "littlenavmap.exe"),
    known("volanta", "Volanta", "Volanta", ""),
    Known { wait: "loaded", wait_seconds: 0, ..known("beyondatc", "BeyondATC", "BeyondATC", "") },
    known("fsuipc7", "FSUIPC7", "FSUIPC7", "FSUIPC7.exe"),
    known("simbrief-downloader", "SimBrief Downloader", "SimBrief Downloader", ""),
    Known {
        args: "--processStart Discord.exe",
        process: &["Discord.exe"],
        open: "minimized",
        close: "stay",
        ..known("discord", "Discord", "Discord", "Update.exe")
    },
];

#[derive(Debug, Default)]
struct Uninstall {
    display: String,
    location: Option<String>,
    icon: Option<String>,
    uninstall: Option<String>,
}

#[cfg(windows)]
fn uninstall_entries() -> Vec<Uninstall> {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ};
    use winreg::RegKey;
    let roots = [
        (HKEY_LOCAL_MACHINE, r"Software\Microsoft\Windows\CurrentVersion\Uninstall"),
        (HKEY_LOCAL_MACHINE, r"Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall"),
        (HKEY_CURRENT_USER, r"Software\Microsoft\Windows\CurrentVersion\Uninstall"),
    ];
    let mut out = Vec::new();
    for (root, path) in roots {
        let Ok(key) = RegKey::predef(root).open_subkey_with_flags(path, KEY_READ) else { continue };
        for name in key.enum_keys().flatten() {
            let Ok(sub) = key.open_subkey_with_flags(&name, KEY_READ) else { continue };
            let Ok(display) = sub.get_value::<String, _>("DisplayName") else { continue };
            out.push(Uninstall {
                display,
                location: sub.get_value("InstallLocation").ok(),
                icon: sub.get_value("DisplayIcon").ok(),
                uninstall: sub.get_value("UninstallString").ok(),
            });
        }
    }
    out
}

#[cfg(not(windows))]
fn uninstall_entries() -> Vec<Uninstall> {
    Vec::new()
}

/// Primera ruta de una línea del registro: `"C:\x\a.exe",0` o
/// `C:\x\a.exe --flag` → `C:\x\a.exe`.
fn first_path(s: &str) -> Option<PathBuf> {
    let s = s.trim();
    let raw = if let Some(rest) = s.strip_prefix('"') {
        rest.split('"').next()?.to_string()
    } else {
        let lower = s.to_ascii_lowercase();
        match [".exe", ".ico"].iter().filter_map(|ext| lower.find(ext).map(|i| i + ext.len())).min() {
            Some(end) => s[..end].to_string(),
            None => s.split(',').next()?.trim().to_string(),
        }
    };
    let raw = raw.split(',').next()?.trim().to_string();
    (!raw.is_empty()).then(|| PathBuf::from(raw))
}

/// Carpetas donde puede estar la app de una entrada del desinstalador.
fn dirs_of(e: &Uninstall) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(l) = e.location.as_deref().map(|l| l.trim().trim_matches('"')).filter(|l| !l.is_empty()) {
        out.push(PathBuf::from(l));
    }
    for p in [e.icon.as_deref(), e.uninstall.as_deref()].into_iter().flatten().filter_map(first_path) {
        if let Some(d) = p.parent() {
            out.push(d.to_path_buf());
        }
    }
    out.retain(|d| d.is_dir());
    out
}

/// `get`, no `[..]`: hay nombres con símbolos (™) y el corte caería a media letra.
fn starts_ci(s: &str, prefix: &str) -> bool {
    s.get(..prefix.len()).is_some_and(|p| p.eq_ignore_ascii_case(prefix))
}

/// (06/11) Con el TechLog unificado, las apps antiguas ya no se ofrecen aunque
/// sigan instaladas: sus datos ya pasaron a él.
fn drop_replaced_techlogs(apps: &mut Vec<DetectedApp>) {
    if apps.iter().any(|a| a.key == "techlog") {
        apps.retain(|a| a.key != "fenix-techlog" && a.key != "pmdg-techlog");
    }
}

/// Apps de simulación instaladas en este PC.
pub fn detect_apps() -> Vec<DetectedApp> {
    let entries = uninstall_entries();
    let mut out: Vec<DetectedApp> = Vec::new();
    for k in KNOWN {
        let exe = entries.iter().filter(|e| starts_ci(&e.display, k.uninstall)).find_map(|e| {
            if k.exe.is_empty() {
                e.icon.as_deref().and_then(first_path).filter(|p| is_exe(&p.to_string_lossy()) && p.is_file())
            } else {
                dirs_of(e).into_iter().map(|d| d.join(k.exe)).find(|p| p.is_file())
            }
        });
        if let Some(exe) = exe {
            out.push(DetectedApp {
                key: k.key.into(),
                name: k.name.into(),
                target: exe.to_string_lossy().into_owned(),
                args: k.args.into(),
                process: k.process.iter().map(|p| p.to_string()).collect(),
                open: k.open.into(),
                wait: k.wait.into(),
                wait_seconds: k.wait_seconds,
                close: k.close.into(),
            });
        }
    }
    drop_replaced_techlogs(&mut out);
    // FS2Crew: una app por avión («FS2Crew Fenix A320 for MSFS»…).
    let mut seen = HashSet::new();
    for e in entries.iter().filter(|e| starts_ci(&e.display, "FS2Crew") && !starts_ci(&e.display, "FS2Crew Command Center")) {
        for dir in dirs_of(e) {
            let Ok(rd) = std::fs::read_dir(&dir) else { continue };
            for f in rd.flatten() {
                let p = f.path();
                let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                if !is_exe(&p.to_string_lossy()) || !starts_ci(&stem, "FS2Crew") || stem.to_lowercase().contains("speech") {
                    continue;
                }
                let key = format!("fs2crew:{}", stem.to_lowercase());
                if seen.insert(key.clone()) {
                    out.push(DetectedApp {
                        key,
                        name: stem,
                        target: p.to_string_lossy().into_owned(),
                        args: String::new(),
                        process: Vec::new(),
                        open: "normal".into(),
                        wait: "loaded".into(),
                        wait_seconds: 0,
                        close: "exit".into(),
                    });
                }
            }
        }
    }
    out.sort_by_key(|a| a.name.to_lowercase());
    out
}

fn sim_step() -> LaunchStep {
    LaunchStep {
        id: uuid::Uuid::new_v4().to_string(),
        kind: "sim".into(),
        name: "MSFS".into(),
        target: String::new(),
        args: String::new(),
        process: Vec::new(),
        open: "normal".into(),
        wait: "seconds".into(),
        wait_seconds: 3,
        close: "stay".into(),
    }
}

/// Las tres de ejemplo (decisión del usuario): Fenix A320, PMDG 737 y «Por
/// defecto», con las apps encontradas. El usuario las ajusta.
fn examples(apps: &[DetectedApp], has_sim: bool) -> Vec<LaunchSequence> {
    let find = |key: &str| apps.iter().find(|a| a.key == key);
    let techlog = |old: &str| find("techlog").or_else(|| find(old));
    let fs2 = |needle: &str| {
        apps.iter()
            .find(|a| a.key.starts_with("fs2crew:") && a.key.contains(needle) && !a.key.contains("animated"))
    };
    let build = |first: Option<&DetectedApp>, last: Option<&DetectedApp>| -> Vec<LaunchStep> {
        let mut v: Vec<LaunchStep> = first.into_iter().map(DetectedApp::step).collect();
        v.extend([find("activesky"), find("navigraph-simlink")].into_iter().flatten().map(DetectedApp::step));
        if has_sim {
            v.push(sim_step());
        }
        v.extend(find("sayintentions").map(DetectedApp::step));
        v.extend(last.map(DetectedApp::step));
        if let Some(s) = v.first_mut() {
            if s.wait == "seconds" {
                s.wait_seconds = 0;
            }
        }
        v
    };
    let seq = |name: String, aircraft: &[&str], is_default: bool, steps: Vec<LaunchStep>| LaunchSequence {
        id: 0,
        name,
        aircraft: aircraft.iter().map(|a| a.to_string()).collect(),
        is_default,
        steps,
        missing: Vec::new(),
    };
    [
        seq(
            "Fenix A320".into(),
            &["A319", "A320", "A20N", "A321", "A21N"],
            false,
            build(techlog("fenix-techlog"), fs2("fenix")),
        ),
        seq(
            "PMDG 737".into(),
            &["B736", "B737", "B738", "B739", "B38M", "B39M"],
            false,
            build(techlog("pmdg-techlog"), fs2("pmdg 737")),
        ),
        seq(crate::tr!("launch.defaultName"), &[], true, build(None, None)),
    ]
    .into_iter()
    .filter(|s| !s.steps.is_empty())
    .collect()
}

async fn ensure_examples(pool: &SqlitePool) {
    let done: Option<String> = sqlx::query_scalar("SELECT value FROM settings WHERE key = ?1")
        .bind(EXAMPLES_KEY)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();
    if done.is_some() {
        return;
    }
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM launch_sequence")
        .fetch_one(pool)
        .await
        .unwrap_or(0);
    if count == 0 {
        // Si la detección falla, se reintenta la próxima vez.
        let Ok(apps) = tokio::task::spawn_blocking(detect_apps).await else { return };
        let has_sim = sim_target().is_some();
        for s in examples(&apps, has_sim) {
            if let Err(e) = save(pool, s).await {
                tracing::warn!(target: "launch", "secuencia de ejemplo: {e:#}");
            }
        }
    }
    let _ = sqlx::query("INSERT OR REPLACE INTO settings (key, value) VALUES (?1, '1')")
        .bind(EXAMPLES_KEY)
        .execute(pool)
        .await;
}

// ---------------------------------------------------------------------------
// El simulador
// ---------------------------------------------------------------------------

enum SimTarget {
    Url(&'static str),
    AppsFolder(&'static str),
    Exe(PathBuf),
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SimInfo {
    pub available: bool,
    /// «MSFS 2024 (Steam)»…
    pub label: String,
    pub running: bool,
    pub test_env: bool,
}

fn sim_target() -> Option<(String, SimTarget)> {
    use crate::community::MsfsVariant as V;
    if crate::test_env::is_active() && !crate::test_env::real_sim() {
        return std::env::var_os("SIMFLEET_LAUNCH_SIM")
            .map(PathBuf::from)
            .filter(|p| p.is_file())
            .map(|p| ("MSFS (prueba)".to_string(), SimTarget::Exe(p)));
    }
    let variants = crate::community::installed_variants();
    let is_2024 = |v: &V| matches!(v, V::MsStore2024 | V::Steam2024);
    let want = crate::sim::is_2024();
    // Dentro de la versión elegida, Steam primero.
    let pick = [V::Steam2024, V::MsStore2024, V::Steam2020, V::MsStore2020]
        .into_iter()
        .filter(|v| variants.contains(v))
        .min_by_key(|v| (is_2024(v) != want) as u8)?;
    Some(match pick {
        V::Steam2024 => ("MSFS 2024 (Steam)".into(), SimTarget::Url("steam://rungameid/2537590")),
        V::Steam2020 => ("MSFS 2020 (Steam)".into(), SimTarget::Url("steam://rungameid/1250410")),
        V::MsStore2024 => ("MSFS 2024 (Microsoft Store)".into(), SimTarget::AppsFolder("Microsoft.Limitless_8wekyb3d8bbwe!App")),
        V::MsStore2020 => (
            "MSFS 2020 (Microsoft Store)".into(),
            SimTarget::AppsFolder("Microsoft.FlightSimulator_8wekyb3d8bbwe!App"),
        ),
    })
}

fn sim_process_names() -> Vec<String> {
    if crate::test_env::is_active() && !crate::test_env::real_sim() {
        return std::env::var("SIMFLEET_LAUNCH_SIM_PROCESS")
            .map(|p| vec![p.to_lowercase()])
            .unwrap_or_default();
    }
    vec!["flightsimulator.exe".into(), "flightsimulator2024.exe".into()]
}

pub async fn sim_info() -> SimInfo {
    let target = sim_target();
    SimInfo {
        available: target.is_some(),
        label: target.map(|(l, _)| l).unwrap_or_default(),
        running: sim_running_now().await,
        test_env: crate::test_env::is_active(),
    }
}

/// -1 = sin datos, 0 = en el menú, 1 = vuelo cargado (evento «Sim»).
static IN_FLIGHT: AtomicI8 = AtomicI8::new(-1);

/// El watcher: el simulador entró a un vuelo (`true`) o volvió al menú.
pub fn sim_event(running: bool) {
    IN_FLIGHT.store(running as i8, Ordering::Relaxed);
}

/// El watcher perdió SimConnect.
pub fn sim_disconnected() {
    IN_FLIGHT.store(-1, Ordering::Relaxed);
}

/// Simulador de prueba: desde cuándo está vivo su proceso.
fn fake_since() -> &'static Mutex<Option<Instant>> {
    static F: OnceLock<Mutex<Option<Instant>>> = OnceLock::new();
    F.get_or_init(|| Mutex::new(None))
}

pub(crate) async fn sim_running_now() -> bool {
    let names = sim_process_names();
    if names.is_empty() {
        return false;
    }
    let procs = tokio::task::spawn_blocking(processes).await.unwrap_or_default();
    let running = procs.iter().any(|p| names.contains(&p.name));
    if crate::test_env::is_active() {
        let mut f = fake_since().lock().unwrap_or_else(|e| e.into_inner());
        match (running, f.is_some()) {
            (true, false) => *f = Some(Instant::now()),
            (false, true) => *f = None,
            _ => {}
        }
    }
    running
}

/// En pruebas con simulador simulado: segundos que lleva vivo.
fn fake_age() -> Option<Duration> {
    if !crate::test_env::is_active() || std::env::var_os("SIMFLEET_LAUNCH_SIM_PROCESS").is_none() {
        return None;
    }
    let since = *fake_since().lock().unwrap_or_else(|e| e.into_inner());
    Some(since.map(|t| t.elapsed()).unwrap_or_default())
}

/// ¿MSFS ya está abierto (proceso detectado o SimConnect activo)?
async fn sim_at_menu(app: &AppHandle) -> bool {
    if fake_age().is_some() {
        sim_running_now().await;
        return fake_age().is_some_and(|a| a >= Duration::from_secs(3));
    }
    let Some(state) = app.try_state::<crate::simconnect_watcher::SharedState>() else { return false };
    let running = state.lock().await.status.sim_running;
    running
}

/// ¿Hay un vuelo cargado? (datos de avión fluyendo + simulación activa).
async fn sim_loaded(app: &AppHandle) -> bool {
    if fake_age().is_some() {
        sim_running_now().await;
        return fake_age().is_some_and(|a| a >= Duration::from_secs(8));
    }
    let Some(state) = app.try_state::<crate::simconnect_watcher::SharedState>() else { return false };
    let connected = state.lock().await.status.simconnect_connected;
    connected && IN_FLIGHT.load(Ordering::Relaxed) == 1
}

// ---------------------------------------------------------------------------
// Procesos
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Proc {
    pid: u32,
    parent: Option<u32>,
    /// En minúsculas.
    name: String,
    start: u64,
}

fn processes() -> Vec<Proc> {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};
    let mut sys = System::new();
    sys.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::new());
    sys.processes()
        .iter()
        .map(|(pid, p)| Proc {
            pid: pid.as_u32(),
            parent: p.parent().map(|x| x.as_u32()),
            name: p.name().to_string_lossy().to_lowercase(),
            start: p.start_time(),
        })
        .collect()
}

/// Procesos que delatan que el paso ya está abierto.
fn watch_names(step: &LaunchStep) -> Vec<String> {
    if !step.process.is_empty() {
        return step.process.iter().map(|p| p.to_lowercase()).collect();
    }
    Path::new(&step.target)
        .file_name()
        .map(|n| vec![n.to_string_lossy().to_lowercase()])
        .unwrap_or_default()
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Un proceso que abrió la secuencia y que se cierra al salir del simulador.
#[derive(Debug, Clone, PartialEq)]
struct Owned {
    step: String,
    pid: u32,
    start: u64,
    admin: bool,
}

fn closing() -> &'static Mutex<Vec<Owned>> {
    static C: OnceLock<Mutex<Vec<Owned>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(Vec::new()))
}

/// (05/11) Cómo acabó el cierre al salir del simulador.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
struct CloseReport {
    closed: Vec<String>,
    open: Vec<String>,
}

/// Un paso cuenta como cerrado solo si no le queda ningún proceso vivo.
fn close_report(targets: &[Owned], alive: &[Owned]) -> CloseReport {
    let mut report = CloseReport::default();
    for o in targets {
        let list = if alive.iter().any(|a| a.step == o.step) { &mut report.open } else { &mut report.closed };
        if !list.contains(&o.step) {
            list.push(o.step.clone());
        }
    }
    report
}

/// Nombres (sin repetir) de lo que se cerrará al salir del simulador.
fn closing_names() -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for o in closing().lock().unwrap_or_else(|e| e.into_inner()).iter() {
        if !names.contains(&o.step) {
            names.push(o.step.clone());
        }
    }
    names
}

/// ¿`p` desciende de `root`? (cadena de padres; el lanzador puede haber
/// terminado y su PID sigue como padre del hijo).
fn descends(p: &Proc, root: u32, by_pid: &HashMap<u32, &Proc>) -> bool {
    let mut cur = p.parent;
    for _ in 0..16 {
        match cur {
            Some(x) if x == root => return true,
            Some(x) => cur = by_pid.get(&x).and_then(|q| q.parent),
            None => return false,
        }
    }
    false
}

/// Olvida lo que ya no existe (el usuario lo cerró a mano).
async fn prune_closing() {
    let procs = tokio::task::spawn_blocking(processes).await.unwrap_or_default();
    closing()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .retain(|o| procs.iter().any(|p| p.pid == o.pid && (o.start == 0 || p.start == o.start)));
}

/// Pasadas de cierre hechas: tras una, nadie sigue apuntando lo que se cierra.
static CLOSE_PASS: AtomicU64 = AtomicU64::new(0);

/// Durante un minuto apunta lo que abrió el paso: el proceso lanzado, sus
/// descendientes y (sin PID) sus procesos que arrancaron después.
async fn track(app: AppHandle, step: LaunchStep, pid: Option<u32>, since: u64) {
    let watch = watch_names(&step);
    let admin = step.open == "admin";
    let pass = CLOSE_PASS.load(Ordering::SeqCst);
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let procs = tokio::task::spawn_blocking(processes).await.unwrap_or_default();
        if CLOSE_PASS.load(Ordering::SeqCst) != pass {
            break;
        }
        let by_pid: HashMap<u32, &Proc> = procs.iter().map(|p| (p.pid, p)).collect();
        let fresh = |p: &Proc| p.start == 0 || p.start + 1 >= since;
        let mut added = false;
        for p in procs.iter().filter(|p| fresh(p)) {
            let ours = pid.is_some_and(|root| p.pid == root || descends(p, root, &by_pid))
                || (watch.contains(&p.name) && p.start + 1 >= since);
            if !ours {
                continue;
            }
            let o = Owned { step: step.name.clone(), pid: p.pid, start: p.start, admin };
            let mut list = closing().lock().unwrap_or_else(|e| e.into_inner());
            if !list.iter().any(|x| x.pid == o.pid && x.start == o.start) {
                list.push(o);
                added = true;
            }
        }
        if added {
            update(&app, |r| r.closing = closing_names());
            ensure_exit_watch(app.clone());
        }
        if Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(Duration::from_millis(1500)).await;
    }
}

/// Vigila el simulador: cuando se cierra (tras haberlo visto abierto), cierra
/// lo apuntado en `closing`.
static EXIT_WATCH: AtomicBool = AtomicBool::new(false);

fn ensure_exit_watch(app: AppHandle) {
    if EXIT_WATCH.swap(true, Ordering::SeqCst) {
        return;
    }
    tauri::async_runtime::spawn(async move {
        let mut seen = false;
        let mut gone = 0;
        let mut ticks = 0u32;
        loop {
            // Cada minuto: fuera lo que el usuario ya cerró a mano.
            ticks += 1;
            if ticks % 12 == 0 {
                prune_closing().await;
                update(&app, |r| r.closing = closing_names());
            }
            if closing().lock().unwrap_or_else(|e| e.into_inner()).is_empty() {
                break;
            }
            if sim_running_now().await {
                seen = true;
                gone = 0;
            } else if seen {
                gone += 1;
                if gone >= 2 {
                    CLOSE_PASS.fetch_add(1, Ordering::SeqCst);
                    let list = std::mem::take(&mut *closing().lock().unwrap_or_else(|e| e.into_inner()));
                    let report = close_all(list).await;
                    tracing::info!(
                        target: "launch",
                        "simulador cerrado: cerradas {:?}, siguen abiertas {:?}",
                        report.closed,
                        report.open
                    );
                    update(&app, |r| r.closing = closing_names());
                    let _ = app.emit("launch://closed", &report);
                    seen = false;
                    gone = 0;
                }
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
        EXIT_WATCH.store(false, Ordering::SeqCst);
        if !closing().lock().unwrap_or_else(|e| e.into_inner()).is_empty() {
            ensure_exit_watch(app);
        }
    });
}

/// Cierra lo apuntado: primero con el aviso normal (WM_CLOSE; las de
/// administrador con `taskkill` elevado), y lo que siga vivo a los 10 s, a la
/// fuerza. Sólo toca procesos con el mismo PID y la misma hora de arranque.
/// (05/11) Al final mira qué sigue vivo: terminar a la fuerza puede fallar (una
/// app que corre como administrador no se deja) y antes contaba como cerrada.
async fn close_all(list: Vec<Owned>) -> CloseReport {
    let alive = |list: &[Owned], procs: &[Proc]| -> Vec<Owned> {
        list.iter()
            .filter(|o| procs.iter().any(|p| p.pid == o.pid && (o.start == 0 || p.start == o.start)))
            .cloned()
            .collect()
    };
    let procs = tokio::task::spawn_blocking(processes).await.unwrap_or_default();
    let targets = alive(&list, &procs);
    if targets.is_empty() {
        return CloseReport::default();
    }
    let normal: HashSet<u32> = targets.iter().filter(|o| !o.admin).map(|o| o.pid).collect();
    let admin: Vec<u32> = targets.iter().filter(|o| o.admin).map(|o| o.pid).collect();
    if !normal.is_empty() {
        let n = normal.clone();
        let _ = tokio::task::spawn_blocking(move || post_close(&n)).await;
    }
    if !admin.is_empty() {
        let a = admin.clone();
        let _ = tokio::task::spawn_blocking(move || elevated_taskkill(&a, false)).await;
    }
    let mut left = targets.clone();
    for _ in 0..10 {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let procs = tokio::task::spawn_blocking(processes).await.unwrap_or_default();
        left = alive(&left, &procs);
        if left.is_empty() {
            return close_report(&targets, &left);
        }
    }
    let force_normal: Vec<Owned> = left.iter().filter(|o| !o.admin).cloned().collect();
    let force_admin: Vec<u32> = left.iter().filter(|o| o.admin).map(|o| o.pid).collect();
    if !force_normal.is_empty() {
        let _ = tokio::task::spawn_blocking(move || kill_verified(&force_normal)).await;
    }
    if !force_admin.is_empty() {
        let _ = tokio::task::spawn_blocking(move || elevated_taskkill(&force_admin, true)).await;
    }
    for _ in 0..5 {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let procs = tokio::task::spawn_blocking(processes).await.unwrap_or_default();
        left = alive(&left, &procs);
        if left.is_empty() {
            break;
        }
    }
    close_report(&targets, &left)
}

/// Termina a la fuerza (sólo si PID y hora de arranque coinciden).
fn kill_verified(list: &[Owned]) {
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
    let mut sys = System::new();
    sys.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::new());
    for o in list {
        if let Some(p) = sys.process(Pid::from_u32(o.pid)) {
            if o.start == 0 || p.start_time() == o.start {
                let ok = p.kill();
                tracing::info!(target: "launch", "«{}» (pid {}) no se cerró con el aviso: terminado = {ok}", o.step, o.pid);
            }
        }
    }
}

/// WM_CLOSE a todas las ventanas de esos procesos (lo que hace `taskkill`
/// sin /F). Devuelve cuántas ventanas lo recibieron.
#[cfg(windows)]
fn post_close(pids: &HashSet<u32>) -> usize {
    use windows::core::BOOL;
    use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{EnumWindows, GetWindowThreadProcessId, PostMessageW, WM_CLOSE};
    struct Ctx<'a> {
        pids: &'a HashSet<u32>,
        sent: usize,
    }
    unsafe extern "system" fn each(hwnd: HWND, lp: LPARAM) -> BOOL {
        // SAFETY: `lp` apunta al `Ctx` vivo de `post_close` durante EnumWindows.
        let ctx = unsafe { &mut *(lp.0 as *mut Ctx) };
        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid as *mut u32)) };
        if ctx.pids.contains(&pid) && unsafe { PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0)) }.is_ok() {
            ctx.sent += 1;
        }
        BOOL(1)
    }
    let mut ctx = Ctx { pids, sent: 0 };
    // SAFETY: callback síncrono; `ctx` vive hasta que EnumWindows vuelve.
    let _ = unsafe { EnumWindows(Some(each), LPARAM(&mut ctx as *mut Ctx as isize)) };
    ctx.sent
}

#[cfg(not(windows))]
fn post_close(_pids: &HashSet<u32>) -> usize {
    0
}

/// `taskkill` como administrador (Windows pide permiso).
fn elevated_taskkill(pids: &[u32], force: bool) {
    let mut params: String = pids.iter().map(|p| format!("/PID {p} ")).collect();
    if force {
        params.push_str("/F");
    }
    if let Err(e) = shell_exec("taskkill.exe", params.trim(), None, Some("runas"), Show::Hidden) {
        tracing::warn!(target: "launch", "taskkill elevado: {e:#}");
    }
}

// ---------------------------------------------------------------------------
// Abrir
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
enum Show {
    Normal,
    Minimized,
    Hidden,
}

/// `ShellExecuteExW`: como un doble clic (o «Ejecutar como administrador» con
/// `runas`). Devuelve el PID de lo que abrió, si hay proceso.
#[cfg(windows)]
fn shell_exec(file: &str, params: &str, dir: Option<&Path>, verb: Option<&str>, show: Show) -> Result<Option<u32>> {
    use windows::core::{HSTRING, PCWSTR};
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE};
    use windows::Win32::System::Threading::GetProcessId;
    use windows::Win32::UI::Shell::{
        ShellExecuteExW, SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
    };
    use windows::Win32::UI::WindowsAndMessaging::{SW_HIDE, SW_SHOWMINNOACTIVE, SW_SHOWNORMAL};
    // ShellExecuteEx puede delegar en extensiones del Shell: pide COM iniciado.
    struct Com(bool);
    impl Drop for Com {
        fn drop(&mut self) {
            if self.0 {
                // SAFETY: empareja el CoInitializeEx que tuvo éxito en este hilo.
                unsafe { CoUninitialize() };
            }
        }
    }
    // SAFETY: sin punteros; si el hilo ya tenía otro modo, sigue con ese.
    let _com = Com(unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) }.is_ok());
    let file_w = HSTRING::from(file);
    let params_w = HSTRING::from(params);
    let dir_w = dir.map(|d| HSTRING::from(d.as_os_str()));
    let verb_w = verb.map(HSTRING::from);
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI,
        lpVerb: verb_w.as_ref().map(|v| PCWSTR(v.as_ptr())).unwrap_or(PCWSTR::null()),
        lpFile: PCWSTR(file_w.as_ptr()),
        lpParameters: if params.is_empty() { PCWSTR::null() } else { PCWSTR(params_w.as_ptr()) },
        lpDirectory: dir_w.as_ref().map(|d| PCWSTR(d.as_ptr())).unwrap_or(PCWSTR::null()),
        nShow: match show {
            Show::Normal => SW_SHOWNORMAL.0,
            Show::Minimized => SW_SHOWMINNOACTIVE.0,
            Show::Hidden => SW_HIDE.0,
        },
        ..Default::default()
    };
    // SAFETY: las cadenas viven hasta que la llamada vuelve (SEE_MASK_NOASYNC).
    if let Err(e) = unsafe { ShellExecuteExW(&mut info) } {
        // 1223 = el usuario dijo que no al permiso de Windows; 2/3 = no existe.
        bail!(match e.code().0 as u32 & 0xFFFF {
            1223 => "uac_cancelled".to_string(),
            2 | 3 => "not_found".to_string(),
            _ => format!("failed:{}", e.message()),
        });
    }
    if info.hProcess.is_invalid() {
        return Ok(None);
    }
    // SAFETY: handle válido devuelto por ShellExecuteExW; lo cerramos aquí.
    let pid = unsafe { GetProcessId(info.hProcess) };
    unsafe {
        let _ = CloseHandle(info.hProcess);
    }
    Ok((pid != 0).then_some(pid))
}

#[cfg(not(windows))]
fn shell_exec(_file: &str, _params: &str, _dir: Option<&Path>, _verb: Option<&str>, _show: Show) -> Result<Option<u32>> {
    bail!("failed:unsupported")
}

/// Abre un paso. `Ok(pid)` del proceso que arrancó (si lo hay).
fn open_step(step: &LaunchStep) -> Result<Option<u32>> {
    match step.kind.as_str() {
        "app" => {
            let exe = Path::new(&step.target);
            if !exe.is_file() {
                bail!("not_found");
            }
            let (verb, show) = match step.open.as_str() {
                "admin" => (Some("runas"), Show::Normal),
                "minimized" => (None, Show::Minimized),
                _ => (None, Show::Normal),
            };
            shell_exec(&step.target, &step.args, exe.parent(), verb, show)
        }
        "web" => {
            if !is_web(&step.target) {
                bail!("bad_web");
            }
            shell_exec(&step.target, "", None, None, Show::Normal).map(|_| None)
        }
        "folder" => {
            if !Path::new(&step.target).is_dir() {
                bail!("not_found");
            }
            shell_exec(&step.target, "", None, Some("open"), Show::Normal).map(|_| None)
        }
        "sim" => match sim_target() {
            None => bail!(if crate::test_env::is_active() { "test_env" } else { "no_sim" }),
            Some((_, SimTarget::Url(u))) => shell_exec(u, "", None, None, Show::Normal).map(|_| None),
            Some((_, SimTarget::AppsFolder(id))) => {
                shell_exec("explorer.exe", &format!("shell:AppsFolder\\{id}"), None, None, Show::Normal).map(|_| None)
            }
            Some((_, SimTarget::Exe(p))) => shell_exec(&p.to_string_lossy(), "", p.parent(), None, Show::Normal),
        },
        _ => bail!("bad_kind"),
    }
}

// ---------------------------------------------------------------------------
// Scripts
// ---------------------------------------------------------------------------
//
// (07/11) Etapa F. Un script corre lo que diga su archivo, así que cada versión
// necesita tu permiso: se guarda la huella del contenido y, si el archivo
// cambia, se vuelve a pedir. Corre sin ventana, con un límite, y si falla se
// detiene el arranque.

const SCRIPTS_OK_KEY: &str = "launch_scripts_ok";
const SCRIPT_LIMIT: Duration = Duration::from_secs(60);

fn script_hash(path: &str) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    Some(sha2::Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect())
}

/// Las primeras líneas, para que veas qué vas a permitir.
fn script_preview(path: &str) -> String {
    let bytes = std::fs::read(path).unwrap_or_default();
    let text = String::from_utf8_lossy(&bytes);
    let lines: Vec<&str> = text.lines().take(12).collect();
    lines.join("\n").chars().take(600).collect()
}

/// Un script que aún no tiene tu permiso.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptPermission {
    pub name: String,
    pub path: String,
    pub hash: String,
    pub preview: String,
}

async fn approved_scripts(pool: &SqlitePool) -> HashSet<String> {
    let raw: Option<String> = sqlx::query_scalar("SELECT value FROM settings WHERE key = ?1")
        .bind(SCRIPTS_OK_KEY)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();
    raw.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

/// Los scripts de estos pasos sin permiso. Un archivo que no existe no pide
/// nada: fallará al ejecutarlo con su propio error.
pub async fn scripts_pending(pool: &SqlitePool, steps: &[LaunchStep]) -> Vec<ScriptPermission> {
    let ok = approved_scripts(pool).await;
    let mut seen = HashSet::new();
    steps
        .iter()
        .filter(|s| s.kind == "script")
        .filter_map(|s| {
            let hash = script_hash(&s.target)?;
            (!ok.contains(&hash) && seen.insert(hash.clone())).then(|| ScriptPermission {
                name: s.name.clone(),
                path: s.target.clone(),
                hash,
                preview: script_preview(&s.target),
            })
        })
        .collect()
}

pub async fn approve_scripts(pool: &SqlitePool, hashes: Vec<String>) -> Result<()> {
    let mut ok = approved_scripts(pool).await;
    ok.extend(hashes.into_iter().filter(|h| h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit())));
    sqlx::query("INSERT OR REPLACE INTO settings (key, value) VALUES (?1, ?2)")
        .bind(SCRIPTS_OK_KEY)
        .bind(serde_json::to_string(&ok)?)
        .execute(pool)
        .await?;
    Ok(())
}

/// El comando del script, sin ventana. `.ps1` con la política saltada sólo para
/// esta ejecución: el archivo ya tiene tu permiso y Windows no cambia nada.
#[cfg(windows)]
fn script_command(step: &LaunchStep) -> std::process::Command {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let ps1 = Path::new(&step.target).extension().is_some_and(|e| e.eq_ignore_ascii_case("ps1"));
    let mut cmd = if ps1 {
        let mut c = std::process::Command::new("powershell.exe");
        c.args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File", &step.target]);
        if !step.args.is_empty() {
            c.raw_arg(&step.args);
        }
        c
    } else {
        let mut c = std::process::Command::new("cmd.exe");
        c.raw_arg(format!("/D /S /C \"\"{}\" {}\"", step.target, step.args));
        c
    };
    if let Some(dir) = Path::new(&step.target).parent() {
        cmd.current_dir(dir);
    }
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}

#[cfg(not(windows))]
fn script_command(step: &LaunchStep) -> std::process::Command {
    std::process::Command::new(&step.target)
}

/// Mata el script y lo que haya abierto.
fn kill_script(pid: Option<u32>) {
    let Some(pid) = pid else { return };
    let mut kill = std::process::Command::new("taskkill");
    kill.args(["/PID", &pid.to_string(), "/T", "/F"]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        kill.creation_flags(0x0800_0000);
    }
    let _ = kill.output();
}

/// Lo último que escribió, para explicar el fallo.
fn script_tail(output: &std::process::Output) -> String {
    let text = if output.stderr.iter().any(|b| !b.is_ascii_whitespace()) { &output.stderr } else { &output.stdout };
    let text = String::from_utf8_lossy(text);
    let text = text.trim();
    let start = text.char_indices().rev().nth(239).map_or(0, |(i, _)| i);
    text[start..].to_string()
}

/// Ejecuta el script y espera a que termine. `stop`: lo cancelaste.
async fn run_script(step: &LaunchStep, limit: Duration, stop: impl Fn() -> bool) -> Result<()> {
    if !Path::new(&step.target).is_file() {
        bail!("not_found");
    }
    let mut cmd = tokio::process::Command::from(script_command(step));
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    let child = cmd.spawn().map_err(|e| anyhow::anyhow!("failed:{e}"))?;
    let pid = child.id();
    let finished = child.wait_with_output();
    tokio::pin!(finished);
    let start = Instant::now();
    loop {
        tokio::select! {
            out = &mut finished => {
                let out = out?;
                if out.status.success() {
                    return Ok(());
                }
                bail!("script_failed:{}:{}", out.status.code().unwrap_or(-1), script_tail(&out));
            }
            _ = tokio::time::sleep(Duration::from_millis(150)) => {
                if start.elapsed() >= limit {
                    tokio::task::spawn_blocking(move || kill_script(pid)).await.ok();
                    bail!("script_timeout");
                }
                if stop() {
                    tokio::task::spawn_blocking(move || kill_script(pid)).await.ok();
                    bail!("cancelled");
                }
            }
        }
    }
}

/// «Probar»: abre sólo este paso, ya. `already` si ya estaba abierto.
pub async fn test_step(pool: &SqlitePool, step: LaunchStep) -> Result<&'static str> {
    if step.kind == "script" {
        if !scripts_pending(pool, std::slice::from_ref(&step)).await.is_empty() {
            bail!("script_permission");
        }
        run_script(&step, SCRIPT_LIMIT, || false).await?;
        return Ok("opened");
    }
    if step.kind == "app" {
        let names = watch_names(&step);
        let procs = tokio::task::spawn_blocking(processes).await.unwrap_or_default();
        if procs.iter().any(|p| names.contains(&p.name)) {
            return Ok("already");
        }
    }
    if step.kind == "sim" && sim_running_now().await {
        return Ok("already");
    }
    tokio::task::spawn_blocking(move || open_step(&step)).await??;
    Ok("opened")
}

// ---------------------------------------------------------------------------
// La secuencia en marcha
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchRunStep {
    pub id: String,
    pub name: String,
    pub kind: String,
    /// `pending` | `waiting` | `opening` | `opened` | `already` | `skipped` |
    /// `failed` | `cancelled`.
    pub status: String,
    /// Qué espera (`seconds` | `menu` | `loaded`) mientras `waiting`.
    pub wait: String,
    /// Fin de la espera en segundos (epoch ms), para la cuenta atrás.
    pub wait_until_ms: Option<i64>,
    /// Código del error (`not_found`, `uac_cancelled`, `failed:…`).
    pub error: Option<String>,
    /// (03/11) Resultado del paso de addons.
    pub addons: Option<AddonsDone>,
}

/// (03/11) Lo que cambió el paso de addons.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AddonsDone {
    pub on: usize,
    pub off: usize,
    pub failed: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchRun {
    pub run_id: u64,
    pub sequence_id: i64,
    pub name: String,
    /// «Probar todo»: sin simulador ni esperas, y nada se cierra después.
    pub test_all: bool,
    /// `running` | `done` | `cancelled`.
    pub state: String,
    pub current: Option<usize>,
    pub steps: Vec<LaunchRunStep>,
    /// Apps que se cerrarán al salir del simulador.
    pub closing: Vec<String>,
    /// (03/11) Se cambiaron addons con el simulador abierto.
    pub restart_sim: bool,
}

static RUN_SEQ: AtomicU64 = AtomicU64::new(0);
static CANCEL: AtomicU64 = AtomicU64::new(0);
static SKIP_WAIT: AtomicBool = AtomicBool::new(false);

fn current() -> &'static Mutex<Option<LaunchRun>> {
    static R: OnceLock<Mutex<Option<LaunchRun>>> = OnceLock::new();
    R.get_or_init(|| Mutex::new(None))
}

pub fn status() -> Option<LaunchRun> {
    current().lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Cambia la secuencia en marcha (si hay) y avisa al front.
fn update(app: &AppHandle, f: impl FnOnce(&mut LaunchRun)) {
    let snapshot = {
        let mut cur = current().lock().unwrap_or_else(|e| e.into_inner());
        let Some(run) = cur.as_mut() else { return };
        f(run);
        run.clone()
    };
    let _ = app.emit("launch://progress", &snapshot);
}

fn set_step(app: &AppHandle, run_id: u64, i: usize, f: impl FnOnce(&mut LaunchRunStep)) {
    update(app, |r| {
        if r.run_id == run_id {
            if let Some(s) = r.steps.get_mut(i) {
                f(s);
            }
        }
    });
}

pub fn cancel() {
    if let Some(r) = status().filter(|r| r.state == "running") {
        CANCEL.store(r.run_id, Ordering::SeqCst);
    }
}

pub fn skip_wait() {
    SKIP_WAIT.store(true, Ordering::SeqCst);
}

/// Arranca una secuencia (o «Probar todo»).
pub async fn run(
    app: AppHandle,
    pool: &SqlitePool,
    sequence_id: i64,
    test_all: bool,
    changes: Option<DispatchChanges>,
) -> Result<LaunchRun> {
    if status().is_some_and(|r| r.state == "running") {
        bail!("busy");
    }
    let seq = get(pool, sequence_id).await?;
    if seq.steps.is_empty() {
        bail!("empty");
    }
    // Antes de tocar nada: un script sin permiso para el arranque aquí.
    if !scripts_pending(pool, &seq.steps).await.is_empty() {
        bail!("script_permission");
    }
    prune_closing().await;
    let run_id = RUN_SEQ.fetch_add(1, Ordering::SeqCst) + 1;
    let sim_label = sim_target().map(|(l, _)| l);
    // (03/11) Etapa 7: los addons van antes que nada (el simulador sólo los
    // lee al arrancar). «Probar todo» nunca los toca.
    let changes = changes.filter(|c| !test_all && !c.is_empty());
    let addons_step = changes.as_ref().map(|_| LaunchRunStep {
        id: "addons".into(),
        name: crate::tr!("launch.addonsStep"),
        kind: "addons".into(),
        status: "pending".into(),
        wait: "seconds".into(),
        wait_until_ms: None,
        error: None,
        addons: None,
    });
    let run = LaunchRun {
        run_id,
        sequence_id,
        name: seq.name.clone(),
        test_all,
        state: "running".into(),
        current: None,
        steps: addons_step
            .into_iter()
            .chain(seq.steps.iter().map(|s| LaunchRunStep {
                id: s.id.clone(),
                name: sim_label.clone().filter(|_| s.kind == "sim").unwrap_or_else(|| s.name.clone()),
                kind: s.kind.clone(),
                status: "pending".into(),
                wait: s.wait.clone(),
                wait_until_ms: None,
                error: None,
                addons: None,
            }))
            .collect(),
        closing: closing_names(),
        restart_sim: false,
    };
    *current().lock().unwrap_or_else(|e| e.into_inner()) = Some(run.clone());
    let _ = app.emit("launch://progress", &run);
    tracing::info!(target: "launch", "secuencia «{}» ({} pasos){}", seq.name, seq.steps.len(), if test_all { " — probar todo" } else { "" });
    tauri::async_runtime::spawn(drive(app, pool.clone(), seq, run_id, test_all, changes));
    Ok(run)
}

fn cancelled(run_id: u64) -> bool {
    CANCEL.load(Ordering::SeqCst) == run_id
}

/// Espera antes del paso `i`. `false` si se canceló.
async fn wait_for(app: &AppHandle, run_id: u64, i: usize, step: &LaunchStep, test_all: bool) -> bool {
    SKIP_WAIT.store(false, Ordering::SeqCst);
    let (kind, secs) = if test_all {
        ("seconds", if i == 0 { 0 } else { 1 })
    } else {
        (step.wait.as_str(), step.wait_seconds)
    };
    let ready_now = match kind {
        "menu" => sim_at_menu(app).await,
        "loaded" => sim_loaded(app).await,
        _ => secs == 0,
    };
    if ready_now {
        return !cancelled(run_id);
    }
    let until = (kind == "seconds").then(|| chrono::Utc::now().timestamp_millis() + i64::from(secs) * 1000);
    set_step(app, run_id, i, |s| {
        s.status = "waiting".into();
        s.wait = kind.to_string();
        s.wait_until_ms = until;
    });
    let started = Instant::now();
    let max_wait = match kind {
        "menu" => Duration::from_secs(15 * 60),
        "loaded" => Duration::from_secs(20 * 60),
        _ => Duration::from_secs(u64::from(secs) + 5),
    };
    loop {
        if cancelled(run_id) {
            return false;
        }
        if SKIP_WAIT.swap(false, Ordering::SeqCst) {
            return true;
        }
        if started.elapsed() > max_wait {
            tracing::warn!(target: "launch", "wait_for({kind}) expiró tras {:.0}s", started.elapsed().as_secs_f64());
            return false;
        }
        let ready = match kind {
            "menu" => sim_at_menu(app).await,
            "loaded" => sim_loaded(app).await,
            _ => started.elapsed() >= Duration::from_secs(u64::from(secs)),
        };
        if ready {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(if kind == "seconds" { 250 } else { 1000 })).await;
    }
}

/// (03/11) Paso de addons: aplica lo confirmado y sigue pase lo que pase (lo
/// que falle se ve en la tarjeta).
async fn apply_addons(app: &AppHandle, pool: &SqlitePool, run_id: u64, changes: DispatchChanges) {
    update(app, |r| r.current = Some(0));
    set_step(app, run_id, 0, |s| s.status = "opening".into());
    let sim_open = sim_running_now().await;
    match crate::dispatch::apply_and_notify(app, pool, changes).await {
        Ok(rep) => {
            let done = AddonsDone { on: rep.enabled.len(), off: rep.disabled.len(), failed: rep.failed.len() };
            tracing::info!(target: "launch", "addons: {} encendidos, {} apagados, {} fallaron", done.on, done.off, done.failed);
            let changed = done.on + done.off > 0;
            set_step(app, run_id, 0, |s| {
                s.status = if done.failed > 0 { "failed" } else { "opened" }.into();
                s.error = (done.failed > 0).then(|| "addons".to_string());
                s.addons = Some(done);
            });
            if sim_open && changed {
                update(app, |r| r.restart_sim = true);
            }
        }
        Err(e) => {
            tracing::warn!(target: "launch", "addons: {e:#}");
            set_step(app, run_id, 0, |s| {
                s.status = "failed".into();
                s.error = Some(format!("failed:{e}"));
            });
        }
    }
}

async fn drive(
    app: AppHandle,
    pool: SqlitePool,
    seq: LaunchSequence,
    run_id: u64,
    test_all: bool,
    changes: Option<DispatchChanges>,
) {
    // El paso de addons es el primero de la tarjeta.
    let first = usize::from(changes.is_some());
    if let Some(changes) = changes {
        apply_addons(&app, &pool, run_id, changes).await;
    }
    let mut stopped = false;
    for (n, step) in seq.steps.iter().enumerate() {
        let i = n + first;
        update(&app, |r| r.current = Some(i));
        if !wait_for(&app, run_id, i, step, test_all).await {
            break;
        }
        // Lo que ya está abierto no se vuelve a abrir (y nunca se cierra).
        let already = match step.kind.as_str() {
            "sim" => sim_running_now().await,
            "app" => {
                let names = watch_names(step);
                let procs = tokio::task::spawn_blocking(processes).await.unwrap_or_default();
                procs.iter().any(|p| names.contains(&p.name))
            }
            _ => false,
        };
        if already {
            set_step(&app, run_id, i, |s| s.status = "already".into());
            continue;
        }
        if test_all && step.kind == "sim" {
            set_step(&app, run_id, i, |s| s.status = "skipped".into());
            continue;
        }
        set_step(&app, run_id, i, |s| {
            s.status = "opening".into();
            s.wait_until_ms = None;
        });
        // (07/11) Etapa F: un script se espera y, si falla, para el arranque.
        if step.kind == "script" {
            // El archivo pudo cambiar mientras esperaba su turno.
            let outcome = if scripts_pending(&pool, std::slice::from_ref(step)).await.is_empty() {
                run_script(step, SCRIPT_LIMIT, || cancelled(run_id)).await
            } else {
                Err(anyhow::anyhow!("script_permission"))
            };
            match outcome {
                Ok(()) => {
                    tracing::info!(target: "launch", "script «{}» terminado", step.name);
                    set_step(&app, run_id, i, |s| s.status = "opened".into());
                    continue;
                }
                Err(_) if cancelled(run_id) => break,
                Err(e) => {
                    tracing::warn!(target: "launch", "script «{}»: {e:#}", step.name);
                    set_step(&app, run_id, i, |s| {
                        s.status = "failed".into();
                        s.error = Some(e.to_string());
                    });
                    stopped = true;
                    break;
                }
            }
        }
        let since = now_secs();
        let st = step.clone();
        let opened = tokio::task::spawn_blocking(move || open_step(&st)).await;
        match opened {
            Ok(Ok(pid)) => {
                tracing::info!(target: "launch", "abierto «{}» (pid {:?})", step.name, pid);
                set_step(&app, run_id, i, |s| s.status = "opened".into());
                if !test_all && step.kind == "app" && step.close == "exit" {
                    tauri::async_runtime::spawn(track(app.clone(), step.clone(), pid, since));
                }
            }
            Ok(Err(e)) => {
                tracing::warn!(target: "launch", "no se pudo abrir «{}»: {e:#}", step.name);
                set_step(&app, run_id, i, |s| {
                    s.status = "failed".into();
                    s.error = Some(e.to_string());
                });
            }
            Err(e) => set_step(&app, run_id, i, |s| {
                s.status = "failed".into();
                s.error = Some(format!("failed:{e}"));
            }),
        }
    }
    let was_cancelled = cancelled(run_id);
    update(&app, |r| {
        if r.run_id != run_id {
            return;
        }
        r.current = None;
        r.state = if was_cancelled {
            "cancelled"
        } else if stopped {
            "stopped"
        } else {
            "done"
        }
        .into();
        for s in &mut r.steps {
            if matches!(s.status.as_str(), "pending" | "waiting") {
                s.status = "cancelled".into();
                s.wait_until_ms = None;
            }
        }
        r.closing = closing_names();
    });
    tracing::info!(target: "launch", "secuencia «{}» {}", seq.name, if was_cancelled { "cancelada" } else { "terminada" });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app_step(target: &str) -> LaunchStep {
        LaunchStep {
            id: String::new(),
            kind: "app".into(),
            name: String::new(),
            target: target.into(),
            args: String::new(),
            process: Vec::new(),
            open: "admin".into(),
            wait: "loaded".into(),
            wait_seconds: 9999,
            close: "exit".into(),
        }
    }

    #[test]
    fn registry_lines_give_the_executable() {
        assert_eq!(first_path(r#""C:\a b\app.exe",0"#), Some(PathBuf::from(r"C:\a b\app.exe")));
        assert_eq!(first_path(r"C:\x\Navigraph Charts.exe,0"), Some(PathBuf::from(r"C:\x\Navigraph Charts.exe")));
        assert_eq!(
            first_path(r"C:\x\Update.exe --uninstall"),
            Some(PathBuf::from(r"C:\x\Update.exe"))
        );
        assert_eq!(first_path(r"B:\Sim\Simlink.ico"), Some(PathBuf::from(r"B:\Sim\Simlink.ico")));
    }

    #[test]
    fn names_with_symbols_do_not_break_detection() {
        assert!(starts_ci("vPilot 3.0", "VPILOT"));
        assert!(!starts_ci("NVIDIA™ Foo Bar", "NVIDIA Foo"));
        assert!(!starts_ci("a", "abc"));
    }

    #[test]
    fn the_editor_input_is_cleaned() {
        let mut web = app_step("https://www.simbrief.com/home/");
        web.kind = "web".into();
        let seq = LaunchSequence {
            id: 0,
            name: "  Fenix  ".into(),
            aircraft: vec!["a20n".into(), "A20N".into(), "toolong".into(), " b738 ".into()],
            is_default: false,
            steps: vec![app_step(r"C:\x\app.exe"), web],
            missing: Vec::new(),
        };
        let c = clean(seq).unwrap();
        assert_eq!(c.name, "Fenix");
        assert_eq!(c.aircraft, vec!["A20N", "B738"]);
        let (a, w) = (&c.steps[0], &c.steps[1]);
        assert!(!a.id.is_empty() && a.id != w.id);
        assert_eq!((a.name.as_str(), a.open.as_str(), a.close.as_str(), a.wait_seconds), ("app", "admin", "exit", 600));
        // Las webs: siempre al navegador y se quedan; nombre = dominio.
        assert_eq!((w.name.as_str(), w.open.as_str(), w.close.as_str()), ("simbrief.com", "normal", "stay"));
    }

    #[test]
    fn bad_steps_are_refused() {
        let mk = |steps: Vec<LaunchStep>| LaunchSequence {
            id: 0,
            name: "x".into(),
            aircraft: vec![],
            is_default: false,
            steps,
            missing: Vec::new(),
        };
        assert!(clean(mk(vec![app_step(r"C:\x\doc.txt")])).is_err());
        let mut web = app_step("javascript:alert(1)");
        web.kind = "web".into();
        assert!(clean(mk(vec![web])).is_err());
        let sim = || LaunchStep { kind: "sim".into(), ..app_step("") };
        assert!(clean(mk(vec![sim(), sim()])).is_err());
        assert!(clean(LaunchSequence { name: "  ".into(), ..mk(vec![]) }).is_err());
    }

    #[test]
    fn examples_use_what_was_found() {
        let app = |key: &str, name: &str| DetectedApp {
            key: key.into(),
            name: name.into(),
            target: format!(r"C:\{name}.exe"),
            args: String::new(),
            process: Vec::new(),
            open: "normal".into(),
            wait: "seconds".into(),
            wait_seconds: 2,
            close: "exit".into(),
        };
        let apps = vec![
            app("fenix-techlog", "Fenix TechLog"),
            app("activesky", "Active Sky FS"),
            app("fs2crew:fs2crew fenix a320 for msfs", "FS2Crew Fenix A320 for MSFS"),
            app("fs2crew:fs2crew animated fo fenix a320 project", "FS2Crew Animated FO Fenix A320 Project"),
        ];
        let ex = examples(&apps, true);
        let names: Vec<&str> = ex.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names.len(), 3, "{names:?}");
        let fenix: Vec<&str> = ex[0].steps.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(fenix, vec!["Fenix TechLog", "Active Sky FS", "MSFS", "FS2Crew Fenix A320 for MSFS"]);
        assert_eq!(ex[0].steps[0].wait_seconds, 0);
        // Sin PMDG TechLog: la de PMDG empieza por Active Sky.
        assert_eq!(ex[1].steps[0].name, "Active Sky FS");
        assert!(ex[2].is_default && ex[2].aircraft.is_empty());
        // Sin nada encontrado y sin simulador, no hay ejemplos.
        assert!(examples(&[], false).is_empty());
    }

    /// (06/11) TechLog 1.2 sustituye a las dos apps, en las secuencias nuevas
    /// y en las guardadas.
    #[test]
    fn the_unified_techlog_replaces_the_old_apps() {
        let app = |key: &str, name: &str| DetectedApp {
            key: key.into(),
            name: name.into(),
            target: format!(r"C:\{name}.exe"),
            args: String::new(),
            process: Vec::new(),
            open: "minimized".into(),
            wait: "seconds".into(),
            wait_seconds: 2,
            close: "stay".into(),
        };
        let mut apps = vec![app("fenix-techlog", "Fenix TechLog"), app("pmdg-techlog", "PMDG TechLog")];
        drop_replaced_techlogs(&mut apps);
        assert_eq!(apps.len(), 2, "sin el nuevo se quedan las antiguas");
        apps.push(app("techlog", "TechLog"));
        apps.push(app("activesky", "Active Sky FS"));
        drop_replaced_techlogs(&mut apps);
        let keys: Vec<&str> = apps.iter().map(|a| a.key.as_str()).collect();
        assert_eq!(keys, vec!["techlog", "activesky"]);
        let ex = examples(&apps, true);
        assert_eq!(ex[0].steps[0].name, "TechLog");
        assert_eq!(ex[1].steps[0].name, "TechLog");

        let new = r"C:\Users\x\AppData\Local\Programs\TechLog\app\TechLog.exe";
        let mut fenix = app_step(r"C:\Users\x\AppData\Local\Programs\Fenix TechLog\app\TechLog.exe");
        fenix.name = "Fenix TechLog".into();
        fenix.open = "minimized".into();
        let mut pmdg = app_step(r"C:\Users\x\AppData\Local\Programs\PMDG TechLog\app\PMDGTechLog.exe");
        pmdg.name = "Mi TechLog".into();
        let other = app_step(r"C:\Programas\Otra\app\TechLog.exe");
        let mut steps = vec![fenix, pmdg, other];
        assert!(swap_techlog(&mut steps, new));
        assert_eq!(steps[0].target, new);
        assert_eq!((steps[0].name.as_str(), steps[0].open.as_str()), ("TechLog", "minimized"));
        assert_eq!((steps[1].target.as_str(), steps[1].name.as_str()), (new, "Mi TechLog"));
        assert_eq!(steps[2].target, r"C:\Programas\Otra\app\TechLog.exe");
        assert!(!swap_techlog(&mut steps, new), "una segunda vez no cambia nada");
    }

    fn script_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("simfleet-script-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn script_step(path: &Path) -> LaunchStep {
        LaunchStep { kind: "script".into(), name: "Previo".into(), ..app_step(&path.to_string_lossy()) }
    }

    #[test]
    fn only_batch_and_powershell_files_are_scripts() {
        assert!(is_script(r"C:\x\antes.bat") && is_script(r"C:\x\antes.CMD") && is_script(r"C:\x\antes.ps1"));
        assert!(!is_script(r"C:\x\antes.exe") && !is_script(r"C:\x\antes"));
        let seq = |target: &str| LaunchSequence {
            id: 0,
            name: "x".into(),
            aircraft: vec![],
            is_default: false,
            steps: vec![LaunchStep { args: "-Rapido".into(), ..script_step(Path::new(target)) }],
            missing: Vec::new(),
        };
        let c = clean(seq(r"C:\x\antes.ps1")).unwrap();
        assert_eq!(c.steps[0].args, "-Rapido", "un script conserva sus argumentos");
        assert_eq!((c.steps[0].open.as_str(), c.steps[0].close.as_str()), ("normal", "stay"));
        assert_eq!(clean(seq(r"C:\x\antes.exe")).unwrap_err().to_string(), "bad_script");
    }

    /// Permiso por contenido: la misma ruta con otro contenido vuelve a pedirlo.
    #[tokio::test]
    async fn a_changed_script_asks_again() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
        sqlx::query("CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT)").execute(&pool).await.unwrap();
        let dir = script_dir("permiso");
        let path = dir.join("antes.bat");
        std::fs::write(&path, "@echo hola\r\n").unwrap();
        let steps = vec![script_step(&path), script_step(&path)];
        let pending = scripts_pending(&pool, &steps).await;
        assert_eq!(pending.len(), 1, "el mismo archivo se pregunta una vez");
        assert!(pending[0].preview.contains("@echo hola"));
        approve_scripts(&pool, vec![pending[0].hash.clone(), "no-es-una-huella".into()]).await.unwrap();
        assert!(scripts_pending(&pool, &steps).await.is_empty());
        std::fs::write(&path, "@echo otra cosa\r\n").unwrap();
        assert_eq!(scripts_pending(&pool, &steps).await.len(), 1);
        assert!(scripts_pending(&pool, &[script_step(&dir.join("no-existe.bat"))]).await.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn a_script_runs_hidden_and_reports_how_it_ended() {
        let dir = script_dir("corre");
        let ok = dir.join("con espacios ok.bat");
        std::fs::write(&ok, "@echo off\r\nif \"%~1\"==\"hola mundo\" exit /b 0\r\nexit /b 9\r\n").unwrap();
        let mut step = script_step(&ok);
        step.args = "\"hola mundo\"".into();
        run_script(&step, Duration::from_secs(20), || false).await.unwrap();

        let bad = dir.join("mal.bat");
        std::fs::write(&bad, "@echo off\r\necho falta el archivo 1>&2\r\nexit /b 3\r\n").unwrap();
        let e = run_script(&script_step(&bad), Duration::from_secs(20), || false).await.unwrap_err();
        assert_eq!(e.to_string(), "script_failed:3:falta el archivo");

        let ps = dir.join("prueba.ps1");
        std::fs::write(&ps, "param($Nombre)\r\nif ($Nombre -eq 'Fenix') { exit 0 } else { exit 4 }\r\n").unwrap();
        let mut step = script_step(&ps);
        step.args = "-Nombre Fenix".into();
        run_script(&step, Duration::from_secs(30), || false).await.unwrap();
        step.args = "-Nombre PMDG".into();
        let e = run_script(&step, Duration::from_secs(30), || false).await.unwrap_err();
        assert!(e.to_string().starts_with("script_failed:4:"), "{e}");

        let slow = dir.join("lento.bat");
        std::fs::write(&slow, "@ping -n 30 127.0.0.1 >nul\r\n").unwrap();
        let t0 = Instant::now();
        let e = run_script(&script_step(&slow), Duration::from_millis(600), || false).await.unwrap_err();
        assert_eq!(e.to_string(), "script_timeout");
        assert!(t0.elapsed() < Duration::from_secs(10), "no espera al script");
        let e = run_script(&script_step(&slow), Duration::from_secs(20), || true).await.unwrap_err();
        assert_eq!(e.to_string(), "cancelled");
        let e = run_script(&script_step(&dir.join("no-esta.bat")), Duration::from_secs(5), || false).await.unwrap_err();
        assert_eq!(e.to_string(), "not_found");
        std::thread::sleep(Duration::from_millis(300));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn launchers_are_watched_by_their_real_process() {
        let mut s = app_step(r"C:\Discord\Update.exe");
        assert_eq!(watch_names(&s), vec!["update.exe"]);
        s.process = vec!["Discord.exe".into()];
        assert_eq!(watch_names(&s), vec!["discord.exe"]);
    }

    fn owned(step: &str, pid: u32) -> Owned {
        Owned { step: step.into(), pid, start: 1, admin: false }
    }

    #[test]
    fn only_what_really_closed_counts_as_closed() {
        let targets = [owned("PMDG TechLog", 1), owned("Active Sky FS", 2)];
        let report = close_report(&targets, &[owned("Active Sky FS", 2)]);
        assert_eq!(report.closed, ["PMDG TechLog"]);
        assert_eq!(report.open, ["Active Sky FS"]);
    }

    #[test]
    fn a_step_with_any_process_alive_stays_open() {
        let targets = [owned("Active Sky FS", 2), owned("Active Sky FS", 3), owned("Simlink", 4)];
        let report = close_report(&targets, &[owned("Active Sky FS", 3)]);
        assert_eq!(report.closed, ["Simlink"], "ni repetido ni medio cerrado");
        assert_eq!(report.open, ["Active Sky FS"]);
    }

    #[test]
    fn with_nothing_left_everything_closed() {
        let targets = [owned("Simlink", 4), owned("Simlink", 5)];
        let report = close_report(&targets, &[]);
        assert_eq!(report.closed, ["Simlink"]);
        assert!(report.open.is_empty());
    }

    #[test]
    fn descendants_follow_the_parent_chain() {
        let p = |pid, parent| Proc { pid, parent, name: String::new(), start: 1 };
        let all = [p(10, Some(1)), p(20, Some(10)), p(30, Some(20)), p(40, Some(99))];
        let by: HashMap<u32, &Proc> = all.iter().map(|x| (x.pid, x)).collect();
        assert!(descends(&all[2], 10, &by));
        assert!(!descends(&all[3], 10, &by));
        // El lanzador ya terminó: su PID sigue como padre del hijo.
        let orphan = p(50, Some(77));
        assert!(descends(&orphan, 77, &by));
    }
}
