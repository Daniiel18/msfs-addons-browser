use crate::simbrief::{self, RouteFix, SimBriefBriefing, SimBriefFlight, SimBriefRefreshResult};
use crate::AppState;

/// (v4.12.0) Devuelve la ruta planificada (navlog) PEGADA a un vuelo.
/// Lee `flight_log.route_fixes`; si está vacío pero el vuelo está enlazado
/// a un OFP que sigue en `simbrief_flights`, lo resuelve por el link.
#[tauri::command]
pub async fn flight_route_fixes(
    flight_id: i64,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<RouteFix>, String> {
    // (v4.17.0) Cadena de fallbacks para que la ruta SIEMPRE se dibuje si
    // existe un plan compatible (el mapa de Weather la traza encima de las
    // capas meteo):
    //   1. route_fixes pegado al vuelo (NULLIF '[]' para no cortar la
    //      cadena con un JSON vacío).
    //   2. route_fixes del OFP linkeado (simbrief_ofp_id).
    //   3. OFP MÁS RECIENTE del caché cuyo origen+destino coinciden con el
    //      vuelo — cubre los vuelos sin link (cuenta SimBrief compartida)
    //      sin inventar nada: el plan es del mismo par de aeropuertos.
    let json: Option<String> = sqlx::query_scalar(
        r#"
        SELECT COALESCE(
            NULLIF(fl.route_fixes, '[]'),
            NULLIF((SELECT sf.route_fixes FROM simbrief_flights sf
                    WHERE sf.ofp_id = fl.simbrief_ofp_id), '[]'),
            (SELECT sf2.route_fixes FROM simbrief_flights sf2
             WHERE UPPER(sf2.origin_icao) = UPPER(COALESCE(fl.origin_icao, ''))
               AND UPPER(sf2.destination_icao) = UPPER(COALESCE(fl.destination_icao, ''))
               AND sf2.route_fixes IS NOT NULL
               AND sf2.route_fixes <> '[]'
             ORDER BY CAST(sf2.generated_at AS INTEGER) DESC
             LIMIT 1)
        )
        FROM flight_log fl
        WHERE fl.id = ?1
        "#,
    )
    .bind(flight_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| e.to_string())?
    .flatten();

    match json {
        Some(s) if !s.trim().is_empty() => {
            serde_json::from_str::<Vec<RouteFix>>(&s).map_err(|e| e.to_string())
        }
        _ => Ok(Vec::new()),
    }
}

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

/// (v5.2.7) Números de vuelo del AMIGO en cuenta SimBrief compartida.
/// El usuario los configura (CSV, ej. "357"); los OFP con esos números se
/// ignoran al elegir el plan — todos los demás son del usuario.
#[tauri::command]
pub async fn get_simbrief_friend_flights(
    state: tauri::State<'_, AppState>,
) -> Result<Option<String>, String> {
    simbrief::get_friend_flights_raw(&state.db)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn set_simbrief_friend_flights(
    value: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    simbrief::set_friend_flights(&state.db, value.trim())
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
/// (v4.17.0) Resultado del intento de link — la UI reacciona según el
/// status: "linked" → toast de éxito; "regMismatch" → alerta de conflicto
/// de matrícula que exige una última confirmación manual (reintenta con
/// `force = true`).
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkOutcome {
    pub status: String,
    pub simbrief_reg: Option<String>,
    pub sim_reg: Option<String>,
}

#[tauri::command]
pub async fn link_simbrief_ofp(
    flight_id: i64,
    ofp_id: String,
    force: Option<bool>,
    state: tauri::State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<LinkOutcome, String> {
    use tauri::Emitter;
    let ofp = sqlx::query_as::<_, SimBriefFlight>(
        r#"
        SELECT ofp_id, pilot_id, flight_number, callsign, aircraft_icao,
               origin_icao, origin_name, origin_lat, origin_lon,
               destination_icao, destination_name, destination_lat, destination_lon,
               route, distance_nm, est_time_enroute_s, generated_at, fetched_at,
               pax_count, cargo_kg, fuel_burn_kg, units,
               aircraft_reg, plan_ramp_kg,
               COALESCE(route_fixes, '[]') AS route_fixes
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

    // (v4.17.0) DOBLE VALIDACIÓN por matrícula (cuenta SimBrief compartida):
    // comparamos el ATC TAIL NUMBER capturado por SimConnect contra el
    // `registration` del OFP, con estas reglas de negocio:
    //   · OFP sin matrícula (campo vacío/null) → bypass: la confirmación
    //     manual del usuario ("SÍ") basta — se linkea.
    //   · Match exacto (normalizado) → se linkea.
    //   · Mismatch y `force` no seteado → NO se guarda; devolvemos
    //     "regMismatch" para que la UI muestre la alerta de conflicto y
    //     exija la última confirmación (reintento con force=true).
    let sim_reg: Option<String> = sqlx::query_scalar(
        "SELECT aircraft_registration FROM flight_log WHERE id = ?1",
    )
    .fetch_optional(&state.db)
    .await
    .map_err(|e| e.to_string())?
    .flatten();
    let ofp_reg_norm = ofp
        .aircraft_reg
        .as_deref()
        .map(crate::simbrief::normalize_reg)
        .filter(|s| !s.is_empty());
    let sim_reg_norm = sim_reg
        .as_deref()
        .map(crate::simbrief::normalize_reg)
        .filter(|s| !s.is_empty());
    if let (Some(ofp_n), Some(sim_n)) = (ofp_reg_norm.as_deref(), sim_reg_norm.as_deref()) {
        if ofp_n != sim_n && !force.unwrap_or(false) {
            tracing::warn!(
                target: "simbrief",
                "link BLOQUEADO por mismatch de matrícula: OFP={:?} vs sim={:?} (flight {})",
                ofp.aircraft_reg, sim_reg, flight_id
            );
            return Ok(LinkOutcome {
                status: "regMismatch".to_string(),
                simbrief_reg: ofp.aircraft_reg.clone(),
                sim_reg,
            });
        }
    }

    let derived_airline =
        crate::flight_log::derive_airline_icao(ofp.callsign.as_deref());

    // (v4.17.0) En confirmación EXPLÍCITA del usuario, el OFP elegido
    // SOBRESCRIBE flight_number/callsign/airline (puede haber un link
    // automático previo equivocado de la cuenta compartida). pax/cargo/fuel
    // conservan COALESCE para no pisar ediciones manuales.
    let res = sqlx::query(
        r#"
        UPDATE flight_log
        SET simbrief_ofp_id  = ?1,
            flight_number    = ?2,
            callsign         = ?3,
            airline_icao     = ?4,
            passengers       = COALESCE(passengers, ?5),
            cargo_kg         = COALESCE(cargo_kg, ?6),
            fuel_used_kg     = COALESCE(fuel_used_kg, ?7),
            route_fixes      = COALESCE(NULLIF(?9, '[]'), route_fixes)
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
    .bind(serde_json::to_string(&ofp.route_fixes).unwrap_or_else(|_| "[]".to_string()))
    .execute(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    if res.rows_affected() == 0 {
        return Err(format!("flight_log id={} no existe", flight_id));
    }

    tracing::info!(
        target: "simbrief",
        "MANUAL LINK ofp_id={} → flight_id={} (user confirm, force={}) fn={:?} cs={:?}",
        ofp.ofp_id, flight_id, force.unwrap_or(false), ofp.flight_number, ofp.callsign
    );

    let _ = app.emit("flightlog://changed", ());
    Ok(LinkOutcome {
        status: "linked".to_string(),
        simbrief_reg: ofp.aircraft_reg.clone(),
        sim_reg,
    })
}
