//! (v6 #2b) Grabación nativa de "Best Landings".
//!
//! Porta la idea de LandingToast (capturar el aterrizaje) **dentro** de
//! SimFleet, sin lanzar la app externa. El motor de grabación es **ffmpeg**
//! (gdigrab en Windows) — el binario se resuelve de varias fuentes y, si no
//! está, se puede descargar a la carpeta de datos.
//!
//! Este módulo es autocontenido y NO toca el `simconnect_watcher` (que está
//! procesando el vuelo en vivo): expone una API para
//!   · leer/escribir la config de grabación (heredada de LandingToast),
//!   · resolver/descargar ffmpeg,
//!   · grabar un clip de prueba (testeable sin sim),
//!   · y gestionar la librería de clips (favoritos, borrado, retención).
//!
//! El disparo automático en el touchdown y el OSD en vivo se cablearán
//! después (necesitan un vuelo real para validarse).

use serde::{Deserialize, Serialize};
use std::io::Cursor;
use std::path::{Path, PathBuf};

use sqlx::SqlitePool;

/// Nombre del manifiesto de clips dentro de la carpeta de salida.
const MANIFEST: &str = "simfleet_landings.json";
/// Build estático de ffmpeg (essentials) para la descarga on-demand.
const FFMPEG_ZIP_URL: &str = "https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip";

// ─────────────────────────────────────────────────────────────────────────
// Config
// ─────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingConfig {
    pub enabled: bool,
    /// Posición del OSD: 0 = Arriba · 1 = Abajo (igual que LandingToast).
    pub osd_position: i64,
    pub output_path: String,
    pub clip_seconds: i64,
    /// Si true, graba todo el aterrizaje sin recortar a `clip_seconds`.
    pub unlimited: bool,
    /// Índice del monitor a grabar (Target). 0 = primario.
    pub monitor_index: i64,
    /// 0 = pantalla (monitor) · 1 = ventana de MSFS.
    pub source_type: i64,
    pub max_clips: i64,
    pub ffmpeg_path: Option<String>,
    /// Dispositivo de audio dshow: None/"" = auto-detectar loopback;
    /// "off" = sin audio; cualquier otro = nombre del dispositivo.
    pub audio_device: Option<String>,
}

async fn kv(pool: &SqlitePool, key: &str) -> Option<String> {
    sqlx::query_as::<_, (String,)>("SELECT value FROM settings WHERE key = ?1")
        .bind(key)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
        .map(|(v,)| v)
        .filter(|s| !s.trim().is_empty())
}

async fn kv_i64(pool: &SqlitePool, key: &str, default: i64) -> i64 {
    kv(pool, key).await.and_then(|s| s.parse().ok()).unwrap_or(default)
}

/// Carpeta de salida por defecto: la de LandingToast si existe su config, si
/// no `<Videos>\SimFleet Landings`, si no la carpeta de datos.
fn default_output_path(fallback_data: &Path) -> String {
    if let Some(p) = landingtoast_video_path() {
        return p;
    }
    if let Ok(profile) = std::env::var("USERPROFILE") {
        return PathBuf::from(profile)
            .join("Videos")
            .join("SimFleet Landings")
            .to_string_lossy()
            .into_owned();
    }
    fallback_data
        .join("landings")
        .to_string_lossy()
        .into_owned()
}

/// Lee el `config.json` de LandingToast si está instalado.
fn landingtoast_config() -> Option<serde_json::Value> {
    let appdata = std::env::var("APPDATA").ok()?;
    let cfg = PathBuf::from(appdata).join("LandingToast").join("config.json");
    let text = std::fs::read_to_string(cfg).ok()?;
    serde_json::from_str(&text).ok()
}

fn landingtoast_video_path() -> Option<String> {
    landingtoast_config()?
        .get("VideoOutputPath")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .filter(|s| !s.trim().is_empty())
}

