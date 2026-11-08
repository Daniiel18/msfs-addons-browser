//! (07/11) Etapa F — editor de `Content.xml`: apagar contenido de fábrica de MSFS 2024.
//!
//! `%APPDATA%\Microsoft Flight Simulator 2024\<usuario>\Content.xml` dice qué
//! paquetes carga el simulador: `<Package name="…" active="Activated"/>`.
//!
//! Es delicado, así que:
//!   · Sólo se cambia entre `Activated` y `UserDisabled`. Lo que puso el sistema
//!     (`SystemDisabled`), los paquetes base y las entradas de Community no se tocan.
//!   · Se escribe únicamente el valor de `active` de lo que cambió: el resto del
//!     archivo queda byte a byte como estaba.
//!   · Con MSFS abierto no se escribe, porque puede reescribirlo al salir.
//!   · Antes de escribir se guarda una copia con fecha (las últimas 10) y se
//!     comprueba que el archivo sigue siendo el que viste en pantalla.
//!   · MSFS puede regenerar el archivo: SimFleet recuerda lo que ajustaste, avisa
//!     si cambió y ofrece volver a aplicarlo. Nunca lo hace solo.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{anyhow, bail, Context, Result};
use chrono::{DateTime, Local, Utc};
use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;

const ACTIVE: &str = "Activated";
const USER_OFF: &str = "UserDisabled";
const FILE_NAME: &str = "Content.xml";
const MAX_BACKUPS: usize = 10;

static PACKAGE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"<Package\s+name="([^"]*)"\s+active="([^"]*)"\s*/>"#).expect("regex"));

/// Una entrada del archivo y dónde está su valor de `active`.
#[derive(Debug, Clone, PartialEq)]
struct Entry {
    name: String,
    state: String,
    value: std::ops::Range<usize>,
}

/// Las entradas, o error si el archivo tiene algo que no reconocemos: mejor no
/// tocar un formato nuevo que reescribirlo mal.
fn parse(xml: &str) -> Result<Vec<Entry>> {
    let entries: Vec<Entry> = PACKAGE
        .captures_iter(xml)
        .map(|c| Entry {
            name: c[1].to_string(),
            state: c[2].to_string(),
            value: c.get(2).expect("grupo 2").range(),
        })
        .collect();
    let tags = xml.matches("<Package").count() - xml.matches("<Packages").count();
    if entries.is_empty() || entries.len() != tags {
        bail!("unknown_format");
    }
    Ok(entries)
}

/// Por qué no se puede cambiar un paquete. `None`: se puede.
fn lock_of(name: &str, state: &str) -> Option<&'static str> {
    if name.starts_with("community") {
        return Some("community");
    }
    if state != ACTIVE && state != USER_OFF {
        return Some("system");
    }
    let mut parts = name.split('-');
    if parts.nth(2) == Some("base") {
        return Some("base");
    }
    None
}

/// Tipo de un paquete de fábrica, por el tercer tramo de su nombre
/// (`fs24-asobo-aircraft-…`). Sólo sirve para filtrar la lista.
fn kind_of(name: &str) -> &'static str {
    if name.contains("character") {
        return "character";
    }
    match name.split('-').nth(2).unwrap_or_default() {
        "aircraft" | "vliveries" => "aircraft",
        "airport" => "airport",
        "passiveaircraft" | "ships" => "traffic",
        "discovery" | "discoverylegacy" | "travelbook" | "guidedtour" | "spotlight" => "discovery",
        "landingchallenge" | "challenges" | "challenge" | "airrace" | "event" | "liveevent" => "challenge",
        "trainings" | "certification" | "certifications" => "training",
        "bushtrip" => "bush",
        "activities" | "career" | "flight" | "freeflight" | "freeflights" | "mission" | "scenarii" => "activity",
        "scenery" | "countryside" | "germany" | "italy" | "uk" | "australia" | "baltic" | "canada" | "caribbean" => {
            "scenery"
        }
        "simobjects" | "instruments" | "vcockpits" | "modellib" | "simattachmentlib" | "texturelib" | "textlib"
        | "palettelib" | "material" | "simpropcontainer" | "surface" | "services" | "environment" | "pgg"
        | "proceduralgeneration" => "library",
        _ => "other",
    }
}

