//! Watcher de "vuelo en curso".
//!
//! ## Dos capas de detección
//!
//! 1. **SimConnect FFI** (preferida) — abre conexión real con MSFS
//!    vía `SimConnect.dll`. Suscribe simvars de posición / velocidad
//!    / on-ground y emite eventos al frontend con coordenadas en
//!    tiempo real. Cuando detecta despegue (transición ground→air
//!    con groundspeed > 30 kt) inserta una fila en `flight_log`.
//!    Cuando detecta aterrizaje, la cierra con destino + distancia.
//!
//! 2. **Fallback proceso + SimBrief** — si SimConnect.dll no está
//!    disponible (sim no instalado / SDK no en PATH) o el handshake
//!    falla, caemos a polling de proceso (`sysinfo`) y cruzamos con
//!    la última OFP fresca de SimBrief para responder al menos
//!    "BIKF→KJFK estás volando ahora".
//!
//! El watcher emite los mismos eventos en ambas capas — el frontend
//! no nota la diferencia salvo por la presencia de coordenadas
//! reales (live position).

use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use sqlx::SqlitePool;
use tauri::AppHandle;
use tauri::Emitter;
use tokio::sync::Mutex;

const FALLBACK_POLL_INTERVAL: Duration = Duration::from_secs(5);

#[cfg(target_os = "windows")]
const MSFS_PROCESS_NAMES: &[&str] = &["FlightSimulator.exe", "FlightSimulator2024.exe"];

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FlightStatus {
    /// `true` si detectamos un proceso de MSFS corriendo o si el
    /// handshake de SimConnect tuvo éxito.
    pub sim_running: bool,
    /// `true` si la conexión SimConnect está activa — en ese caso
    /// los campos `currentLat/Lon/Alt` están poblados en vivo.
    pub simconnect_connected: bool,
    pub origin_icao: Option<String>,
    pub origin_name: Option<String>,
    pub destination_icao: Option<String>,
    pub destination_name: Option<String>,
    pub aircraft_icao: Option<String>,
    pub distance_nm: Option<i64>,
    /// Posición actual del user aircraft (sólo cuando SimConnect
    /// está conectado). Se actualiza cada segundo aprox.
    pub current_lat: Option<f64>,
    pub current_lon: Option<f64>,
    pub current_alt_ft: Option<i64>,
    pub current_ground_speed_kt: Option<i64>,
    pub on_ground: Option<bool>,
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
        tracing::info!(target: "simconnect", "watcher arrancando");

        // Intento principal: SimConnect FFI. Vive en su propio
        // thread blocking (la API de SimConnect es sync y nos exige
        // poll loop con sleeps).
        #[cfg(target_os = "windows")]
        {
            let pool_for_sc = pool.clone();
            let app_for_sc = app.clone();
            let state_for_sc = task_state.clone();
            tokio::task::spawn_blocking(move || {
                windows_simconnect::run_loop(pool_for_sc, app_for_sc, state_for_sc);
            });
        }

        // Fallback proceso+SimBrief — corre siempre en paralelo. Si
        // SimConnect está activo, el watcher principal sobrescribe
        // los campos relevantes; si no, este fallback es la única
        // fuente.
        let mut last_emitted = FlightStatus::default();
        loop {
            let status = compute_fallback_status(&pool, &task_state).await;
            if status_changed_for_fallback(&last_emitted, &status) {
                if let Err(e) = app.emit("flight://current", &status) {
                    tracing::warn!(target: "simconnect", "emit flight://current falló: {e:#}");
                }
                last_emitted = status.clone();
            }
            {
                let mut guard = task_state.lock().await;
                // Sólo actualizamos los campos del fallback (proceso +
                // SimBrief). Los campos de SimConnect (current_lat
                // etc.) los pone el otro thread.
                guard.status.sim_running = status.sim_running;
                guard.status.origin_icao = status.origin_icao.clone();
                guard.status.origin_name = status.origin_name.clone();
                guard.status.destination_icao = status.destination_icao.clone();
                guard.status.destination_name = status.destination_name.clone();
                guard.status.aircraft_icao = status.aircraft_icao.clone();
                guard.status.distance_nm = status.distance_nm;
                guard.status.last_checked_at = status.last_checked_at.clone();
            }
            tokio::time::sleep(FALLBACK_POLL_INTERVAL).await;
        }
    });

    state
}

