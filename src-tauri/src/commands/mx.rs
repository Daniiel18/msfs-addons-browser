//! (19/10) Etapa 3 — comandos del Tech Log propio de SimFleet.

use tauri::State;

use crate::techlog::{TechLogAircraft, TechLogFlight, TlActionRequest, TlActionResult};
use crate::AppState;

/// Estado de mantenimiento de una matrícula en el Tech Log de SimFleet (si no
/// existe, se da de alta como avión nuevo). `title` ayuda a elegir el perfil.
#[tauri::command]
pub async fn mx_aircraft(
    state: State<'_, AppState>,
    registration: String,
    title: Option<String>,
) -> Result<Option<TechLogAircraft>, String> {
    let mut a = crate::mx::store::aircraft_view(&state.db, &registration, title.as_deref())
        .await
        .map_err(|e| e.to_string())?;
    // (23/10) Nombres, defectos y motivos en el idioma de la app.
    if let Some(a) = a.as_mut() {
        crate::tl_i18n::localize_aircraft(a);
    }
    Ok(a)
}

/// Acción de mantenimiento (reparar, diferir, reemplazar, servicio, check…).
/// Error `in_flight` si el avión está volando.
#[tauri::command]
pub async fn mx_action(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    request: TlActionRequest,
) -> Result<TlActionResult, String> {
    let mut r = crate::mx::store::action(&state.db, request)
        .await
        .map_err(|e| e.to_string())?;
    r.message = crate::tl_i18n::sentence(&r.message);
    // (23/10) La campana de avisos y los paneles se refrescan con este evento:
    // sin él, un avión reparado seguía avisando hasta reiniciar la app.
    if r.ok {
        use tauri::Emitter;
        let _ = app.emit("mx://changed", 0);
    }
    Ok(r)
}

/// Ritmo de desgaste actual (piezas, cabina).
#[tauri::command]
pub async fn mx_get_wear_rates(state: State<'_, AppState>) -> Result<(f64, f64), String> {
    Ok(crate::mx::store::get_wear_rates(&state.db).await)
}

/// Cambia el ritmo de desgaste sin saltos en el % restante de las piezas.
#[tauri::command]
pub async fn mx_set_wear_rates(
    state: State<'_, AppState>,
    life: f64,
    cabin: f64,
) -> Result<(), String> {
    crate::mx::store::set_wear_rates(&state.db, life, cabin)
        .await
        .map_err(|e| e.to_string())
}

/// Registro de mantenimiento de un vuelo del FlightBook.
#[tauri::command]
pub async fn mx_flight(
    state: State<'_, AppState>,
    flight_id: i64,
) -> Result<Option<TechLogFlight>, String> {
    let mut f = crate::mx::store::flight_view(&state.db, flight_id)
        .await
        .map_err(|e| e.to_string())?;
    if let Some(f) = f.as_mut() {
        crate::tl_i18n::localize_flight(f);
    }
    Ok(f)
}
