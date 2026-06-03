use crate::simbrief::{self, SimBriefBriefing, SimBriefFlight, SimBriefRefreshResult};
use crate::AppState;

#[tauri::command]
pub async fn get_simbrief_pilot_id(
    state: tauri::State<'_, AppState>,
) -> Result<Option<String>, String> {
    simbrief::get_pilot_id(&state.db).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn set_simbrief_pilot_id(
    pilot_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let trimmed = pilot_id.trim();
    if trimmed.is_empty() {
        return Err("El pilot_id no puede estar vacío".into());
    }
    simbrief::set_pilot_id(&state.db, trimmed)
        .await
        .map_err(|e| e.to_string())
}

/// Descarga el último OFP de SimBrief y lo persiste si es nuevo.
/// Si no hay `simbrief_pilot_id` configurado en settings, devuelve
/// error para que la UI pida al usuario que lo configure.
#[tauri::command]
pub async fn refresh_simbrief(
    state: tauri::State<'_, AppState>,
) -> Result<SimBriefRefreshResult, String> {
    let pilot_id = simbrief::get_pilot_id(&state.db)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| {
            "Configura tu SimBrief Pilot ID antes de refrescar.".to_string()
        })?;
    simbrief::refresh_latest(&state.db, &state.http, &pilot_id)
        .await
        .map_err(|e| e.to_string())
}

/// (v3.32.0 #4/#6) Briefing on-demand del OFP más reciente: METAR/TAF de
/// origen/destino/alterno + NOTAMs. La UI lo pide al abrir Weather/NOTAMs
/// y sólo lo muestra si el OFP matchea origen+destino del vuelo. No
/// persiste nada (clima de la vida real al momento del OFP).
#[tauri::command]
pub async fn simbrief_briefing(
    state: tauri::State<'_, AppState>,
) -> Result<SimBriefBriefing, String> {
    let pilot_id = simbrief::get_pilot_id(&state.db)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "Configura tu SimBrief Pilot ID para ver el briefing.".to_string())?;
    simbrief::fetch_briefing(&state.http, &pilot_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn list_simbrief_flights(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<SimBriefFlight>, String> {
    simbrief::list_flights(&state.db)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn delete_simbrief_flight(
    ofp_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    simbrief::delete_flight(&state.db, &ofp_id)
        .await
        .map_err(|e| e.to_string())
}

/// (v4.0.0 P7.5b) Link manual de OFP a un vuelo. Lo invoca el toast
/// de UI cuando el scorer devolvió `Ambiguous` y el usuario eligió.
///
/// Realiza:
/// 1. UPDATE flight_log → simbrief_ofp_id + flight_number + callsign +
///    airline_icao + pax/cargo/fuel (COALESCE — no sobrescribe valores
///    ya editados manualmente).
/// 2. Emite `flightlog://changed` para que la UI refresque.
///
/// Validaciones:
/// - El OFP debe existir en `simbrief_flights`.
/// - El vuelo debe existir y NO estar cerrado (ended_at IS NULL).
///   Si está cerrado, devolvemos error — un OFP ya consumido por otro
///   vuelo no debería re-linkear sobre uno terminado.
#[tauri::command]
pub async fn link_simbrief_ofp(
    flight_id: i64,
    ofp_id: String,
    state: tauri::State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<(), String> {
    use tauri::Emitter;
    let ofp = sqlx::query_as::<_, SimBriefFlight>(
        r#"
        SELECT ofp_id, pilot_id, flight_number, callsign, aircraft_icao,
               origin_icao, origin_name, origin_lat, origin_lon,
               destination_icao, destination_name, destination_lat, destination_lon,
               route, distance_nm, est_time_enroute_s, generated_at, fetched_at,
               pax_count, cargo_kg, fuel_burn_kg, units,
               aircraft_reg, plan_ramp_kg
        FROM simbrief_flights
        WHERE ofp_id = ?1
        LIMIT 1
        "#,
    )
    .bind(&ofp_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| e.to_string())?
    .ok_or_else(|| format!("OFP {} no existe en simbrief_flights", ofp_id))?;

    let derived_airline =
        crate::flight_log::derive_airline_icao(ofp.callsign.as_deref());

    // (v4.0.0 P7.6b iter 4) Permitimos link tanto en vuelos abiertos
    // como cerrados. Si el toast aparece despues del engine shutdown
    // (caso bug que arreglamos en esta iter, pero seguimos siendo
    // defensivos), el usuario debe poder elegir y que el link funcione
    // retroactivamente.
    let res = sqlx::query(
        r#"
        UPDATE flight_log
        SET simbrief_ofp_id  = ?1,
            flight_number    = COALESCE(flight_number, ?2),
            callsign         = COALESCE(callsign, ?3),
            airline_icao     = COALESCE(airline_icao, ?4),
            passengers       = COALESCE(passengers, ?5),
            cargo_kg         = COALESCE(cargo_kg, ?6),
            fuel_used_kg     = COALESCE(fuel_used_kg, ?7)
        WHERE id = ?8
        "#,
    )
    .bind(&ofp.ofp_id)
    .bind(ofp.flight_number.as_deref())
    .bind(ofp.callsign.as_deref())
    .bind(derived_airline.as_deref())
    .bind(ofp.pax_count)
    .bind(ofp.cargo_kg)
    .bind(ofp.fuel_burn_kg)
    .bind(flight_id)
    .execute(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    if res.rows_affected() == 0 {
        return Err(format!("flight_log id={} no existe", flight_id));
    }

    tracing::info!(
        target: "simbrief",
        "MANUAL LINK ofp_id={} → flight_id={} (user choice) fn={:?} cs={:?}",
        ofp.ofp_id, flight_id, ofp.flight_number, ofp.callsign
    );

    let _ = app.emit("flightlog://changed", ());
    Ok(())
}
