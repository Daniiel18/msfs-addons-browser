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

/// (v6 #2b) Emite un evento OSD de PRUEBA (sin volar) para validar el overlay.
#[tauri::command]
pub async fn osd_test(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    use tauri::Emitter;
    let cfg = landing_recorder::load_config(&state.db, &app_data_dir(&app)).await;
    let payload = serde_json::json!({
        "fpm": -142,
        "grade": "butter",
        "position": cfg.osd_position,
    });
    app.emit("landing://osd", payload).map_err(|e| e.to_string())?;
    cmd_log!("osd_test", "OSD de prueba emitido");
    Ok(())
}

/// (v6 #2b) Log desde la ventana OSD (diagnóstico — confirma qué hace su JS).
#[tauri::command]
pub fn osd_debug(msg: String) {
    tracing::info!(target: "rec", "[osd-js] {msg}");
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

    // (v6 #2b fix) Serializa: si el auto-disparo ya está grabando (replay buffer
    // armado en aproximación), una 2ª captura del mismo monitor se desconecta.
    if !landing_recorder::try_acquire_recording() {
        return Err(
            "Hay una grabación de aterrizaje en curso. Inténtalo de nuevo cuando termine el vuelo."
                .to_string(),
        );
    }

    let output_dir = PathBuf::from(&cfg.output_path);
    std::fs::create_dir_all(&output_dir).map_err(|e| {
        landing_recorder::release_recording();
        e.to_string()
    })?;
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let file = output_dir.join(format!("simfleet_test_{millis}.mp4"));

    let dur = duration_s.clamp(3, 30);
    // (v6 #2b fix) SIN micrófono: windows-record 0.1.0 crashea el proceso al
    // mezclar audio del sistema + un micrófono virtual (p.ej. SteelSeries
    // Sonar). Solo necesitamos el audio del sistema (sonido del sim).
    let (w, h) = landing_recorder::target_monitor_size(&app, true);
    let fps = cfg.fps.clamp(24, 120) as u32;
    let file_c = file.clone();
    let rec_result = tokio::task::spawn_blocking(move || {
        landing_recorder::record_window_clip(
            landing_recorder::SELF_WINDOW,
            &file_c,
            dur,
            false,
            w,
            h,
            fps,
        )
    })
    .await;
    // Liberamos la reserva pase lo que pase con la grabación.
    landing_recorder::release_recording();
    rec_result
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