/// Default tomado de LandingToast (clave `key`) o `fallback` si no existe.
fn lt_i64(lt: &Option<serde_json::Value>, key: &str, fallback: i64) -> i64 {
    lt.as_ref()
        .and_then(|j| j.get(key))
        .and_then(|v| v.as_i64())
        .unwrap_or(fallback)
}

fn lt_bool(lt: &Option<serde_json::Value>, key: &str, fallback: bool) -> bool {
    lt.as_ref()
        .and_then(|j| j.get(key))
        .and_then(|v| v.as_bool())
        .unwrap_or(fallback)
}

pub async fn load_config(pool: &SqlitePool, fallback_data: &Path) -> RecordingConfig {
    // En la 1ª vez (sin settings propios) heredamos de LandingToast.
    let lt = landingtoast_config();
    let enabled = kv(pool, "rec_enabled")
        .await
        .map(|s| matches!(s.as_str(), "1" | "true" | "yes"))
        .unwrap_or(false);
    let output_path = kv(pool, "rec_output_path")
        .await
        .unwrap_or_else(|| default_output_path(fallback_data));
    let audio_device = kv(pool, "rec_audio_device").await;

    let unlimited = match kv(pool, "rec_unlimited").await {
        Some(s) => matches!(s.as_str(), "1" | "true" | "yes"),
        None => lt_bool(&lt, "UnlimitedDuration", true),
    };
    RecordingConfig {
        enabled,
        osd_position: match kv(pool, "rec_osd_position").await {
            Some(s) => s.parse().unwrap_or(0),
            None => lt_i64(&lt, "Position", 0),
        },
        output_path,
        clip_seconds: match kv(pool, "rec_clip_seconds").await {
            Some(s) => s.parse().unwrap_or(45),
            None => lt_i64(&lt, "ToastDuration", 45),
        },
        unlimited,
        monitor_index: match kv(pool, "rec_monitor_index").await {
            Some(s) => s.parse().unwrap_or(0),
            None => lt_i64(&lt, "MonitorIndex", 0),
        },
        source_type: match kv(pool, "rec_source_type").await {
            Some(s) => s.parse().unwrap_or(0),
            None => lt_i64(&lt, "SourceType", 0),
        },
        max_clips: kv_i64(pool, "rec_max_clips", 20).await,
        ffmpeg_path: kv(pool, "rec_ffmpeg_path").await,
        audio_device,
    }
}

// ─────────────────────────────────────────────────────────────────────────
// ffmpeg
// ─────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FfmpegStatus {
    pub present: bool,
    pub path: Option<String>,
    /// "configured" | "bundled" | "appdata" | "path" | "missing".
    pub source: String,
}

#[cfg(windows)]
const FFMPEG_EXE: &str = "ffmpeg.exe";
#[cfg(not(windows))]
const FFMPEG_EXE: &str = "ffmpeg";

/// Resuelve la ruta de ffmpeg en orden: configurado → appdata → bundled →
/// PATH. Devuelve `(path, source)`.
pub fn resolve_ffmpeg(
    configured: Option<&str>,
    app_data_dir: &Path,
    resource_dir: Option<&Path>,
) -> (Option<PathBuf>, String) {
    if let Some(c) = configured.filter(|s| !s.trim().is_empty()) {
        let p = PathBuf::from(c);
        if p.is_file() {
            return (Some(p), "configured".into());
        }
    }
    let appdata_bin = app_data_dir.join("ffmpeg").join(FFMPEG_EXE);
    if appdata_bin.is_file() {
        return (Some(appdata_bin), "appdata".into());
    }
    if let Some(res) = resource_dir {
        let bundled = res.join("ffmpeg").join(FFMPEG_EXE);
        if bundled.is_file() {
            return (Some(bundled), "bundled".into());
        }
    }
    // Último recurso: confiar en el PATH.
    if ffmpeg_on_path() {
        return (Some(PathBuf::from(FFMPEG_EXE)), "path".into());
    }
    (None, "missing".into())
}

fn ffmpeg_on_path() -> bool {
    run_quiet(&PathBuf::from(FFMPEG_EXE), &["-version"]).unwrap_or(false)
}