fn state_of(active: bool) -> &'static str {
    if active {
        ACTIVE
    } else {
        USER_OFF
    }
}

fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentChange {
    pub name: String,
    pub active: bool,
}

/// Qué hay que escribir: comprueba cada cambio y quita los que no cambian nada.
fn plan(entries: &[Entry], changes: &[ContentChange]) -> Result<HashMap<String, &'static str>> {
    let by_name: HashMap<&str, &Entry> = entries.iter().map(|e| (e.name.as_str(), e)).collect();
    let mut out = HashMap::new();
    for c in changes {
        let entry = by_name.get(c.name.as_str()).ok_or_else(|| anyhow!("not_found"))?;
        if lock_of(&entry.name, &entry.state).is_some() {
            bail!("locked");
        }
        let target = state_of(c.active);
        if entry.state != target {
            out.insert(c.name.clone(), target);
        }
    }
    Ok(out)
}

/// El archivo con esos cambios. Todo lo demás queda tal cual.
fn edited(xml: &str, entries: &[Entry], plan: &HashMap<String, &'static str>) -> String {
    let mut out = String::with_capacity(xml.len() + plan.len() * 4);
    let mut at = 0;
    for e in entries {
        if let Some(state) = plan.get(&e.name) {
            out.push_str(&xml[at..e.value.start]);
            out.push_str(state);
            at = e.value.end;
        }
    }
    out.push_str(&xml[at..]);
    out
}

/// Se escribe aparte y se cambia de golpe: si algo falla, el original sigue entero.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension("xml.simfleet-tmp");
    std::fs::write(&tmp, bytes).with_context(|| format!("escribiendo {}", tmp.display()))?;
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e).with_context(|| format!("reemplazando {}", path.display()));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Copias de seguridad
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentBackup {
    /// Nombre del archivo: identifica la copia al restaurar.
    pub file: String,
    pub saved_at: Option<String>,
    /// Paquetes apagados por el usuario en esa copia.
    pub disabled: usize,
}

fn backups_dir(data_dir: &Path, user: &str) -> PathBuf {
    data_dir.join("content-backups").join(user)
}

/// Guarda el archivo actual con fecha y deja las últimas `MAX_BACKUPS`.
fn save_backup(data_dir: &Path, user: &str, bytes: &[u8]) -> Result<PathBuf> {
    let dir = backups_dir(data_dir, user);
    std::fs::create_dir_all(&dir)?;
    let stamp = Local::now().format("%Y%m%d-%H%M%S");
    let free = (0..100).map(|n| match n {
        0 => dir.join(format!("Content-{stamp}.xml")),
        n => dir.join(format!("Content-{stamp}-{n:02}.xml")),
    });
    let path = free.into_iter().find(|p| !p.exists()).ok_or_else(|| anyhow!("no_backup"))?;
    std::fs::write(&path, bytes).with_context(|| format!("copiando a {}", path.display()))?;
    let mut names = backup_files(&dir);
    names.sort();
    let extra = names.len().saturating_sub(MAX_BACKUPS);
    for old in &names[..extra] {
        let _ = std::fs::remove_file(dir.join(old));
    }
    Ok(path)
}

fn backup_files(dir: &Path) -> Vec<String> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    rd.flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with("Content-") && n.ends_with(".xml"))
        .collect()
}

