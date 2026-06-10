//! Comandos Tauri para `flight_log`.
//!
//! Expuestos al frontend para listar el historial, leer la traza
//! de un vuelo, borrar entradas y poblar con datos demo para
//! validación de UI sin necesidad de MSFS corriendo.

use crate::flight_log::{
    self, FlightFinishMetrics, FlightLogEntry, FlightTrackPoint, UpdateEntryInput,
};
use crate::msfs_logbook::{self, ImportReport, LogbookCandidate};
use crate::vas_acars::{self, VasFlightCandidate, VasImportReport};
use crate::simconnect_watcher::FlightStatus;
use crate::AppState;
use serde::Deserialize;
use tauri::Manager;

#[tauri::command]
pub async fn list_flight_log(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<FlightLogEntry>, String> {
    flight_log::list_entries(&state.db)
        .await
        .map_err(|e| e.to_string())
}

/// Devuelve la polyline real (lista de track points cada ~10s)
/// para el vuelo indicado. La UI lo invoca al clicar un vuelo en
/// la lista de FlightBook para mostrar la ruta real en el mapa
/// en lugar de la great-circle aproximada.
#[tauri::command]
pub async fn get_flight_track(
    flight_id: i64,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<FlightTrackPoint>, String> {
    flight_log::list_track_for_flight(&state.db, flight_id)
        .await
        .map_err(|e| e.to_string())
}

/// (v4.0.0 — P5) Track completo para el Performance modal: incluye
/// altitud, velocidades (gs/ias), VS, attitude (pitch/bank/g),
/// flaps/gear/spoilers, y 6 métricas × 4 engine slots.
/// Para vuelos VAS imports muchos campos vienen NULL — el frontend
/// detecta series vacías y oculta esas curvas.
#[tauri::command]
pub async fn get_flight_track_full(
    flight_id: i64,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<flight_log::FlightTrackFullPoint>, String> {
    flight_log::list_track_full(&state.db, flight_id)
        .await
        .map_err(|e| e.to_string())
}

/// (v4.0.0 P7.9b) Weather samples de un vuelo — viento/temp/presión/
/// precipitación capturados por sample durante el vuelo.
///
/// (v3.25.0) **Fallback Open-Meteo Archive**: si el vuelo NO capturó
/// weather AMBIENT (vuelos viejos pre-captura / imports VAS), reconstruimos
/// el clima REAL del día del vuelo desde Open-Meteo Archive usando el track
/// real. Así TODAS las capas (viento, temperatura, nubes, precipitación,
/// visibilidad) funcionan para cualquier vuelo, con datos de la vida real
/// —no del simulador— que es justo lo que pidió el usuario. Si el archivo
/// falla (sin internet, etc.) devolvemos lo que haya capturado.
#[tauri::command]
pub async fn get_flight_weather(
    flight_id: i64,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<flight_log::WeatherSample>, String> {
    let captured = flight_log::list_weather_for_flight(&state.db, flight_id)
        .await
        .map_err(|e| e.to_string())?;

    // ¿Tiene weather AMBIENT real (viento/temp), no sólo nubes? Si sí, ese
    // es el dato del propio vuelo y lo preferimos.
    let has_ambient = captured
        .iter()
        .any(|w| w.wind_speed_kt.is_some() || w.oat_c.is_some());
    if has_ambient {
        return Ok(captured);
    }

    // Fallback: reconstruir el clima real del día desde Open-Meteo Archive
    // con el track del vuelo.
    let track = match flight_log::list_track_for_flight(&state.db, flight_id).await {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!(target: "weather", "fallback: no se pudo leer track de {flight_id}: {e:#}");
            return Ok(captured);
        }
    };
    let pts: Vec<(f64, f64, String)> = track
        .iter()
        .map(|p| (p.lat, p.lon, p.ts.clone()))
        .collect();
    if pts.is_empty() {
        return Ok(captured);
    }

    match crate::openmeteo::reconstruct_weather_from_archive(&pts).await {
        Ok(arch) => {
            let mapped: Vec<flight_log::WeatherSample> = arch
                .into_iter()
                .map(|a| flight_log::WeatherSample {
                    ts: a.ts,
                    lat: a.lat,
                    lon: a.lon,
                    alt_ft: a.alt_ft,
                    wind_dir_deg: a.wind_dir_deg,
                    wind_speed_kt: a.wind_speed_kt,
                    oat_c: a.oat_c,
                    baro_hpa: a.baro_hpa,
                    visibility_m: a.visibility_m,
                    precip_state: a.precip_state,
                    cloud_cover_pct: a.cloud_cover_pct,
                    cloud_low_pct: a.cloud_low_pct,
                    cloud_mid_pct: a.cloud_mid_pct,
                    cloud_high_pct: a.cloud_high_pct,
                })
                .collect();
            tracing::info!(
                target: "weather",
                "vuelo {} sin AMBIENT — clima real reconstruido del archivo: {} samples",
                flight_id,
                mapped.len()
            );
            Ok(mapped)
        }
        Err(e) => {
            tracing::warn!(
                target: "weather",
                "fallback archive falló para vuelo {flight_id}: {e:#}"
            );
            Ok(captured)
        }
    }
}

/// (v3.26.0 — P7.10) Analiza la traza del vuelo y devuelve el veredicto
/// de daños (LIMPIO / FORZADO / DAÑADO) según sobrevelocidad, pérdida,
/// presión de aceite y fuerza-G. "no_data" para vuelos viejos / VAS que
/// no capturaron estos simvars.
#[tauri::command]
pub async fn analyze_flight_damage(
    flight_id: i64,
    state: tauri::State<'_, AppState>,
) -> Result<crate::damage::DamageReport, String> {
    crate::damage::analyze(&state.db, flight_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn delete_flight_log_entry(
    id: i64,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    flight_log::delete_entry(&state.db, id)
        .await
        .map_err(|e| e.to_string())
}

/// (v2.0.3) Cierre forzado de un vuelo abierto. Usado por el botón
/// "Forzar cierre" del FlightBook cuando un vuelo quedó huérfano
/// (sim crasheó, app cerró mid-flight, etc.) y el usuario quiere
/// que aparezca en el histórico en vez de quedar en "En vuelo ahora".
#[tauri::command]
pub async fn force_close_flight_log_entry(
    id: i64,
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    flight_log::force_close_flight(&state.db, id)
        .await
        .map_err(|e| e.to_string())?;
    // Notifica al store del frontend para que re-fetchee.
    use tauri::Emitter;
    let _ = app.emit("flightlog://changed", ());
    Ok(())
}

/// Input para `update_flight_log_entry` desde el frontend. Cada
/// campo es opcional — `None` significa "no tocar". El frontend
/// envía sólo los campos que el usuario realmente editó.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateEntryDto {
    pub flight_time_s: Option<i64>,
    pub passengers: Option<i64>,
    pub cargo_kg: Option<i64>,
    pub fuel_used_kg: Option<i64>,
    pub departure_gate: Option<String>,
    pub arrival_gate: Option<String>,
    pub aircraft_registration: Option<String>,
    /// (v4.0.0 — P6.2) Origen y destino editables — el watcher a
    /// veces detecta mal el ICAO de spawn o el vuelo VAS importa
    /// con ICAOs anómalos. El usuario los corrige aquí.
    pub origin_icao: Option<String>,
    pub destination_icao: Option<String>,
}

/// Edita campos manualmente — usado por el botón "Editar" del panel
/// de detalle del vuelo. Permite al usuario corregir lo que el
/// watcher detectó mal (gate equivocado, block time inflado por
/// pausa no detectada en versiones viejas, pasajeros sin GSX).
#[tauri::command]
pub async fn update_flight_log_entry(
    id: i64,
    input: UpdateEntryDto,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let mapped = UpdateEntryInput {
        flight_time_s: input.flight_time_s,
        passengers: input.passengers,
        cargo_kg: input.cargo_kg,
        fuel_used_kg: input.fuel_used_kg,
        departure_gate: input.departure_gate,
        arrival_gate: input.arrival_gate,
        aircraft_registration: input.aircraft_registration,
        origin_icao: input.origin_icao,
        destination_icao: input.destination_icao,
    };
    flight_log::update_entry(&state.db, id, &mapped)
        .await
        .map_err(|e| e.to_string())
}

/// Devuelve el estado actual del watcher: ¿MSFS corriendo? ¿qué
/// vuelo cuadra como "activo"? El watcher emite eventos cuando
/// cambia, pero el frontend también necesita un getter para el
/// montaje inicial.
#[tauri::command]
pub async fn get_flight_status(app: tauri::AppHandle) -> Result<FlightStatus, String> {
    let state = app
        .try_state::<crate::simconnect_watcher::SharedState>()
        .ok_or_else(|| "watcher state no inicializado".to_string())?;
    let guard = state.lock().await;
    Ok(guard.status.clone())
}

/// Definición de un vuelo demo para validar la UI sin MSFS.
/// Incluye los pares origen/destino + métricas verosímiles para
/// que la pestaña FlightBook se vea poblada. La polyline real se
/// sintetiza con `synthetic_track` (great circle + jitter).
struct DemoFlight {
    aircraft_title: &'static str,
    aircraft_icao: &'static str,
    origin: (&'static str, f64, f64),
    destination: (&'static str, f64, f64),
    max_alt_ft: i64,
    landing_fpm: i64,
    max_gs_kt: i64,
    max_tas_kt: i64,
    cruise_alt_ft: i64,
    /// Fuel quemado en kg + pasajeros + carga + passengers — para
    /// que el panel detalle muestre métricas realistas en demos.
    fuel_used_kg: i64,
    passengers: i64,
    cargo_kg: i64,
}

/// Lista de vuelos demo distribuidos por el mundo — pensados
/// específicamente para validar la vista "globo terráqueo" con
/// rutas en varios continentes y latitudes. Cubrimos:
///   · Europa: EBBR→LEMD, LFPG→EGLL
///   · Atlántico: EGLL→KJFK
///   · USA doméstico: KJFK→KLAX
///   · Caribe: MDSD→KMIA
///   · Asia: VHHH→RJTT, OMDB→VTBS
///   · Sudamérica: SBGR→SCEL
///   · Polar: KORD→ZBAA
const DEMO_FLIGHTS: &[DemoFlight] = &[
    DemoFlight {
        aircraft_title: "Airbus A320 Demo",
        aircraft_icao: "A320",
        origin: ("EBBR", 50.901, 4.484),
        destination: ("LEMD", 40.471, -3.564),
        max_alt_ft: 36_000,
        landing_fpm: -180,
        max_gs_kt: 454,
        max_tas_kt: 462,
        cruise_alt_ft: 36_000,
        fuel_used_kg: 4_800,
        passengers: 152,
        cargo_kg: 1_800,
    },
    DemoFlight {
        aircraft_title: "Airbus A350-900",
        aircraft_icao: "A359",
        origin: ("EGLL", 51.470, -0.454),
        destination: ("KJFK", 40.640, -73.779),
        max_alt_ft: 39_000,
        landing_fpm: -140,
        max_gs_kt: 538,
        max_tas_kt: 482,
        cruise_alt_ft: 39_000,
        fuel_used_kg: 62_500,
        passengers: 287,
        cargo_kg: 14_200,
    },
    DemoFlight {
        aircraft_title: "Boeing 737-800",
        aircraft_icao: "B738",
        origin: ("KJFK", 40.640, -73.779),
        destination: ("KLAX", 33.943, -118.408),
        max_alt_ft: 35_000,
        landing_fpm: -220,
        max_gs_kt: 481,
        max_tas_kt: 460,
        cruise_alt_ft: 35_000,
        fuel_used_kg: 18_400,
        passengers: 162,
        cargo_kg: 3_100,
    },
    DemoFlight {
        aircraft_title: "Boeing 787-9",
        aircraft_icao: "B789",
        origin: ("VHHH", 22.309, 113.915),
        destination: ("RJTT", 35.553, 139.781),
        max_alt_ft: 38_000,
        landing_fpm: -110,
        max_gs_kt: 502,
        max_tas_kt: 480,
        cruise_alt_ft: 38_000,
        fuel_used_kg: 22_700,
        passengers: 245,
        cargo_kg: 8_900,
    },
    DemoFlight {
        aircraft_title: "Airbus A380-800",
        aircraft_icao: "A388",
        origin: ("OMDB", 25.253, 55.365),
        destination: ("VTBS", 13.690, 100.750),
        max_alt_ft: 41_000,
        landing_fpm: -160,
        max_gs_kt: 525,
        max_tas_kt: 488,
        cruise_alt_ft: 41_000,
        fuel_used_kg: 75_300,
        passengers: 469,
        cargo_kg: 18_700,
    },
    DemoFlight {
        aircraft_title: "Boeing 777-300ER",
        aircraft_icao: "B77W",
        origin: ("SBGR", -23.435, -46.473),
        destination: ("SCEL", -33.393, -70.785),
        max_alt_ft: 37_000,
        landing_fpm: -250,
        max_gs_kt: 491,
        max_tas_kt: 475,
        cruise_alt_ft: 37_000,
        fuel_used_kg: 19_800,
        passengers: 312,
        cargo_kg: 11_400,
    },
    DemoFlight {
        aircraft_title: "Airbus A321neo",
        aircraft_icao: "A21N",
        origin: ("MDSD", 18.430, -69.669),
        destination: ("KMIA", 25.794, -80.290),
        max_alt_ft: 34_000,
        landing_fpm: -190,
        max_gs_kt: 466,
        max_tas_kt: 458,
        cruise_alt_ft: 34_000,
        fuel_used_kg: 6_200,
        passengers: 178,
        cargo_kg: 2_100,
    },
    DemoFlight {
        aircraft_title: "Boeing 747-400",
        aircraft_icao: "B744",
        origin: ("KORD", 41.978, -87.904),
        destination: ("ZBAA", 40.080, 116.585),
        max_alt_ft: 40_000,
        landing_fpm: -130,
        max_gs_kt: 558,
        max_tas_kt: 502,
        cruise_alt_ft: 40_000,
        fuel_used_kg: 145_600,
        passengers: 416,
        cargo_kg: 26_300,
    },
    DemoFlight {
        aircraft_title: "Airbus A320neo (greaser)",
        aircraft_icao: "A20N",
        origin: ("LFPG", 49.013, 2.550),
        destination: ("EGLL", 51.470, -0.454),
        max_alt_ft: 28_000,
        landing_fpm: -85, // touchdown muy fino
        max_gs_kt: 422,
        max_tas_kt: 440,
        cruise_alt_ft: 28_000,
        fuel_used_kg: 1_900,
        passengers: 138,
        cargo_kg: 1_400,
    },
];

/// Slerp (spherical linear interpolation) entre dos puntos de la
/// esfera. Devuelve el punto al parámetro `t ∈ [0, 1]` siguiendo la
/// great-circle exacta. Crítico para que las trazas demo
/// transatlánticas/polares se vean correctas en el globo (linear
/// lat/lon iba en diagonal recta — para EGLL→KJFK aparecía
/// "atravesando España" en vez de pasar por Groenlandia).
fn slerp_geo(lat1: f64, lon1: f64, lat2: f64, lon2: f64, t: f64) -> (f64, f64) {
    let phi1 = lat1.to_radians();
    let phi2 = lat2.to_radians();
    let lam1 = lon1.to_radians();
    let lam2 = lon2.to_radians();

    // Distancia angular total (haversine en forma simplificada).
    let cos_d =
        phi1.sin() * phi2.sin() + phi1.cos() * phi2.cos() * (lam2 - lam1).cos();
    let d = cos_d.clamp(-1.0, 1.0).acos();

    if d.abs() < 1e-10 {
        // Puntos coincidentes o casi — devolver origen sin dividir
        // por sin(d)=0.
        return (lat1, lon1);
    }

    let sin_d = d.sin();
    let a = ((1.0 - t) * d).sin() / sin_d;
    let b = (t * d).sin() / sin_d;
    // Slerp en coordenadas 3D unitarias.
    let x = a * phi1.cos() * lam1.cos() + b * phi2.cos() * lam2.cos();
    let y = a * phi1.cos() * lam1.sin() + b * phi2.cos() * lam2.sin();
    let z = a * phi1.sin() + b * phi2.sin();
    let phi = z.atan2((x * x + y * y).sqrt());
    let lam = y.atan2(x);
    (phi.to_degrees(), lam.to_degrees())
}

/// Helper: simula una traza realista entre dos aeropuertos con N
/// puntos. Sigue la **great-circle exacta** (slerp 3D) + perfil
/// vertical típico (climb → cruise → descent).
fn synthetic_track(
    origin: (f64, f64),
    dest: (f64, f64),
    points: usize,
    cruise_alt_ft: i64,
    max_gs_kt: i64,
) -> Vec<(f64, f64, i64, i64)> {
    let mut out = Vec::with_capacity(points);
    for i in 0..points {
        let t = i as f64 / (points - 1).max(1) as f64;
        let (lat, lon) = slerp_geo(origin.0, origin.1, dest.0, dest.1, t);
        // Perfil vertical trapezoidal: ascenso primer 15%, crucero
        // 15-85%, descenso 85-100%.
        let alt = if t < 0.15 {
            (t / 0.15 * cruise_alt_ft as f64) as i64
        } else if t > 0.85 {
            (((1.0 - t) / 0.15) * cruise_alt_ft as f64) as i64
        } else {
            cruise_alt_ft
        };
        // GS sube en climb, baja en descent.
        let gs = if t < 0.10 {
            (t / 0.10 * max_gs_kt as f64) as i64
        } else if t > 0.90 {
            (((1.0 - t) / 0.10) * max_gs_kt as f64).max(150.0) as i64
        } else {
            max_gs_kt
        };
        out.push((lat, lon, alt, gs));
    }
    out
}

/// Helper de testing: borra los vuelos demo previos e inserta una
/// colección variada con tracks sintéticos para validar la vista
/// globo + el detalle de ruta. Idempotente: ejecutarlo varias veces
/// no acumula duplicados.
#[tauri::command]
pub async fn debug_seed_flight_log(
    state: tauri::State<'_, AppState>,
) -> Result<i64, String> {
    let pool = &state.db;

    // Limpiamos demos previos — los identificamos por aircraft_title
    // que termine en "Demo" o por algún match conocido. Mejor: borrar
    // todos los `flight_log` cuyo `aircraft_title` matchee con
    // alguno de nuestros demos.
    for f in DEMO_FLIGHTS {
        let _ = sqlx::query("DELETE FROM flight_log WHERE aircraft_title = ?1")
            .bind(f.aircraft_title)
            .execute(pool)
            .await;
    }

    let mut last_id: i64 = 0;
    for f in DEMO_FLIGHTS {
        let id = flight_log::start_flight(
            pool,
            f.origin.1,
            f.origin.2,
            Some(f.aircraft_title),
            Some(f.aircraft_icao),
        )
        .await
        .map_err(|e| e.to_string())?;

        // Sintetizamos ~80 puntos de track para que la polyline se
        // vea fluida en el mapa.
        let track = synthetic_track(
            (f.origin.1, f.origin.2),
            (f.destination.1, f.destination.2),
            80,
            f.cruise_alt_ft,
            f.max_gs_kt,
        );
        for (lat, lon, alt, gs) in track {
            if let Err(e) =
                flight_log::insert_track_point(pool, id, lat, lon, alt, gs).await
            {
                tracing::warn!("seed: insert_track_point falló: {e:#}");
            }
        }

        flight_log::finish_flight(
            pool,
            id,
            f.destination.1,
            f.destination.2,
            FlightFinishMetrics {
                max_altitude_ft: Some(f.max_alt_ft),
                landing_fpm: Some(f.landing_fpm),
                max_ground_speed_kt: Some(f.max_gs_kt),
                max_true_airspeed_kt: Some(f.max_tas_kt),
                fuel_used_kg: Some(f.fuel_used_kg),
                paused_seconds: 0,
                fallback_arrival_gate: None,
            },
        )
        .await
        .map_err(|e| e.to_string())?;
        // Pasajeros + carga después del finish (no son métricas
        // capturadas en vivo todavía — vienen vía edit manual o
        // futura integración GSX).
        let _ = flight_log::update_entry(
            pool,
            id,
            &flight_log::UpdateEntryInput {
                passengers: Some(f.passengers),
                cargo_kg: Some(f.cargo_kg),
                ..Default::default()
            },
        )
        .await;
        last_id = id;
    }
    Ok(last_id)
}

// =============================================================================
// (v3.5.0) MSFS Logbook import — auto-detect 2020/2024 × Steam/MS Store
// =============================================================================

/// Lista los candidatos LOGBOOK.BIN encontrados en disco. El frontend
/// usa esto para informar al usuario qué versiones tiene instaladas
/// antes de invocar el import — y para mostrar "no se encontró
/// logbook" cuando la lista está vacía. Es síncrono y NO falla
/// (devuelve lista vacía si no hay nada).
#[tauri::command]
pub async fn msfs_logbook_detect() -> Result<Vec<LogbookCandidate>, String> {
    let candidates = msfs_logbook::detect_candidates();
    tracing::info!(
        target: "msfs_logbook",
        "detect invoked — found {} candidate(s)",
        candidates.len()
    );
    for c in &candidates {
        tracing::info!(
            target: "msfs_logbook",
            "  · candidate: sim={:?} store={} path={} size={}B",
            c.sim, c.store, c.path, c.size_bytes
        );
    }
    Ok(candidates)
}

/// Importa el LOGBOOK.BIN más reciente (preferimos MSFS 2024 sobre
/// 2020, MS Store sobre Steam — orden de `detect_candidates`).
/// Inserta los vuelos en `flight_log` con `source='msfs-logbook'`,
/// deduplicando por (origen, destino, source). Re-pulsar el botón
/// no genera dupes.
///
/// Devuelve `ImportReport` con conteos para feedback al usuario.
/// Si no hay candidatos, devuelve un reporte vacío (`source_path = None`)
/// — el frontend muestra el mensaje "MSFS logbook no encontrado".
#[tauri::command]
pub async fn msfs_logbook_import(
    state: tauri::State<'_, AppState>,
) -> Result<ImportReport, String> {
    // SIEMPRE loggea la invocación, incluso si no hay candidatos —
    // así el usuario puede confirmar que el clic llegó al backend cuando
    // reporta «no pasa nada» en la UI. Sin esto, una detección vacía
    // sale silenciosa por la rama del `let-else`.
    tracing::info!(target: "msfs_logbook", "import invoked");

    let candidates = msfs_logbook::detect_candidates();
    tracing::info!(
        target: "msfs_logbook",
        "detect returned {} candidate(s)",
        candidates.len()
    );
    for c in &candidates {
        tracing::info!(
            target: "msfs_logbook",
            "  · candidate: sim={:?} store={} path={} size={}B modified={:?}",
            c.sim, c.store, c.path, c.size_bytes, c.modified_at
        );
    }

    let Some(top) = candidates.into_iter().next() else {
        // Lista los paths que SÍ chequeamos para que el usuario pueda
        // ver dónde NO está su logbook — útil para debug.
        tracing::warn!(
            target: "msfs_logbook",
            "no LOGBOOK.BIN found in any of the 4 known locations; checked paths:"
        );
        for path in msfs_logbook::candidate_paths_for_log() {
            tracing::warn!(target: "msfs_logbook", "  · {}", path.display());
        }
        return Ok(ImportReport::default());
    };

    tracing::info!(
        target: "msfs_logbook",
        "importing from {} (sim={:?})",
        top.path, top.sim
    );

    msfs_logbook::import_logbook(&state.db, std::path::Path::new(&top.path), top.sim)
        .await
        .map_err(|e| {
            tracing::error!(target: "msfs_logbook", "import failed: {}", e);
            e.to_string()
        })
}

// =============================================================================
// (v3.5.0 + F2) VAS-ACARS import — fallback when user has no MSFS logbook
// =============================================================================

/// Detecta los archivos `.bin` de VAS-ACARS sin importar nada. Útil
/// para que el UI muestre "X vuelos detectados, ¿importar?" antes
/// de invocar el comando pesado.
#[tauri::command]
pub async fn vas_acars_detect() -> Result<Vec<VasFlightCandidate>, String> {
    let cands = vas_acars::detect_flights();
    tracing::info!(
        target: "vas_acars",
        "detect invoked — found {} flight file(s)",
        cands.len()
    );
    Ok(cands)
}

/// Borra TODOS los vuelos de una fuente específica (`source` field).
/// Usado por el botón "Borrar todos los vuelos importados" en Settings
/// para limpiar imports erróneos en fase de prueba y error sin tocar
/// los SimConnect captures (source='simconnect').
///
/// Devuelve `(flights_deleted, tracks_deleted)` para feedback al UI.
/// Tracks se borran por CASCADE en el esquema — los contamos antes
/// para mostrar el total.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteBySourceReport {
    pub flights_deleted: u64,
    pub tracks_deleted: u64,
}

#[tauri::command]
pub async fn delete_flights_by_source(
    source: String,
    state: tauri::State<'_, AppState>,
) -> Result<DeleteBySourceReport, String> {
    tracing::warn!(
        target: "flight_log",
        "delete_flights_by_source invoked: source={}",
        source
    );
    let (flights, tracks) = flight_log::delete_by_source(&state.db, &source)
        .await
        .map_err(|e| {
            tracing::error!(target: "flight_log", "delete_by_source failed: {}", e);
            e.to_string()
        })?;
    tracing::info!(
        target: "flight_log",
        "deleted {} flights ({} tracks) from source={}",
        flights, tracks, source
    );
    Ok(DeleteBySourceReport {
        flights_deleted: flights,
        tracks_deleted: tracks,
    })
}

/// Importa todos los `.bin` de VAS-ACARS al `flight_log`. Deduplica por
/// `external_id` (UUID = filename), así que re-pulsar no genera dupes.
///
/// V1 limitations:
///   · Solo extraemos **origen** (resuelto via nearest-airport sobre
///     la posición de spawn). El destino queda NULL — los ICAOs no
///     viven en el `.bin` (ver doc del módulo `vas_acars`).
///   · El aircraft se reconoce por patrones en el `aircraft.CFG` path
///     (Fenix, PMDG, iniBuilds, FBW, Aerosoft, Headwind). Si no hay
///     match, fallback al livery name.
#[tauri::command]
pub async fn vas_acars_import(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<VasImportReport, String> {
    use tauri::Emitter;
    tracing::info!(target: "vas_acars", "import invoked");
    let report = vas_acars::import_all(&state.db, Some(&app))
        .await
        .map_err(|e| {
            tracing::error!(target: "vas_acars", "import failed: {}", e);
            e.to_string()
        })?;

    // (v3.6.5 fix L1) Emit `flightlog://changed` para que el store del
    // frontend refresque entries Y airlines. Sin esto, los chips de
    // aerolínea no aparecían tras re-importar (el usuario lo reportó:
    // "cuando vuelvo a importar no me vuelven a aparecer los tags").
    // El watcher de SimConnect emite este evento al finish_flight, pero
    // el path de import VAS lo omitía — la suscripción universal en
    // useFlightLogStore.bootstrap llama tanto reload() como
    // reloadAirlines() al recibirlo.
    let _ = app.emit("flightlog://changed", ());

    // (v3.6.1 fix I5) Auto-upload al cloud después del batch — si el
    // usuario está conectado a Google Drive. Esto es lo que pidió:
    // "el score se sincroniza en la nube automáticamente". Lo hacemos
    // UNA vez al final del batch (no por vuelo) para no spam-eat la
    // API. Errores se loguean + emiten como evento `score:upload:error`
    // pero NO abortan el import (los datos ya están persistidos local).
    if report.imported_count > 0 || report.updated_count > 0 {
        let pool_c = state.db.clone();
        let http_c = state.http.clone();
        let app_c = app.clone();
        tokio::spawn(async move {
            let _ = app_c.emit(
                "score:upload:started",
                &serde_json::json!({ "source": "vas-acars-batch" }),
            );
            match crate::cloud_sync::upload_all(&pool_c, &http_c, None).await {
                Ok(rep) => {
                    tracing::info!(
                        target: "vas_acars",
                        "auto-upload after VAS import OK: {:?}", rep
                    );
                    let _ = app_c.emit("score:upload:success", &rep);
                }
                Err(e) => {
                    tracing::warn!(
                        target: "vas_acars",
                        "auto-upload after VAS import failed (probably not connected to cloud — ignoring): {:#}",
                        e
                    );
                    // No emitimos error si el usuario no está conectado
                    // — sería ruido. Sólo si está conectado y el upload falló.
                }
            }
        });
    }

    Ok(report)
}
