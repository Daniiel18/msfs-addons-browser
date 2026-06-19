//! (v6 #2b) Grabación nativa de "Best Landings".
//!
//! Motor: **windows-record** (Windows.Graphics / Desktop Duplication + WASAPI
//! loopback). Reemplaza a ffmpeg/gdigrab porque:
//!   · captura el **audio del sistema** de forma nativa (sin Stereo Mix ni
//!     dispositivos virtuales),
//!   · captura ventanas **DirectX / fullscreen** (MSFS) sin negro,
//!   · trae **replay buffer** (guardar los últimos N s al tocar pista),
//!   · es Rust puro compilado en la app → **sin descargas**.
//!
//! El objetivo de captura es una **ventana por título** (substring): MSFS para
//! los aterrizajes reales, y la ventana de la propia app ("SimFleet") para la
//! grabación de PRUEBA (testeable sin el sim).
//!
//! Este módulo NO toca el `simconnect_watcher` (que procesa el vuelo en vivo).
//! El disparo automático al touchdown + el OSD en vivo se cablearán después.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use sqlx::SqlitePool;

/// Nombre del manifiesto de clips dentro de la carpeta de salida.
const MANIFEST: &str = "simfleet_landings.json";

/// Substring del título de la ventana de MSFS (cubre 2020 y 2024).
pub const MSFS_WINDOW: &str = "Microsoft Flight Simulator";
/// Título de la ventana de la propia app — objetivo de la grabación de prueba.
pub const SELF_WINDOW: &str = "SimFleet";

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
    /// Capturar también el micrófono (comentario), además del audio del sistema.
    pub capture_microphone: bool,
    pub max_clips: i64,
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
    kv(pool, key)
        .await
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

/// Carpeta de salida por defecto (hereda de LandingToast si existe).
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

fn landingtoast_config() -> Option<serde_json::Value> {
    let appdata = std::env::var("APPDATA").ok()?;
    let cfg = PathBuf::from(appdata).join("LandingToast").join("config.json");
    serde_json::from_str(&std::fs::read_to_string(cfg).ok()?).ok()
}

fn landingtoast_video_path() -> Option<String> {
    landingtoast_config()?
        .get("VideoOutputPath")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .filter(|s| !s.trim().is_empty())
}

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
    let lt = landingtoast_config();
    RecordingConfig {
        enabled: kv(pool, "rec_enabled")
            .await
            .map(|s| matches!(s.as_str(), "1" | "true" | "yes"))
            .unwrap_or(false),
        osd_position: match kv(pool, "rec_osd_position").await {
            Some(s) => s.parse().unwrap_or(0),
            None => lt_i64(&lt, "Position", 0),
        },
        output_path: kv(pool, "rec_output_path")
            .await
            .unwrap_or_else(|| default_output_path(fallback_data)),
        clip_seconds: match kv(pool, "rec_clip_seconds").await {
            Some(s) => s.parse().unwrap_or(45),
            None => lt_i64(&lt, "ToastDuration", 45),
        },
        unlimited: match kv(pool, "rec_unlimited").await {
            Some(s) => matches!(s.as_str(), "1" | "true" | "yes"),
            None => lt_bool(&lt, "UnlimitedDuration", true),
        },
        capture_microphone: kv(pool, "rec_microphone")
            .await
            .map(|s| matches!(s.as_str(), "1" | "true" | "yes"))
            .unwrap_or(false),
        max_clips: kv_i64(pool, "rec_max_clips", 20).await,
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Motor de grabación
// ─────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineStatus {
    /// true si el motor de grabación está disponible (Windows).
    pub available: bool,
    pub engine: String,
}

pub fn engine_status() -> EngineStatus {
    EngineStatus {
        available: cfg!(windows),
        engine: "windows-record".into(),
    }
}

