//! (v6.1) Comando Tauri para la economía de aerolíneas.
//!
//! El módulo es backend-only por ahora (no hay pantalla). Este comando ya
//! deja el contrato listo para cuando se construya la UI: recalcula el ledger
//! consumiendo la data actual de `flight_log` (+ recibos GSX) y lo devuelve.

use crate::airline_economy::{self, AirlineLedger, AirlinePolicy, BalancePoint};
use crate::AppState;

/// Devuelve el ledger económico por aerolínea (valor base, ingresos, costes,
/// neto y saldo). Recalcula desde los vuelos actuales en cada llamada.
#[tauri::command]
pub async fn airline_economy(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<AirlineLedger>, String> {
    airline_economy::list_economy(&state.db)
        .await
        .map_err(|e| e.to_string())
}

/// (v6.1) Serie temporal del saldo ("banco") de una aerolínea para la gráfica
/// del detalle. Reconstruye la curva vuelo a vuelo desde el caché de P&L.
#[tauri::command]
pub async fn airline_balance_history(
    state: tauri::State<'_, AppState>,
    key: String,
) -> Result<Vec<BalancePoint>, String> {
    airline_economy::balance_history(&state.db, &key)
        .await
        .map_err(|e| e.to_string())
}

/// (v6.1) Política de gestión de una aerolínea (mantenimiento + servicios).
#[tauri::command]
pub async fn airline_policy(
    state: tauri::State<'_, AppState>,
    key: String,
) -> Result<AirlinePolicy, String> {
    airline_economy::get_policy(&state.db, &key)
        .await
        .map_err(|e| e.to_string())
}

/// (v6.1) Guarda la política de una aerolínea y recalcula el ledger.
#[tauri::command]
pub async fn set_airline_policy(
    state: tauri::State<'_, AppState>,
    key: String,
    policy: AirlinePolicy,
) -> Result<(), String> {
    airline_economy::set_policy(&state.db, &key, &policy)
        .await
        .map_err(|e| e.to_string())
}

/// (v6.1) Mantenimiento de la flota de una aerolínea (desgaste por componente).
#[tauri::command]
pub async fn airline_fleet_maintenance(
    state: tauri::State<'_, AppState>,
    key: String,
) -> Result<Vec<crate::aircraft_maintenance::AircraftMaint>, String> {
    crate::aircraft_maintenance::fleet_maintenance(&state.db, &key)
        .await
        .map_err(|e| e.to_string())
}

/// (v6.1) Hace un servicio a una aeronave (resetea el componente y carga el
/// coste — según el nivel de la aerolínea — a su economía).
#[tauri::command]
pub async fn service_aircraft(
    state: tauri::State<'_, AppState>,
    registration: String,
    component: String,
    airline_key: String,
) -> Result<(), String> {
    crate::aircraft_maintenance::service_aircraft(&state.db, &registration, &component, &airline_key)
        .await
        .map_err(|e| e.to_string())
}

/// (v6.1) Mantenimiento de UNA matrícula (Hangar → silueta con la misma data).
#[tauri::command]
pub async fn aircraft_maintenance(
    state: tauri::State<'_, AppState>,
    registration: String,
) -> Result<Option<crate::aircraft_maintenance::AircraftMaint>, String> {
    crate::aircraft_maintenance::single_aircraft_maintenance(&state.db, &registration)
        .await
        .map_err(|e| e.to_string())
}

/// (v6.1) Historial de mantenimiento real de una matrícula (servicios hechos).
#[tauri::command]
pub async fn aircraft_maintenance_history(
    state: tauri::State<'_, AppState>,
    registration: String,
) -> Result<Vec<crate::aircraft_maintenance::MaintRecord>, String> {
    crate::aircraft_maintenance::maintenance_history(&state.db, &registration)
        .await
        .map_err(|e| e.to_string())
}
