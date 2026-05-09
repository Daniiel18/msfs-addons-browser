//! Watcher de "vuelo en curso".
//!
//! ## Estado actual
//!
//! La integración pura con `SimConnect.dll` está pendiente — los
//! crates de Rust que la envuelven (`simconnect`, `simconnect-sdk`)
//! requieren libclang/bindgen en el build. Para evitar esa
//! fricción usamos una **aproximación pragmática**:
//!
//!   1. Polling cada 5s para detectar si `FlightSimulator.exe`
//!      (MSFS 2020) o `FlightSimulator2024.exe` (MSFS 2024) está
//!      corriendo.
//!   2. Cuando el sim arranca, miramos la última OFP de SimBrief.
//!      Si es reciente (< 6 h) la consideramos el "vuelo activo"
//!      y emitimos un evento `flight://current` con el origen y
//!      destino.
//!   3. Cuando el sim se cierra, emitimos `flight://current` con
//!      `null` para que la UI lo limpie.
//!
//! Esto da una respuesta utilizable a "¿qué estoy volando ahora?"
//! sin SimConnect real. Para coords en tiempo real haría falta el
//! binding nativo — lo dejamos para una iteración cuando libclang
//! esté disponible o usemos `windows-sys` directamente.

use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use sqlx::SqlitePool;
use tauri::AppHandle;
use tauri::Emitter;
use tokio::sync::Mutex;

const POLL_INTERVAL: Duration = Duration::from_secs(5);

/// Procesos típicos del simulador. Comprobamos los dos porque
/// usuarios tienen MSFS 2020 o 2024 (a veces ambos instalados).
#[cfg(target_os = "windows")]
const MSFS_PROCESS_NAMES: &[&str] = &[
    "FlightSimulator.exe",
    "FlightSimulator2024.exe",
];

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FlightStatus {
    /// `true` si detectamos un proceso de MSFS corriendo.
    pub sim_running: bool,
    /// Origen ICAO del vuelo activo (de la última OFP fresca).
    pub origin_icao: Option<String>,
    pub origin_name: Option<String>,
    /// Destino ICAO del vuelo activo.
    pub destination_icao: Option<String>,
    pub destination_name: Option<String>,
    /// Aircraft del último OFP — útil para mostrar al usuario.
    pub aircraft_icao: Option<String>,
    /// Distancia total del plan en NM.
    pub distance_nm: Option<i64>,
    /// Última vez que se evaluó (ISO-8601).
    pub last_checked_at: String,
}

#[derive(Debug, Clone, Default)]
#[allow(dead_code)]
pub struct WatcherState {
    pub status: FlightStatus,
}

pub type SharedState = Arc<Mutex<WatcherState>>;

pub fn spawn(pool: SqlitePool, app: AppHandle) -> SharedState {
    let state: SharedState = Arc::new(Mutex::new(WatcherState::default()));
    let task_state = state.clone();

    tokio::spawn(async move {
        tracing::info!("simconnect: watcher arrancando (modo proceso+SimBrief)");
        let mut last_emitted = FlightStatus::default();
        loop {
            let status = compute_status(&pool).await;

            // Emitimos sólo cuando cambia algo relevante para evitar
            // spam de eventos al frontend. La comparación incluye los
            // campos que la UI realmente refleja.
            if status_changed(&last_emitted, &status) {
                if let Err(e) = app.emit("flight://current", &status) {
                    tracing::warn!("simconnect: emit flight://current falló: {e:#}");
                }
                last_emitted = status.clone();
            }

            {
                let mut guard = task_state.lock().await;
                guard.status = status;
            }

            tokio::time::sleep(POLL_INTERVAL).await;
        }
    });

    state
}

/// Calcula el estado actual: ¿está MSFS corriendo? ¿tenemos un OFP
/// reciente que cuadre? Lo hacemos en spawn_blocking porque
/// `sysinfo::System::refresh_processes` es CPU-bound.
async fn compute_status(pool: &SqlitePool) -> FlightStatus {
    let sim_running = tokio::task::spawn_blocking(detect_sim_running)
        .await
        .unwrap_or(false);

    let mut status = FlightStatus {
        sim_running,
        last_checked_at: chrono::Utc::now()
            .format("%Y-%m-%dT%H:%M:%SZ")
            .to_string(),
        ..Default::default()
    };

    // Si el sim no está corriendo, no hace falta consultar SimBrief —
    // el "vuelo activo" sólo tiene sentido cuando estás volando.
    if !sim_running {
        return status;
    }

    // Última OFP — la consideramos válida si fue generada en las
    // últimas 6 horas. Eso cubre el caso típico (generas el plan
    // y vuelas ese día) sin marcar como "actual" un OFP de hace
    // semanas.
    if let Ok(Some(latest)) = latest_recent_simbrief(pool).await {
        status.origin_icao = Some(latest.origin_icao);
        status.origin_name = latest.origin_name;
        status.destination_icao = Some(latest.destination_icao);
        status.destination_name = latest.destination_name;
        status.aircraft_icao = latest.aircraft_icao;
        status.distance_nm = latest.distance_nm;
    }

    status
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct LatestFlight {
    origin_icao: String,
    origin_name: Option<String>,
    destination_icao: String,
    destination_name: Option<String>,
    aircraft_icao: Option<String>,
    distance_nm: Option<i64>,
}

async fn latest_recent_simbrief(pool: &SqlitePool) -> anyhow::Result<Option<LatestFlight>> {
    // `generated_at` viene como UNIX seconds desde SimBrief — en
    // SQLite lo guardamos como TEXT. Usamos `datetime('now', '-6 hours')`
    // para comparar contra "ahora menos 6h".
    let row = sqlx::query_as::<_, LatestFlight>(
        r#"
        SELECT origin_icao, origin_name, destination_icao, destination_name,
               aircraft_icao, distance_nm
        FROM simbrief_flights
        WHERE generated_at IS NOT NULL
          AND CAST(generated_at AS INTEGER) > strftime('%s', 'now', '-6 hours')
        ORDER BY CAST(generated_at AS INTEGER) DESC
        LIMIT 1
        "#,
    )
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

fn status_changed(prev: &FlightStatus, next: &FlightStatus) -> bool {
    prev.sim_running != next.sim_running
        || prev.origin_icao != next.origin_icao
        || prev.destination_icao != next.destination_icao
}

#[cfg(target_os = "windows")]
fn detect_sim_running() -> bool {
    use sysinfo::{ProcessRefreshKind, RefreshKind, System};
    // En sysinfo 0.32 los constructores «sin nada» se llaman `new()`
    // y se completan con `with_processes(...)`; no hay `nothing()`.
    let mut sys =
        System::new_with_specifics(RefreshKind::new().with_processes(ProcessRefreshKind::new()));
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, false);
    for (_, proc) in sys.processes() {
        let name = proc.name().to_string_lossy();
        for target in MSFS_PROCESS_NAMES {
            if name.eq_ignore_ascii_case(target) {
                return true;
            }
        }
    }
    false
}

#[cfg(not(target_os = "windows"))]
fn detect_sim_running() -> bool {
    false
}

/// Notifica al frontend que el flight_log cambió.
#[allow(dead_code)]
pub fn emit_flight_log_changed(app: &AppHandle) {
    if let Err(e) = app.emit("flightlog://changed", ()) {
        tracing::warn!("simconnect: no se pudo emitir flightlog://changed: {e:#}");
    }
}
