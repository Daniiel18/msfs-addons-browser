//! (19/10) Etapa 2 — comandos de la sincronía con Fenix/PMDG TechLog.

use tauri::State;

use crate::techlog::{
    self, TechLogAircraft, TechLogFlight, TlActionRequest, TlActionResult, TlAppStatus, TlCabin, TlPendingOrder,
    TlReadings, TlSource,
};
use crate::AppState;

/// Apps TechLog instaladas / abiertas y cuántos aviones conoce cada una.
#[tauri::command]
pub async fn techlog_status() -> Result<Vec<TlAppStatus>, String> {
    Ok(techlog::status().await)
}

/// Estado TechLog de una matrícula (`null` si ningún TechLog la conoce).
#[tauri::command]
pub async fn techlog_aircraft(registration: String) -> Result<Option<TechLogAircraft>, String> {
    let mut a = techlog::aircraft(&registration).await.map_err(|e| e.to_string())?;
    // (23/10) Nombres, defectos y motivos en el idioma de la app.
    if let Some(a) = a.as_mut() {
        crate::tl_i18n::localize_aircraft(a);
    }
    Ok(a)
}

/// Matrículas (normalizadas) que gestiona TechLog → SimFleet no les aplica su
/// propio desgaste.
#[tauri::command]
pub async fn techlog_managed_registrations() -> Result<Vec<String>, String> {
    let mut v: Vec<String> = techlog::managed_registrations().await.into_keys().collect();
    v.sort();
    Ok(v)
}

/// Acción de mantenimiento en TechLog (reparar, diferir, servicio, check…).
/// (20/10) Si TechLog está cerrado lo abre; si no deja elegir el avión (el
/// sim tiene otro cargado) la orden queda en cola (`queued: true`).
#[tauri::command]
pub async fn techlog_action(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    request: TlActionRequest,
) -> Result<TlActionResult, String> {
    let reg = request.registration.clone();
    let mut r = techlog::action(&state.db, request)
        .await
        .map_err(|e| e.to_string())?;
    // (24/10) En cola, `message` es un código (`sim_closed`…) que traduce el front.
    if !r.queued {
        r.message = crate::tl_i18n::sentence(&r.message);
    }
    // (23/10) No depender sólo del aviso en vivo de TechLog (si cambió de
    // avión para la orden, su evento puede no llegar): la campana y los
    // paneles se refrescan ya.
    if r.ok && !r.queued {
        use tauri::Emitter;
        let _ = app.emit("techlog://changed", reg);
    }
    Ok(r)
}

/// (20/10) Órdenes en cola de una matrícula.
#[tauri::command]
pub async fn techlog_pending(
    state: State<'_, AppState>,
    registration: String,
) -> Result<Vec<TlPendingOrder>, String> {
    techlog::pending(&state.db, &registration)
        .await
        .map_err(|e| e.to_string())
}

/// (20/10) Quita una orden de la cola.
#[tauri::command]
pub async fn techlog_cancel_pending(state: State<'_, AppState>, id: i64) -> Result<(), String> {
    techlog::cancel_pending(&state.db, id)
        .await
        .map_err(|e| e.to_string())
}

/// (20/10) Abre la app TechLog (minimizada). `false` si ya estaba abierta.
#[tauri::command]
pub async fn techlog_launch(source: String) -> Result<bool, String> {
    let source = TlSource::parse(&source)
        .filter(|s| *s != TlSource::Simfleet)
        .ok_or_else(|| "not_installed".to_string())?;
    techlog::launch(source).await.map_err(|e| e.to_string())
}

/// (23/10) Cabina del Fenix cargado (TechLog 2.1+). `null` si esa matrícula
/// no es el avión activo de un TechLog abierto.
#[tauri::command]
pub async fn techlog_cabin(registration: String) -> Result<Option<TlCabin>, String> {
    techlog::cabin(&registration).await.map_err(|e| e.to_string())
}

/// (23/10) Lecturas en vivo del Fenix cargado (TechLog 2.2+). `null` si esa
/// matrícula no es el avión activo de un TechLog abierto.
#[tauri::command]
pub async fn techlog_readings(registration: String) -> Result<Option<TlReadings>, String> {
    let mut r = techlog::readings(&registration).await.map_err(|e| e.to_string())?;
    if let Some(r) = r.as_mut() {
        crate::tl_i18n::localize_readings(r);
    }
    Ok(r)
}

/// (23/10) Luces de cabina (`control: "lights"`, value "Bright"|"Dim1"|"Dim2"|"Off")
/// o pre-select de agua (`"preselect"`, value 0–100 en pasos de 25).
#[tauri::command]
pub async fn techlog_cabin_set(
    registration: String,
    control: String,
    value: serde_json::Value,
) -> Result<TlActionResult, String> {
    let mut r = techlog::cabin_set(&registration, &control, &value)
        .await
        .map_err(|e| e.to_string())?;
    r.message = crate::tl_i18n::sentence(&r.message);
    Ok(r)
}

/// Vuelo de TechLog que corresponde a un vuelo del FlightBook.
#[tauri::command]
pub async fn techlog_flight(
    registration: String,
    started_at: String,
    ended_at: Option<String>,
) -> Result<Option<TechLogFlight>, String> {
    let mut f = techlog::flight_for(&registration, &started_at, ended_at.as_deref())
        .await
        .map_err(|e| e.to_string())?;
    if let Some(f) = f.as_mut() {
        crate::tl_i18n::localize_flight(f);
    }
    Ok(f)
}