fn list_backups(data_dir: &Path, user: &str) -> Vec<ContentBackup> {
    let dir = backups_dir(data_dir, user);
    let mut names = backup_files(&dir);
    names.sort();
    names
        .into_iter()
        .rev()
        .map(|file| {
            let path = dir.join(&file);
            let saved_at = std::fs::metadata(&path)
                .and_then(|m| m.modified())
                .ok()
                .map(|t| DateTime::<Utc>::from(t).to_rfc3339());
            let disabled = std::fs::read_to_string(&path)
                .ok()
                .and_then(|xml| parse(&xml).ok())
                .map_or(0, |e| e.iter().filter(|x| x.state == USER_OFF).count());
            ContentBackup { file, saved_at, disabled }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Lo que el usuario ajustó (para avisar si MSFS lo deshace)
// ---------------------------------------------------------------------------

fn wanted_key(user: &str) -> String {
    format!("content_wanted:{user}")
}

async fn load_wanted(pool: &SqlitePool, user: &str) -> BTreeMap<String, bool> {
    let raw: Option<String> = sqlx::query_scalar("SELECT value FROM settings WHERE key = ?1")
        .bind(wanted_key(user))
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();
    raw.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

async fn save_wanted(pool: &SqlitePool, user: &str, wanted: &BTreeMap<String, bool>) -> Result<()> {
    sqlx::query("INSERT OR REPLACE INTO settings (key, value) VALUES (?1, ?2)")
        .bind(wanted_key(user))
        .bind(serde_json::to_string(wanted)?)
        .execute(pool)
        .await?;
    Ok(())
}

/// Lo recordado que el archivo ya no cumple y se puede volver a aplicar.
fn drift_of(entries: &[Entry], wanted: &BTreeMap<String, bool>) -> Vec<ContentChange> {
    entries
        .iter()
        .filter(|e| lock_of(&e.name, &e.state).is_none())
        .filter_map(|e| {
            let active = *wanted.get(&e.name)?;
            (e.state != state_of(active)).then(|| ContentChange { name: e.name.clone(), active })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Dónde está el archivo
// ---------------------------------------------------------------------------

/// `(usuario, ruta)`, el más reciente primero: un archivo por usuario de MSFS.
fn files_in(root: &Path) -> Vec<(String, PathBuf)> {
    let Ok(rd) = std::fs::read_dir(root) else { return Vec::new() };
    let mut found: Vec<(SystemTime, String, PathBuf)> = rd
        .flatten()
        .filter_map(|e| {
            let path = e.path().join(FILE_NAME);
            let modified = std::fs::metadata(&path).and_then(|m| m.modified()).ok()?;
            Some((modified, e.file_name().to_string_lossy().into_owned(), path))
        })
        .collect();
    found.sort_by_key(|f| std::cmp::Reverse(f.0));
    found.into_iter().map(|(_, user, path)| (user, path)).collect()
}

/// Solo MSFS 2024 guarda así su contenido.
fn files() -> Vec<(String, PathBuf)> {
    if let Some(root) = crate::test_env::content_root() {
        return files_in(&root);
    }
    if !crate::sim::is_2024() {
        return Vec::new();
    }
    let root = crate::community::active_user_cfg().and_then(|cfg| cfg.parent().map(Path::to_path_buf));
    root.map(|r| files_in(&r)).unwrap_or_default()
}

fn pick(user: Option<&str>) -> Option<(String, PathBuf)> {
    let all = files();
    match user {
        Some(u) => all.into_iter().find(|(name, _)| name == u),
        None => all.into_iter().next(),
    }
}

/// Lo que impide escribir: MSFS abierto, o pruebas sin carpeta falsa.
async fn write_guard() -> Result<()> {
    if crate::launch_flow::sim_running_now().await {
        bail!("sim_running");
    }
    if crate::test_env::is_active() && crate::test_env::content_root().is_none() && !crate::test_env::content_real() {
        bail!("test_only");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Lo que usa la app
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentPackage {
    pub name: String,
    pub active: bool,
    /// `base` | `system`. `None`: se puede cambiar.
    pub locked: Option<&'static str>,
    pub kind: &'static str,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentReport {
    pub users: Vec<String>,
    pub user: Option<String>,
    /// Huella del archivo leído: se comprueba al aplicar.
    pub stamp: Option<String>,
    pub sim_running: bool,
    /// El archivo no tiene el formato conocido: se muestra pero no se toca.
    pub unknown_format: bool,
    /// De fábrica; las entradas de Community ni se listan.
    pub packages: Vec<ContentPackage>,
    pub community: usize,
    pub drift: Vec<ContentChange>,
    pub backups: Vec<ContentBackup>,
}

async fn report_file(pool: &SqlitePool, data_dir: &Path, user: String, path: &Path) -> Result<ContentReport> {
    let bytes = std::fs::read(path).with_context(|| format!("leyendo {}", path.display()))?;
    let backups = list_backups(data_dir, &user);
    let mut report = ContentReport { stamp: Some(hash(&bytes)), backups, ..ContentReport::default() };
    let entries = std::str::from_utf8(&bytes).map_err(|_| anyhow!("unknown_format")).and_then(parse);
    let entries = match entries {
        Ok(e) => e,
        Err(_) => {
            report.unknown_format = true;
            report.user = Some(user);
            return Ok(report);
        }
    };
    report.drift = drift_of(&entries, &load_wanted(pool, &user).await);
    report.community = entries.iter().filter(|e| e.name.starts_with("community")).count();
    report.packages = entries
        .iter()
        .filter(|e| !e.name.starts_with("community"))
        .map(|e| ContentPackage {
            name: e.name.clone(),
            active: e.state == ACTIVE,
            locked: lock_of(&e.name, &e.state),
            kind: kind_of(&e.name),
        })
        .collect();
    report.user = Some(user);
    Ok(report)
}

pub async fn report(pool: &SqlitePool, data_dir: &Path, user: Option<String>) -> Result<ContentReport> {
    let users: Vec<String> = files().into_iter().map(|(name, _)| name).collect();
    let sim_running = crate::launch_flow::sim_running_now().await;
    let Some((user, path)) = pick(user.as_deref()).or_else(|| pick(None)) else {
        return Ok(ContentReport { sim_running, ..ContentReport::default() });
    };
    let mut report = report_file(pool, data_dir, user, &path).await?;
    report.users = users;
    report.sim_running = sim_running;
    Ok(report)
}

async fn apply_file(
    pool: &SqlitePool,
    data_dir: &Path,
    user: &str,
    path: &Path,
    stamp: &str,
    changes: &[ContentChange],
) -> Result<usize> {
    let bytes = std::fs::read(path).with_context(|| format!("leyendo {}", path.display()))?;
    if hash(&bytes) != stamp {
        bail!("changed");
    }
    let xml = std::str::from_utf8(&bytes).map_err(|_| anyhow!("unknown_format"))?;
    let entries = parse(xml)?;
    let plan = plan(&entries, changes)?;
    let mut wanted = load_wanted(pool, user).await;
    if !plan.is_empty() {
        let new_xml = edited(xml, &entries, &plan);
        save_backup(data_dir, user, &bytes)?;
        write_atomic(path, new_xml.as_bytes())?;
        if std::fs::read(path)? != new_xml.as_bytes() {
            bail!("write_check");
        }
    }
    for c in changes {
        wanted.insert(c.name.clone(), c.active);
    }
    save_wanted(pool, user, &wanted).await?;
    tracing::info!(target: "content", "Content.xml de {user}: {} paquetes cambiados", plan.len());
    Ok(plan.len())
}

pub async fn apply(
    pool: &SqlitePool,
    data_dir: &Path,
    user: String,
    stamp: String,
    changes: Vec<ContentChange>,
) -> Result<usize> {
    write_guard().await?;
    let (user, path) = pick(Some(&user)).ok_or_else(|| anyhow!("not_found"))?;
    apply_file(pool, data_dir, &user, &path, &stamp, &changes).await
}

async fn restore_file(pool: &SqlitePool, data_dir: &Path, user: &str, path: &Path, file: &str) -> Result<()> {
    if !backup_files(&backups_dir(data_dir, user)).iter().any(|n| n == file) {
        bail!("not_found");
    }
    let bytes = std::fs::read(backups_dir(data_dir, user).join(file))?;
    let xml = std::str::from_utf8(&bytes).map_err(|_| anyhow!("unknown_format"))?;
    let entries = parse(xml)?;
    save_backup(data_dir, user, &std::fs::read(path)?)?;
    write_atomic(path, &bytes)?;
    // Restaurar es una decisión: lo recordado que no coincida deja de contar.
    let mut wanted = load_wanted(pool, user).await;
    wanted.retain(|name, active| entries.iter().any(|e| e.name == *name && e.state == state_of(*active)));
    save_wanted(pool, user, &wanted).await?;
    tracing::info!(target: "content", "Content.xml de {user}: restaurada la copia {file}");
    Ok(())
}

pub async fn restore(pool: &SqlitePool, data_dir: &Path, user: String, file: String) -> Result<()> {
    write_guard().await?;
    let (user, path) = pick(Some(&user)).ok_or_else(|| anyhow!("not_found"))?;
    restore_file(pool, data_dir, &user, &path, &file).await
}

#[cfg(test)]
mod tests {
    use super::*;

    const XML: &str = "<Packages>\n\t<Package name=\"communityfs24-aerosoft-airport-eddb\" active=\"Activated\"/>\n\t<Package name=\"fs24-asobo-aircraft-b737max\" active=\"Activated\"/>\n\t<Package name=\"fs24-fs-base\" active=\"Activated\"/>\n\t<Package name=\"fs24-asobo-airport-biis-isafjordur\" active=\"UserDisabled\"/>\n\t<Package name=\"fs24-microsoft-spotlight-town\" active=\"SystemDisabled\"/>\n</Packages>\n\n";

    fn on(name: &str, active: bool) -> ContentChange {
        ContentChange { name: name.into(), active }
    }

    async fn memory() -> SqlitePool {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query("CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT)").execute(&pool).await.unwrap();
        pool
    }

    fn scratch(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("simfleet-content-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("n0x")).unwrap();
        std::fs::write(root.join("n0x").join(FILE_NAME), XML).unwrap();
        root
    }

    #[test]
    fn only_the_changed_values_are_rewritten() {
        let entries = parse(XML).unwrap();
        assert_eq!(entries.len(), 5);
        let p = plan(
            &entries,
            &[on("fs24-asobo-aircraft-b737max", false), on("fs24-asobo-airport-biis-isafjordur", true)],
        )
        .unwrap();
        let out = edited(XML, &entries, &p);
        let expected = XML
            .replace("b737max\" active=\"Activated\"", "b737max\" active=\"UserDisabled\"")
            .replace("isafjordur\" active=\"UserDisabled\"", "isafjordur\" active=\"Activated\"");
        assert_eq!(out, expected);
        assert_eq!(edited(XML, &entries, &HashMap::new()), XML);
    }

    #[test]
    fn what_is_protected_is_refused() {
        let entries = parse(XML).unwrap();
        for name in ["fs24-fs-base", "fs24-microsoft-spotlight-town", "communityfs24-aerosoft-airport-eddb"] {
            assert_eq!(plan(&entries, &[on(name, false)]).unwrap_err().to_string(), "locked", "{name}");
        }
        assert_eq!(plan(&entries, &[on("no-existe", false)]).unwrap_err().to_string(), "not_found");
        // Pedir lo que ya está no cambia nada.
        assert!(plan(&entries, &[on("fs24-asobo-aircraft-b737max", true)]).unwrap().is_empty());
        assert_eq!(lock_of("fs24-asobo-base-x", ACTIVE), Some("base"));
        assert_eq!(lock_of("fs24-asobo-x", "OtroEstado"), Some("system"));
        assert_eq!(lock_of("fs24-asobo-baseball-x", ACTIVE), None);
    }

    #[test]
    fn an_unknown_format_is_not_touched() {
        assert_eq!(parse("<Packages></Packages>").unwrap_err().to_string(), "unknown_format");
        let extra = XML.replace(
            "<Package name=\"fs24-fs-base\" active=\"Activated\"/>",
            "<Package active=\"Activated\" name=\"fs24-fs-base\"/>",
        );
        assert_eq!(parse(&extra).unwrap_err().to_string(), "unknown_format");
    }

    #[test]
    fn packages_get_a_type_from_their_name() {
        assert_eq!(kind_of("fs24-asobo-aircraft-b737max"), "aircraft");
        assert_eq!(kind_of("fs20-asobo-aircraft-b787-10-livery-klm"), "aircraft");
        assert_eq!(kind_of("fs24-asobo-airport-c53-lowerloon"), "airport");
        assert_eq!(kind_of("fs24-asobo-proceduralgeneration-character-pilot"), "character");
        assert_eq!(kind_of("fs24-asobo-modellib"), "library");
        assert_eq!(kind_of("fs24-asobo-bushtrip-alaska"), "bush");
        assert_eq!(kind_of("fs24-loquesea"), "other");
    }

    #[tokio::test]
    async fn applying_keeps_a_copy_and_remembers_it() {
        let root = scratch("apply");
        let path = root.join("n0x").join(FILE_NAME);
        let data = root.join("data");
        let pool = memory().await;
        let report = report_file(&pool, &data, "n0x".into(), &path).await.unwrap();
        assert_eq!(report.community, 1);
        assert_eq!(report.packages.len(), 4);
        assert_eq!(report.packages.iter().filter(|p| p.locked.is_some()).count(), 2);
        let stamp = report.stamp.unwrap();

        // Con el archivo cambiado desde que lo viste, no se escribe.
        let off = [on("fs24-asobo-aircraft-b737max", false)];
        let e = apply_file(&pool, &data, "n0x", &path, "otra", &off).await.unwrap_err();
        assert_eq!(e.to_string(), "changed");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), XML);

        assert_eq!(apply_file(&pool, &data, "n0x", &path, &stamp, &off).await.unwrap(), 1);
        let now = std::fs::read_to_string(&path).unwrap();
        assert!(now.contains("b737max\" active=\"UserDisabled\""));
        let backups = list_backups(&data, "n0x");
        assert_eq!(backups.len(), 1);
        assert_eq!(backups[0].disabled, 1, "la copia es el archivo de antes");
        assert!(!root.join("n0x").join("Content.xml.simfleet-tmp").exists());

        // MSFS lo regenera: se avisa y se puede reaplicar.
        std::fs::write(&path, XML).unwrap();
        let again = report_file(&pool, &data, "n0x".into(), &path).await.unwrap();
        assert_eq!(again.drift, off);
        let stamp = again.stamp.unwrap();
        assert_eq!(apply_file(&pool, &data, "n0x", &path, &stamp, &again.drift).await.unwrap(), 1);
        let settled = report_file(&pool, &data, "n0x".into(), &path).await.unwrap();
        assert!(settled.drift.is_empty());

        // Restaurar la copia más antigua deja el original y olvida lo recordado.
        let oldest = list_backups(&data, "n0x").last().unwrap().file.clone();
        restore_file(&pool, &data, "n0x", &path, &oldest).await.unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), XML);
        let restored = report_file(&pool, &data, "n0x".into(), &path).await.unwrap();
        assert!(restored.drift.is_empty(), "{:?}", restored.drift);
        let e = restore_file(&pool, &data, "n0x", &path, "..\\otro.xml").await.unwrap_err();
        assert_eq!(e.to_string(), "not_found");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn only_the_last_ten_copies_stay() {
        let root = scratch("copias");
        for _ in 0..13 {
            save_backup(&root, "n0x", XML.as_bytes()).unwrap();
        }
        assert_eq!(list_backups(&root, "n0x").len(), MAX_BACKUPS);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn each_user_has_their_file() {
        let root = scratch("usuarios");
        std::fs::create_dir_all(root.join("sin-archivo")).unwrap();
        let found = files_in(&root);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].0, "n0x");
        let _ = std::fs::remove_dir_all(&root);
    }
}
