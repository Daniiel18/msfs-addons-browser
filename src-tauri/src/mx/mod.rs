//! (19/10) Etapa 3 — Tech Log propio de SimFleet.
//!
//! Sustituye el sistema anterior de daños/desgaste para todos los aviones que
//! NO tienen Fenix/PMDG TechLog (iFly 737 MAX, PMDG 777, iniBuilds A350/A310,
//! FlyByWire A32NX y cualquier otro con un catálogo genérico). Mismo modelo
//! que TechLog (piezas con vida, MEL, checks, GO/NO-GO), "solo registro": no
//! activa fallos en el avión.
//!
//!   · `catalog`  — catálogos por familia y detección de perfil.
//!   · `recorder` — registrador en vuelo (lo alimenta el watcher).
//!   · `store`    — reglas al cerrar el vuelo, vistas y acciones.

pub mod catalog;
pub mod recorder;
pub mod store;

/// El sistema anterior de desgaste (aircraft_maintenance: tires/brakes/oil…
/// derivado de los vuelos + auto-servicio + avisos) queda retirado: ahora lo
/// lleva este Tech Log y, para Fenix/PMDG, sus TechLog.
pub const LEGACY_WEAR_ENABLED: bool = false;

use sqlx::SqlitePool;
use tauri::{AppHandle, Emitter};

/// (3B) Mantenimiento nocturno de la aerolínea: una vez por día local, en
/// cuanto SimFleet está abierto pasada la medianoche (si el PC estaba apagado
/// a medianoche, se hace al abrirlo).
pub fn spawn_nightly(app: AppHandle, pool: SqlitePool) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(90)).await;
        loop {
            let today = chrono::Local::now().date_naive().to_string();
            let last: Option<String> =
                sqlx::query_scalar("SELECT value FROM settings WHERE key = 'mx_nightly_last'")
                    .fetch_optional(&pool)
                    .await
                    .ok()
                    .flatten();
            if last.as_deref() != Some(today.as_str()) {
                match store::nightly(&pool).await {
                    Ok(n) => {
                        let _ = sqlx::query(
                            "INSERT INTO settings (key, value) VALUES ('mx_nightly_last', ?1) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                        )
                        .bind(&today)
                        .execute(&pool)
                        .await;
                        if n > 0 {
                            let _ = app.emit("mx://changed", 0);
                        }
                    }
                    Err(e) => tracing::warn!(target: "mx", "mantenimiento nocturno: {e}"),
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(30 * 60)).await;
        }
    });
}

/// El watcher cerró el vuelo: aplica su registro al Tech Log.
pub async fn flight_closed(pool: &SqlitePool, app: &AppHandle, flight_id: i64, landing_fpm: Option<i64>) {
    let Some(rec) = recorder::take(flight_id) else {
        return;
    };
    match store::commit_flight(pool, rec, landing_fpm).await {
        Ok(()) => {
            let _ = app.emit("mx://changed", flight_id);
            // (21/10) El puntaje del FlightBook se calcula antes de que el
            // registro del Tech Log exista: se recalcula para sus reglas.
            if let Err(e) = crate::scoring::score_flight(pool, flight_id).await {
                tracing::debug!(target: "mx", "recalcular puntaje del vuelo {flight_id}: {e:#}");
            }
        }
        Err(e) => tracing::warn!(target: "mx", "no pude aplicar el vuelo {flight_id} al Tech Log: {e}"),
    }
}
