//! (v6 #2b) Comandos Tauri para la grabación de "Best Landings".

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tauri::Manager;

use crate::landing_recorder::{
    self, FfmpegStatus, LandingClip, RecordOptions, RecordingConfig,
};
use crate::{cmd_log, AppState};

fn app_data_dir(app: &tauri::AppHandle) -> PathBuf {
    app.path()
        .app_data_dir()
        .unwrap_or_else(|_| std::env::temp_dir())
}

/// Un monitor del sistema (para el selector "Target").
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MonitorInfo {
    pub index: i64,
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub primary: bool,
}

fn enumerate_monitors(app: &tauri::AppHandle) -> Vec<MonitorInfo> {
    let Some(win) = app.get_webview_window("main") else {
        return Vec::new();
    };
    let primary_name = win
        .primary_monitor()
        .ok()
        .flatten()
        .and_then(|m| m.name().cloned());
    win.available_monitors()
        .unwrap_or_default()
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let pos = m.position();
            let size = m.size();
            let name = m
                .name()
                .cloned()
                .unwrap_or_else(|| format!("Display {}", i + 1));
            let primary = primary_name.as_ref() == Some(&name);
            MonitorInfo {
                index: i as i64,
                name,
                x: pos.x,
                y: pos.y,
                width: size.width,
                height: size.height,
                primary,
            }
        })
        .collect()
}

/// Título de la ventana de MSFS según la versión activa (Source = MSFS).
fn msfs_window_title() -> String {
    if crate::sim::is_2024() {
        "Microsoft Flight Simulator 2024".to_string()
    } else {
        "Microsoft Flight Simulator".to_string()
    }
}

/// Resuelve ffmpeg; si falta, intenta descargarlo automáticamente.
async fn ensure_ffmpeg(
    app: &tauri::AppHandle,
    state: &AppState,
    cfg: &RecordingConfig,
) -> Result<PathBuf, String> {
    let data = app_data_dir(app);
    let resource = app.path().resource_dir().ok();
    let (path, _src) =
        landing_recorder::resolve_ffmpeg(cfg.ffmpeg_path.as_deref(), &data, resource.as_deref());
    if let Some(p) = path {
        return Ok(p);
    }
    // Auto-provisión silenciosa — el usuario no debe descargar nada a mano.
    landing_recorder::download_ffmpeg(&state.http, &data)
        .await
        .map_err(|e| format!("no se pudo preparar ffmpeg: {e:#}"))
}

/// Monitores disponibles (selector "Target").
#[tauri::command]
pub async fn list_monitors(app: tauri::AppHandle) -> Result<Vec<MonitorInfo>, String> {
    Ok(enumerate_monitors(&app))
}