/// Graba `duration_s` s de la ventana cuyo título contiene `target_title`
/// (substring, case-insensitive), con audio del sistema. Bloqueante
/// (~duration_s) — llamar desde `spawn_blocking`.
#[cfg(windows)]
pub fn record_window_clip(
    target_title: &str,
    out: &Path,
    duration_s: i64,
    capture_microphone: bool,
) -> anyhow::Result<()> {
    use std::time::Duration;
    use windows_record::{AudioSource, Recorder};

    let config = Recorder::builder()
        .fps(30, 1)
        .capture_audio(true)
        .capture_microphone(capture_microphone)
        .audio_source(AudioSource::Desktop)
        .output_path(out.to_path_buf())
        .build();

    let recorder = Recorder::new(config)
        .map_err(|e| anyhow::anyhow!("no se pudo crear el grabador: {e:?}"))?
        .with_process_name(target_title);

    recorder.start_recording().map_err(|e| {
        anyhow::anyhow!(
            "no se pudo iniciar la grabación (¿está abierta la ventana «{target_title}»?): {e:?}"
        )
    })?;

    std::thread::sleep(Duration::from_secs(duration_s.clamp(2, 600) as u64));

    recorder
        .stop_recording()
        .map_err(|e| anyhow::anyhow!("fallo al finalizar la grabación: {e:?}"))?;

    if !out.is_file() {
        anyhow::bail!("la grabación no produjo ningún archivo");
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn record_window_clip(
    _target_title: &str,
    _out: &Path,
    _duration_s: i64,
    _capture_microphone: bool,
) -> anyhow::Result<()> {
    anyhow::bail!("la grabación solo está soportada en Windows")
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
    let Ok(text) = std::fs::read_to_string(manifest_path(output_dir)) else {
        return Vec::new();
    };
    let mut clips: Vec<LandingClip> = serde_json::from_str(&text).unwrap_or_default();
    clips.retain(|c| Path::new(&c.path).is_file());
    clips
}

fn save_clips(output_dir: &Path, clips: &[LandingClip]) -> anyhow::Result<()> {
    std::fs::create_dir_all(output_dir)?;
    std::fs::write(manifest_path(output_dir), serde_json::to_string_pretty(clips)?)?;
    Ok(())
}

pub fn add_clip(output_dir: &Path, clip: LandingClip, max_clips: i64) -> anyhow::Result<()> {
    let mut clips = load_clips(output_dir);
    clips.push(clip);
    apply_retention(&mut clips, max_clips);
    save_clips(output_dir, &clips)
}

/// Conserva favoritos + clips de prueba; del resto, los de mejor FPM hasta
/// `max_clips`. Borra del disco lo descartado.
fn apply_retention(clips: &mut Vec<LandingClip>, max_clips: i64) {
    let max = max_clips.max(1) as usize;
    let mut prunable: Vec<usize> = clips
        .iter()
        .enumerate()
        .filter(|(_, c)| !c.favorite && !c.is_test)
        .map(|(i, _)| i)
        .collect();
    let keep_real =
        max.saturating_sub(clips.iter().filter(|c| c.favorite || c.is_test).count());
    if prunable.len() <= keep_real {
        return;
    }
    // Peor FPM (más negativo) primero = primeros en caer.
    prunable.sort_by(|&a, &b| {
        clips[a]
            .fpm
            .unwrap_or(i64::MIN)
            .cmp(&clips[b].fpm.unwrap_or(i64::MIN))
    });
    let remove_count = prunable.len().saturating_sub(keep_real);
    let remove_set: std::collections::HashSet<usize> =
        prunable.into_iter().take(remove_count).collect();
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

#[allow(clippy::too_many_arguments)]
fn update_clip_meta(
    out_dir: &Path,
    id: &str,
    fpm: Option<i64>,
    grade: Option<String>,
    icao: Option<String>,
    name: Option<String>,
    model: Option<String>,
    reg: Option<String>,
) -> anyhow::Result<()> {
    let mut clips = load_clips(out_dir);
    if let Some(c) = clips.iter_mut().find(|c| c.id == id) {
        if c.fpm.is_none() {
            c.fpm = fpm;
        }
        if c.grade.is_none() {
            c.grade = grade;
        }
        if c.airport_icao.is_none() {
            c.airport_icao = icao;
        }
        if c.airport_name.is_none() {
            c.airport_name = name;
        }
        if c.model.is_none() {
            c.model = model;
        }
        if c.registration.is_none() {
            c.registration = reg;
        }
    }
    save_clips(out_dir, &clips)
}

// ─────────────────────────────────────────────────────────────────────────
// (v6 #2b) Auto-disparo: replay buffer armado en aproximación → al detectar el
// touchdown se guarda el clip. NO toca el watcher: una tarea async sondea el
// `FlightStatus` compartido (fase/on_ground/altitud) y manda comandos a un
// hilo dedicado que es dueño del `Recorder` (windows-record es !Send / COM).
// Opt-in: solo corre si la grabación está activada en Ajustes.
// ─────────────────────────────────────────────────────────────────────────

#[cfg(windows)]
enum RecCmd {
    Arm {
        output_path: String,
        clip_seconds: i64,
    },
    Save {
        path: String,
        reply: tokio::sync::oneshot::Sender<Result<(), String>>,
    },
    Stop,
}

/// Hilo dedicado dueño del `Recorder` (replay buffer). Recibe comandos por un
/// canal sync. Aislar el `Recorder` aquí evita líos de `Send`/COM con tokio.
#[cfg(windows)]
fn recorder_thread(rx: std::sync::mpsc::Receiver<RecCmd>) {
    use windows_record::{AudioSource, Recorder};
    let mut rec: Option<Recorder> = None;
    while let Ok(cmd) = rx.recv() {
        match cmd {
            RecCmd::Arm {
                output_path,
                clip_seconds,
            } => {
                if rec.is_some() {
                    continue;
                }
                let base = Path::new(&output_path).join("simfleet_buffer.mp4");
                let config = Recorder::builder()
                    .fps(30, 1)
                    .capture_audio(true)
                    .capture_microphone(false)
                    .audio_source(AudioSource::Desktop)
                    .output_path(base)
                    .enable_replay_buffer(true)
                    .replay_buffer_seconds(clip_seconds.clamp(15, 120) as u32)
                    .build();
                match Recorder::new(config) {
                    Ok(r) => {
                        let r = r.with_process_name(MSFS_WINDOW);
                        match r.start_recording() {
                            Ok(_) => {
                                tracing::info!(target: "rec", "replay buffer armado (MSFS)");
                                rec = Some(r);
                            }
                            Err(e) => {
                                tracing::warn!(target: "rec", "no se pudo armar replay buffer: {e:?}")
                            }
                        }
                    }
                    Err(e) => tracing::warn!(target: "rec", "Recorder::new falló: {e:?}"),
                }
            }
            RecCmd::Save { path, reply } => {
                let res = match &rec {
                    Some(r) => r.save_replay(path.as_str()).map_err(|e| format!("{e:?}")),
                    None => Err("recorder no armado".to_string()),
                };
                let _ = reply.send(res);
            }
            RecCmd::Stop => {
                if let Some(r) = rec.take() {
                    let _ = r.stop_recording();
                    tracing::info!(target: "rec", "replay buffer detenido");
                }
            }
        }
    }
}

#[cfg(windows)]
pub fn spawn_controller(pool: SqlitePool, app: tauri::AppHandle, data_dir: PathBuf) {
    let (tx, rx) = std::sync::mpsc::channel::<RecCmd>();
    let _ = std::thread::Builder::new()
        .name("landing-recorder".into())
        .spawn(move || recorder_thread(rx));
    tokio::spawn(controller_loop(pool, app, data_dir, tx));
}

#[cfg(windows)]
async fn controller_loop(
    pool: SqlitePool,
    app: tauri::AppHandle,
    data_dir: PathBuf,
    tx: std::sync::mpsc::Sender<RecCmd>,
) {
    use std::time::Duration;
    use tauri::Manager;

    let mut armed = false;
    let mut prev_on_ground = true;
    let mut saved = false;

    loop {
        tokio::time::sleep(Duration::from_secs(2)).await;

        let cfg = load_config(&pool, &data_dir).await;
        if !cfg.enabled {
            if armed {
                let _ = tx.send(RecCmd::Stop);
                armed = false;
                saved = false;
            }
            continue;
        }

        let status = match app.try_state::<crate::simconnect_watcher::SharedState>() {
            Some(s) => s.lock().await.status.clone(),
            None => continue,
        };
        if !status.sim_running {
            if armed {
                let _ = tx.send(RecCmd::Stop);
                armed = false;
                saved = false;
            }
            prev_on_ground = true;
            continue;
        }

        let phase = status.phase_label.clone().unwrap_or_default();
        let on_ground = status.on_ground.unwrap_or(true);
        let approaching = matches!(phase.as_str(), "descent" | "approach")
            || (!on_ground && status.current_alt_ft.map_or(false, |a| a < 12_000));

        // Armar el replay buffer al entrar en aproximación.
        if approaching && !on_ground && !armed {
            let _ = tx.send(RecCmd::Arm {
                output_path: cfg.output_path.clone(),
                clip_seconds: cfg.clip_seconds,
            });
            armed = true;
            saved = false;
        }

        // Touchdown: transición airborne → on_ground estando armado.
        if armed && on_ground && !prev_on_ground && !saved {
            saved = true;
            schedule_save(
                &pool,
                &tx,
                &cfg.output_path,
                cfg.clip_seconds,
                cfg.max_clips,
                status.destination_icao.clone(),
                status.destination_name.clone(),
                status.aircraft_icao.clone(),
            );
        }

        // Desarmar tras llegar a tierra (taxi/parking).
        if armed && on_ground && matches!(phase.as_str(), "taxi_in" | "parking" | "deboarding") {
            let _ = tx.send(RecCmd::Stop);
            armed = false;
            saved = false;
        }
        prev_on_ground = on_ground;
    }
}

#[cfg(windows)]
#[allow(clippy::too_many_arguments)]
fn schedule_save(
    pool: &SqlitePool,
    tx: &std::sync::mpsc::Sender<RecCmd>,
    output_path: &str,
    clip_seconds: i64,
    max_clips: i64,
    dest_icao: Option<String>,
    dest_name: Option<String>,
    aircraft_icao: Option<String>,
) {
    use std::time::Duration;
    let pool = pool.clone();
    let tx = tx.clone();
    let out_dir = PathBuf::from(output_path);
    tokio::spawn(async move {
        // ~5s para que el rollout entre en el buffer antes de guardar.
        tokio::time::sleep(Duration::from_secs(5)).await;
        let millis = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let path = out_dir
            .join(format!("simfleet_landing_{millis}.mp4"))
            .to_string_lossy()
            .into_owned();
        let (rtx, rrx) = tokio::sync::oneshot::channel();
        if tx.send(RecCmd::Save { path: path.clone(), reply: rtx }).is_err() {
            return;
        }
        match rrx.await {
            Ok(Ok(())) => {
                let recorded_at = sqlx::query_as::<_, (String,)>("SELECT datetime('now')")
                    .fetch_one(&pool)
                    .await
                    .map(|(s,)| s)
                    .unwrap_or_default();
                let clip = LandingClip {
                    id: uuid::Uuid::new_v4().to_string(),
                    path,
                    recorded_at,
                    fpm: None,
                    grade: None,
                    airport_icao: dest_icao,
                    airport_name: dest_name,
                    model: aircraft_icao,
                    registration: None,
                    favorite: false,
                    duration_s: clip_seconds,
                    is_test: false,
                };
                let id = clip.id.clone();
                if let Err(e) = add_clip(&out_dir, clip, max_clips) {
                    tracing::warn!(target: "rec", "add_clip falló: {e:#}");
                }
                tracing::info!(target: "rec", "Best Landing guardado ({id})");
                // Enriquecer con metadata del flight_log ya finalizado (~20s).
                tokio::time::sleep(Duration::from_secs(20)).await;
                enrich_clip_from_flightlog(&pool, &out_dir, &id).await;
            }
            Ok(Err(e)) => tracing::warn!(target: "rec", "save_replay falló: {e}"),
            Err(_) => tracing::warn!(target: "rec", "recorder no respondió al guardar"),
        }
    });
}

#[cfg(windows)]
async fn enrich_clip_from_flightlog(pool: &SqlitePool, out_dir: &Path, id: &str) {
    let row = sqlx::query_as::<
        _,
        (
            Option<i64>,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
        ),
    >(
        "SELECT landing_fpm, destination_icao, destination_name, \
                aircraft_model, aircraft_atc_type, aircraft_registration \
         FROM flight_log WHERE status = 'completed' ORDER BY started_at DESC LIMIT 1",
    )
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();
    let Some((fpm, dico, dname, model, atc, reg)) = row else {
        return;
    };
    let grade = fpm.map(|f| crate::flight_log::landing_grade(f).to_string());
    let model = model.or(atc);
    let _ = update_clip_meta(out_dir, id, fpm, grade, dico, dname, model, reg);
}

#[cfg(not(windows))]
pub fn spawn_controller(_pool: SqlitePool, _app: tauri::AppHandle, _data_dir: PathBuf) {}