/// Descarga el zip estático de ffmpeg y extrae `ffmpeg.exe` a
/// `<app_data>/ffmpeg/`. Devuelve la ruta del binario.
pub async fn download_ffmpeg(
    http: &reqwest::Client,
    app_data_dir: &Path,
) -> anyhow::Result<PathBuf> {
    let dest_dir = app_data_dir.join("ffmpeg");
    std::fs::create_dir_all(&dest_dir)?;
    let dest = dest_dir.join(FFMPEG_EXE);

    let bytes = http
        .get(FFMPEG_ZIP_URL)
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;

    // Extraer SOLO el binario (la entrada que termina en /bin/ffmpeg.exe).
    let dest_clone = dest.clone();
    tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        let mut zip = zip::ZipArchive::new(Cursor::new(bytes))?;
        for i in 0..zip.len() {
            let mut f = zip.by_index(i)?;
            let name = f.name().replace('\\', "/");
            if name.ends_with("bin/ffmpeg.exe") || name.ends_with("/ffmpeg") {
                let mut out = std::fs::File::create(&dest_clone)?;
                std::io::copy(&mut f, &mut out)?;
                return Ok(());
            }
        }
        anyhow::bail!("no se encontró ffmpeg dentro del zip")
    })
    .await??;

    Ok(dest)
}

// ─────────────────────────────────────────────────────────────────────────
// Grabación
// ─────────────────────────────────────────────────────────────────────────

#[cfg(windows)]
fn run_quiet(exe: &Path, args: &[&str]) -> anyhow::Result<bool> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let status = std::process::Command::new(exe)
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    Ok(status.map(|s| s.success()).unwrap_or(false))
}

#[cfg(not(windows))]
fn run_quiet(exe: &Path, args: &[&str]) -> anyhow::Result<bool> {
    let status = std::process::Command::new(exe)
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    Ok(status.map(|s| s.success()).unwrap_or(false))
}

/// Opciones de captura.
pub struct RecordOptions {
    pub duration_s: i64,
    /// Región a capturar `(x, y, w, h)` en píxeles = UN monitor. None = todo
    /// el escritorio. Se ignora si hay `window_title`.
    pub region: Option<(i32, i32, u32, u32)>,
    /// Título de ventana a capturar (Source = MSFS). Prioritario sobre region.
    pub window_title: Option<String>,
    /// Dispositivo de audio dshow ya resuelto (None = sin audio).
    pub audio_device: Option<String>,
}

/// Graba un clip con ffmpeg (gdigrab + dshow opcional). Bloqueante
/// (~duration_s) — llamar desde `spawn_blocking`.
#[cfg(windows)]
pub fn record_clip(ffmpeg: &Path, out: &Path, opts: &RecordOptions) -> anyhow::Result<()> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let mut args: Vec<String> = vec!["-y".into(), "-f".into(), "gdigrab".into(),
        "-framerate".into(), "30".into()];
    // Región de monitor (solo si NO capturamos una ventana concreta).
    if opts.window_title.is_none() {
        if let Some((x, y, w, h)) = opts.region {
            args.push("-offset_x".into());
            args.push(x.to_string());
            args.push("-offset_y".into());
            args.push(y.to_string());
            args.push("-video_size".into());
            args.push(format!("{w}x{h}"));
        }
    }
    args.push("-i".into());
    match &opts.window_title {
        Some(title) => args.push(format!("title={title}")),
        None => args.push("desktop".into()),
    }
    // Audio (dshow) opcional — loopback del sistema si está disponible.
    let has_audio = opts.audio_device.is_some();
    if let Some(dev) = &opts.audio_device {
        args.push("-f".into());
        args.push("dshow".into());
        args.push("-i".into());
        args.push(format!("audio={dev}"));
    }
    args.push("-t".into());
    args.push(opts.duration_s.clamp(2, 600).to_string());
    args.push("-c:v".into());
    args.push("libx264".into());
    args.push("-preset".into());
    args.push("veryfast".into());
    args.push("-pix_fmt".into());
    args.push("yuv420p".into());
    if has_audio {
        args.push("-c:a".into());
        args.push("aac".into());
    }
    args.push(out.to_string_lossy().into_owned());

    let output = std::process::Command::new(ffmpeg)
        .args(&args)
        .creation_flags(CREATE_NO_WINDOW)
        .output()?;
    if output.status.success() && out.is_file() {
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!(
            "ffmpeg falló: {}",
            err.lines().last().unwrap_or("error desconocido")
        )
    }
}

