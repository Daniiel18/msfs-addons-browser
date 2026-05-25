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
    /// (v1.1.4) Rumbo verdadero del avión en grados (0..360). Usado
    /// por el mapa del FlightBook para rotar el icono del avión en
    /// la posición en vivo.
    pub current_heading_deg: Option<i64>,
    pub on_ground: Option<bool>,
    /// (v0.1.25) Fase granular del vuelo en lenguaje humano:
    /// "preflight" | "engine_start" | "pushback" | "taxi_out" |
    /// "takeoff" | "climbing" | "cruise" | "descent" | "approach" |
    /// "landed_rollout" | "taxi_in" | "parking" | "deboarding".
    /// Lo deriva el watcher en cada tick desde los simvars (vs,
    /// altitud, gs, on_ground, parking_brake, engines). El frontend
    /// lo usa para mostrar status profesional en vez de "Volando".
    pub phase_label: Option<String>,
    /// (v3.0.0) Gate / parking actual del avión cuando está en tierra.
    /// Se popula al conectar SimConnect (request asíncrona al
    /// aeropuerto más cercano) y se persiste en `flight_log.departure_gate`
    /// al disparar el OUT. La UI lo usa para pintar el gate en el
    /// FlyingNowBadge antes de que arranque el vuelo.
    pub current_gate: Option<String>,
    /// (v3.0.0) ICAO del aeropuerto donde está el avión ahora (nearest).
    /// Usado por el frontend para detener la auto-refresh de SimBrief
    /// cuando el origen del OFP coincide con la posición actual del
    /// avión.
    pub current_airport_icao: Option<String>,
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
        // (v3.4.10) Heartbeat — forzar emit cada N polls aunque el
        // status no haya cambiado. Asegura que el frontend siempre
        // tiene el estado actual aunque el listener se haya suscrito
        // tarde y el emit inicial se haya perdido.
        const FALLBACK_HEARTBEAT_EVERY: u32 = 6; // ~30s con FALLBACK_POLL_INTERVAL=5s
        let mut polls_since_emit: u32 = 0;
        loop {
            let status = compute_fallback_status(&pool, &task_state).await;
            let changed = status_changed_for_fallback(&last_emitted, &status);
            let should_emit = changed || polls_since_emit >= FALLBACK_HEARTBEAT_EVERY;
            if should_emit {
                tracing::info!(
                    target: "simconnect",
                    "emit flight://current (fallback{}) sim_running={} sc_connected={} origin={:?}",
                    if changed { ", changed" } else { ", heartbeat" },
                    status.sim_running, status.simconnect_connected, status.origin_icao
                );
                if let Err(e) = app.emit("flight://current", &status) {
                    tracing::warn!(target: "simconnect", "emit flight://current falló: {e:#}");
                }
                last_emitted = status.clone();
                polls_since_emit = 0;
            } else {
                polls_since_emit += 1;
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
    // (v3.0.0) Cuando SimConnect dice "conectado" PERO el proceso ya
    // NO está, asumimos que MSFS crasheó y todavía no se cerró la
    // conexión SimConnect del lado nuestro. Forzar sim_running=false
    // hace que el watcher principal entre en su heartbeat de
    // desconexión más rápido — la UI deja de "esperar" un sim que
    // no va a volver.
    let sc_connected = state.lock().await.status.simconnect_connected;
    let sim_running = if sc_connected && !sim_running_proc {
        // Conexión presente pero proceso ausente — dejamos
        // sim_running=true (sc_connected es señal autoritativa) sólo
        // por un tick más; en el siguiente ciclo del watcher principal
        // el heartbeat resetea sc_connected y caemos a false aquí.
        sc_connected
    } else {
        sim_running_proc || sc_connected
    };

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
    ///
    /// `eng_combustion_N` (v0.1.22): `ENG COMBUSTION:N` con units
    /// "bool" — 1.0 si el motor está encendido (combustión activa),
    /// 0.0 si apagado. Para aviones con menos motores que el max (4),
    /// los slots sobrantes leen 0.0 siempre. La condición "todos
    /// apagados" se cumple naturalmente cuando los slots reales caen
    /// a 0, sin tener que conocer NUMBER OF ENGINES.
    #[repr(C, packed(4))]
    #[derive(Debug, Default, Clone, Copy)]
    struct AircraftData {
        latitude_deg: f64,
        longitude_deg: f64,
        altitude_ft: f64,
        /// `INDICATED ALTITUDE` (v1.1.4) — la lectura del altímetro
        /// del avión (con QNH ajustado por encima/debajo de transition,
        /// 29.92" arriba). Esta es la altura que el piloto VE en el
        /// avión y por tanto la que reportamos en `currentAltFt`.
        /// `PLANE ALTITUDE` (MSL geométrica) sigue capturándose en
        /// `altitude_ft` para `max_altitude_ft` consistente.
        indicated_alt_ft: f64,
        ground_velocity_kt: f64,
        on_ground: f64, // bool en SimConnect viene como float64 0.0/1.0
        vertical_speed_fpm: f64,
        true_airspeed_kt: f64,
        /// `PLANE HEADING DEGREES TRUE` — rumbo verdadero 0..360°.
        /// Usado para rotar el icono del avión en el mapa en vivo.
        heading_deg: f64,
        eng_combustion_1: f64,
        eng_combustion_2: f64,
        eng_combustion_3: f64,
        eng_combustion_4: f64,
        /// `BRAKE PARKING POSITION` (bool) — 1.0 freno puesto, 0.0
        /// freno suelto. La transición 1.0→0.0 estando en tierra con
        /// motores corriendo dispara el OUT event (block-out, inicio
        /// del block time real al estilo ACARS).
        parking_brake: f64,
        /// `FUEL TOTAL QUANTITY WEIGHT` en libras (v0.1.25). Lo
        /// capturamos al OUT (initial) y al IN (final), y la
        /// diferencia → `fuel_used_kg` después de convertir lb→kg.
        fuel_total_weight_lb: f64,
        /// (v2.2.0) `PUSHBACK STATE` enum 0..3:
        ///   0 = NoPushback (in motion or stopped, no pushback)
        ///   1 = Pushing back (moving backwards via tug)
        ///   2 = Pushing forward (rare — tow forward)
        ///   3 = Stopped (pushback paused)
        /// Lo usamos para etiquetar EXPLÍCITAMENTE "pushback" en el
        /// phase_label cuando GSX o el ATC interno lo dispara, en vez
        /// de adivinar por threshold de velocidad.
        pushback_state: f64,
    }

    /// (v3.5.0) Struct compañero a `AircraftData` con los simvars
    /// STRING256 que describen la aeronave (TITLE / ATC TYPE / ATC
    /// AIRLINE / ATC ID).
    ///
    /// **BUG FIX post-rebuild**: la versión inicial incluía
    /// `ATC MODEL` como 5° campo, pero esa simvar **no existe** en
    /// el SimConnect SDK (la confundí con el campo `model=` del
    /// `aircraft.cfg`). SimConnect rechazaba el AddToDataDefinition
    /// con `DEFINITION_ERROR` y arrastraba al watcher a un estado
    /// zombi donde seguía logueando pausas pero no procesaba más
    /// Facility Data ni transiciones de phase. Sin `ATC MODEL`,
    /// derivamos el modelo de `TITLE` (que típicamente incluye el
    /// nombre del avión).
    ///
    /// SimConnect llena los 4 buffers contiguos en orden de
    /// AddToDataDefinition. Cada uno son **256 bytes de ASCII
    /// terminado en NUL** — no UTF-8 estricto pero `from_utf8_lossy`
    /// maneja addons rebeldes con caracteres extendidos.
    ///
    /// Se requestea con `PERIOD_SECOND + FLAG_DEFAULT`: emisión cada
    /// segundo para que tras un `start_flight()` el siguiente
    /// dispatch persista el meta en menos de 1s.
    #[repr(C, packed(4))]
    #[derive(Clone, Copy)]
    struct AircraftMeta {
        title: [u8; 256],
        atc_type: [u8; 256],
        atc_airline: [u8; 256],
        atc_id: [u8; 256],
    }

    impl Default for AircraftMeta {
        fn default() -> Self {
            Self {
                title: [0; 256],
                atc_type: [0; 256],
                atc_airline: [0; 256],
                atc_id: [0; 256],
            }
        }
    }

    /// Lee una cstring de un buffer de 256 bytes — corta en el
    /// primer NUL, trim, retorna None si queda vacío. SimConnect
    /// rellena el buffer con NULs después del string real.
    fn read_cstr(buf: &[u8; 256]) -> Option<String> {
        let end = buf.iter().position(|&b| b == 0).unwrap_or(256);
        if end == 0 { return None; }
        let s = String::from_utf8_lossy(&buf[..end]).trim().to_string();
        if s.is_empty() { None } else { Some(s) }
    }

    const DEFINE_ID_AIRCRAFT: u32 = 1;
    const REQUEST_ID_AIRCRAFT: u32 = 1;
    /// (v3.5.0) Meta strings de la aeronave — TITLE, ATC TYPE, ATC
    /// MODEL, ATC AIRLINE, ATC ID. Separamos del DEFINE_ID_AIRCRAFT
    /// porque mezclar STRING256 (256 bytes) con FLOAT64 (8 bytes) en
    /// un mismo `#[repr(C, packed(4))]` puede dar problemas de
    /// alignment según el SDK exacto. Lo pedimos como request aparte
    /// con periodo SECOND + FLAG_CHANGED (sólo emite cuando cambia).
    const DEFINE_ID_AIRCRAFT_META: u32 = 2;
    const REQUEST_ID_AIRCRAFT_META: u32 = 2;
    /// ID local arbitrario para el evento "Pause" — sólo nosotros
    /// usamos esta ID dentro del proceso, no choca con nada.
    const EVENT_ID_PAUSE: u32 = 100;
    /// Definición de campos para los parkings de un AIRPORT (v0.1.26).
    /// AddToFacilityDefinition se llama una vez con esta ID; MSFS la
    /// recuerda y la usa para todas las RequestFacilityData siguientes.
    const DEFINE_ID_AIRPORT_PARKING: u32 = 200;
    /// Ranges de request_id para las requests de parking. Los OUT
    /// usan 1000-1999, IN 2000-2999, preflight (v3.0.0) 3000-3999.
    /// Wrapping circular dentro del rango (el usuario rara vez hace
    /// > 1000 vuelos en una sesión).
    const REQUEST_ID_GATE_DEP_BASE: u32 = 1000;
    const REQUEST_ID_GATE_ARR_BASE: u32 = 2000;
    const REQUEST_ID_GATE_PRE_BASE: u32 = 3000;

    /// Cuántos ticks de gs < 1 kt hay que ver en `Landed` antes de
    /// cerrar el vuelo si los engines no se han apagado. Fallback para
    /// gliders y aviones cuya definición de motor MSFS no se traduce
    /// a `ENG COMBUSTION` (helicópteros eléctricos, ULMs raros).
    ///
    /// (v1.1.4) Watcher ahora corre a ~4Hz (SIM_FRAME, interval=15),
    /// así que 90 segundos = 360 ticks.
    const LANDED_IDLE_TIMEOUT_TICKS: u32 = 360;

    /// (v1.1.4) Tasa aproximada de muestreo en Hz — usada para
    /// convertir ticks⇔segundos en cálculos del watcher.
    const TICKS_PER_SECOND: u32 = 4;
    /// Ticks entre cada persist de live position + track point. ~10s
    /// igual que antes (10 ticks a 1Hz).
    const PERSIST_INTERVAL_TICKS: u32 = 10 * TICKS_PER_SECOND;
    /// Cada cuántos ticks emitimos `flight://current` al frontend.
    /// Sin throttle, 4Hz inunda la UI; con 4 ticks emitimos cada ~1s
    /// que es lo mismo que antes desde el punto de vista del usuario.
    const EMIT_INTERVAL_TICKS: u32 = TICKS_PER_SECOND;

    /// State machine para detectar el ciclo OOOI completo:
    /// `OnGround → BlockOut → Airborne → Landed → OnGround`.
    ///
    /// Cambio v0.1.22: añadida `Landed` para no cerrar el vuelo en
    /// pista; el cierre real es al engine shutdown.
    /// Cambio v0.1.23: añadida `BlockOut` para arrancar el vuelo al
    /// soltar el freno de mano (push-back) en lugar de al despegar.
    /// El `flight_time_s` ahora es block-to-block real — incluye taxi
    /// out, takeoff roll, vuelo, taxi in.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum FlightPhase {
        Disconnected,
        OnGround,
        /// OUT event disparado: freno suelto + motores running en
        /// tierra. `start_flight()` ya se llamó. Esperamos despegue
        /// o engine shutdown (cancelación de salida).
        BlockOut,
        Airborne,
        /// Touchdown completado, taxiando al gate. El vuelo sigue
        /// abierto en DB. Espera engine shutdown (o timeout idle).
        Landed,
    }

    /// Estado pendiente de una request de gate via Facility Data
    /// API (v0.1.26). Se crea cuando hacemos `RequestFacilityData`
    /// para el aeropuerto de origen (OUT/preflight) o destino (IN),
    /// se va llenando con cada `RECV_FACILITY_DATA` event, y se
    /// procesa en el `RECV_FACILITY_DATA_END`.
    ///
    /// (v3.0.0) `flight_id` es `Option`: las requests "preflight" se
    /// disparan ANTES del OUT (cuando el watcher detecta el avión en
    /// el gate con motores apagados), así que no hay flight_id aún.
    /// El resultado se guarda en el SharedState (`current_gate`)
    /// para que la UI lo muestre y luego, al OUT, lo copiamos al
    /// `flight_log.departure_gate`.
    struct PendingGate {
        flight_id: Option<i64>,
        /// "departure" | "arrival" | "preflight"
        role: &'static str,
        player_lat: f64,
        player_lon: f64,
        airport_icao: String,
        /// (v3.4.5) Centro del aeropuerto en coordenadas absolutas
        /// (degrees). Se popula al recibir el primer
        /// `SIMCONNECT_RECV_FACILITY_DATA` con type=AIRPORT.
        /// Las coordenadas absolutas de cada parking se calculan
        /// como airport_center + bias_offset.
        airport_lat: Option<f64>,
        airport_lon: Option<f64>,
        parkings: Vec<ParkingSpot>,
    }

    /// Una entrada de TAXI_PARKING parseada del buffer de
    /// SIMCONNECT_RECV_FACILITY_DATA. Layout esperado del buffer
    /// (orden de AddToFacilityDefinition — v3.4.5):
    ///   [u32 TYPE enum] [u32 NAME enum] [u32 NUMBER] [u32 SUFFIX]
    ///   [f64 BIAS_X] [f64 BIAS_Y]
    /// Total 4×4 + 8×2 = 32 bytes.
    ///
    /// `BIAS_X` (east) y `BIAS_Y` (north) son offsets en METROS
    /// desde el centro del aeropuerto — para convertirlos a degrees:
    ///   d_lat = bias_y / 111_320
    ///   d_lon = bias_x / (111_320 * cos(airport_lat_rad))
    #[derive(Debug, Clone)]
    struct ParkingSpot {
        /// SIMCONNECT_AIRPORT_PARKING_TYPE — categoría del parking
        /// (GATE_SMALL/MEDIUM/HEAVY, RAMP_GA, CARGO, MIL, DOCK_GA…).
        /// La usamos para decidir prefix "Gate" vs "Ramp" vs "Stand".
        type_id: u32,
        /// SIMCONNECT_AIRPORT_PARKING_NAME — letra o cardinal del
        /// parking. 13..=38 son GATE_A..GATE_Z; 2..=10 son
        /// PARKING/N_PARKING/NE_PARKING/…; 11=GATE genérico; 12=DOCK.
        name_id: u32,
        number: u32,
        suffix: u32,
        /// (v3.4.5) BIAS_X — offset este en metros desde el centro
        /// del aeropuerto. NOTA: antes leíamos LATITUDE directo pero
        /// SimConnect lo rechazaba con DATA_ERROR — TAXI_PARKING no
        /// tiene esos campos en el SDK MSFS 2020.
        bias_x: f64,
        /// BIAS_Y — offset norte en metros desde el centro del aeropuerto.
        bias_y: f64,
    }

    impl ParkingSpot {
        /// (v3.0.0) Formato legible — "A34" / "B12" / "Stand 3" /
        /// "Ramp 4". Antes (v2.x) leíamos sólo `NAME` y lo
        /// interpretábamos como TYPE, lo que daba "Stand · 341° 855m"
        /// genérico. Ahora separamos TYPE (categoría) y NAME (letra),
        /// y formateamos como lo hace el sim:
        ///   · GATE_A..GATE_Z + número  →  "A34", "C7B"
        ///   · GATE genérico + número   →  "Gate 5"
        ///   · PARKING cardinal         →  "NE Parking 12"
        ///   · RAMP/DOCK_GA             →  "Ramp 4" / "Dock 2"
        fn display_name(&self) -> String {
            // Letra del NAME enum (13..=38 = A..Z) si aplica.
            let letter_from_name: Option<char> = match self.name_id {
                n if (13..=38).contains(&n) => {
                    Some((b'A' + (n - 13) as u8) as char)
                }
                _ => None,
            };
            // Sufijo opcional (1=A, 2=B…).
            let suffix_char: String = match self.suffix {
                n if (1..=26).contains(&n) => {
                    ((b'A' + (n as u8 - 1)) as char).to_string()
                }
                _ => String::new(),
            };

            // 1) Gates con letra explícita en NAME → "A34" / "C7B".
            if let Some(letter) = letter_from_name {
                if self.number == 0 && suffix_char.is_empty() {
                    return format!("Gate {}", letter);
                }
                return format!("{}{}{}", letter, self.number, suffix_char);
            }

            // 2) Prefijo desde el NAME enum (cardinales + genéricos).
            let name_prefix = match self.name_id {
                2 => "Parking",
                3 => "N Parking",
                4 => "NE Parking",
                5 => "E Parking",
                6 => "SE Parking",
                7 => "S Parking",
                8 => "SW Parking",
                9 => "W Parking",
                10 => "NW Parking",
                11 => "Gate",
                12 => "Dock",
                _ => "",
            };

            // 3) Si NAME no nos dijo nada útil, usamos TYPE como fallback.
            let prefix = if !name_prefix.is_empty() {
                name_prefix.to_string()
            } else {
                match self.type_id {
                    2..=5 => "Ramp".to_string(),     // RAMP_GA*
                    6 => "Cargo".to_string(),         // RAMP_CARGO
                    7 | 8 => "Military".to_string(),  // RAMP_MIL_*
                    9..=11 => "Gate".to_string(),     // GATE_SMALL/MED/HEAVY
                    12 => "Dock".to_string(),         // DOCK_GA
                    13 => "Fuel".to_string(),
                    14 => "Vehicles".to_string(),
                    _ => "Stand".to_string(),
                }
            };

            if self.number == 0 && suffix_char.is_empty() {
                prefix
            } else {
                format!("{} {}{}", prefix, self.number, suffix_char)
            }
        }
    }

    /// (v2.0.3) Snapshot del estado de un vuelo restaurado, usado para
    /// validar al recibir la primera muestra de SimConnect si el
    /// player está "donde lo dejamos" o se trata de una sesión nueva.
    #[derive(Debug, Clone)]
    struct RestoreCheck {
        flight_id: i64,
        last_lat: f64,
        last_lon: f64,
        last_at: Option<String>,
    }

    /// (v2.0.3) Decide si el restore es válido. Si el player está a
    /// >50nm del último punto guardado, O han pasado >6h desde el
    /// último tick, lo consideramos huérfano: cerramos el vuelo viejo
    /// y reseteamos el state para empezar fresco.
    fn restore_is_stale(check: &RestoreCheck, cur_lat: f64, cur_lon: f64) -> bool {
        let dist =
            crate::flight_log::haversine_nm(check.last_lat, check.last_lon, cur_lat, cur_lon);
        if dist > 50.0 {
            tracing::info!(
                target: "simconnect",
                "restore stale por distancia: {:.1} nm desde último punto",
                dist
            );
            return true;
        }
        if let Some(ref ts) = check.last_at {
            if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(ts, "%Y-%m-%d %H:%M:%S") {
                let then = dt.and_utc();
                let elapsed = chrono::Utc::now()
                    .signed_duration_since(then)
                    .num_seconds();
                if elapsed > 6 * 60 * 60 {
                    tracing::info!(
                        target: "simconnect",
                        "restore stale por tiempo: {} s desde último tick",
                        elapsed
                    );
                    return true;
                }
            }
        }
        false
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
        //
        // (v3.0.0) Limpia TODOS los campos derivados de la conexión
        // — incluyendo `phase_label`, que antes quedaba con el último
        // valor ("cruise", "approach"…) cuando MSFS crasheaba. Ahora
        // la UI ve `phaseLabel = null` + `simconnectConnected = false`
        // y el badge "Flying now" desaparece o cae al fallback de
        // proceso-only.
        for _ in 0..50 {
            if let Ok(mut guard) = state.try_lock() {
                guard.status.simconnect_connected = false;
                guard.status.current_lat = None;
                guard.status.current_lon = None;
                guard.status.current_alt_ft = None;
                guard.status.current_ground_speed_kt = None;
                guard.status.current_heading_deg = None;
                guard.status.on_ground = None;
                guard.status.phase_label = None;
                guard.status.current_gate = None;
                guard.status.current_airport_icao = None;
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
        let app_name = sc::cstr("SimFleet");
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
        let units_pounds = sc::cstr("pounds");

        let names: &[(&str, &std::ffi::CStr)] = &[
            ("PLANE LATITUDE", units_deg.as_c_str()),
            ("PLANE LONGITUDE", units_deg.as_c_str()),
            ("PLANE ALTITUDE", units_feet.as_c_str()),
            // (v1.1.4) Indicated altitude — la del altímetro del avión.
            // El usuario reportó que en FL400 la app marcaba 41,619 ft,
            // esa diferencia es exactamente lo que pasa con PLANE ALTITUDE
            // (MSL geométrica) vs INDICATED ALTITUDE (con QNH 29.92" en FL).
            ("INDICATED ALTITUDE", units_feet.as_c_str()),
            ("GROUND VELOCITY", units_knots.as_c_str()),
            ("SIM ON GROUND", units_bool.as_c_str()),
            // VERTICAL SPEED en feet/minute — lo necesitamos para
            // capturar el FPM del touchdown.
            ("VERTICAL SPEED", units_fpm.as_c_str()),
            // True airspeed — para tracking de velocidad máxima
            // independiente del viento.
            ("AIRSPEED TRUE", units_knots.as_c_str()),
            // (v1.1.4) Heading verdadero — rota el icono del avión
            // en el mapa en vivo del FlightBook.
            ("PLANE HEADING DEGREES TRUE", units_deg.as_c_str()),
            // Engine combustion 1..4 (v0.1.22) — bool 0/1 por motor.
            // Para "todos apagados" sumamos los 4; en aviones con
            // menos de 4 motores los slots sobrantes leen 0.0 siempre,
            // así que la suma cae a 0 cuando los motores reales se
            // apagan. Cierre del vuelo OOOI (on-block) depende de
            // esto.
            ("ENG COMBUSTION:1", units_bool.as_c_str()),
            ("ENG COMBUSTION:2", units_bool.as_c_str()),
            ("ENG COMBUSTION:3", units_bool.as_c_str()),
            ("ENG COMBUSTION:4", units_bool.as_c_str()),
            // (v0.1.23) Parking brake — para detectar OUT (block-out).
            // Release del freno con motores corriendo en tierra = OUT.
            // Real ACARS: "Out of gate".
            ("BRAKE PARKING POSITION", units_bool.as_c_str()),
            // (v0.1.25) Fuel total weight — capturado al OUT y al IN
            // para reportar consumo de combustible (kg) por vuelo.
            ("FUEL TOTAL QUANTITY WEIGHT", units_pounds.as_c_str()),
            // (v2.2.0) Pushback state — enum 0..3. Usado para
            // etiquetar fase "pushback" explícitamente cuando GSX
            // (o el ATC interno) lo dispara, en vez de adivinar
            // por threshold de velocidad.
            ("PUSHBACK STATE", units_bool.as_c_str()),
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

        // (v1.1.4) Subscribe a SIM_FRAME con interval=15 — eso es ~4Hz
        // a 60fps típicos de MSFS (un sample cada ~250ms). Pasamos del
        // SECOND anterior (1Hz) a 4Hz porque el usuario reportó
        // que el FPM al aterrizar marcaba -103 cuando el real era -300:
        // a 1Hz el sample posiblemente caía en el flare (justo antes
        // de touchdown) o post-touchdown, perdiendo el peak. Con 4Hz
        // tenemos 4× más resolución para capturar el momento exacto.
        let hr = unsafe {
            (lib.RequestDataOnSimObject)(
                handle,
                REQUEST_ID_AIRCRAFT,
                DEFINE_ID_AIRCRAFT,
                SIMCONNECT_OBJECT_ID_USER,
                SIMCONNECT_PERIOD_SIM_FRAME,
                SIMCONNECT_DATA_REQUEST_FLAG_DEFAULT,
                0,
                15,
                0,
            )
        };
        if !sc::succeeded(hr) {
            anyhow::bail!("RequestDataOnSimObject falló (0x{:08x})", hr);
        }

        // (v3.5.0) Meta strings de la aeronave — TITLE, ATC TYPE,
        // ATC AIRLINE, ATC ID. STRING256 cada uno. Orden **debe
        // coincidir** con la disposición de `AircraftMeta` arriba.
        //
        // El usuario explícitamente pidió que dejemos de depender de
        // APIs externas (SimBrief) o nombres de carpeta para
        // identificar el avión: estas son las variables CANÓNICAS de
        // SimConnect, las mismas que MSFS expone al pilot ATC.
        //
        // - `ATC ID` reemplaza a `ATC TAIL NUMBER` (deprecada en MSFS
        //   2024); en MSFS 2020 ambas valen lo mismo así que usar
        //   `ATC ID` es portable.
        // - `TITLE` es el path interno del livery (ej. "Asobo A320
        //   Neo Iberia"); útil cuando el usuario quiere distinguir
        //   liveries. También sirve para derivar el "modelo" porque
        //   la simvar `ATC MODEL` **no existe** en el SDK (rejected
        //   con DEFINITION_ERROR + DATA_ERROR — la app cae en estado
        //   zombi después).
        //
        // NOTA importante sobre `UnitsName`: el SDK documenta NULL
        // para tipos string. Usamos `ptr::null()` directo en vez de
        // un puntero a string vacío — SimConnect parecía aceptar
        // "" pero el path seguro es null.
        let meta_names = &[
            "TITLE",
            "ATC TYPE",
            "ATC AIRLINE",
            "ATC ID",
        ];
        let mut meta_defs_ok = true;
        for name in meta_names {
            let n = sc::cstr(name);
            let hr = unsafe {
                (lib.AddToDataDefinition)(
                    handle,
                    DEFINE_ID_AIRCRAFT_META,
                    n.as_ptr(),
                    ptr::null(),
                    sc::SIMCONNECT_DATATYPE_STRING256,
                    0.0,
                    u32::MAX,
                )
            };
            if !sc::succeeded(hr) {
                tracing::warn!(
                    target: "simconnect",
                    "AddToDataDefinition meta '{}' falló (0x{:08x}); aircraft meta deshabilitado",
                    name, hr
                );
                meta_defs_ok = false;
                break;
            }
        }
        if meta_defs_ok {
            // PERIOD_SECOND + FLAG_DEFAULT — emisión cada segundo
            // independiente de cambios. Payload de 1280 bytes/s es
            // despreciable comparado al bandwidth típico de SimConnect.
            // Necesitamos emisión continua para garantizar que tras
            // un `start_flight()` (creado con title=None por timing
            // race entre el dispatch del meta y el callsite del OUT)
            // el siguiente dispatch en <1s persista los strings vía
            // `update_aircraft_meta`.
            let hr = unsafe {
                (lib.RequestDataOnSimObject)(
                    handle,
                    REQUEST_ID_AIRCRAFT_META,
                    DEFINE_ID_AIRCRAFT_META,
                    SIMCONNECT_OBJECT_ID_USER,
                    SIMCONNECT_PERIOD_SECOND,
                    SIMCONNECT_DATA_REQUEST_FLAG_DEFAULT,
                    0,
                    0,
                    0,
                )
            };
            if !sc::succeeded(hr) {
                tracing::warn!(
                    target: "simconnect",
                    "RequestDataOnSimObject meta falló (0x{:08x})",
                    hr
                );
            } else {
                tracing::info!(
                    target: "simconnect",
                    "AircraftMeta subscrito (TITLE, ATC TYPE, AIRLINE, ID) — emit cada 1s (FLAG_DEFAULT)"
                );
            }
        }

        // (v0.1.25) Subscribe al evento "Pause" para no contar el
        // tiempo pausado dentro del block-time del vuelo. El evento
        // dispara con dwData = 1 al pausar y 0 al despausar.
        let event_pause_name = sc::cstr("Pause");
        let hr = unsafe {
            (lib.SubscribeToSystemEvent)(
                handle,
                EVENT_ID_PAUSE,
                event_pause_name.as_ptr(),
            )
        };
        if !sc::succeeded(hr) {
            // No-fatal: si falla la suscripción seguimos sin pause
            // detection (las pausas contarán al block time como antes).
            tracing::warn!(
                target: "simconnect",
                "SubscribeToSystemEvent('Pause') falló (0x{:08x}); pausa no detectable",
                hr
            );
        } else {
            tracing::info!(target: "simconnect", "evento 'Pause' suscrito OK");
        }

        // (v0.1.26 / refactor v3.0.0) Define una vez los campos de
        // TAXI_PARKING. La jerarquía AIRPORT se construye con
        // OPEN/CLOSE markers + fields del child. El orden es el orden
        // en que MSFS escribe los bytes en el buffer del evento.
        //
        // Field types del SDK oficial. **v3.4.4 BUG FIX**: el TAXI_PARKING
        // NO tiene LATITUDE/LONGITUDE directos como yo asumí — SimConnect
        // los rechaza con DATA_ERROR (code=20, sendID=31/32 en el log
        // del usuario). Las coordenadas absolutas se calculan así:
        //   absolute_lat = airport.LATITUDE  + (parking.BIAS_Y / m_per_deg_lat)
        //   absolute_lon = airport.LONGITUDE + (parking.BIAS_X / m_per_deg_lon)
        // BIAS_X y BIAS_Y son offsets en METROS desde el centro del
        // aeropuerto (X=este+, Y=norte+).
        //
        //   AIRPORT level fields:
        //     LATITUDE   → FLOAT64 (degrees) — centro del aeropuerto
        //     LONGITUDE  → FLOAT64 (degrees) — centro del aeropuerto
        //
        //   TAXI_PARKING level fields:
        //     TYPE       → DWORD enum SIMCONNECT_AIRPORT_PARKING_TYPE
        //                  (RAMP_GA, GATE_SMALL/MED/HEAVY, DOCK_GA, …).
        //     NAME       → DWORD enum SIMCONNECT_AIRPORT_PARKING_NAME
        //                  (PARKING, N_PARKING…NW_PARKING, GATE, DOCK,
        //                   GATE_A…GATE_Z = 13..=38).
        //     NUMBER     → DWORD
        //     SUFFIX     → DWORD letter (0=none, 1=A...)
        //     BIAS_X     → FLOAT64 (meters east of airport center)
        //     BIAS_Y     → FLOAT64 (meters north of airport center)
        // (v3.4.6) **Pivot a GSX**: SimConnect Facility Data se reveló
        // como buggy/incompleto en la DLL bundled de MSFS 2020 — 5
        // hotfixes consecutivos fueron destapando bugs distintos
        // (ATC MODEL inexistente, STRING256 type ID, RECV_ID_FACILITY
        // constants, LATITUDE/LONGITUDE en TAXI_PARKING, BIAS_Y). Cada
        // fix destapó el siguiente. La DLL claramente está incompleta
        // para esta API.
        //
        // Decisión: deshabilitar TAXI_PARKING completamente. Mantenemos
        // sólo AIRPORT (LATITUDE/LONGITUDE) como sanity check del
        // request flow. El gate name viene ahora del módulo WASM de
        // GSX vía Client Data Area `FSDT_GSX_AIRCRAFT_DATA` (set up
        // más abajo).
        let mut facility_def_ok = false;
        if let Some(add_fac) = lib.AddToFacilityDefinition {
            let fields = &[
                "OPEN AIRPORT",
                "LATITUDE",
                "LONGITUDE",
                "CLOSE AIRPORT",
            ];
            let mut all_ok = true;
            for fname in fields {
                let cname = sc::cstr(fname);
                let hr = unsafe {
                    add_fac(handle, DEFINE_ID_AIRPORT_PARKING, cname.as_ptr())
                };
                if !sc::succeeded(hr) {
                    tracing::warn!(
                        target: "simconnect",
                        "AddToFacilityDefinition '{}' falló (0x{:08x}); gates reales deshabilitados",
                        fname,
                        hr
                    );
                    all_ok = false;
                    break;
                }
            }
            facility_def_ok = all_ok;
            if all_ok {
                tracing::info!(
                    target: "simconnect",
                    "AIRPORT/TAXI_PARKING facility definition registrada OK ({} fields)",
                    fields.len()
                );
            }
        } else {
            tracing::warn!(
                target: "simconnect",
                "SimConnect.dll no exporta AddToFacilityDefinition; gates reales deshabilitados"
            );
        }

        // (v3.4.7 → eliminado en v3.4.15) **GSX Client Data Area
        // discovery removido**. Probamos 4 CDA names candidatos
        // (`FSDT_GSX_AIRCRAFT_DATA`, `FSDT_GSX_MENU`,
        // `FSDT_GSX_PIPE_TO_PLANE`, `FSDT_GSX_BYPASS_PIN`) y los 4
        // retornaban `ILLEGAL_OPERATION` async porque GSX no
        // publica esos CDAs con esos nombres en la sesión actual.
        // Eso spammeaba 4 EXCEPTIONs/segundo al log y nunca aportó
        // valor — los gates funcionales vienen de `gsx_parking.rs`
        // que lee los INIs del disco. Cleanup aquí.
        //
        // El dispatch handler para `SIMCONNECT_RECV_ID_CLIENT_DATA`
        // se mantiene (no estorba) por si en una iteración futura
        // queremos suscribirnos a algún CDA específico — pero por
        // ahora no se llama nadie.

        // Marca conectado.
        update_connected(state, app, true);

        // Poll loop. SimConnect API es pull — llamamos
        // `GetNextDispatch` repetidamente. Devuelve E_FAIL cuando no
        // hay nada (lo cual es normal); sólo mensajes con dwID
        // específico nos interesan.
        //
        // **v3.0.0 — Heartbeat / crash detection**. MSFS puede:
        //   · Cerrarse limpio: dispara `RECV_ID_QUIT` y salimos.
        //   · Crashear (Alt+F4, CTD, blue-screen del addon): NO
        //     dispara QUIT — el dispatch simplemente devuelve E_FAIL
        //     para siempre. Antes nos quedábamos colgados haciendo
        //     poll eterno y la UI mostraba el último estado como si
        //     siguiera volando.
        //
        // Nueva lógica:
        //   1. Trackeamos `last_dispatch_at` cada vez que recibimos
        //      un mensaje real (no E_FAIL).
        //   2. Cada ~3s sin mensajes válidos chequeamos el proceso
        //      FlightSimulator.exe / FlightSimulator2024.exe.
        //   3. Si no está corriendo → sim crasheó → rompemos el loop,
        //      el caller llama `mark_disconnected` y arranca el retry.
        let mut last_dispatch_at = std::time::Instant::now();
        let mut last_process_check_at = std::time::Instant::now();
        let mut phase = FlightPhase::Disconnected;
        // El id del vuelo activo en `flight_log`. Lo envolvemos en
        // `Arc<Mutex>` para poder pasarlo a tareas async (donde se
        // hace start/finish del vuelo) y leer el resultado de
        // vuelta. Sin esto las async tasks que escriben en él no
        // serían `Send`.
        let current_flight_id: std::sync::Arc<std::sync::Mutex<Option<i64>>> =
            std::sync::Arc::new(std::sync::Mutex::new(None));

        // **Restauración ACARS-like**: al iniciar el watcher, si la
        // DB tiene un vuelo abierto reciente, lo reanudamos. Eso
        // resuelve "?" como origen cuando la app se cerró a mitad
        // de vuelo y se reabrió.
        //
        // Usamos `tokio::runtime::Handle::current().block_on()` porque
        // este hilo es blocking (spawn_blocking) y necesitamos
        // ejecutar el query async aquí. El handle se hereda del
        // runtime que llamó spawn_blocking.
        let restore_result = std::thread::scope(|s| {
            let handle = s.spawn(|| {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .ok()?;
                rt.block_on(crate::flight_log::latest_open_flight(pool)).ok()?
            });
            handle.join().ok().flatten()
        });
        let mut max_alt_ft: i64 = 0;
        let mut max_gs_kt: i64 = 0;
        let mut max_tas_kt: i64 = 0;
        let mut ticks_since_persist: u32 = 0;
        // (v1.1.4) Throttle del emit del frontend — el watcher samplea
        // a 4Hz pero la UI sólo necesita un emit/segundo.
        let mut ticks_since_emit: u32 = 0;
        // Ventana de los últimos VS leídos — al detectar touchdown
        // elegimos el MÁS negativo como "landing FPM". SimConnect
        // a veces emite un 0 espurio en el cambio de fase, así que
        // tomar el mínimo de la ventana evita ese artefacto.
        // (v1.1.4) A 4Hz, 24 samples = ~6s — cubre el flare y la
        // primera fracción de segundo tras touchdown. El usuario
        // reportó que con la ventana antigua (4s a 1Hz) capturaba
        // -103 cuando el real era -300.
        let mut recent_vs: std::collections::VecDeque<f64> =
            std::collections::VecDeque::with_capacity(24);
        // (v0.1.25) Combustible inicial en lb (al OUT). Si SimConnect
        // se conectó después del OUT, queda None y no reportamos fuel.
        let mut initial_fuel_lb: Option<f64> = None;
        // (v0.1.25) Acumulador de tiempo pausado en segundos.
        // Crece cada vez que el sim recibe Unpause con un Pause
        // previo activo. Se resta del flight_time_s al finish.
        let mut paused_seconds_total: u64 = 0;
        // Marca de cuándo empezó la pausa actual; None = sim no
        // pausado. Lo gestiona el handler del SIMCONNECT_RECV_EVENT
        // para `Pause` (dwData = 1 → start, 0 → end).
        let mut paused_since: Option<std::time::Instant> = None;
        // (v1.1.2) Flag que se enciende cuando en BlockOut cruzamos
        // > 10 kt — significa que el pushback ya terminó. Lo usa el
        // phase_label para no volver a etiquetar "pushback" si nos
        // detenemos (esperando ATC, backtrack en pista, etc.).
        let mut passed_taxi_threshold = false;
        // (v0.1.26) Requests de parking activas. Cada entrada es:
        //   request_id → PendingGate { flight_id, role, player lat/lon, parkings recolectados }
        // MSFS envía varios FACILITY_DATA events seguidos de un
        // FACILITY_DATA_END. Al recibir END procesamos la lista,
        // elegimos el parking más cercano al player saved, y
        // actualizamos flight_log.departure_gate / arrival_gate.
        let mut pending_gates: std::collections::HashMap<u32, PendingGate> =
            std::collections::HashMap::new();
        // Contador monótono dentro de cada base. Wrapping al máximo
        // del rango (~1000 requests por sesión es más que de sobra).
        let mut next_gate_seq_dep: u32 = 0;
        let mut next_gate_seq_arr: u32 = 0;
        let mut next_gate_seq_pre: u32 = 0;
        // (v3.0.0) Throttle del preflight gate detection — re-disparamos
        // cada vez que el avión cambia de aeropuerto en tierra. Si
        // está parado en el mismo gate sólo lo pedimos UNA vez.
        let mut preflight_gate_airport: Option<String> = None;
        let mut preflight_gate_at: Option<std::time::Instant> = None;
        // (v3.4.11) Posición del player en la última request de gate
        // (preflight + arrival). Permite re-disparar la detección
        // cuando el avión se movió significativamente desde la última
        // vez — ej. taxi de un gate inicial a otro gate del mismo
        // aeropuerto, o taxi a gate destino tras el touchdown.
        let mut last_gate_request_pos: Option<(f64, f64)> = None;
        let mut last_arrival_check_at = std::time::Instant::now()
            .checked_sub(Duration::from_secs(60))
            .unwrap_or_else(std::time::Instant::now);
        let mut last_known_airport_icao: Option<String> = None;
        // (v3.5.0) BUG FIX — antes inicializábamos a `Instant::now()`, lo
        // que dejaba la detección de gate MUTE durante los primeros 10s
        // tras conectar SimConnect. Si el usuario hacía spawn cold &
        // dark y abría SimFleet justo después, el gate no aparecía
        // hasta unos segundos más tarde y a veces se perdía si el
        // usuario empezaba a moverse antes. Restamos 60s para garantizar
        // que el PRIMER tick con `gs<1 + pos_real` dispare la request
        // inmediatamente — la primera muestra de SimConnect ya trae al
        // avión en su gate. El throttle de 10s sólo aplica entre
        // requests sucesivas, no a la inicial.
        let mut last_airport_check_at = std::time::Instant::now()
            .checked_sub(Duration::from_secs(60))
            .unwrap_or_else(std::time::Instant::now);
        // (v0.1.22) Capturamos el landing_fpm en el touchdown
        // (Airborne → Landed) y lo conservamos aquí hasta el finish.
        // También lo persistimos en DB inmediatamente vía
        // `touch_landing` para no perderlo si la app crashea entre
        // touchdown y engine shutdown.
        let mut captured_landing_fpm: Option<i64> = None;
        // True si vimos al menos un motor encendido durante este
        // vuelo. Sin esto, la condición "todos los motores apagados"
        // sería trivialmente cierta al arrancar el sim antes de
        // encender motores — y dispararía un finish espurio. También
        // se setea a true en el restore por seguridad (un vuelo
        // abierto en DB implica que en algún momento había engines
        // running).
        let mut engines_seen_running = false;
        // Ticks consecutivos en Landed con gs < 1 kt. Fallback para
        // cerrar gliders/eléctricos que nunca disparan ENG COMBUSTION
        // o usuarios que se olvidan de apagar motores y dejan la
        // app abierta.
        let mut idle_ticks_in_landed: u32 = 0;

        // (v3.5.0) Cache de meta strings de la aeronave — actualizado
        // cuando llega un dispatch de REQUEST_ID_AIRCRAFT_META.
        // Lo usamos al disparar `start_flight` (OUT/OFF) para
        // poblar de entrada los campos aircraft_title / atc_type /
        // model / airline / registration, y vuelve a persistirse vía
        // `update_aircraft_meta` cuando cambia mid-flight (el usuario
        // entró al Hangar y cambió livery).
        let mut aircraft_meta_cache: Option<AircraftMeta> = None;

        // (v2.0.3) Datos del restore pendiente — los necesitamos para
        // hacer el "smart restore check" al recibir la primera muestra
        // de SimConnect: si el player está LEJOS de la última posición
        // guardada (>50 nm) o ha pasado mucho tiempo desde el último
        // tick (>6h), tratamos el vuelo viejo como huérfano y empezamos
        // limpio. Sin esto, restaurábamos el vuelo y al despegar
        // sumábamos distancias absurdas (e.g. EBBR→KJFK volviéndose
        // EBBR→Dubai porque el usuario voló otra cosa).
        let mut restore_check: Option<RestoreCheck> = None;
        if let Some(open) = restore_result {
            tracing::info!(
                target: "simconnect",
                "RESTAURANDO vuelo abierto id={} origen={:?} (started_at={})",
                open.id,
                open.origin_icao,
                open.started_at
            );
            if let Ok(mut g) = current_flight_id.lock() {
                *g = Some(open.id);
            }
            phase = FlightPhase::Airborne;
            // Si la fila tenía max_altitude_ft, lo seedeamos para
            // que no sobreescribamos con un valor menor al primer
            // tick (caso descenso).
            if let Some(alt) = open.max_altitude_ft {
                if alt > 0 {
                    max_alt_ft = alt;
                }
            }
            engines_seen_running = true;

            if let (Some(lat), Some(lon)) =
                (open.last_position_lat, open.last_position_lon)
            {
                restore_check = Some(RestoreCheck {
                    flight_id: open.id,
                    last_lat: lat,
                    last_lon: lon,
                    last_at: open.last_position_at.clone(),
                });
            }
        } else {
            tracing::debug!(target: "simconnect", "no hay vuelo abierto previo — empezamos limpio");
        }

        loop {
            let mut p_data: *mut sc::SIMCONNECT_RECV = ptr::null_mut();
            let mut cb_data: u32 = 0;
            let hr = unsafe {
                (lib.GetNextDispatch)(handle, &mut p_data, &mut cb_data)
            };
            if !sc::succeeded(hr) {
                // E_FAIL = nada en cola. Aquí hacemos el heartbeat:
                // si llevamos > 5s sin mensajes Y el proceso de MSFS
                // ya no está corriendo, asumimos crash y rompemos el
                // loop para que el caller reinicie la conexión.
                if last_process_check_at.elapsed() >= Duration::from_secs(3)
                    && last_dispatch_at.elapsed() >= Duration::from_secs(5)
                {
                    last_process_check_at = std::time::Instant::now();
                    if !super::detect_sim_running() {
                        tracing::warn!(
                            target: "simconnect",
                            "MSFS no responde (sin dispatch en {}s + proceso ausente) — asumiendo crash; cerrando conexión",
                            last_dispatch_at.elapsed().as_secs()
                        );
                        // Si quedó un vuelo abierto, no lo cerramos
                        // automáticamente — el restore del próximo
                        // arranque decidirá. Pero limpiamos el state
                        // visible para que la UI deje de mostrar el
                        // avión "en vuelo".
                        return Ok(());
                    }
                }
                std::thread::sleep(Duration::from_millis(80));
                continue;
            }
            if p_data.is_null() {
                std::thread::sleep(Duration::from_millis(80));
                continue;
            }

            // Recibimos algo válido → reseteamos el heartbeat.
            last_dispatch_at = std::time::Instant::now();
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
                    // (v2.0.0) Loggeamos también `dwSendID` y `dwIndex`
                    // para correlacionar la excepción con el send que
                    // la disparó. Esencial para diagnosticar problemas
                    // de RequestFacilityData (errores típicos: 22 =
                    // UNRECOGNIZED_ID, 30 = NAME_UNRECOGNIZED, etc).
                    let exc_code = exc.dwException;
                    let send_id = exc.dwSendID;
                    let index = exc.dwIndex;
                    let label = match exc_code {
                        1 => "ERROR",
                        2 => "SIZE_MISMATCH",
                        3 => "UNRECOGNIZED_ID",
                        4 => "UNOPENED",
                        5 => "VERSION_MISMATCH",
                        6 => "TOO_MANY_GROUPS",
                        7 => "NAME_UNRECOGNIZED",
                        8 => "TOO_MANY_EVENT_NAMES",
                        9 => "EVENT_ID_DUPLICATE",
                        10 => "TOO_MANY_MAPS",
                        11 => "TOO_MANY_OBJECTS",
                        12 => "TOO_MANY_REQUESTS",
                        13 => "WEATHER_INVALID_PORT",
                        14 => "WEATHER_INVALID_METAR",
                        15 => "WEATHER_UNABLE_TO_GET_OBSERVATION",
                        16 => "WEATHER_UNABLE_TO_CREATE_STATION",
                        17 => "WEATHER_UNABLE_TO_REMOVE_STATION",
                        18 => "INVALID_DATA_TYPE",
                        19 => "INVALID_DATA_SIZE",
                        20 => "DATA_ERROR",
                        21 => "INVALID_ARRAY",
                        22 => "CREATE_OBJECT_FAILED",
                        23 => "LOAD_FLIGHTPLAN_FAILED",
                        24 => "OPERATION_INVALID_FOR_OBJECT_TYPE",
                        25 => "ILLEGAL_OPERATION",
                        26 => "ALREADY_SUBSCRIBED",
                        27 => "INVALID_ENUM",
                        28 => "DEFINITION_ERROR",
                        29 => "DUPLICATE_ID",
                        30 => "DATUM_ID",
                        31 => "OUT_OF_BOUNDS",
                        32 => "ALREADY_CREATED",
                        33 => "OBJECT_OUTSIDE_REALITY_BUBBLE",
                        34 => "OBJECT_CONTAINER",
                        35 => "OBJECT_AI",
                        36 => "OBJECT_ATC",
                        37 => "OBJECT_SCHEDULE",
                        _ => "OTHER",
                    };
                    tracing::warn!(
                        target: "simconnect",
                        "EXCEPTION code={} ({}) sendID={} index={}",
                        exc_code, label, send_id, index
                    );
                }
                SIMCONNECT_RECV_ID_EVENT => {
                    // (v0.1.25) Eventos de sistema — actualmente sólo
                    // suscribimos "Pause". dwData = 1 → sim pausado,
                    // 0 → sim corriendo.
                    let evt = unsafe {
                        &*(p_data as *const sc::SIMCONNECT_RECV_EVENT)
                    };
                    if evt.uEventID == EVENT_ID_PAUSE {
                        let paused = evt.dwData == 1;
                        if paused && paused_since.is_none() {
                            paused_since = Some(std::time::Instant::now());
                            tracing::info!(target: "simconnect", "sim PAUSADO");
                        } else if !paused {
                            if let Some(start) = paused_since.take() {
                                let secs = start.elapsed().as_secs();
                                paused_seconds_total =
                                    paused_seconds_total.saturating_add(secs);
                                tracing::info!(
                                    target: "simconnect",
                                    "sim DESPAUSADO — esta pausa {}s, total acumulado {}s",
                                    secs,
                                    paused_seconds_total
                                );
                            }
                        }
                    }
                }
                SIMCONNECT_RECV_ID_SIMOBJECT_DATA => {
                    // Acceso a los datos: el struct comienza en el
                    // offset de `dwData` (que es donde colocamos el
                    // marker `[DWORD; 1]`).
                    let header = p_data as *const sc::SIMCONNECT_RECV_SIMOBJECT_DATA;
                    let request_id = unsafe { (*header).dwRequestID };

                    // (v3.5.0) Meta strings de la aeronave — caché +
                    // persist a DB si hay vuelo abierto.
                    if request_id == REQUEST_ID_AIRCRAFT_META {
                        let data_ptr = unsafe {
                            let header_size =
                                std::mem::size_of::<sc::SIMCONNECT_RECV_SIMOBJECT_DATA>();
                            let base = p_data as *const u8;
                            base.add(header_size - 4) as *const AircraftMeta
                        };
                        if data_ptr.is_null() {
                            continue;
                        }
                        let meta = unsafe { *data_ptr };
                        let title = read_cstr(&meta.title);
                        let atc_type = read_cstr(&meta.atc_type);
                        let airline = read_cstr(&meta.atc_airline);
                        let registration = read_cstr(&meta.atc_id);
                        // Derivamos `model` del TITLE — la simvar
                        // `ATC MODEL` no existe en el SDK. Heurística
                        // simple: nos quedamos con la primera fragmento
                        // antes del primer paréntesis o palabra de
                        // separación de livery (no perfecto, pero útil
                        // para agrupar). El usuario puede editar a mano.
                        let model = title.as_ref().map(|t| {
                            t.split(['(', '|', '-']).next().unwrap_or(t).trim().to_string()
                        });
                        // (v3.4.15) Throttle del log — antes loggeábamos
                        // cada segundo (con FLAG_DEFAULT). Ahora sólo
                        // loggeamos cuando el TITLE (o atc_type/reg)
                        // cambia respecto del cache previo. Eso evita
                        // 60+ líneas/min idénticas en el log sin perder
                        // la traza del primer fetch ni de un cambio de
                        // avión mid-session.
                        let changed = match &aircraft_meta_cache {
                            None => true,
                            Some(prev) => {
                                read_cstr(&prev.title) != title
                                    || read_cstr(&prev.atc_type) != atc_type
                                    || read_cstr(&prev.atc_airline) != airline
                                    || read_cstr(&prev.atc_id) != registration
                            }
                        };
                        if changed {
                            tracing::info!(
                                target: "simconnect",
                                "AircraftMeta cambio — title={:?} atc_type={:?} model={:?} airline={:?} reg={:?}",
                                title, atc_type, model, airline, registration
                            );
                        }
                        aircraft_meta_cache = Some(meta);

                        // Si hay vuelo abierto, persistirlo. La
                        // tarea async corre en su propio thread
                        // porque este bloque es síncrono (spawn_blocking).
                        if let Ok(g) = current_flight_id.lock() {
                            if let Some(id) = *g {
                                let pool_c = pool.clone();
                                std::thread::spawn(move || {
                                    let rt = tokio::runtime::Builder::new_current_thread()
                                        .enable_all()
                                        .build()
                                        .ok();
                                    if let Some(rt) = rt {
                                        let _ = rt.block_on(
                                            crate::flight_log::update_aircraft_meta(
                                                &pool_c, id,
                                                title.as_deref(),
                                                atc_type.as_deref(),
                                                model.as_deref(),
                                                airline.as_deref(),
                                                registration.as_deref(),
                                            ),
                                        );
                                    }
                                });
                            }
                        }
                        continue;
                    }

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
                    let prev_phase = phase;
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
                        &mut ticks_since_persist,
                        &mut ticks_since_emit,
                        &mut captured_landing_fpm,
                        &mut engines_seen_running,
                        &mut idle_ticks_in_landed,
                        &mut initial_fuel_lb,
                        &mut paused_seconds_total,
                        &mut passed_taxi_threshold,
                    );
                    // (v0.1.26) Si la transición acabó de marcar OUT
                    // o entró en Landed (touchdown), disparamos la
                    // request de Facility Data para obtener el gate
                    // real. La response llega de forma asíncrona y
                    // se procesa en los SIMCONNECT_RECV_FACILITY_DATA
                    // events más abajo.
                    if facility_def_ok {
                        if prev_phase == FlightPhase::OnGround
                            && phase == FlightPhase::BlockOut
                        {
                            request_gate_facility(
                                lib,
                                handle,
                                pool,
                                app,
                                state,
                                "departure",
                                data.latitude_deg,
                                data.longitude_deg,
                                &current_flight_id,
                                &mut next_gate_seq_dep,
                                &mut pending_gates,
                            );
                        } else if prev_phase == FlightPhase::Airborne
                            && phase == FlightPhase::Landed
                        {
                            request_gate_facility(
                                lib,
                                handle,
                                pool,
                                app,
                                state,
                                "arrival",
                                data.latitude_deg,
                                data.longitude_deg,
                                &current_flight_id,
                                &mut next_gate_seq_arr,
                                &mut pending_gates,
                            );
                        }

                        // (v3.0.0) Preflight gate — detectamos el gate
                        // antes del OUT para que la UI pueda mostrarlo
                        // mientras el usuario está cargando combustible,
                        // pax, ATC, etc. Trigger:
                        //   · phase = OnGround
                        //   · gs < 1 kt (avión parado)
                        //   · posición real (no (0,0) de carga)
                        //   · aún no hemos pedido para este aeropuerto
                        //     (o han pasado >15min desde la última)
                        let lat = data.latitude_deg;
                        let lon = data.longitude_deg;
                        let gs = data.ground_velocity_kt;
                        let pos_real = lat.abs() > 0.01 || lon.abs() > 0.01;
                        if matches!(phase, FlightPhase::OnGround)
                            && gs < 1.0
                            && pos_real
                            && last_airport_check_at.elapsed()
                                >= Duration::from_secs(10)
                        {
                            last_airport_check_at = std::time::Instant::now();
                            // Resolver nearest airport para saber si
                            // cambió desde la última request.
                            let nearest_icao = std::thread::scope(|s| {
                                let h = s.spawn(|| {
                                    let rt = tokio::runtime::Builder::new_current_thread()
                                        .enable_all()
                                        .build()
                                        .ok()?;
                                    rt.block_on(
                                        crate::flight_log::nearest_airport_with_coords(
                                            pool, lat, lon,
                                        ),
                                    )
                                    .ok()
                                    .flatten()
                                    .map(|n| n.icao)
                                });
                                h.join().ok().flatten()
                            });
                            if let Some(icao) = nearest_icao.as_ref() {
                                let last = last_known_airport_icao.as_ref();
                                if last != Some(icao) {
                                    last_known_airport_icao =
                                        Some(icao.clone());
                                    // Update SharedState.current_airport_icao
                                    // — frontend lo usa para SimBrief match.
                                    if let Ok(mut g) = state.try_lock() {
                                        g.status.current_airport_icao =
                                            Some(icao.clone());
                                    }
                                }
                                // (v3.4.11) Re-dispara la detección si:
                                //   · cambió el aeropuerto, O
                                //   · pasaron >15min (fallback periódico
                                //     por si la primera detección
                                //     devolvió un gate equivocado), O
                                //   · **el avión se movió >50m desde la
                                //     última request** — esto resuelve
                                //     el caso "spawneo en gate inicial
                                //     y luego taxi a otro gate del mismo
                                //     aeropuerto". Antes el watcher se
                                //     quedaba en el gate viejo.
                                let stale = preflight_gate_at
                                    .map(|t| t.elapsed() > Duration::from_secs(15 * 60))
                                    .unwrap_or(true);
                                let new_airport = preflight_gate_airport.as_ref()
                                    != Some(icao);
                                let moved_significantly = last_gate_request_pos
                                    .map(|(plat, plon)| {
                                        let m_per_deg_lat = 111_320.0_f64;
                                        let m_per_deg_lon = 111_320.0_f64
                                            * (lat.to_radians().cos()).abs();
                                        let dlat_m = (lat - plat) * m_per_deg_lat;
                                        let dlon_m = (lon - plon) * m_per_deg_lon;
                                        (dlat_m.powi(2) + dlon_m.powi(2)).sqrt() > 50.0
                                    })
                                    .unwrap_or(false);
                                if new_airport || stale || moved_significantly {
                                    preflight_gate_airport = Some(icao.clone());
                                    preflight_gate_at = Some(std::time::Instant::now());
                                    last_gate_request_pos = Some((lat, lon));
                                    if moved_significantly {
                                        tracing::info!(
                                            target: "simconnect",
                                            "preflight gate re-detect: avión se movió >50m → re-trigger en {}",
                                            icao
                                        );
                                    }
                                    request_gate_facility(
                                        lib,
                                        handle,
                                        pool,
                                        app,
                                        state,
                                        "preflight",
                                        lat,
                                        lon,
                                        &current_flight_id,
                                        &mut next_gate_seq_pre,
                                        &mut pending_gates,
                                    );
                                }
                            }
                        }

                        // (v3.4.11) **Arrival gate post-taxi**.
                        // El trigger del touchdown (Airborne→Landed,
                        // arriba en la match) se dispara en la pista
                        // — lejos del gate, sin parking dentro de
                        // 75m, → None. Ahora también disparamos
                        // mientras `phase == Landed` cada vez que el
                        // avión está parado (`gs<1`) y se movió >50m
                        // desde la última detección. Eso captura el
                        // momento "ya estoy en el gate destino".
                        if matches!(phase, FlightPhase::Landed)
                            && gs < 1.0
                            && pos_real
                            && last_arrival_check_at.elapsed()
                                >= Duration::from_secs(10)
                        {
                            last_arrival_check_at = std::time::Instant::now();
                            let moved_significantly = last_gate_request_pos
                                .map(|(plat, plon)| {
                                    let m_per_deg_lat = 111_320.0_f64;
                                    let m_per_deg_lon = 111_320.0_f64
                                        * (lat.to_radians().cos()).abs();
                                    let dlat_m = (lat - plat) * m_per_deg_lat;
                                    let dlon_m = (lon - plon) * m_per_deg_lon;
                                    (dlat_m.powi(2) + dlon_m.powi(2)).sqrt() > 50.0
                                })
                                .unwrap_or(true); // first arrival check after touchdown
                            if moved_significantly {
                                last_gate_request_pos = Some((lat, lon));
                                tracing::info!(
                                    target: "simconnect",
                                    "arrival gate re-detect post-taxi (gs<1, pos cambió) → trigger"
                                );
                                request_gate_facility(
                                    lib,
                                    handle,
                                    pool,
                                    app,
                                    state,
                                    "arrival",
                                    lat,
                                    lon,
                                    &current_flight_id,
                                    &mut next_gate_seq_arr,
                                    &mut pending_gates,
                                );
                            }
                        }
                    }
                }
                SIMCONNECT_RECV_ID_FACILITY_DATA => {
                    // (v0.1.26) Un nodo de la jerarquía AIRPORT.
                    // Sólo nos interesan los TAXI_PARKING.
                    let evt = unsafe {
                        &*(p_data as *const sc::SIMCONNECT_RECV_FACILITY_DATA)
                    };
                    let req_id = evt.UserRequestId;
                    let ntype = evt.Type;
                    let is_list = evt.IsListItem;
                    let item_idx = evt.ItemIndex;
                    let list_size = evt.ListSize;
                    // (v2.0.0) Loggeamos cada evento para diagnosticar
                    // por qué a veces no recolectamos parkings. El
                    // usuario reportó "Stand · X° Ym de ICAO" (fallback)
                    // incluso en aeropuertos grandes — si vemos tipos
                    // distintos de 15 (TAXI_PARKING) tendremos pistas.
                    tracing::debug!(
                        target: "simconnect",
                        "FACILITY_DATA req={} type={} is_list={} item={} list_size={}",
                        req_id, ntype, is_list, item_idx, list_size
                    );
                    // (v3.4.5) Parsear separadamente AIRPORT (centro del
                    // aeropuerto) y TAXI_PARKING (bias offsets en metros).
                    let base = unsafe {
                        (p_data as *const u8).add(
                            std::mem::size_of::<sc::SIMCONNECT_RECV_FACILITY_DATA>() - 4,
                        )
                    };
                    if ntype == sc::SIMCONNECT_FACILITY_DATA_AIRPORT {
                        // Layout AIRPORT en nuestra def:
                        //   [f64 LATITUDE] [f64 LONGITUDE]
                        let lat = unsafe {
                            std::ptr::read_unaligned(base as *const f64)
                        };
                        let lon = unsafe {
                            std::ptr::read_unaligned(base.add(8) as *const f64)
                        };
                        if let Some(p) = pending_gates.get_mut(&req_id) {
                            p.airport_lat = Some(lat);
                            p.airport_lon = Some(lon);
                            tracing::debug!(
                                target: "simconnect",
                                "RECV_FACILITY_DATA req={} AIRPORT centro=({:.4},{:.4})",
                                req_id, lat, lon
                            );
                        }
                    } else if ntype == sc::SIMCONNECT_FACILITY_DATA_TAXI_PARKING {
                        // Layout TAXI_PARKING en nuestra def (v3.4.5):
                        //   [u32 TYPE] [u32 NAME] [u32 NUMBER] [u32 SUFFIX]
                        //   [f64 BIAS_X] [f64 BIAS_Y]
                        // unaligned reads — los bytes son packed(4).
                        let type_id = unsafe {
                            std::ptr::read_unaligned(base as *const u32)
                        };
                        let name_id = unsafe {
                            std::ptr::read_unaligned(base.add(4) as *const u32)
                        };
                        let number = unsafe {
                            std::ptr::read_unaligned(base.add(8) as *const u32)
                        };
                        let suffix = unsafe {
                            std::ptr::read_unaligned(base.add(12) as *const u32)
                        };
                        let bias_x = unsafe {
                            std::ptr::read_unaligned(base.add(16) as *const f64)
                        };
                        let bias_y = unsafe {
                            std::ptr::read_unaligned(base.add(24) as *const f64)
                        };
                        if let Some(p) = pending_gates.get_mut(&req_id) {
                            p.parkings.push(ParkingSpot {
                                type_id,
                                name_id,
                                number,
                                suffix,
                                bias_x,
                                bias_y,
                            });
                        }
                    }
                }
                SIMCONNECT_RECV_ID_FACILITY_DATA_END => {
                    let evt = unsafe {
                        &*(p_data as *const sc::SIMCONNECT_RECV_FACILITY_DATA_END)
                    };
                    let req_id = evt.RequestId;
                    // (v3.4.4) Log de diagnóstico — si llegamos acá pero
                    // pending_gates no tiene la entry, sabemos que el
                    // RequestId no matchea. Antes este branch era
                    // silencioso y nos dejaba ciegos cuando algo fallaba.
                    if let Some(pending) = pending_gates.remove(&req_id) {
                        tracing::info!(
                            target: "simconnect",
                            "RECV_FACILITY_DATA_END req={} → procesando {} parkings",
                            req_id,
                            pending.parkings.len()
                        );
                        process_pending_gate(pool, app, state, pending);
                    } else {
                        tracing::warn!(
                            target: "simconnect",
                            "RECV_FACILITY_DATA_END req={} pero no había entry en pending_gates (¿response orphan?)",
                            req_id
                        );
                    }
                }
                SIMCONNECT_RECV_ID_CLIENT_DATA => {
                    // (v3.4.7) GSX Client Data Area dispatch — modo
                    // discovery con MÚLTIPLES candidates. Cada uno
                    // tiene un request_id distinto (100..103), así
                    // sabemos cuál CDA fue el que emitió.
                    let evt = unsafe {
                        &*(p_data as *const sc::SIMCONNECT_RECV_CLIENT_DATA)
                    };
                    let request_id = evt.dwRequestID;
                    let cda_name = match request_id {
                        100 => Some("FSDT_GSX_AIRCRAFT_DATA"),
                        101 => Some("FSDT_GSX_MENU"),
                        102 => Some("FSDT_GSX_PIPE_TO_PLANE"),
                        103 => Some("FSDT_GSX_BYPASS_PIN"),
                        _ => None,
                    };
                    if let Some(name) = cda_name {
                        let base = unsafe {
                            (p_data as *const u8).add(
                                std::mem::size_of::<sc::SIMCONNECT_RECV_CLIENT_DATA>() - 4,
                            )
                        };
                        // Dump 128 bytes — hex en bloques de 16
                        let mut all_hex = String::with_capacity(300);
                        let mut all_ascii = String::with_capacity(140);
                        let mut nonzero = 0_usize;
                        for i in 0..128_usize {
                            let b = unsafe { *base.add(i) };
                            if b != 0 { nonzero += 1; }
                            all_hex.push_str(&format!("{:02x}", b));
                            if i % 16 == 15 { all_hex.push(' '); }
                            all_ascii.push(if (0x20..0x7F).contains(&b) {
                                b as char
                            } else {
                                '.'
                            });
                            if i % 16 == 15 { all_ascii.push(' '); }
                        }
                        // Sólo logueamos cuando hay bytes no-cero o
                        // cada 30 emisiones de zeros — evita inundar
                        // el log con "todos ceros" eternos.
                        let should_log = nonzero > 0 || (ticks_since_emit % 30 == 0);
                        if should_log {
                            tracing::info!(
                                target: "simconnect",
                                "GSX_CDA[{}] req={} nonzero={}B hex: {}",
                                name, request_id, nonzero, all_hex.trim()
                            );
                            tracing::info!(
                                target: "simconnect",
                                "GSX_CDA[{}] req={} ascii: \"{}\"",
                                name, request_id, all_ascii.trim()
                            );
                        }
                    }
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
        ticks_since_persist: &mut u32,
        ticks_since_emit: &mut u32,
        captured_landing_fpm: &mut Option<i64>,
        engines_seen_running: &mut bool,
        idle_ticks_in_landed: &mut u32,
        initial_fuel_lb: &mut Option<f64>,
        paused_seconds_total: &mut u64,
        passed_taxi_threshold: &mut bool,
    ) {
        let lat = data.latitude_deg;
        let lon = data.longitude_deg;
        let alt = data.altitude_ft;
        let gs = data.ground_velocity_kt;
        let on_ground = data.on_ground >= 0.5;
        let vs = data.vertical_speed_fpm;
        let tas = data.true_airspeed_kt;
        let parking_brake_set = data.parking_brake >= 0.5;
        // Suma de los 4 slots de ENG COMBUSTION. >= 0.5 en CUALQUIER
        // slot significa que ese motor está encendido. "Todos
        // apagados" = ningún slot ≥ 0.5. Para aviones < 4 motores
        // los slots no usados son 0.0 constantes, así que no hace
        // falta saber NUMBER OF ENGINES.
        let engine_slots = [
            data.eng_combustion_1,
            data.eng_combustion_2,
            data.eng_combustion_3,
            data.eng_combustion_4,
        ];
        let any_engine_running = engine_slots.iter().any(|v| *v >= 0.5);
        let all_engines_off = !any_engine_running;
        // (v2.2.0) PUSHBACK STATE enum 0..3 — copia local del packed field.
        let pushback_raw = data.pushback_state;
        let in_pushback = pushback_raw >= 0.5 && pushback_raw < 2.5;

        // Validación básica — ocasionalmente SimConnect emite NaN
        // durante carga del flight.
        if !(lat.is_finite() && lon.is_finite()) {
            return;
        }

        // Marcador one-shot: una vez visto un motor encendido en
        // este vuelo, lo recordamos. Sin esto el chequeo "all engines
        // off" sería trivialmente true al cargar el sim antes de
        // encender — y dispararía un finish espurio. Cuenta durante
        // todas las fases con vuelo activo (BlockOut/Airborne/Landed).
        if matches!(
            *phase,
            FlightPhase::BlockOut | FlightPhase::Airborne | FlightPhase::Landed
        ) && any_engine_running
        {
            *engines_seen_running = true;
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
        if matches!(*phase, FlightPhase::Airborne | FlightPhase::Landed) {
            let gs_int = gs as i64;
            if gs_int > *max_gs_kt {
                *max_gs_kt = gs_int;
            }
            let tas_int = tas as i64;
            if tas_int > *max_tas_kt {
                *max_tas_kt = tas_int;
            }
        }

        // Ventana de VS — capacidad 24 (~6s a 4Hz) para capturar
        // el VS del touchdown sin perder el peak.
        if recent_vs.len() >= 24 {
            recent_vs.pop_front();
        }
        recent_vs.push_back(vs);

        // Track idle ticks en Landed — usado como fallback timeout
        // si los engines nunca se apagan (gliders, eléctricos,
        // usuario que dejó la app abierta tras parquear).
        if matches!(*phase, FlightPhase::Landed) {
            if gs < 1.0 {
                *idle_ticks_in_landed = idle_ticks_in_landed.saturating_add(1);
            } else {
                *idle_ticks_in_landed = 0;
            }
        } else {
            *idle_ticks_in_landed = 0;
        }

        // (v1.1.1) Guard de posición — MSFS reporta el avión en
        // (0, 0) durante el splash/menú/carga antes de que cargue
        // el escenario. Sin esto, el watcher disparaba OUT con
        // origin=(0,0) y registraba un "vuelo fantasma" cada vez
        // que abrías el sim. Posiciones reales de aviación siempre
        // están a >0.01° del meridiano cero × ecuador (no hay
        // aeropuertos en el océano Atlántico ecuatorial).
        let position_seems_real = lat.abs() > 0.01 || lon.abs() > 0.01;

        // State machine completa OOOI (v0.1.23):
        //
        //   OnGround  ──!parking_brake & engines_running──> BlockOut    (OUT: block time start)
        //   BlockOut  ──gs≥30 & !on_ground──>               Airborne    (OFF: takeoff)
        //   Airborne  ──gs<50 & on_ground──>                Landed      (ON: touchdown, captura FPM)
        //   Landed    ──gs≥30 & !on_ground──>               Airborne    (touch-and-go)
        //   Landed    ──all_engines_off──>                  OnGround    (IN: cierre con gate real)
        //   Landed    ──gs<1 por ~90s──>                    OnGround    (fallback timeout)
        //
        // Fallbacks (compat):
        //   OnGround  ──gs≥30 & !on_ground──>               Airborne    (spawn-in-air / no pushback)
        //   BlockOut  ──parking_brake & engines_off──>      OnGround    (cancelar salida)
        let landed_should_finish = matches!(*phase, FlightPhase::Landed)
            && ((all_engines_off && *engines_seen_running)
                || *idle_ticks_in_landed >= LANDED_IDLE_TIMEOUT_TICKS);
        // Cancelar BlockOut: si el usuario suelta el freno por error
        // y luego vuelve a frenar + apagar motores sin haber despegado,
        // volvemos a OnGround sin haber cerrado fila (cierra como
        // vuelo vacío con destination=origin, lo cual es OK).
        let blockout_should_cancel = matches!(*phase, FlightPhase::BlockOut)
            && parking_brake_set
            && all_engines_off;
        let new_phase = match (*phase, on_ground, gs) {
            (FlightPhase::Disconnected, true, _) => FlightPhase::OnGround,
            (FlightPhase::Disconnected, false, _) => FlightPhase::Airborne,
            // (v2.2.0) OUT — Flight Time arranca cuando el avión SE MUEVE
            // con motores encendidos. Esto distingue del towing de GSX
            // (motores apagados + movimiento = NO contar) y del pushback
            // con motores corriendo (cuenta como inicio del bloque).
            // El "freno suelto" se relaja — algunos pilotos arrancan
            // motores con freno suelto y eso ya no es bloqueante.
            // Conditions: en tierra + motores ON + gs > 0.5 kt + posición
            // real + no flight activo.
            (FlightPhase::OnGround, true, gs)
                if any_engine_running
                    && gs > 0.5
                    && position_seems_real
                    && current_flight_id.lock().ok().and_then(|g| *g).is_none() =>
            {
                FlightPhase::BlockOut
            }
            // OFF — takeoff desde BlockOut (camino normal).
            (FlightPhase::BlockOut, false, gs) if gs >= 30.0 => FlightPhase::Airborne,
            // OFF — takeoff sin BlockOut previo (spawn-in-air,
            // teletransporte, app abierta mid-flight). Fallback de
            // compat con v0.1.22: dispara start_flight. Mismo guard
            // de posición que el OUT principal.
            (FlightPhase::OnGround, false, gs)
                if gs >= 30.0 && position_seems_real =>
            {
                FlightPhase::Airborne
            }
            // ON — touchdown.
            (FlightPhase::Airborne, true, gs) if gs < 50.0 => FlightPhase::Landed,
            // Touch-and-go.
            (FlightPhase::Landed, false, gs) if gs >= 30.0 => FlightPhase::Airborne,
            // Cancelar BlockOut sin despegar.
            _ if blockout_should_cancel => FlightPhase::OnGround,
            // IN — engine shutdown completo o timeout idle.
            _ if landed_should_finish => FlightPhase::OnGround,
            _ => *phase,
        };

        if new_phase != *phase {
            tracing::info!(
                target: "simconnect",
                "phase {:?} → {:?} (lat={:.4} lon={:.4} alt={:.0}ft gs={:.0}kt onGround={} engines_off={})",
                *phase, new_phase, lat, lon, alt, gs, on_ground, all_engines_off
            );
            match (*phase, new_phase) {
                (FlightPhase::OnGround, FlightPhase::BlockOut) => {
                    // OUT — pushback / freno suelto. Block time
                    // arranca AQUÍ. (v2.0.0) Llamamos a `start_flight`
                    // de forma SÍNCRONA via block_on antes de continuar
                    // — antes lo hacíamos en `tokio::spawn` async, lo
                    // que dejaba `current_flight_id = None` cuando el
                    // main loop disparaba `request_gate_facility`
                    // inmediatamente después. Resultado: el gate de
                    // salida nunca se actualizaba y quedaba el fallback
                    // "Stand · X° Ym de ICAO". Bloquear ~10 ms aquí es
                    // aceptable; el watcher samplea a 4 Hz, así que un
                    // tick perdido es invisible.
                    let fuel_lb = data.fuel_total_weight_lb;
                    *initial_fuel_lb = Some(fuel_lb);
                    tracing::info!(
                        target: "simconnect",
                        "OUT — fuel inicial capturado: {:.0} lb",
                        fuel_lb
                    );
                    let pool_c = pool.clone();
                    let lat_c = lat;
                    let lon_c = lon;
                    let id_slot = current_flight_id.clone();
                    let app_c = app.clone();
                    let start_result = std::thread::scope(|s| {
                        let h = s.spawn(|| {
                            let rt = tokio::runtime::Builder::new_current_thread()
                                .enable_all()
                                .build()
                                .ok()?;
                            rt.block_on(crate::flight_log::start_flight(
                                &pool_c, lat_c, lon_c, None, None,
                            ))
                            .ok()
                        });
                        h.join().ok().flatten()
                    });
                    if let Some(id) = start_result {
                        tracing::info!(
                            target: "simconnect",
                            "OUT — flight_log id={} pushback en ({:.4}, {:.4})",
                            id, lat_c, lon_c
                        );
                        if let Ok(mut g) = id_slot.lock() {
                            *g = Some(id);
                        }
                        let _ = app_c.emit("flightlog://changed", ());
                        // (v2.0.0) Pre-populate SimBrief en BACKGROUND —
                        // no bloquea el watcher. Ventana 7 días (era 48h
                        // que el usuario reportó como demasiado estricta;
                        // los OFPs viejos también deberían contar).
                        let pool_b = pool_c.clone();
                        tokio::spawn(async move {
                            populate_simbrief_async(&pool_b, id, lat_c, lon_c).await;
                        });
                    } else {
                        tracing::error!(
                            target: "simconnect",
                            "start_flight (OUT) falló — block_on devolvió None"
                        );
                    }
                }
                (FlightPhase::BlockOut, FlightPhase::Airborne) => {
                    // OFF — el vuelo ya estaba abierto desde el OUT;
                    // sólo logueamos la transición. start_flight ya
                    // fue llamado.
                    tracing::info!(
                        target: "simconnect",
                        "OFF — takeoff (vuelo ya abierto desde OUT)"
                    );
                }
                (FlightPhase::OnGround, FlightPhase::Airborne)
                | (FlightPhase::Disconnected, FlightPhase::Airborne) => {
                    // Fallback: despegue sin BlockOut previo (spawn
                    // in air, teletransporte, app abierta mid-flight).
                    // Mantenemos el comportamiento de v0.1.22:
                    // start_flight con origen = posición actual.
                    let pool_c = pool.clone();
                    let lat_c = lat;
                    let lon_c = lon;
                    let id_slot = current_flight_id.clone();
                    let app_c = app.clone();
                    tokio::spawn(async move {
                        match crate::flight_log::start_flight(
                            &pool_c, lat_c, lon_c, None, None,
                        )
                        .await
                        {
                            Ok(id) => {
                                tracing::info!(
                                    target: "simconnect",
                                    "OFF (fallback sin OUT) — flight_log id={} en ({:.4}, {:.4})",
                                    id, lat_c, lon_c
                                );
                                if let Ok(mut g) = id_slot.lock() {
                                    *g = Some(id);
                                }
                                let _ = app_c.emit("flightlog://changed", ());
                            }
                            Err(e) => {
                                tracing::error!(
                                    target: "simconnect",
                                    "start_flight (fallback OFF) falló: {e:#}"
                                );
                            }
                        }
                    });
                }
                (FlightPhase::BlockOut, FlightPhase::OnGround) => {
                    // Cancelación de salida: el usuario re-frenó y
                    // apagó motores sin despegar. Cerramos la fila
                    // como un vuelo vacío (destination = origin) para
                    // no dejarla huérfana.
                    let id_opt = current_flight_id
                        .lock()
                        .ok()
                        .and_then(|mut g| g.take());
                    if let Some(id) = id_opt {
                        let pool_c = pool.clone();
                        let lat_c = lat;
                        let lon_c = lon;
                        let app_c = app.clone();
                        tokio::spawn(async move {
                            let metrics =
                                crate::flight_log::FlightFinishMetrics::default();
                            if let Err(e) = crate::flight_log::finish_flight(
                                &pool_c, id, lat_c, lon_c, metrics,
                            )
                            .await
                            {
                                tracing::error!(
                                    target: "simconnect",
                                    "finish_flight (cancel OUT) falló: {e:#}"
                                );
                            } else {
                                tracing::info!(
                                    target: "simconnect",
                                    "BLOCK-OUT CANCELADO — flight_log id={} cerrado sin despegar",
                                    id
                                );
                                let _ = app_c.emit("flightlog://changed", ());
                            }
                        });
                        *engines_seen_running = false;
                    }
                }
                (FlightPhase::Airborne, FlightPhase::Landed) => {
                    // Touchdown — captura landing_fpm pero NO cierra
                    // la fila. El vuelo sigue abierto durante el
                    // taxi-in; el cierre real es al engine shutdown.
                    let landing_vs = recent_vs
                        .iter()
                        .copied()
                        .fold(f64::INFINITY, f64::min);
                    let fpm = if landing_vs.is_finite() {
                        Some(landing_vs as i64)
                    } else {
                        None
                    };
                    *captured_landing_fpm = fpm;
                    let id_opt = current_flight_id.lock().ok().and_then(|g| *g);
                    if let Some(id) = id_opt {
                        if let Some(fpm_val) = fpm {
                            // Persistimos inmediatamente. Si la app
                            // muere entre touchdown y shutdown,
                            // conservamos al menos el FPM (perdemos
                            // sólo el gate exacto y los ~minutos de
                            // taxi).
                            let pool_c = pool.clone();
                            tokio::spawn(async move {
                                let _ = crate::flight_log::touch_landing(
                                    &pool_c, id, fpm_val,
                                )
                                .await;
                            });
                        }
                    }
                    tracing::info!(
                        target: "simconnect",
                        "TOUCHDOWN — landing_fpm={:?} (taxi pendiente, esperando engine shutdown)",
                        fpm
                    );
                    *idle_ticks_in_landed = 0;
                }
                (FlightPhase::Landed, FlightPhase::Airborne) => {
                    // Touch-and-go o despegue forzado tras Landed
                    // (raro pero posible). Reseteamos idle, dejamos
                    // captured_landing_fpm como está — si el usuario
                    // hace varios touch-and-go, conservamos sólo el
                    // primer FPM. Aceptable: el de "real" suele ser
                    // ese; los siguientes son re-aproximaciones.
                    tracing::info!(
                        target: "simconnect",
                        "TOUCH-AND-GO — fpm conservado, vuelo sigue abierto"
                    );
                    *idle_ticks_in_landed = 0;
                }
                (FlightPhase::Landed, FlightPhase::OnGround) => {
                    // OOOI on-block — cierre real del vuelo. La lat/lon
                    // de aquí es la posición actual = gate / parking
                    // donde el usuario apagó los motores (o donde lleva
                    // 90s parado).
                    let id_opt = current_flight_id
                        .lock()
                        .ok()
                        .and_then(|mut g| g.take());
                    if let Some(id) = id_opt {
                        // Fuel used (kg) = (initial_lb - final_lb) / 2.2046.
                        // Si SimConnect se conectó después del OUT,
                        // initial_fuel_lb es None y no reportamos
                        // consumo (mejor sin dato que con uno engañoso).
                        let final_fuel_lb = data.fuel_total_weight_lb;
                        let fuel_used = match *initial_fuel_lb {
                            Some(init) if init > final_fuel_lb => {
                                Some(((init - final_fuel_lb) / 2.2046_f64) as i64)
                            }
                            _ => None,
                        };
                        // (v3.1.0) Fallback arrival_gate desde SharedState.
                        // Si la facility data request del IN no ha resuelto
                        // todavía (o falló por scenery sin TAXI_PARKING),
                        // usamos el current_gate como respaldo. Si Facility
                        // Data ya populó arrival_gate en la fila, COALESCE
                        // en finish_flight lo conserva.
                        let fallback_gate = state
                            .try_lock()
                            .ok()
                            .and_then(|g| g.status.current_gate.clone());
                        let metrics = crate::flight_log::FlightFinishMetrics {
                            max_altitude_ft: Some(*max_alt_ft),
                            landing_fpm: *captured_landing_fpm,
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
                            fuel_used_kg: fuel_used,
                            paused_seconds: (*paused_seconds_total) as i64,
                            fallback_arrival_gate: fallback_gate,
                        };
                        let reason = if all_engines_off {
                            "ENGINE SHUTDOWN"
                        } else {
                            "IDLE TIMEOUT"
                        };
                        let pool_c = pool.clone();
                        let lat_c = lat;
                        let lon_c = lon;
                        let app_c = app.clone();
                        let metrics_c = metrics.clone();
                        let reason_c = reason.to_string();
                        tokio::spawn(async move {
                            match crate::flight_log::finish_flight(
                                &pool_c, id, lat_c, lon_c, metrics_c.clone(),
                            )
                            .await
                            {
                                Ok(()) => {
                                    tracing::info!(
                                        target: "simconnect",
                                        "VUELO CERRADO ({}) — flight_log id={} en ({:.4}, {:.4}) max_alt={:?}ft landing_fpm={:?} max_gs={:?}kt max_tas={:?}kt",
                                        reason_c,
                                        id,
                                        lat_c,
                                        lon_c,
                                        metrics_c.max_altitude_ft,
                                        metrics_c.landing_fpm,
                                        metrics_c.max_ground_speed_kt,
                                        metrics_c.max_true_airspeed_kt
                                    );
                                    // (v2.2.0) Re-intentamos pre-populate
                                    // de SimBrief al IN — si el usuario
                                    // planificó el OFP DESPUÉS del OUT,
                                    // ahora puede que esté disponible y
                                    // los pax/cargo/fuel se completan.
                                    // update_entry ignora None, así que
                                    // sólo escribe los nuevos.
                                    populate_simbrief_async(&pool_c, id, lat_c, lon_c).await;
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
                        *captured_landing_fpm = None;
                        *engines_seen_running = false;
                        *idle_ticks_in_landed = 0;
                        *initial_fuel_lb = None;
                        *paused_seconds_total = 0;
                    }
                }
                _ => {}
            }
            *phase = new_phase;
        }

        // **Persist live position cada 10 ticks** (≈10s) si hay un
        // vuelo abierto. Doble propósito:
        //   1. ACARS-like: si la app se cierra a mitad de vuelo, al
        //      reabrirse podemos restaurar el state desde la última
        //      posición conocida (last_position_*).
        //   2. (v0.1.23) Track polyline: además inserta un punto en
        //      `flight_log_track` cada muestreo. Una consulta
        //      ORDER BY ts reconstruye la traza real del vuelo —
        //      lo que pinta el FlightBook detail map.
        *ticks_since_persist = ticks_since_persist.saturating_add(1);
        if *ticks_since_persist >= PERSIST_INTERVAL_TICKS {
            *ticks_since_persist = 0;
            let id_opt = current_flight_id.lock().ok().and_then(|g| *g);
            if let Some(id) = id_opt {
                let pool_c = pool.clone();
                let lat_c = lat;
                let lon_c = lon;
                let alt_c = alt as i64;
                let gs_c = gs as i64;
                tokio::spawn(async move {
                    if let Err(e) = crate::flight_log::touch_live_position(
                        &pool_c, id, lat_c, lon_c, alt_c, gs_c,
                    )
                    .await
                    {
                        tracing::warn!(
                            target: "simconnect",
                            "touch_live_position falló: {e:#}"
                        );
                    }
                    // Append a la traza — independiente del update
                    // de last_position porque queremos conservar TODOS
                    // los puntos.
                    if let Err(e) = crate::flight_log::insert_track_point(
                        &pool_c, id, lat_c, lon_c, alt_c, gs_c,
                    )
                    .await
                    {
                        tracing::warn!(
                            target: "simconnect",
                            "insert_track_point falló: {e:#}"
                        );
                    }
                });
            }
        }

        // (v1.1.4) Tracking sticky para distinguir pushback de taxi.
        // Pushback real raras veces pasa de 2-3 kt; en cuanto el
        // avión rueda por su propio motor cruza 4-5 kt sostenidos.
        // Threshold a 3 kt → cambia el label rápido al taxi real.
        // Sticky: una vez cruzado, el label es "taxi_out" aunque
        // baje a 0 (esperando ATC, backtrack, etc.).
        // (v3.4.13) Threshold subido de 3 → 8 kt. Antes el truck de
        // pushback excedía fácil los 3 kt y volvía sticky el
        // `passed_taxi_threshold`, lo que hacía que el phase_label
        // saltase a "taxi_out" durante el pushback. 8 kt es el límite
        // típico real del taxi (los trucks de pushback raramente
        // pasan de 5-6 kt) — eso mantiene "pushback" mientras estás
        // siendo empujado y sólo cambia a "taxi_out" cuando vos
        // empezás a rodar con tus propios motores.
        if matches!(*phase, FlightPhase::BlockOut) && gs > 8.0 {
            *passed_taxi_threshold = true;
        }
        // Reset al cerrar el vuelo (Landed → OnGround se gestiona en
        // la transición ya — agregamos aquí el reset cuando salimos
        // de phases activas).
        if matches!(*phase, FlightPhase::OnGround | FlightPhase::Disconnected)
            && current_flight_id.lock().ok().and_then(|g| *g).is_none()
        {
            *passed_taxi_threshold = false;
        }

        // (v0.1.25) Phase label granular para UI profesional.
        // Derivado del estado actual + simvars. El frontend lo mapea
        // a un string traducido para el badge "En vuelo ahora".
        let phase_label = derive_phase_label(
            *phase,
            on_ground,
            gs,
            alt,
            vs,
            parking_brake_set,
            any_engine_running,
            *passed_taxi_threshold,
            in_pushback,
        );

        // (v1.1.4) Update shared state SIEMPRE (los lectores leen
        // del state directamente para FFI con frontend), pero
        // sólo emitimos al frontend cada `EMIT_INTERVAL_TICKS`
        // (~1s a 4Hz) para no inundar la UI con eventos a 4Hz.
        *ticks_since_emit = ticks_since_emit.saturating_add(1);
        if let Ok(mut guard) = state.try_lock() {
            let st = &mut guard.status;
            st.simconnect_connected = true;
            st.sim_running = true;
            st.current_lat = Some(lat);
            st.current_lon = Some(lon);
            // (v1.1.4) Usamos INDICATED ALTITUDE para la lectura
            // visible — el usuario reportó "FL400 marcaba 41,619 ft"
            // porque antes usábamos PLANE ALTITUDE (MSL geométrica).
            // Copia local del field packed para evitar el warning de
            // unaligned reference.
            let indicated_alt = data.indicated_alt_ft;
            st.current_alt_ft = Some(indicated_alt as i64);
            st.current_ground_speed_kt = Some(gs as i64);
            // (v1.1.4) Heading 0..360 — copia local del field packed.
            let heading = data.heading_deg;
            // Normaliza al rango [0, 360) — SimConnect ocasionalmente
            // devuelve valores fuera de rango durante init.
            let heading_norm = ((heading % 360.0) + 360.0) % 360.0;
            st.current_heading_deg = Some(heading_norm as i64);
            st.on_ground = Some(on_ground);
            st.phase_label = Some(phase_label);
            st.last_checked_at = chrono::Utc::now()
                .format("%Y-%m-%dT%H:%M:%SZ")
                .to_string();
            let should_emit = *ticks_since_emit >= EMIT_INTERVAL_TICKS;
            if should_emit {
                let snapshot = st.clone();
                drop(guard);
                // (v3.4.10) Log diagnóstico throttled — cada ~10s
                // mostramos un summary del payload para confirmar
                // que el emit está corriendo. Sin esto, si el badge
                // no aparece en la UI no sabemos si es bug de
                // backend (no emite) o frontend (no recibe).
                static LAST_LOG_TS: std::sync::OnceLock<std::sync::Mutex<std::time::Instant>> =
                    std::sync::OnceLock::new();
                let last_log = LAST_LOG_TS.get_or_init(|| {
                    std::sync::Mutex::new(
                        std::time::Instant::now() - std::time::Duration::from_secs(60),
                    )
                });
                if let Ok(mut t) = last_log.lock() {
                    if t.elapsed() >= std::time::Duration::from_secs(10) {
                        tracing::info!(
                            target: "simconnect",
                            "emit flight://current (tick) sim_running={} sc_connected={} gate={:?} phase={:?}",
                            snapshot.sim_running,
                            snapshot.simconnect_connected,
                            snapshot.current_gate,
                            snapshot.phase_label
                        );
                        *t = std::time::Instant::now();
                    }
                }
                let _ = app.emit("flight://current", &snapshot);
                *ticks_since_emit = 0;
            }
        }
    }

    /// (v0.1.26) Dispara una `RequestFacilityData` para el aeropuerto
    /// más cercano al player y registra la request como pendiente.
    /// Cuando lleguen los `FACILITY_DATA_END` events procesaremos la
    /// lista y elegiremos el parking más cercano.
    ///
    /// El ICAO se resuelve via `nearest_airport_with_coords` (mismo
    /// helper que el fallback). Si no hay aeropuerto cercano, la
    /// request no se lanza (no nos sirve).
    /// (v3.4.8) **Reescrito** — en lugar de pedir TAXI_PARKING vía
    /// SimConnect Facility Data (5 hotfixes consecutivos demostraron
    /// que la DLL bundled rechaza fields críticos) ahora leemos el
    /// INI de GSX directamente desde el disco. Es file IO local +
    /// math: 100% determinístico, cero fragilidad SDK.
    ///
    /// El `lib`, `handle`, `next_seq` y `pending_gates` se siguen
    /// recibiendo para mantener la signatura del callsite igual (no
    /// quería tocar 3 callsites para limpiar uno) pero `lib` y
    /// `handle` ya no se usan dentro de la función — y `pending_gates`
    /// queda intacto.
    #[allow(unused_variables)]
    fn request_gate_facility(
        lib: &sc::SimConnectLib,
        handle: sc::HANDLE,
        pool: &SqlitePool,
        app: &AppHandle,
        state: &SharedState,
        role: &'static str,
        player_lat: f64,
        player_lon: f64,
        current_flight_id: &std::sync::Arc<std::sync::Mutex<Option<i64>>>,
        next_seq: &mut u32,
        pending_gates: &mut std::collections::HashMap<u32, PendingGate>,
    ) {
        // "preflight" no requiere flight activo — actualiza SharedState
        // sólo. "departure"/"arrival" sí requieren flight_id.
        let flight_id = current_flight_id.lock().ok().and_then(|g| *g);
        if role != "preflight" && flight_id.is_none() {
            return;
        }

        // Resolver ICAO del aeropuerto más cercano.
        let icao_opt = std::thread::scope(|s| {
            let h = s.spawn(|| {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .ok()?;
                rt.block_on(crate::flight_log::nearest_airport_with_coords(
                    pool, player_lat, player_lon,
                ))
                .ok()
                .flatten()
                .map(|n| n.icao)
            });
            h.join().ok().flatten()
        });
        let Some(icao) = icao_opt else {
            tracing::debug!(
                target: "simconnect",
                "request_gate_facility ({}): sin aeropuerto cercano a ({:.4}, {:.4}); skip",
                role, player_lat, player_lon
            );
            return;
        };

        *next_seq = next_seq.wrapping_add(1);

        // **Lookup en disco: GSX INI parser.**
        let Some(parking) = crate::gsx_parking::find_nearest_parking(
            &icao, player_lat, player_lon,
        ) else {
            tracing::debug!(
                target: "simconnect",
                "request_gate_facility ({} {}): sin GSX INI o sin parking <200m del player",
                role, icao
            );
            return;
        };

        let name = parking.name.clone();
        tracing::info!(
            target: "simconnect",
            "gate {} {} → \"{}\" (GSX INI)",
            role, icao, name
        );

        // 1) Actualizar SharedState + emit a UI inmediato (preflight
        //    incluido — para que el chip "Volando ahora" pinte el gate
        //    ANTES de que el usuario haga pushback).
        {
            let state_c = state.clone();
            let app_c = app.clone();
            let name_c = name.clone();
            tokio::spawn(async move {
                let mut guard = state_c.lock().await;
                guard.status.current_gate = Some(name_c.clone());
                let snapshot = guard.status.clone();
                drop(guard);
                let _ = app_c.emit("flight://current", &snapshot);
            });
        }
        // 2) Persistir a DB para departure/arrival (preflight no
        //    necesita — el gate se copia al departure_gate al disparar
        //    el OUT en start_flight, vía un mecanismo separado o via
        //    el siguiente "departure" trigger).
        if let Some(fid) = flight_id {
            let pool_c = pool.clone();
            let role_owned = role;
            let name_for_db = name.clone();
            tokio::spawn(async move {
                let input = match role_owned {
                    "departure" => crate::flight_log::UpdateEntryInput {
                        departure_gate: Some(name_for_db),
                        ..Default::default()
                    },
                    "arrival" => crate::flight_log::UpdateEntryInput {
                        arrival_gate: Some(name_for_db),
                        ..Default::default()
                    },
                    _ => return,
                };
                let _ = crate::flight_log::update_entry(&pool_c, fid, &input).await;
            });
        }
        return;
        // **El resto del cuerpo viejo (SimConnect Facility Data) queda
        // dead code después del `return` — lo dejamos como referencia
        // histórica y para no tener un diff masivo. Compilador lo
        // elimina del binario.**
        #[allow(unreachable_code)]
        {
        let req_fac = match lib.RequestFacilityData {
            Some(f) => f,
            None => return,
        };
        let base = match role {
            "departure" => REQUEST_ID_GATE_DEP_BASE,
            "preflight" => REQUEST_ID_GATE_PRE_BASE,
            _ => REQUEST_ID_GATE_ARR_BASE,
        };
        let req_id = base + (*next_seq % 1000);

        let icao_c = sc::cstr(&icao);
        let region_c = sc::cstr("");
        let hr = unsafe {
            req_fac(
                handle,
                DEFINE_ID_AIRPORT_PARKING,
                req_id,
                icao_c.as_ptr(),
                region_c.as_ptr(),
            )
        };
        if !sc::succeeded(hr) {
            tracing::warn!(
                target: "simconnect",
                "RequestFacilityData('{}') falló (0x{:08x}); gate fallback estará en use",
                icao, hr
            );
            return;
        }
        pending_gates.insert(
            req_id,
            PendingGate {
                flight_id,
                role,
                player_lat,
                player_lon,
                airport_icao: icao.clone(),
                airport_lat: None,
                airport_lon: None,
                parkings: Vec::new(),
            },
        );
        tracing::info!(
            target: "simconnect",
            "RequestFacilityData ({} req={}) → {} desde ({:.4}, {:.4})",
            role, req_id, icao, player_lat, player_lon
        );
        }  // cierra el #[allow(unreachable_code)] block del v3.4.8 refactor
    }

    /// (v0.1.26) Procesa una request de gate completada — elige el
    /// parking más cercano al player saved y actualiza la fila en
    /// flight_log. Llamado en el `FACILITY_DATA_END` event.
    /// (v2.0.0) Pre-populate de pax/cargo/fuel desde el último OFP
    /// de SimBrief para el origen detectado. Antes vivía inline en el
    /// handler OUT pero ahora se llama desde el spawn async post
    /// start_flight síncrono.
    ///
    /// Ventana 7 días — antes era 48h, pero el usuario reportó
    /// vuelos sin valores cuando el OFP estaba en otro horario.
    pub(crate) async fn populate_simbrief_async(
        pool: &SqlitePool,
        flight_id: i64,
        lat: f64,
        lon: f64,
    ) {
        // (v2.2.0) Origen del vuelo — primero intentamos leerlo del
        // flight_log (caso IN: el vuelo ya tiene origin_icao). Si no
        // hay, caemos al nearest airport del lat/lon (caso OUT: la
        // fila aún no tiene origin_icao porque start_flight no lo
        // guardó todavía).
        let origin_from_db: Option<String> =
            sqlx::query_scalar("SELECT origin_icao FROM flight_log WHERE id = ?1")
                .bind(flight_id)
                .fetch_optional(pool)
                .await
                .ok()
                .flatten();
        let icao = match origin_from_db {
            Some(s) if !s.is_empty() => s,
            _ => {
                match crate::flight_log::nearest_airport_with_coords(pool, lat, lon)
                    .await
                {
                    Ok(Some(n)) => n.icao,
                    _ => return,
                }
            }
        };
        match crate::simbrief::find_recent_for_origin(pool, &icao, 7 * 24).await {
            Ok(Some(ofp)) => {
                let input = crate::flight_log::UpdateEntryInput {
                    passengers: ofp.pax_count,
                    cargo_kg: ofp.cargo_kg,
                    fuel_used_kg: ofp.fuel_burn_kg,
                    ..Default::default()
                };
                if let Err(e) =
                    crate::flight_log::update_entry(pool, flight_id, &input).await
                {
                    tracing::warn!(
                        target: "simconnect",
                        "pre-populate desde SimBrief falló: {e:#}"
                    );
                } else {
                    tracing::info!(
                        target: "simconnect",
                        "OFP {} pre-populó pax={:?} cargo={:?}kg fuel_burn={:?}kg en flight_log id={}",
                        ofp.ofp_id,
                        ofp.pax_count,
                        ofp.cargo_kg,
                        ofp.fuel_burn_kg,
                        flight_id
                    );
                }
            }
            Ok(None) => {
                tracing::debug!(
                    target: "simconnect",
                    "sin OFP de SimBrief reciente para {} — pax/cargo/fuel queda NULL",
                    icao
                );
            }
            Err(e) => {
                tracing::warn!(
                    target: "simconnect",
                    "find_recent_for_origin falló: {e:#}"
                );
            }
        }
    }

    fn process_pending_gate(
        pool: &SqlitePool,
        app: &AppHandle,
        state: &SharedState,
        pending: PendingGate,
    ) {
        if pending.parkings.is_empty() {
            tracing::info!(
                target: "simconnect",
                "gate request {} ({}) — {} parkings recolectados, manteniendo fallback",
                pending.airport_icao, pending.role, 0
            );
            return;
        }
        // (v3.4.5) Calcular coordenadas absolutas de cada parking
        // como airport_center + bias_offset (convertir metros a
        // grados). Si no llegó el `AIRPORT` facility data (no
        // debería pasar, pero por seguridad) caemos a `player_lat/lon`
        // como aproximación del centro — los biases siguen siendo
        // offsets relativos consistentes.
        let airport_lat = pending.airport_lat.unwrap_or(pending.player_lat);
        let airport_lon = pending.airport_lon.unwrap_or(pending.player_lon);
        if pending.airport_lat.is_none() {
            tracing::warn!(
                target: "simconnect",
                "gate request {} ({}) — AIRPORT facility data no llegó; usando player como centro",
                pending.airport_icao, pending.role
            );
        }

        // Conversión metros → grados, aproximación local.
        let m_per_deg_lat = 111_320.0_f64;
        let m_per_deg_lon = 111_320.0_f64 * airport_lat.to_radians().cos().abs().max(1e-6);

        let mut best: Option<(f64, &ParkingSpot)> = None;
        for p in &pending.parkings {
            let p_lat = airport_lat + (p.bias_y / m_per_deg_lat);
            let p_lon = airport_lon + (p.bias_x / m_per_deg_lon);
            let dlat = p_lat - pending.player_lat;
            let dlon = p_lon - pending.player_lon;
            let d = dlat * dlat + dlon * dlon;
            if best.map(|(b, _)| d < b).unwrap_or(true) {
                best = Some((d, p));
            }
        }
        let Some((_, picked)) = best else { return };
        let name = picked.display_name();
        tracing::info!(
            target: "simconnect",
            "gate request {} ({}) → \"{}\" (parking #{} entre {} candidates)",
            pending.airport_icao,
            pending.role,
            name,
            picked.number,
            pending.parkings.len()
        );

        // (v3.0.0) Para "preflight" sólo populamos SharedState — no
        // hay flight_log row aún. Para "departure"/"arrival" hacemos
        // update del DB Y refrescamos también el SharedState para
        // que el badge muestre el gate mientras está en tierra.
        let pool_c = pool.clone();
        let app_c = app.clone();
        let state_c = state.clone();
        let flight_id_opt = pending.flight_id;
        let role = pending.role;
        let name_c = name.clone();
        tokio::spawn(async move {
            // 1) Actualiza SharedState (visible inmediatamente en UI).
            {
                let mut guard = state_c.lock().await;
                guard.status.current_gate = Some(name_c.clone());
                let snapshot = guard.status.clone();
                drop(guard);
                let _ = app_c.emit("flight://current", &snapshot);
            }
            // 2) Si hay flight_id, persiste también al DB.
            if let Some(flight_id) = flight_id_opt {
                let input = match role {
                    "departure" => crate::flight_log::UpdateEntryInput {
                        departure_gate: Some(name_c),
                        ..Default::default()
                    },
                    "arrival" => crate::flight_log::UpdateEntryInput {
                        arrival_gate: Some(name_c),
                        ..Default::default()
                    },
                    _ => return,
                };
                if let Err(e) =
                    crate::flight_log::update_entry(&pool_c, flight_id, &input).await
                {
                    tracing::warn!(
                        target: "simconnect",
                        "process_pending_gate: update_entry falló: {e:#}"
                    );
                } else {
                    let _ = app_c.emit("flightlog://changed", ());
                }
            }
        });
    }

    /// Mapea (phase + simvars + flag de progreso) → string corto.
    /// Keys (no localizadas):
    ///   · preflight       — sim abierto, sin vuelo activo, engines off
    ///   · engine_running  — engines on, en gate, freno set, no flight aún
    ///   · pushback        — BlockOut, antes de cruzar 10 kt (sticky)
    ///   · taxi_out        — BlockOut, ya pasamos pushback, gs típico de taxi
    ///   · takeoff         — BlockOut o Airborne con gs ≥ 60 kt (takeoff roll)
    ///   · climbing        — Airborne, vs > 500
    ///   · cruise          — Airborne, |vs| ≤ 500, alt > 10000
    ///   · descent         — Airborne, vs < -500, alt > 8000
    ///   · approach        — Airborne, alt ≤ 3000 (o vs<-500 & alt<8000)
    ///   · landed_rollout  — Landed, gs ≥ 30 (post-touchdown)
    ///   · taxi_in         — Landed, gs < 30 con o sin pasar threshold
    ///   · parking         — Landed, gs < 3, engines aún encendidos
    ///   · deboarding      — engines apagados después de un IN reciente
    ///
    /// (v1.1.2) `passed_taxi_threshold` evita el bug de etiquetar
    /// "pushback" cuando el usuario está parado en pista esperando
    /// ATC o haciendo backtrack — una vez pasamos 10 kt es taxi para
    /// siempre dentro del mismo bloque.
    fn derive_phase_label(
        phase: FlightPhase,
        on_ground: bool,
        gs: f64,
        alt_ft: f64,
        vs_fpm: f64,
        parking_brake_set: bool,
        any_engine_running: bool,
        passed_taxi_threshold: bool,
        in_pushback: bool,
    ) -> String {
        // Takeoff roll: gs alto en suelo (o recién levantado del
        // suelo). Aplica desde BlockOut o Airborne — la condición de
        // velocidad domina al phase enum.
        if matches!(phase, FlightPhase::BlockOut | FlightPhase::Airborne)
            && gs >= 60.0
            && (on_ground || alt_ft < 500.0)
        {
            return "takeoff".to_string();
        }

        // (v2.2.0 → v3.0.0) Detección de pushback más robusta. Tres
        // señales convergen al label "pushback":
        //
        //   1. PUSHBACK STATE simvar = 1 o 2 (ATC interno de MSFS).
        //   2. GSX towing: en tierra + motores APAGADOS + se mueve
        //      (gs > 0.3 kt) + freno suelto + no estamos esperando ATC
        //      con engines off (parking + gs ≈ 0). GSX baja los motores
        //      durante el tow y mueve el avión con su propio script;
        //      esto es lo que dispara el caso del usuario "se salta
        //      pushback y va directo a taxi out".
        //   3. Antes del taxi threshold en BlockOut (camino normal con
        //      motores ON: pilotos que arrancan en gate y empujan con
        //      motores running, o ATC pushback dentro del sim).
        let gsx_towing = matches!(phase, FlightPhase::OnGround)
            && on_ground
            && !any_engine_running
            && gs > 0.3
            && !parking_brake_set;
        if (in_pushback || gsx_towing)
            && matches!(
                phase,
                FlightPhase::OnGround | FlightPhase::BlockOut
            )
        {
            return "pushback".to_string();
        }

        match phase {
            FlightPhase::Disconnected => "preflight".to_string(),
            FlightPhase::OnGround => {
                if !any_engine_running {
                    "preflight".to_string()
                } else if parking_brake_set {
                    "engine_running".to_string()
                } else {
                    // Engines on, brake released, pero no estamos en
                    // BlockOut formal — caso raro (current_flight_id
                    // ya None tras IN reciente). Probable "deboarding"
                    // o trotando pero sin OUT detectado.
                    "deboarding".to_string()
                }
            }
            FlightPhase::BlockOut => {
                // (v2.2.0) Si pasamos taxi threshold → taxi_out.
                // Si no, asumimos pushback (compat con casos donde
                // PUSHBACK STATE no se reporta o GSX no lo expone).
                if passed_taxi_threshold {
                    "taxi_out".to_string()
                } else {
                    "pushback".to_string()
                }
            }
            FlightPhase::Airborne => {
                // (v1.1.4) Orden corregido: PRIMERO chequeamos VS (la
                // intención del piloto), DESPUÉS la altitud. El usuario
                // reportó que justo tras despegar (alt < 3000ft, VS
                // positivo grande) decía "approach" — porque la rama
                // alt <= 3000 ganaba antes de mirar VS. Ahora:
                //   1. VS positivo fuerte → climbing (independiente de alt)
                //   2. VS negativo fuerte → descent o approach según alt
                //   3. VS ≈ 0 → cruise si alto, approach si bajando, climbing si recién despegó
                if vs_fpm > 500.0 {
                    "climbing".to_string()
                } else if vs_fpm < -500.0 {
                    if alt_ft > 8000.0 {
                        "descent".to_string()
                    } else {
                        "approach".to_string()
                    }
                } else if alt_ft <= 3000.0 {
                    // VS ≈ 0 bajo: normalmente final approach steady.
                    "approach".to_string()
                } else if alt_ft > 10_000.0 {
                    "cruise".to_string()
                } else {
                    // 3000-10000 con vs cercano a 0 — transición.
                    "cruise".to_string()
                }
            }
            FlightPhase::Landed => {
                if gs >= 30.0 {
                    "landed_rollout".to_string()
                } else if gs >= 3.0 {
                    "taxi_in".to_string()
                } else {
                    "parking".to_string()
                }
            }
        }
    }

    fn update_connected(state: &SharedState, app: &AppHandle, connected: bool) {
        for _ in 0..50 {
            if let Ok(mut guard) = state.try_lock() {
                guard.status.simconnect_connected = connected;
                guard.status.sim_running = guard.status.sim_running || connected;
                let snapshot = guard.status.clone();
                drop(guard);
                tracing::info!(
                    target: "simconnect",
                    "emit flight://current (update_connected) sim_running={} sc_connected={} gate={:?}",
                    snapshot.sim_running, snapshot.simconnect_connected, snapshot.current_gate
                );
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