async fn compute_fallback_status(pool: &SqlitePool, state: &SharedState) -> FlightStatus {
    let sim_running_proc = tokio::task::spawn_blocking(detect_sim_running)
        .await
        .unwrap_or(false);
    // Si SimConnect ya nos dijo que está conectado, asumimos sim_running
    // aunque el polling de proceso no lo vea (corre como servicio o
    // permisos diferentes).
    let sc_connected = state.lock().await.status.simconnect_connected;
    let sim_running = sim_running_proc || sc_connected;

    let mut status = FlightStatus {
        sim_running,
        simconnect_connected: sc_connected,
        last_checked_at: chrono::Utc::now()
            .format("%Y-%m-%dT%H:%M:%SZ")
            .to_string(),
        ..Default::default()
    };

    if !sim_running {
        return status;
    }

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

fn status_changed_for_fallback(prev: &FlightStatus, next: &FlightStatus) -> bool {
    prev.sim_running != next.sim_running
        || prev.origin_icao != next.origin_icao
        || prev.destination_icao != next.destination_icao
}

#[cfg(target_os = "windows")]
fn detect_sim_running() -> bool {
    use sysinfo::{ProcessRefreshKind, RefreshKind, System};
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

#[allow(dead_code)]
pub fn emit_flight_log_changed(app: &AppHandle) {
    if let Err(e) = app.emit("flightlog://changed", ()) {
        tracing::warn!(target: "simconnect", "emit flightlog://changed: {e:#}");
    }
}

// ============================================================================
// SimConnect FFI loop (Windows only)
// ============================================================================

#[cfg(target_os = "windows")]
mod windows_simconnect {
    use std::ffi::c_void;
    use std::ptr;
    use std::time::Duration;

    use sqlx::SqlitePool;
    use tauri::{AppHandle, Emitter};
    use tokio::sync::Mutex as TokioMutex;

    use crate::simconnect_ffi as sc;
    use crate::simconnect_ffi::consts::*;

    use super::SharedState;

    /// Datos que pedimos al usuario aircraft. ORDEN IMPORTA — debe
    /// coincidir con el orden en que llamamos a `AddToDataDefinition`.
    /// SimConnect llena este struct directamente con los bytes de la
    /// payload, así que `#[repr(C, packed(4))]` es obligatorio para
    /// que el alignment coincida.
    #[repr(C, packed(4))]
    #[derive(Debug, Default, Clone, Copy)]
    struct AircraftData {
        latitude_deg: f64,
        longitude_deg: f64,
        altitude_ft: f64,
        ground_velocity_kt: f64,
        on_ground: f64, // bool en SimConnect viene como float64 0.0/1.0
        vertical_speed_fpm: f64,
        true_airspeed_kt: f64,
    }

    const DEFINE_ID_AIRCRAFT: u32 = 1;
    const REQUEST_ID_AIRCRAFT: u32 = 1;

    /// State machine simple para detectar despegue / aterrizaje.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum FlightPhase {
        Disconnected,
        OnGround,
        Airborne,
    }

    /// Bucle principal — corre en un thread blocking dedicado.
    /// Patrón: try_connect → poll loop → on disconnect, sleep + retry.
    pub fn run_loop(pool: SqlitePool, app: AppHandle, state: SharedState) {
        let lib = unsafe {
            match sc::SimConnectLib::load() {
                Ok(l) => l,
                Err(e) => {
                    tracing::info!(
                        target: "simconnect",
                        "SimConnect.dll no disponible — fallback a process+SimBrief: {e:#}"
                    );
                    return;
                }
            }
        };
        tracing::info!(target: "simconnect", "SimConnect.dll cargada OK");

        loop {
            match try_connect_and_pump(&lib, &pool, &app, &state) {
                Ok(()) => {
                    tracing::info!(target: "simconnect", "ciclo limpio — reintentando en 10s");
                }
                Err(e) => {
                    tracing::debug!(target: "simconnect", "ciclo falló: {e:#}");
                }
            }
            // Marca desconectado y emite evento al frontend.
            mark_disconnected(&app, &state);
            std::thread::sleep(Duration::from_secs(10));
        }
    }

    fn mark_disconnected(app: &AppHandle, state: &SharedState) {
        // Spinlock breve sobre el lock async — el thread blocking
        // no puede await, así que `try_lock` en un loop corto. Si
        // el lock sigue ocupado tras 50 intentos, dejamos pasar
        // (el siguiente ciclo lo intentará de nuevo).
        for _ in 0..50 {
            if let Ok(mut guard) = state.try_lock() {
                guard.status.simconnect_connected = false;
                guard.status.current_lat = None;
                guard.status.current_lon = None;
                guard.status.current_alt_ft = None;
                guard.status.current_ground_speed_kt = None;
                guard.status.on_ground = None;
                let snapshot = guard.status.clone();
                drop(guard);
                let _ = app.emit("flight://current", &snapshot);
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn try_connect_and_pump(
        lib: &sc::SimConnectLib,
        pool: &SqlitePool,
        app: &AppHandle,
        state: &SharedState,
    ) -> anyhow::Result<()> {
        // Open
        let mut handle: sc::HANDLE = ptr::null_mut();
        let app_name = sc::cstr("MSFS Addons Browser");
        let hr = unsafe {
            (lib.Open)(
                &mut handle,
                app_name.as_ptr(),
                ptr::null_mut(),
                0,
                ptr::null_mut(),
                SIMCONNECT_OPEN_CONFIGINDEX_LOCAL,
            )
        };
        if !sc::succeeded(hr) {
            anyhow::bail!("SimConnect_Open falló (HRESULT 0x{:08x})", hr);
        }
        if handle.is_null() {
            anyhow::bail!("SimConnect_Open devolvió handle nulo");
        }
        tracing::info!(target: "simconnect", "Open OK — conectado a MSFS");

        // Cleanup garantizado al salir.
        struct Guard<'a> {
            lib: &'a sc::SimConnectLib,
            handle: sc::HANDLE,
        }
        impl Drop for Guard<'_> {
            fn drop(&mut self) {
                if !self.handle.is_null() {
                    unsafe {
                        (self.lib.Close)(self.handle);
                    }
                }
            }
        }
        let _guard = Guard { lib, handle };

        // Add data definitions — orden coincide con AircraftData.
        let units_deg = sc::cstr("degrees");
        let units_feet = sc::cstr("feet");
        let units_knots = sc::cstr("knots");
        let units_bool = sc::cstr("bool");
        let units_fpm = sc::cstr("feet per minute");

        let names: &[(&str, &std::ffi::CStr)] = &[
            ("PLANE LATITUDE", units_deg.as_c_str()),
            ("PLANE LONGITUDE", units_deg.as_c_str()),
            ("PLANE ALTITUDE", units_feet.as_c_str()),
            ("GROUND VELOCITY", units_knots.as_c_str()),
            ("SIM ON GROUND", units_bool.as_c_str()),
            // VERTICAL SPEED en feet/minute — lo necesitamos para
            // capturar el FPM del touchdown.
            ("VERTICAL SPEED", units_fpm.as_c_str()),
            // True airspeed — para tracking de velocidad máxima
            // independiente del viento.
            ("AIRSPEED TRUE", units_knots.as_c_str()),
        ];

        for (name, units) in names {
            let n = sc::cstr(name);
            let hr = unsafe {
                (lib.AddToDataDefinition)(
                    handle,
                    DEFINE_ID_AIRCRAFT,
                    n.as_ptr(),
                    units.as_ptr(),
                    SIMCONNECT_DATATYPE_FLOAT64,
                    0.0,
                    u32::MAX, // SIMCONNECT_UNUSED — Datum ID auto
                )
            };
            if !sc::succeeded(hr) {
                anyhow::bail!("AddToDataDefinition '{}' falló (0x{:08x})", name, hr);
            }
        }
        tracing::info!(
            target: "simconnect",
            "data definitions registradas ({} simvars)",
            names.len()
        );

        // Subscribe — período SECOND para no inundar la app con
        // updates sub-segundo (no las usamos para nada visual y
        // ahorra CPU/IPC).
        let hr = unsafe {
            (lib.RequestDataOnSimObject)(
                handle,
                REQUEST_ID_AIRCRAFT,
                DEFINE_ID_AIRCRAFT,
                SIMCONNECT_OBJECT_ID_USER,
                SIMCONNECT_PERIOD_SECOND,
                SIMCONNECT_DATA_REQUEST_FLAG_DEFAULT,
                0,
                0,
                0,
            )
        };
        if !sc::succeeded(hr) {
            anyhow::bail!("RequestDataOnSimObject falló (0x{:08x})", hr);
        }

        // Marca conectado.
        update_connected(state, app, true);

        // Poll loop. SimConnect API es pull — llamamos
        // `GetNextDispatch` repetidamente. Devuelve E_FAIL cuando no
        // hay nada (lo cual es normal); sólo mensajes con dwID
        // específico nos interesan.
        let mut phase = FlightPhase::Disconnected;
        // El id del vuelo activo en `flight_log`. Lo envolvemos en
        // `Arc<Mutex>` para poder pasarlo a tareas async (donde se
        // hace start/finish del vuelo) y leer el resultado de
        // vuelta. Sin esto las async tasks que escriben en él no
        // serían `Send`.
        let current_flight_id: std::sync::Arc<std::sync::Mutex<Option<i64>>> =
            std::sync::Arc::new(std::sync::Mutex::new(None));
        let mut max_alt_ft: i64 = 0;
        let mut max_gs_kt: i64 = 0;
        let mut max_tas_kt: i64 = 0;
        // Ventana corta de los últimos VS leídos — al detectar
        // touchdown elegimos el MÁS negativo de los últimos ~3s
        // como "landing FPM". SimConnect a veces emite un 0
        // espurio justo en el cambio de fase, así que tomar el
        // mínimo de la ventana evita ese artefacto.
        let mut recent_vs: std::collections::VecDeque<f64> =
            std::collections::VecDeque::with_capacity(4);

        loop {
            let mut p_data: *mut sc::SIMCONNECT_RECV = ptr::null_mut();
            let mut cb_data: u32 = 0;
            let hr = unsafe {
                (lib.GetNextDispatch)(handle, &mut p_data, &mut cb_data)
            };
            if !sc::succeeded(hr) {
                // E_FAIL = nada en cola. Sleep corto y reintenta.
                std::thread::sleep(Duration::from_millis(80));
                continue;
            }
            if p_data.is_null() {
                std::thread::sleep(Duration::from_millis(80));
                continue;
            }

            let dw_id = unsafe { (*p_data).dwID };
            match dw_id {
                SIMCONNECT_RECV_ID_QUIT => {
                    tracing::info!(target: "simconnect", "MSFS envió QUIT — cerrando ciclo");
                    return Ok(());
                }
                SIMCONNECT_RECV_ID_EXCEPTION => {
                    let exc = unsafe {
                        &*(p_data as *const sc::SIMCONNECT_RECV_EXCEPTION)
                    };
                    let exc_code = exc.dwException;
                    tracing::warn!(target: "simconnect", "SimConnect EXCEPTION code={}", exc_code);
                }
                SIMCONNECT_RECV_ID_SIMOBJECT_DATA => {
                    // Acceso a los datos: el struct comienza en el
                    // offset de `dwData` (que es donde colocamos el
                    // marker `[DWORD; 1]`).
                    let header = p_data as *const sc::SIMCONNECT_RECV_SIMOBJECT_DATA;
                    let request_id = unsafe { (*header).dwRequestID };
                    if request_id != REQUEST_ID_AIRCRAFT {
                        continue;
                    }
                    let data_ptr = unsafe {
                        let header_size =
                            std::mem::size_of::<sc::SIMCONNECT_RECV_SIMOBJECT_DATA>();
                        let base = p_data as *const u8;
                        // El offset de dwData es ese tamaño - el
                        // marker `[DWORD;1]` (4 bytes).
                        base.add(header_size - 4) as *const AircraftData
                    };
                    if data_ptr.is_null() {
                        continue;
                    }
                    let data = unsafe { *data_ptr };
                    handle_aircraft_data(
                        data,
                        pool,
                        app,
                        state,
                        &mut phase,
                        &current_flight_id,
                        &mut max_alt_ft,
                        &mut max_gs_kt,
                        &mut max_tas_kt,
                        &mut recent_vs,
                    );
                }
                _ => {
                    // Otros mensajes (OPEN, EVENT, etc.) los
                    // ignoramos — sólo nos interesa SIMOBJECT_DATA.
                }
            }
        }
    }

    fn handle_aircraft_data(
        data: AircraftData,
        pool: &SqlitePool,
        app: &AppHandle,
        state: &SharedState,
        phase: &mut FlightPhase,
        current_flight_id: &std::sync::Arc<std::sync::Mutex<Option<i64>>>,
        max_alt_ft: &mut i64,
        max_gs_kt: &mut i64,
        max_tas_kt: &mut i64,
        recent_vs: &mut std::collections::VecDeque<f64>,
    ) {
        let lat = data.latitude_deg;
        let lon = data.longitude_deg;
        let alt = data.altitude_ft;
        let gs = data.ground_velocity_kt;
        let on_ground = data.on_ground >= 0.5;
        let vs = data.vertical_speed_fpm;
        let tas = data.true_airspeed_kt;

        // Validación básica — ocasionalmente SimConnect emite NaN
        // durante carga del flight.
        if !(lat.is_finite() && lon.is_finite()) {
            return;
        }

        // Update max altitude tracking.
        let alt_int = alt as i64;
        if alt_int > *max_alt_ft {
            *max_alt_ft = alt_int;
            let id_opt = current_flight_id.lock().ok().and_then(|g| *g);
            if let Some(id) = id_opt {
                let pool_c = pool.clone();
                let alt_c = *max_alt_ft;
                tokio::spawn(async move {
                    let _ = crate::flight_log::touch_max_altitude(&pool_c, id, alt_c).await;
                });
            }
        }

        // Track max GS / TAS sólo mientras hay un vuelo abierto
        // (evitamos contar el taxi previo al despegue como "max").
        if matches!(*phase, FlightPhase::Airborne) {
            let gs_int = gs as i64;
            if gs_int > *max_gs_kt {
                *max_gs_kt = gs_int;
            }
            let tas_int = tas as i64;
            if tas_int > *max_tas_kt {
                *max_tas_kt = tas_int;
            }
        }

        // Ventana corta de VS — usamos los últimos 4 ticks (~4s)
        // para capturar el VS del touchdown.
        if recent_vs.len() >= 4 {
            recent_vs.pop_front();
        }
        recent_vs.push_back(vs);

        // State machine: detect takeoff / landing.
        let new_phase = match (*phase, on_ground, gs) {
            (FlightPhase::Disconnected, true, _) => FlightPhase::OnGround,
            (FlightPhase::Disconnected, false, _) => FlightPhase::Airborne,
            (FlightPhase::OnGround, false, gs) if gs >= 30.0 => FlightPhase::Airborne,
            (FlightPhase::Airborne, true, gs) if gs < 50.0 => FlightPhase::OnGround,
            _ => *phase,
        };

        if new_phase != *phase {
            tracing::info!(
                target: "simconnect",
                "phase {:?} → {:?} (lat={:.4} lon={:.4} alt={:.0}ft gs={:.0}kt onGround={})",
                *phase, new_phase, lat, lon, alt, gs, on_ground
            );
            match (*phase, new_phase) {
                (FlightPhase::OnGround, FlightPhase::Airborne)
                | (FlightPhase::Disconnected, FlightPhase::Airborne) => {
                    // Despegue. La tarea async escribe el id de
                    // vuelta en el `Arc<Mutex>` compartido — el
                    // poll loop lo lee en el siguiente tick para
                    // saber qué fila actualizar.
                    let pool_c = pool.clone();
                    let lat_c = lat;
                    let lon_c = lon;
                    let id_slot = current_flight_id.clone();
                    tokio::spawn(async move {
                        match crate::flight_log::start_flight(
                            &pool_c, lat_c, lon_c, None, None,
                        )
                        .await
                        {
                            Ok(id) => {
                                tracing::info!(
                                    target: "simconnect",
                                    "DESPEGUE — flight_log id={} en ({:.4}, {:.4})",
                                    id, lat_c, lon_c
                                );
                                if let Ok(mut g) = id_slot.lock() {
                                    *g = Some(id);
                                }
                            }
                            Err(e) => {
                                tracing::error!(
                                    target: "simconnect",
                                    "start_flight falló: {e:#}"
                                );
                            }
                        }
                    });
                }
                (FlightPhase::Airborne, FlightPhase::OnGround) => {
                    // Aterrizaje. Tomamos el id (.take() lo deja
                    // None para no re-cerrar) y lanzamos la tarea
                    // que actualiza la fila con destino + tiempo
                    // + métricas (landing FPM, max GS/TAS).
                    let id_opt = current_flight_id
                        .lock()
                        .ok()
                        .and_then(|mut g| g.take());
                    if let Some(id) = id_opt {
                        // El landing FPM real es el más negativo de
                        // la ventana de los últimos ~3s — protección
                        // contra el spurious 0 que SimConnect emite
                        // justo en el cambio de fase.
                        let landing_vs = recent_vs
                            .iter()
                            .copied()
                            .fold(f64::INFINITY, f64::min);
                        let landing_fpm = if landing_vs.is_finite() {
                            Some(landing_vs as i64)
                        } else {
                            None
                        };
                        let metrics = crate::flight_log::FlightFinishMetrics {
                            max_altitude_ft: Some(*max_alt_ft),
                            landing_fpm,
                            max_ground_speed_kt: if *max_gs_kt > 0 {
                                Some(*max_gs_kt)
                            } else {
                                None
                            },
                            max_true_airspeed_kt: if *max_tas_kt > 0 {
                                Some(*max_tas_kt)
                            } else {
                                None
                            },
                        };
                        let pool_c = pool.clone();
                        let lat_c = lat;
                        let lon_c = lon;
                        let app_c = app.clone();
                        let metrics_c = metrics;
                        tokio::spawn(async move {
                            match crate::flight_log::finish_flight(
                                &pool_c, id, lat_c, lon_c, metrics_c,
                            )
                            .await
                            {
                                Ok(()) => {
                                    tracing::info!(
                                        target: "simconnect",
                                        "ATERRIZAJE — flight_log id={} cerrado en ({:.4}, {:.4}) max_alt={:?}ft landing_fpm={:?} max_gs={:?}kt max_tas={:?}kt",
                                        id,
                                        lat_c,
                                        lon_c,
                                        metrics_c.max_altitude_ft,
                                        metrics_c.landing_fpm,
                                        metrics_c.max_ground_speed_kt,
                                        metrics_c.max_true_airspeed_kt
                                    );
                                    let _ = app_c.emit("flightlog://changed", ());
                                }
                                Err(e) => {
                                    tracing::error!(
                                        target: "simconnect",
                                        "finish_flight falló: {e:#}"
                                    );
                                }
                            }
                        });
                        *max_alt_ft = 0;
                        *max_gs_kt = 0;
                        *max_tas_kt = 0;
                        recent_vs.clear();
                    }
                }
                _ => {}
            }
            *phase = new_phase;
        }

        // Update shared state + emit. Hacemos try_lock para no
        // bloquear el poll loop si el lock está ocupado — peor
        // caso, sólo perdemos un emit y emite el siguiente segundo.
        if let Ok(mut guard) = state.try_lock() {
            let st = &mut guard.status;
            st.simconnect_connected = true;
            st.sim_running = true;
            st.current_lat = Some(lat);
            st.current_lon = Some(lon);
            st.current_alt_ft = Some(alt as i64);
            st.current_ground_speed_kt = Some(gs as i64);
            st.on_ground = Some(on_ground);
            st.last_checked_at = chrono::Utc::now()
                .format("%Y-%m-%dT%H:%M:%SZ")
                .to_string();
            let snapshot = st.clone();
            drop(guard);
            let _ = app.emit("flight://current", &snapshot);
        }
    }

    fn update_connected(state: &SharedState, app: &AppHandle, connected: bool) {
        for _ in 0..50 {
            if let Ok(mut guard) = state.try_lock() {
                guard.status.simconnect_connected = connected;
                guard.status.sim_running = guard.status.sim_running || connected;
                let snapshot = guard.status.clone();
                drop(guard);
                let _ = app.emit("flight://current", &snapshot);
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    // Necesitamos `TokioMutex` re-export para que los tipos resuelvan
    // sin cambiar el call site externo.
    #[allow(dead_code)]
    fn _force_imports(_a: &TokioMutex<()>, _b: *mut c_void) {}
}