#[cfg(not(windows))]
pub fn record_clip(_ffmpeg: &Path, _out: &Path, _opts: &RecordOptions) -> anyhow::Result<()> {
    anyhow::bail!("la grabación de pantalla solo está soportada en Windows")
}

/// Lista los dispositivos de audio dshow (para el selector de audio).
#[cfg(windows)]
pub fn list_audio_devices(ffmpeg: &Path) -> Vec<String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let out = std::process::Command::new(ffmpeg)
        .args([
            "-hide_banner",
            "-list_devices",
            "true",
            "-f",
            "dshow",
            "-i",
            "dummy",
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    match out {
        Ok(o) => parse_dshow_audio(&String::from_utf8_lossy(&o.stderr)),
        Err(_) => Vec::new(),
    }
}

#[cfg(not(windows))]
pub fn list_audio_devices(_ffmpeg: &Path) -> Vec<String> {
    Vec::new()
}

/// Extrae los nombres de dispositivos de audio del stderr de ffmpeg
/// (`-list_devices`). Soporta el formato nuevo (`"Nombre" (audio)`) y el
/// antiguo (sección "DirectShow audio devices").
fn parse_dshow_audio(stderr: &str) -> Vec<String> {
    let mut out = Vec::new();
    // Formato nuevo: líneas con `(audio)`.
    for line in stderr.lines().filter(|l| l.contains("(audio)")) {
        if let Some(name) = first_quoted(line) {
            out.push(name);
        }
    }
    if !out.is_empty() {
        return out;
    }
    // Formato antiguo: tras "DirectShow audio devices".
    let mut in_audio = false;
    for line in stderr.lines() {
        if line.contains("DirectShow audio devices") {
            in_audio = true;
            continue;
        }
        if line.contains("DirectShow video devices") {
            in_audio = false;
        }
        if in_audio && !line.contains("Alternative name") {
            if let Some(name) = first_quoted(line) {
                out.push(name);
            }
        }
    }
    out
}

fn first_quoted(line: &str) -> Option<String> {
    let start = line.find('"')? + 1;
    let rest = &line[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

/// Elige un dispositivo de audio capaz de capturar el sonido del sistema
/// (loopback): Stereo Mix, "What U Hear", VB-CABLE, VoiceMeeter, etc.
pub fn pick_loopback_audio(devices: &[String]) -> Option<String> {
    const HINTS: [&str; 7] = [
        "stereo mix",
        "what u hear",
        "loopback",
        "virtual-audio",
        "cable output",
        "voicemeeter out",
        "wave out",
    ];
    devices
        .iter()
        .find(|d| {
            let l = d.to_lowercase();
            HINTS.iter().any(|h| l.contains(h))
        })
        .cloned()
}

/// Resuelve qué dispositivo de audio usar según la config.
pub fn resolve_audio(cfg: &RecordingConfig, ffmpeg: &Path) -> Option<String> {
    match cfg.audio_device.as_deref() {
        Some("off") => None,
        Some(d) if !d.trim().is_empty() && d != "auto" => Some(d.to_string()),
        _ => pick_loopback_audio(&list_audio_devices(ffmpeg)),
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Librería de clips (manifiesto JSON en la carpeta de salida)
// ─────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LandingClip {
    pub id: String,
    pub path: String,
    pub recorded_at: String,
    pub fpm: Option<i64>,
    pub grade: Option<String>,
    pub airport_icao: Option<String>,
    pub airport_name: Option<String>,
    pub model: Option<String>,
    pub registration: Option<String>,
    pub favorite: bool,
    pub duration_s: i64,
    /// Clip de prueba (botón "grabar prueba"), no de un aterrizaje real.
    pub is_test: bool,
}

fn manifest_path(output_dir: &Path) -> PathBuf {
    output_dir.join(MANIFEST)
}

pub fn load_clips(output_dir: &Path) -> Vec<LandingClip> {
    let p = manifest_path(output_dir);
    let Ok(text) = std::fs::read_to_string(p) else {
        return Vec::new();
    };
    let mut clips: Vec<LandingClip> = serde_json::from_str(&text).unwrap_or_default();
    // Filtramos los que ya no tienen archivo en disco (borrados a mano).
    clips.retain(|c| Path::new(&c.path).is_file());
    clips
}

fn save_clips(output_dir: &Path, clips: &[LandingClip]) -> anyhow::Result<()> {
    std::fs::create_dir_all(output_dir)?;
    let text = serde_json::to_string_pretty(clips)?;
    std::fs::write(manifest_path(output_dir), text)?;
    Ok(())
}

/// Añade un clip al manifiesto y aplica la retención (conserva favoritos +
/// los mejores por FPM hasta `max_clips`; el resto se borra del disco).
pub fn add_clip(output_dir: &Path, clip: LandingClip, max_clips: i64) -> anyhow::Result<()> {
    let mut clips = load_clips(output_dir);
    clips.push(clip);
    apply_retention(&mut clips, max_clips);
    save_clips(output_dir, &clips)
}

/// Conserva favoritos + clips de prueba; del resto, los de mejor FPM (más
/// cercano a 0) hasta llenar `max_clips`. Borra del disco lo descartado.
fn apply_retention(clips: &mut Vec<LandingClip>, max_clips: i64) {
    let max = max_clips.max(1) as usize;
    // Candidatos a poda = aterrizajes reales no favoritos.
    let mut prunable: Vec<usize> = clips
        .iter()
        .enumerate()
        .filter(|(_, c)| !c.favorite && !c.is_test)
        .map(|(i, _)| i)
        .collect();
    let keep_real = max.saturating_sub(
        clips.iter().filter(|c| c.favorite || c.is_test).count(),
    );
    if prunable.len() <= keep_real {
        return;
    }
    // Ordenar prunables por FPM: peor (más negativo) primero = primeros en caer.
    prunable.sort_by(|&a, &b| {
        let fa = clips[a].fpm.unwrap_or(i64::MIN);
        let fb = clips[b].fpm.unwrap_or(i64::MIN);
        fa.cmp(&fb)
    });
    let to_remove: Vec<usize> = prunable
        .into_iter()
        .take(clips.len()) // safety
        .collect::<Vec<_>>();
    let remove_count = to_remove.len().saturating_sub(keep_real);
    let remove_set: std::collections::HashSet<usize> =
        to_remove.into_iter().take(remove_count).collect();
    let mut idx = 0;
    clips.retain(|c| {
        let keep = !remove_set.contains(&idx);
        if !keep {
            let _ = std::fs::remove_file(&c.path);
        }
        idx += 1;
        keep
    });
}

pub fn set_favorite(output_dir: &Path, id: &str, favorite: bool) -> anyhow::Result<()> {
    let mut clips = load_clips(output_dir);
    if let Some(c) = clips.iter_mut().find(|c| c.id == id) {
        c.favorite = favorite;
    }
    save_clips(output_dir, &clips)
}

pub fn delete_clip(output_dir: &Path, id: &str) -> anyhow::Result<()> {
    let mut clips = load_clips(output_dir);
    if let Some(pos) = clips.iter().position(|c| c.id == id) {
        let _ = std::fs::remove_file(&clips[pos].path);
        clips.remove(pos);
    }
    save_clips(output_dir, &clips)
}
