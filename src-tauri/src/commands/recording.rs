//! (v6 #2b) Comandos Tauri para la grabación de "Best Landings"
//! (motor: windows-record).

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use tauri::Manager;

use crate::landing_recorder::{self, EngineStatus, LandingClip, RecordingConfig};
use crate::{cmd_log, AppState};

fn app_data_dir(app: &tauri::AppHandle) -> PathBuf {
    app.path()
        .app_data_dir()
        .unwrap_or_else(|_| std::env::temp_dir())
}

/// Config actual de grabación (con defaults heredados de LandingToast).
#[tauri::command]
pub async fn recording_config(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<RecordingConfig, String> {
    Ok(landing_recorder::load_config(&state.db, &app_data_dir(&app)).await)
}

/// Estado del motor de grabación (windows-record, integrado — sin descargas).
#[tauri::command]
pub async fn recording_engine_status() -> Result<EngineStatus, String> {
    Ok(landing_recorder::engine_status())
}

/// Graba un clip de PRUEBA de `duration_s` s. Targetea la ventana de la propia
/// app ("SimFleet") para verificar vídeo+audio SIN necesitar el sim abierto.
#[tauri::command]
pub async fn recording_test_clip(
    duration_s: i64,
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<LandingClip, String> {
    cmd_log!("recording_test_clip", "dur={duration_s}");
    let data = app_data_dir(&app);
    let cfg = landing_recorder::load_config(&state.db, &data).await;

    let output_dir = PathBuf::from(&cfg.output_path);
    std::fs::create_dir_all(&output_dir).map_err(|e| e.to_string())?;
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let file = output_dir.join(format!("simfleet_test_{millis}.mp4"));

    let dur = duration_s.clamp(3, 30);
    let mic = cfg.capture_microphone;
    let file_c = file.clone();
    tokio::task::spawn_blocking(move || {
        landing_recorder::record_window_clip(
            landing_recorder::SELF_WINDOW,
            &file_c,
            dur,
            mic,
        )
    })
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