/// Dispositivos de audio dshow (selector de audio). Vacío si ffmpeg falta.
#[tauri::command]
pub async fn list_audio_devices(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<String>, String> {
    let cfg = landing_recorder::load_config(&state.db, &app_data_dir(&app)).await;
    let resource = app.path().resource_dir().ok();
    let (ffmpeg, _) =
        landing_recorder::resolve_ffmpeg(cfg.ffmpeg_path.as_deref(), &app_data_dir(&app), resource.as_deref());
    let Some(ffmpeg) = ffmpeg else {
        return Ok(Vec::new());
    };
    let f = ffmpeg.clone();
    let devices = tokio::task::spawn_blocking(move || landing_recorder::list_audio_devices(&f))
        .await
        .unwrap_or_default();
    Ok(devices)
}

/// Config actual de grabación (con defaults heredados de LandingToast).
#[tauri::command]
pub async fn recording_config(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<RecordingConfig, String> {
    let data = app_data_dir(&app);
    Ok(landing_recorder::load_config(&state.db, &data).await)
}

/// Estado de ffmpeg (presente/ausente + ruta + fuente).
#[tauri::command]
pub async fn recording_ffmpeg_status(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<FfmpegStatus, String> {
    let cfg = landing_recorder::load_config(&state.db, &app_data_dir(&app)).await;
    let resource = app.path().resource_dir().ok();
    let (path, source) = landing_recorder::resolve_ffmpeg(
        cfg.ffmpeg_path.as_deref(),
        &app_data_dir(&app),
        resource.as_deref(),
    );
    Ok(FfmpegStatus {
        present: path.is_some(),
        path: path.map(|p| p.to_string_lossy().into_owned()),
        source,
    })
}

/// Descarga ffmpeg a la carpeta de datos (fallback al bundling).
#[tauri::command]
pub async fn recording_download_ffmpeg(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<FfmpegStatus, String> {
    cmd_log!("recording_download_ffmpeg", "");
    let data = app_data_dir(&app);
    let path = landing_recorder::download_ffmpeg(&state.http, &data)
        .await
        .map_err(|e| format!("descarga de ffmpeg falló: {e:#}"))?;
    Ok(FfmpegStatus {
        present: path.is_file(),
        path: Some(path.to_string_lossy().into_owned()),
        source: "appdata".into(),
    })
}

/// Graba un clip de PRUEBA de `duration_s` segundos del escritorio y lo añade
/// a la librería. Testeable sin sim — verifica que ffmpeg captura bien.
#[tauri::command]
pub async fn recording_test_clip(
    duration_s: i64,
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<LandingClip, String> {
    cmd_log!("recording_test_clip", "dur={duration_s}");
    let data = app_data_dir(&app);
    let cfg = landing_recorder::load_config(&state.db, &data).await;
    let ffmpeg = ensure_ffmpeg(&app, &state, &cfg).await?;

    let output_dir = PathBuf::from(&cfg.output_path);
    std::fs::create_dir_all(&output_dir).map_err(|e| e.to_string())?;
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let file = output_dir.join(format!("simfleet_test_{millis}.mp4"));

    // Source = MSFS → captura su ventana; si no, la región del monitor elegido.
    let (region, window_title) = if cfg.source_type == 1 {
        (None, Some(msfs_window_title()))
    } else {
        let mons = enumerate_monitors(&app);
        let region = mons
            .get(cfg.monitor_index as usize)
            .or_else(|| mons.first())
            .map(|m| (m.x, m.y, m.width, m.height));
        (region, None)
    };
    let dur = duration_s.clamp(3, 30);
    let ffmpeg_c = ffmpeg.clone();
    let file_c = file.clone();
    let audio = landing_recorder::resolve_audio(&cfg, &ffmpeg);
    let opts = RecordOptions {
        duration_s: dur,
        region,
        window_title,
        audio_device: audio,
    };
    tokio::task::spawn_blocking(move || landing_recorder::record_clip(&ffmpeg_c, &file_c, &opts))
        .await
        .map_err(|e| format!("tarea de grabación falló: {e}"))?
        .map_err(|e| e.to_string())?;

    let recorded_at: (String,) = sqlx::query_as("SELECT datetime('now')")
        .fetch_one(&state.db)
        .await
        .map_err(|e| e.to_string())?;

    let clip = LandingClip {
        id: uuid::Uuid::new_v4().to_string(),
        path: file.to_string_lossy().into_owned(),
        recorded_at: recorded_at.0,
        fpm: None,
        grade: None,
        airport_icao: None,
        airport_name: None,
        model: None,
        registration: None,
        favorite: false,
        duration_s: dur,
        is_test: true,
    };
    landing_recorder::add_clip(&output_dir, clip.clone(), cfg.max_clips)
        .map_err(|e| e.to_string())?;
    Ok(clip)
}

/// Lista los clips guardados (los que aún existen en disco).
#[tauri::command]
pub async fn list_landing_clips(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<LandingClip>, String> {
    let cfg = landing_recorder::load_config(&state.db, &app_data_dir(&app)).await;
    let mut clips = landing_recorder::load_clips(&PathBuf::from(&cfg.output_path));
    // Mejores (FPM más fino) primero; los de prueba/sin FPM al final.
    clips.sort_by(|a, b| match (b.fpm, a.fpm) {
        (Some(x), Some(y)) => x.cmp(&y),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => b.recorded_at.cmp(&a.recorded_at),
    });
    Ok(clips)
}

#[tauri::command]
pub async fn set_landing_favorite(
    id: String,
    favorite: bool,
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let cfg = landing_recorder::load_config(&state.db, &app_data_dir(&app)).await;
    landing_recorder::set_favorite(&PathBuf::from(&cfg.output_path), &id, favorite)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn delete_landing_clip(
    id: String,
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let cfg = landing_recorder::load_config(&state.db, &app_data_dir(&app)).await;
    landing_recorder::delete_clip(&PathBuf::from(&cfg.output_path), &id)
        .map_err(|e| e.to_string())
}
