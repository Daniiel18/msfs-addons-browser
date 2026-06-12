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
            // (v4.0.0 — P1 fix flicker) **NUNCA emitir desde el fallback
            // cuando SimConnect está conectado.**
            //
            // Bug reportado por el usuario: badge SimBrief + FlyingNow
            // parpadeaban ~1s cada ~5s durante todo el vuelo. Causa
            // raíz: este thread emite cada 5s un `FlightStatus` con
            // `..Default::default()` → los campos `live` (current_lat,
            // current_lon, phase_label, current_airport_icao…) salen
            // como None. El frontend hacía `set({ status: s })` (replace
            // total) y la badge cae al check
            // `if (!currentAirportIcao && !originIcao) return null` en
            // FlyingNowBadge.tsx hasta el próximo frame del SC thread
            // (4Hz). Resultado: flicker de ~250ms cada 5s.
            //
            // Cuando SimConnect está conectado, **él es la fuente
            // autoritativa** de los campos live. Este fallback se
            // diseñó para cuando NO hay SimConnect (proceso detectado
            // + SimBrief OFP). Mantener el polling es útil para
            // detectar desconexiones del proceso, pero el emit hacia
            // el frontend es ruido contraproducente.
            //
            // Mantenemos `should_emit_now = false` cuando sc_connected,
            // pero seguimos:
            //   · Actualizando `task_state.shared.status` (abajo) para
            //     que `getFlightStatus()` en bootstrap tenga datos
            //     frescos del fallback.
            //   · Computando el status para detectar transiciones.
            let should_emit_now = (changed || polls_since_emit >= FALLBACK_HEARTBEAT_EVERY)
                && !status.simconnect_connected;
            if should_emit_now {
                // (v4.5.1) Solo logueamos cuando CAMBIA el estado; el
                // heartbeat periódico sigue emitiendo a la UI pero en
                // silencio para no inflar el log.
                if changed {
                    tracing::info!(
                        target: "simconnect",
                        "emit flight://current (fallback, changed) sim_running={} sc_connected={} origin={:?}",
                        status.sim_running, status.simconnect_connected, status.origin_icao
                    );
                }
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
    // (v4.20.0) Excluimos OFPs ya CONSUMIDOS: si un vuelo COMPLETADO con
    // el mismo origen→destino (o linkeado a ese ofp_id) terminó DESPUÉS
    // de que el OFP fue generado, ese plan ya se voló. Antes, tras
    // terminar el vuelo, el OFP seguía siendo "reciente" (<6h) y el
    // status volvía a mostrar la ruta vieja con fase PRE-FLIGHT en el
    // panel del mapa y el badge del header (bug reportado).
    let row = sqlx::query_as::<_, LatestFlight>(
        r#"
        SELECT origin_icao, origin_name, destination_icao, destination_name,
               aircraft_icao, distance_nm
        FROM simbrief_flights sf
        WHERE generated_at IS NOT NULL
          AND CAST(generated_at AS INTEGER) > strftime('%s', 'now', '-6 hours')
          AND NOT EXISTS (
            SELECT 1 FROM flight_log fl
            WHERE fl.ended_at IS NOT NULL
              AND (
                fl.simbrief_ofp_id = sf.ofp_id
                OR (fl.origin_icao = sf.origin_icao
                    AND fl.destination_icao = sf.destination_icao)
              )
              AND CAST(strftime('%s', fl.ended_at) AS INTEGER)
                  > CAST(sf.generated_at AS INTEGER)
          )
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
        /// (v3.6.7 N2) `PLANE TOUCHDOWN NORMAL VELOCITY` en
        /// **feet/second** (negativo = descenso) — el simvar EXACTO
        /// que MSFS actualiza AT the moment of touchdown. Lo que usan
        /// todos los landing rate tools serios (LandingToast, Better
        /// Pushback, Smart Landing). Multiplicar por 60 → fpm.
        ///
        /// MSFS lo actualiza con resolución de frame (60Hz típicamente),
        /// no es un promedio sobre samples como nuestra estimación
        /// previa por altitudes. Esto da el FPM REAL del touchdown
        /// que coincide con lo que reportan los VA platforms.
        touchdown_normal_velocity_fps: f64,
        // ==========================================================
        // (v3.7.0 — Phase O) Simvars VAS-aligned para el rubric
        // expandido. Se persisten por sample en `flight_log_track`.
        // Los evaluators leen estos campos por phase (ej. flaps=0
        // durante pre-departure, light_landing=1 durante takeoff…).
        // ==========================================================
        /// `PLANE PITCH DEGREES` — pitch en RADIANES (no degrees a
        /// pesar del nombre — SimConnect quirk). Lo convertimos a
        /// degrees al consumir. Rango ±π/2 ≈ ±1.57 rad.
        pitch_radians: f64,
        /// `PLANE BANK DEGREES` — bank también en RADIANES. Misma
        /// conversión.
        bank_radians: f64,
        /// `G FORCE` — multiplicador G actual (1.0 reposo, >2 turn
        /// agresivo).
        g_force: f64,
        /// `AIRSPEED INDICATED` en knots — la lectura del IAS bug del
        /// avión. Diferente de AIRSPEED TRUE (que ya capturamos en
        /// `true_airspeed_kt`) — IAS es lo que el ATC/VAS-ACARS usa
        /// para reglas de velocidad (ej. "≤250 kt bajo 10k").
        indicated_airspeed_kt: f64,
        /// `FLAPS HANDLE PERCENT` — 0.0..1.0 (NO 0..100 como dice el
        /// nombre — SimConnect quirk). Multiplicar por 100 al
        /// consumir. 0 = retraídos, 1 = full extended.
        flaps_handle_pct: f64,
        /// `GEAR HANDLE POSITION` — 0.0 (up) / 1.0 (down). Bool.
        gear_handle: f64,
        /// `SPOILERS HANDLE POSITION` — 0.0..1.0 (otra que parece %
        /// pero es fracción). 0 = retraídos, 1 = full extended.
        spoilers_handle_pct: f64,
        /// `LIGHT NAV` — bool 0/1. Luces de navegación.
        light_nav: f64,
        /// `LIGHT BEACON` — bool 0/1. Beacon rojo rotatorio.
        light_beacon: f64,
        /// `LIGHT TAXI` — bool 0/1. Luz de taxi (rodaje).
        light_taxi: f64,
        /// `LIGHT LANDING` — bool 0/1. Luces de aterrizaje (las
        /// fuertes que se encienden bajo 10k ft y durante takeoff).
        light_landing: f64,
        /// `LIGHT STROBE` — bool 0/1. Strobes (flashes blancos en
        /// punta de ala).
        light_strobe: f64,
        /// `TRANSPONDER CODE:1` — código BCD del XPDR. Lo trataremos
        /// como i64 al persistir (4 dígitos).
        transponder_code: f64,
        // ==========================================================
        // (v4.0.0 — P4) Métricas de motor por slot 1..4. Persistidas
        // por sample en `flight_log_track` (migration 024). El
        // Performance modal de P5 las grafica con Recharts.
        //
        // Aviones con menos de 4 motores: los slots sobrantes leen
        // valores cercanos a 0 (SimConnect devuelve 0 cuando el slot
        // no está poblado). Esa convención coincide con la que ya
        // usamos para eng_combustion_3 / eng_combustion_4.
        //
        // Units requested al SDK:
        //   N1, N2   → percent
        //   EGT      → celsius
        //   Fuel FF  → pounds per hour
        //   Oil T    → celsius
        //   Oil P    → psi
        // ==========================================================
        eng_n1_1: f64,
        eng_n1_2: f64,
        eng_n1_3: f64,
        eng_n1_4: f64,
        eng_n2_1: f64,
        eng_n2_2: f64,
        eng_n2_3: f64,
        eng_n2_4: f64,
        eng_egt_1: f64,
        eng_egt_2: f64,
        eng_egt_3: f64,
        eng_egt_4: f64,
        eng_ff_pph_1: f64,
        eng_ff_pph_2: f64,
        eng_ff_pph_3: f64,
        eng_ff_pph_4: f64,
        eng_oil_temp_1: f64,
        eng_oil_temp_2: f64,
        eng_oil_temp_3: f64,
        eng_oil_temp_4: f64,
        eng_oil_press_1: f64,
        eng_oil_press_2: f64,
        eng_oil_press_3: f64,
        eng_oil_press_4: f64,
        // ==========================================================
        // (v4.0.0 P7.7) Replay/Slew detection.
        //
        // - `SIMULATION RATE`: 1.0 normal, 0.25/0.5/2.0/4.0/16.0 etc.
        //   en replay forward, ~0 al pausar. Float.
        // - `IS SLEW ACTIVE`: 1 cuando el usuario está moviendo el
        //   avión con el modo Slew (teleport, mover libremente con
        //   teclado). Bool.
        // - `ABSOLUTE TIME`: segundos desde 1/1/0001. Monotónicamente
        //   creciente salvo durante slew/replay backward, que es la
        //   señal más confiable. Float (precisión ~1ms).
        // ==========================================================
        simulation_rate: f64,
        is_slew_active: f64,
        absolute_time_s: f64,
        // (v4.0.0 P7.7 iter 2) Freeze flags. Apps de terceros tipo
        // "Flight Recorder" (SAMMaster) usan estos events SimConnect:
        //   - FREEZE_LATITUDE_LONGITUDE_SET
        //   - FREEZE_ALTITUDE_SET
        //   - FREEZE_ATTITUDE_SET
        // Durante playback, congelan el avión y le inyectan posiciones
        // pre-grabadas. Como el SIM corre normal (sim_rate=1, !slew,
        // abs_time avanza), mi P7.7 iter 1 NO detectaba esto.
        //
        // Estas simvars reportan si CUALQUIERA de los freezes está
        // activo — son la firma directa de un replay externo.
        is_lat_lon_freeze: f64,
        is_altitude_freeze: f64,
        is_attitude_freeze: f64,
        // (v4.0.0 P7.8) Altura sobre el terreno (AGL). El rubric de
        // scoring define sus fases por bandas AGL:
        //   takeoff_accel    0..1000 ft AGL
        //   initial_climb    1000..10000 ft AGL
        //   final_approach   < 1800 ft AGL
        // `PLANE ALTITUDE` (MSL) no sirve para esto — un aeropuerto a
        // 5000ft MSL tiene el avión a 6000 MSL cuando está a 1000 AGL.
        // Por eso capturamos AGL explícitamente para `derive_scoring_phase`.
        alt_above_ground_ft: f64,
        // (v4.0.0 P7.9) Weather AMBIENT — el sim sabe el clima real en
        // la posición del avión. Se persiste por track sample para que
        // el Weather modal dibuje viento/temp/nubes/lluvia REALES del
        // vuelo sin depender de APIs de clima histórico de pago.
        ambient_wind_dir_deg: f64,
        ambient_wind_vel_kt: f64,
        ambient_temp_c: f64,
        ambient_pressure_mbar: f64,
        ambient_visibility_m: f64,
        ambient_precip_state: f64,
        // (v3.21.0) STALL WARNING (bool 0/1) — para la regla "Stall
        // never indicated" del Flight Evaluation. Debe ir AL FINAL del
        // struct (y de AddToDataDefinition) para no mover offsets.
        stall_warning: f64,
        // (v3.26.0 P7.10) Warnings de daño/forzado — TAMBIÉN al final
        // (tras stall_warning), mismo orden en AddToDataDefinition.
        overspeed_warning: f64,
        oil_press_warning: f64,
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

    /// (v4.0.0 P7.6b) Struct minimal del stream de touchdown. Mismo
    /// orden de fields que el `AddToDataDefinition` que registra abajo:
    ///   1. SIM ON GROUND      (bool)
    ///   2. VERTICAL SPEED     (feet per minute)
    ///   3. RADIO HEIGHT       (feet)
    ///   4. G FORCE            (number)
    ///
    /// Sólo 4 doubles = 32 bytes. El dispatch handler lo decodifica
    /// frame-by-frame y detecta `flank 0→1` de on_ground para capturar
    /// el VS exacto del touchdown.
    #[repr(C, packed(4))]
    #[derive(Clone, Copy, Default)]
    struct TouchdownData {
        on_ground: f64,
        vertical_speed_fpm: f64,
        radio_height_ft: f64,
        g_force: f64,
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
    /// (v4.0.0 P7.6b) Stream de touchdown — frame-exacto.
    ///
    /// El stream principal (DEFINE_ID_AIRCRAFT) corre a SIM_FRAME pero
    /// con `interval=15` (cada 15 frames ≈ 4Hz). A esa resolución, el
    /// touchdown puede caer entre dos samples y perdemos el VS exacto.
    ///
    /// Este stream es minimal (32 bytes: SIM_ON_GROUND, VERTICAL_SPEED,
    /// RADIO_HEIGHT, G_FORCE) y se suscribe con `interval=1` — cada
    /// frame. A 60fps = 16ms de resolución. Es lo que usa LandingToast.
    ///
    /// Costo: ~3-5% CPU extra (Q4 del kickoff = aceptado). El struct es
    /// 4 doubles = 32 bytes × 60 Hz = ~2 KB/s de tráfico SimConnect.
    /// El handler en dispatch sólo detecta flanco 0→1 de SIM_ON_GROUND
    /// y guarda el VS instantáneo — el resto de frames se ignora con
    /// un return temprano.
    const DEFINE_ID_TOUCHDOWN: u32 = 3;
    const REQUEST_ID_TOUCHDOWN: u32 = 3;
    /// ID local arbitrario para el evento "Pause" — sólo nosotros
    /// usamos esta ID dentro del proceso, no choca con nada.
    const EVENT_ID_PAUSE: u32 = 100;
    /// Definición de campos para los parkings de un AIRPORT (v0.1.26).
    /// AddToFacilityDefinition se llama una vez con esta ID; MSFS la
    /// recuerda y la usa para todas las RequestFacilityData siguientes.
    const DEFINE_ID_AIRPORT_PARKING: u32 = 200;

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
    /// (v4.18.0) Cadencia ACELERADA del track cuando el avión RUEDA en
    /// tierra (on_ground + gs ≥ 2 kt): un punto cada 2 s ≈ 15-20 m en taxi
    /// → la traza sigue las taxiways con precisión estilo VATSIM-Radar.
    /// Con 10 s salían esquinas angulosas imposibles de suavizar sin
    /// falsear (bug reportado). En el aire se mantienen los 10 s (a 450 kt
    /// ya es suficiente densidad) y parado en gate también (el buffer
    /// pre-departure conserva su ventana de ~40 min).
    const GROUND_MOVING_PERSIST_TICKS: u32 = 2 * TICKS_PER_SECOND;
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

    // (v4.22.0) `RestoreCheck` + `restore_is_stale` eliminados — la
    // validación del restore se hace ahora vía distancia+tiempo dentro
    // de `close_stale_open_flights` y el flujo de reconexión (v2.0.4+);
    // esta copia quedó huérfana sin llamadores.

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
        // (v3.6.7 N2) "feet per second" para PLANE TOUCHDOWN NORMAL VELOCITY.
        let units_fps = sc::cstr("feet per second");
        // (v3.7.0 Phase O) Units para los nuevos simvars VAS-aligned.
        // SimConnect quirk: PITCH/BANK DEGREES vienen en RADIANES — la
        // unidad "radians" es lo que el SDK acepta. "percent over 100"
        // para FLAPS/SPOILERS HANDLE da fracción 0..1 — lo escalamos al
        // consumir. "number" para G FORCE (multiplicador). El xpdr
        // viene como número.
        let units_radians = sc::cstr("radians");
        let units_pct_over_100 = sc::cstr("percent over 100");
        let units_number = sc::cstr("number");
        // (v4.0.0 — P4) Units para simvars de motor.
        //   · "percent"          → N1/N2 (0..100, no la fracción).
        //   · "celsius"          → EGT / Oil Temp (más útil que Rankine).
        //   · "pounds per hour"  → Fuel Flow (PPH es el estándar).
        //   · "psi"              → Oil Pressure (más legible que psf).
        let units_percent = sc::cstr("percent");
        let units_celsius = sc::cstr("celsius");
        let units_pph = sc::cstr("pounds per hour");
        let units_psi = sc::cstr("psi");
        // (v4.0.0 P7.7) "seconds" para ABSOLUTE TIME (replay detection).
        let units_seconds = sc::cstr("seconds");
        // (v4.0.0 P7.9) Units para weather AMBIENT.
        let units_millibars = sc::cstr("millibars");
        let units_meters = sc::cstr("meters");

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
            // (v3.6.7 N2) Touchdown rate REAL via simvar dedicado.
            // MSFS lo actualiza AT touchdown con resolución de frame
            // (60Hz). Unit "feet per second" — multiplicar × 60 → fpm.
            // Es lo que usa LandingToast / Better Pushback / etc.
            //
            // Antes calculábamos VS de altitudes / tiempo a 4Hz, que
            // promediaba el flare + contacto → daba valores 2× los
            // reales (-365 fpm cuando real era -188). Este simvar es
            // el touchdown VS exacto.
            ("PLANE TOUCHDOWN NORMAL VELOCITY", units_fps.as_c_str()),
            // ----------------------------------------------------------
            // (v3.7.0 — Phase O) Simvars VAS-aligned. Mismo orden que
            // los campos nuevos en `AircraftData`.
            // ----------------------------------------------------------
            ("PLANE PITCH DEGREES", units_radians.as_c_str()),
            ("PLANE BANK DEGREES", units_radians.as_c_str()),
            ("G FORCE", units_number.as_c_str()),
            ("AIRSPEED INDICATED", units_knots.as_c_str()),
            ("FLAPS HANDLE PERCENT", units_pct_over_100.as_c_str()),
            ("GEAR HANDLE POSITION", units_bool.as_c_str()),
            ("SPOILERS HANDLE POSITION", units_pct_over_100.as_c_str()),
            ("LIGHT NAV", units_bool.as_c_str()),
            ("LIGHT BEACON", units_bool.as_c_str()),
            ("LIGHT TAXI", units_bool.as_c_str()),
            ("LIGHT LANDING", units_bool.as_c_str()),
            ("LIGHT STROBE", units_bool.as_c_str()),
            ("TRANSPONDER CODE:1", units_number.as_c_str()),
            // ----------------------------------------------------------
            // (v4.0.0 — P4) Métricas de motor por slot 1..4 para el
            // Performance modal de P5. Orden DEBE coincidir con los
            // campos del struct AircraftData (eng_n1_1..4, eng_n2_1..4,
            // egt_1..4, ff_pph_1..4, oil_temp_1..4, oil_press_1..4).
            // SimConnect copia los bytes en orden — un desfase aquí
            // rompe el alineamiento de TODOS los campos siguientes.
            // ----------------------------------------------------------
            ("TURB ENG N1:1", units_percent.as_c_str()),
            ("TURB ENG N1:2", units_percent.as_c_str()),
            ("TURB ENG N1:3", units_percent.as_c_str()),
            ("TURB ENG N1:4", units_percent.as_c_str()),
            ("TURB ENG N2:1", units_percent.as_c_str()),
            ("TURB ENG N2:2", units_percent.as_c_str()),
            ("TURB ENG N2:3", units_percent.as_c_str()),
            ("TURB ENG N2:4", units_percent.as_c_str()),
            ("GENERAL ENG EXHAUST GAS TEMPERATURE:1", units_celsius.as_c_str()),
            ("GENERAL ENG EXHAUST GAS TEMPERATURE:2", units_celsius.as_c_str()),
            ("GENERAL ENG EXHAUST GAS TEMPERATURE:3", units_celsius.as_c_str()),
            ("GENERAL ENG EXHAUST GAS TEMPERATURE:4", units_celsius.as_c_str()),
            ("ENG FUEL FLOW PPH:1", units_pph.as_c_str()),
            ("ENG FUEL FLOW PPH:2", units_pph.as_c_str()),
            ("ENG FUEL FLOW PPH:3", units_pph.as_c_str()),
            ("ENG FUEL FLOW PPH:4", units_pph.as_c_str()),
            ("ENG OIL TEMPERATURE:1", units_celsius.as_c_str()),
            ("ENG OIL TEMPERATURE:2", units_celsius.as_c_str()),
            ("ENG OIL TEMPERATURE:3", units_celsius.as_c_str()),
            ("ENG OIL TEMPERATURE:4", units_celsius.as_c_str()),
            ("ENG OIL PRESSURE:1", units_psi.as_c_str()),
            ("ENG OIL PRESSURE:2", units_psi.as_c_str()),
            ("ENG OIL PRESSURE:3", units_psi.as_c_str()),
            ("ENG OIL PRESSURE:4", units_psi.as_c_str()),
            // ----------------------------------------------------------
            // (v4.0.0 P7.7) Replay/Slew detection. Mismo orden que los
            // campos del struct AircraftData.
            // ----------------------------------------------------------
            ("SIMULATION RATE", units_number.as_c_str()),
            ("IS SLEW ACTIVE", units_bool.as_c_str()),
            ("ABSOLUTE TIME", units_seconds.as_c_str()),
            // (v4.0.0 P7.7 iter 2) Replay externo (Flight Recorder 3rd-party).
            ("IS LATITUDE LONGITUDE FREEZE ON", units_bool.as_c_str()),
            ("IS ALTITUDE FREEZE ON", units_bool.as_c_str()),
            ("IS ATTITUDE FREEZE ON", units_bool.as_c_str()),
            // (v4.0.0 P7.8) AGL para derive_scoring_phase.
            ("PLANE ALT ABOVE GROUND", units_feet.as_c_str()),
            // (v4.0.0 P7.9) Weather AMBIENT — orden = campos del struct.
            ("AMBIENT WIND DIRECTION", units_deg.as_c_str()),
            ("AMBIENT WIND VELOCITY", units_knots.as_c_str()),
            ("AMBIENT TEMPERATURE", units_celsius.as_c_str()),
            ("AMBIENT PRESSURE", units_millibars.as_c_str()),
            ("AMBIENT VISIBILITY", units_meters.as_c_str()),
            ("AMBIENT PRECIP STATE", units_number.as_c_str()),
            // (v3.21.0) Stall warning — al final, igual que en el struct.
            ("STALL WARNING", units_bool.as_c_str()),
            // (v3.26.0 P7.10) Warnings de daño/forzado — mismo orden que
            // los campos overspeed_warning / oil_press_warning del struct.
            ("OVERSPEED WARNING", units_bool.as_c_str()),
            ("WARNING OIL PRESSURE", units_bool.as_c_str()),
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

        // (v4.0.0 P7.6b) Stream B — touchdown frame-exacto.
        //
        // 4 vars en un struct minimal (32 bytes). Subscribed con
        // `interval=1` → cada frame del simulador. A 60fps = 16ms
        // resolución, suficiente para capturar el flank exacto de
        // SIM ON GROUND y leer el VS instantáneo del touchdown.
        //
        // Este stream se procesa en una rama separada del dispatch
        // handler (check `request_id == REQUEST_ID_TOUCHDOWN`).
        let touchdown_names: &[(&str, &std::ffi::CStr)] = &[
            ("SIM ON GROUND", units_bool.as_c_str()),
            ("VERTICAL SPEED", units_fpm.as_c_str()),
            ("RADIO HEIGHT", units_feet.as_c_str()),
            ("G FORCE", units_number.as_c_str()),
        ];
        let mut touchdown_defs_ok = true;
        for (name, unit) in touchdown_names.iter() {
            let cname = sc::cstr(name);
            let hr = unsafe {
                (lib.AddToDataDefinition)(
                    handle,
                    DEFINE_ID_TOUCHDOWN,
                    cname.as_ptr(),
                    unit.as_ptr(),
                    sc::SIMCONNECT_DATATYPE_FLOAT64,
                    0.0,
                    std::u32::MAX,
                )
            };
            if !sc::succeeded(hr) {
                tracing::warn!(
                    target: "simconnect",
                    "AddToDataDefinition touchdown '{}' falló (0x{:08x}); stream B deshabilitado",
                    name, hr
                );
                touchdown_defs_ok = false;
                break;
            }
        }
        if touchdown_defs_ok {
            // SIM_FRAME interval=1 — emit cada frame. Flag DEFAULT
            // (emit always, no filter por changes — SIM ON GROUND
            // cambia raras veces, queremos el flanco exacto).
            let hr = unsafe {
                (lib.RequestDataOnSimObject)(
                    handle,
                    REQUEST_ID_TOUCHDOWN,
                    DEFINE_ID_TOUCHDOWN,
                    SIMCONNECT_OBJECT_ID_USER,
                    SIMCONNECT_PERIOD_SIM_FRAME,
                    SIMCONNECT_DATA_REQUEST_FLAG_DEFAULT,
                    0,
                    1, // interval=1 → cada frame
                    0,
                )
            };
            if !sc::succeeded(hr) {
                tracing::warn!(
                    target: "simconnect",
                    "RequestDataOnSimObject touchdown falló (0x{:08x})",
                    hr
                );
            } else {
                tracing::info!(
                    target: "simconnect",
                    "Touchdown stream subscribed (SIM_FRAME interval=1) — frame-exact FPM capture"
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
        // (v4.2.0) RE-HABILITADO TAXI_PARKING como FALLBACK. El gate
        // preciso sigue viniendo del INI de GSX (prioridad); pero cuando
        // un aeropuerto NO tiene perfil GSX, pedimos los parkings al
        // propio MSFS (de donde GSX también los toma). El layout debe
        // coincidir EXACTO con el parser de RECV_FACILITY_DATA:
        //   AIRPORT      → [f64 LATITUDE][f64 LONGITUDE]
        //   TAXI_PARKING → [u32 TYPE][u32 NAME][u32 NUMBER][u32 SUFFIX]
        //                  [f64 BIAS_X][f64 BIAS_Y]
        let mut facility_def_ok = false;
        if let Some(add_fac) = lib.AddToFacilityDefinition {
            let fields = &[
                "OPEN AIRPORT",
                "LATITUDE",
                "LONGITUDE",
                "OPEN TAXI_PARKING",
                "TYPE",
                "NAME",
                "NUMBER",
                "SUFFIX",
                "BIAS_X",
                "BIAS_Y",
                "CLOSE TAXI_PARKING",
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
        // queremos suscribirnos a algún CDA específico.

        // (v4.0.0 P7.4b) Puente LVar OPCIONAL de MobiFlight para leer la
        // temperatura de frenos (FBW A32NX, etc.). Si el módulo WASM no
        // está instalado, esto no produce datos y nada se rompe — el
        // resto del watcher es independiente. Ver `mobiflight_lvars.rs`.
        let brake_bridge =
            unsafe { crate::mobiflight_lvars::setup(&lib, handle) };

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
        // (v4.3.0) Gate desde el menú in-sim de GSX (archivo en disco, vía
        // `crate::gsx_menu`). Cuando un aeropuerto NO tiene perfil de GSX el
        // INI no da match y la API de Facility Data del simulador no
        // devuelve nada en MSFS 2020; pero GSX SIEMPRE muestra el gate en su
        // menú. Cacheamos el último gate visto + cuándo, y recordamos cuál
        // ya aplicamos para no re-disparar en bucle. NUNCA sobrescribe el
        // gate de un perfil GSX (el INI tiene prioridad en
        // `request_gate_facility`).
        let mut last_menu_gate: Option<(String, std::time::Instant)> = None;
        let mut last_applied_menu_gate: Option<String> = None;
        let mut last_menu_read_at = std::time::Instant::now()
            .checked_sub(Duration::from_secs(60))
            .unwrap_or_else(std::time::Instant::now);
        // (v4.4.0) Último subtítulo del menú de GSX (= el gate) decodificado
        // del LVar `L:FSDT_GSX_MENU_SUBTITLE_*` vía el puente de MobiFlight.
        // Lo actualiza el dispatch de Client Data (1 Hz) y lo consume el
        // bloque de detección de gate de abajo.
        let mut latest_gsx_subtitle: Option<String> = None;
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
        // (v4.0.0 P7.5) Post-touchdown observation window. El simvar
        // `PLANE TOUCHDOWN NORMAL VELOCITY` se actualiza AT touchdown
        // pero a veces MSFS no lo tiene listo en el primer tick post-
        // touchdown (especialmente con aviones custom como Fenix). Por
        // eso, durante los ~3 segundos posteriores al touch (12 ticks
        // a 4Hz) seguimos leyendo el simvar y el `vs` y conservamos el
        // MIN (más negativo). Reset a 0 cuando salimos de Landed o
        // cuando se vence el window.
        let mut post_touchdown_ticks_remaining: u32 = 0;
        // (v4.0.0 P7.6b) State del Stream B (touchdown frame-exact).
        // `prev_on_ground` recuerda el valor previo para detectar el
        // flanco 0→1. `stream_b_recent_vs` es un buffer corto (~500ms
        // a 60fps = 30 frames) para agarrar el min VS justo antes del
        // touchdown (el frame mismo del impacto suele leer vs=0 porque
        // el avión ya está sobre el tren).
        let mut prev_on_ground: Option<bool> = None;
        let mut stream_b_recent_vs: std::collections::VecDeque<f64> =
            std::collections::VecDeque::with_capacity(30);

        // (v4.0.0 P7.7) Replay/Slew detection state.
        //
        // `in_replay_mode`: true mientras el watcher debe IGNORAR todo
        // input del sim (no persistir track, no transicionar phases,
        // no spawn de events OOOI). Se vuelve true cuando detectamos
        // cualquiera de:
        //   - SIMULATION RATE != 1.0
        //   - IS SLEW ACTIVE == 1 (Q14 del kickoff: slew cuenta igual)
        //   - ABSOLUTE TIME retrocede vs el sample anterior
        //
        // `replay_stable_ticks_normal`: ticks consecutivos en
        // condiciones normales. Necesitamos 20 ticks (~5s a 4Hz) de
        // operación normal sostenida antes de SALIR de replay mode.
        // Sin esto, un mini-blip del sim (resume desde replay) podría
        // dispararnos en una transición a media-replay.
        //
        // `prev_absolute_time_s`: para detectar retroceso temporal —
        // la señal más confiable de replay/slew según docs SimConnect.
        let mut in_replay_mode: bool = false;
        let mut replay_stable_ticks_normal: u32 = 0;
        let mut prev_absolute_time_s: Option<f64> = None;
        // (v3.30.0 #2) Buffer de muestras PRE-DEPARTURE (cold-and-dark).
        // El track sólo se persiste cuando ya existe flight_id (post-OUT),
        // así que las reglas de pre-departure salían "No data to evaluate".
        // Aquí bufferizamos un snapshot MÍNIMO (lo que leen esas reglas:
        // freno/N1/flaps/spoilers) en cada ciclo de persist mientras NO
        // hay flight_id y el avión está en tierra; al crearse el vuelo
        // (OUT) se vuelca conservando el ts original y se registra el
        // rango de fase pre_departure. Cap acotado (en handle_aircraft_data)
        // para no crecer en preflights largos.
        let mut predeparture_buffer: std::collections::VecDeque<(
            String,
            crate::flight_log::TrackPointInsert,
        )> = std::collections::VecDeque::new();
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

        // (v4.0.0 P7.4b) Máxima temperatura de frenos (°C) vista en este
        // vuelo vía el puente LVar de MobiFlight. Se persiste con
        // `touch_max_brake_temp` cuando sube y hay un vuelo abierto.
        let mut max_brake_temp_c: Option<f64> = None;

        // (v3.5.0) Cache de meta strings de la aeronave — actualizado
        // cuando llega un dispatch de REQUEST_ID_AIRCRAFT_META.
        // Lo usamos al disparar `start_flight` (OUT/OFF) para
        // poblar de entrada los campos aircraft_title / atc_type /
        // model / airline / registration, y vuelve a persistirse vía
        // `update_aircraft_meta` cuando cambia mid-flight (el usuario
        // entró al Hangar y cambió livery).
        let mut aircraft_meta_cache: Option<AircraftMeta> = None;

        // (v4.23.0) Datos para el SMART RESTORE CHECK: al recibir la
        // PRIMERA muestra de SimConnect comparamos la posición real del
        // avión contra el último punto guardado del vuelo restaurado.
        // Si está a >20 nm o el último tick tiene >6 h (PC apagada a
        // mitad de vuelo, sesión nueva del sim), el vuelo se cierra
        // como 'partial' (NO completed) y el modal de vuelos
        // incompletos pregunta al usuario si mantenerlo o eliminarlo.
        let mut restore_check: Option<(i64, f64, f64, Option<String>)> = None;
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
                restore_check =
                    Some((open.id, lat, lon, open.last_position_at.clone()));
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

                    // (v4.0.0 P7.6b) Stream B — touchdown frame-exacto.
                    // Procesa SOLO la detección de flanco 0→1 de
                    // SIM_ON_GROUND para capturar el VS exacto del
                    // touchdown. NO ejecuta state machine — ese sigue
                    // en el Stream A (4Hz). Performance: ~60 events/s
                    // pero solo guardamos VS en buffer + detectamos
                    // flanco (operaciones O(1)).
                    //
                    // (v4.0.0 P7.7 iter 3) DURANTE REPLAY: skip total.
                    // Si el watcher está en replay mode, no procesamos
                    // este dispatch — Flight Recorder y otros pueden
                    // disparar flancos artificiales al "aterrizar" un
                    // avión replayed. Sin este check, Stream B captura
                    // touchdowns falsos durante el playback.
                    if request_id == REQUEST_ID_TOUCHDOWN && in_replay_mode {
                        // Mantenemos prev_on_ground actualizado para
                        // que al salir de replay no haya un flank
                        // espurio (false→true sin razón real).
                        let data_ptr = unsafe {
                            let header_size =
                                std::mem::size_of::<sc::SIMCONNECT_RECV_SIMOBJECT_DATA>();
                            let base = p_data as *const u8;
                            base.add(header_size - 4) as *const TouchdownData
                        };
                        if !data_ptr.is_null() {
                            let td = unsafe { *data_ptr };
                            prev_on_ground = Some(td.on_ground >= 0.5);
                        }
                        stream_b_recent_vs.clear();
                        continue;
                    }
                    if request_id == REQUEST_ID_TOUCHDOWN {
                        let data_ptr = unsafe {
                            let header_size =
                                std::mem::size_of::<sc::SIMCONNECT_RECV_SIMOBJECT_DATA>();
                            let base = p_data as *const u8;
                            base.add(header_size - 4) as *const TouchdownData
                        };
                        if data_ptr.is_null() {
                            continue;
                        }
                        let td = unsafe { *data_ptr };
                        let on_ground_now = td.on_ground >= 0.5;
                        let vs_now = td.vertical_speed_fpm;
                        let radio_alt = td.radio_height_ft;
                        let g = td.g_force;

                        // Mantener buffer ~30 frames (~500ms a 60fps).
                        if vs_now.is_finite() {
                            if stream_b_recent_vs.len() >= 30 {
                                stream_b_recent_vs.pop_front();
                            }
                            stream_b_recent_vs.push_back(vs_now);
                        }

                        // Detección de flanco 0→1 = touchdown REAL.
                        // Solo dispara cuando:
                        //   1. Estamos en Airborne (no en taxi previo)
                        //   2. RADIO HEIGHT está bajo (≤30 ft típico
                        //      en touchdown — los aviones reportan
                        //      gear-to-ground height aquí)
                        //   3. El estado previo era false (no on_ground)
                        let prev = prev_on_ground.unwrap_or(false);
                        if !prev
                            && on_ground_now
                            && matches!(phase, FlightPhase::Airborne)
                            && radio_alt < 50.0
                        {
                            // (v4.0.0 P7.6b iter 2) ESTRATEGIA QUE
                            // COINCIDE CON LANDING TOAST:
                            //
                            // LandingToast usa el `live_vs` del frame
                            // exacto donde SIM_ON_GROUND transita 0→1
                            // — verificado vs nuestro log:
                            //   STREAM-B fpm=-614 (live_vs=-525.7)
                            //   LandingToast reportó -525.
                            //
                            // El min(buffer) era MÁS negativo (-614)
                            // porque incluía la pendiente del descenso
                            // pre-flare. La convención aeronáutica
                            // (FAA, LandingToast, VA platforms) usa
                            // el VS exacto AL TOQUE, no el peak del
                            // descenso → cambiamos a `live_vs`.
                            //
                            // Fallback al min(buffer_tail=5) si el live
                            // vs en este frame es 0 o positivo (caso
                            // raro donde el gear ya absorbió y vs ya
                            // está cero/positivo en el flank exacto).
                            let fpm_candidate: Option<i64> = if vs_now.is_finite()
                                && vs_now < -0.01
                            {
                                Some(vs_now as i64)
                            } else {
                                // Fallback: min de los últimos 5
                                // frames (~80ms) — captura el VS justo
                                // antes del touch sin agarrar la
                                // pendiente lejana del descenso.
                                let tail_min = stream_b_recent_vs
                                    .iter()
                                    .rev()
                                    .take(5)
                                    .copied()
                                    .filter(|v| v.is_finite())
                                    .fold(f64::INFINITY, f64::min);
                                if tail_min.is_finite() && tail_min < 0.0 {
                                    Some(tail_min as i64)
                                } else {
                                    None
                                }
                            };

                            if let Some(fpm_val) = fpm_candidate {
                                // (v4.0.0 P7.6b iter 2) Política
                                // "most negative wins" — si hay rebote
                                // posterior con valor menos negativo,
                                // CONSERVAMOS el peor (primer touch real).
                                let should_update = match captured_landing_fpm {
                                    None => true,
                                    Some(prev) => fpm_val < prev,
                                };

                                // (v4.0.0 P7.6b iter 3) Detección de
                                // rebote — si ya teníamos un FPM
                                // capturado y volvió a disparar el
                                // flanco 0→1, es porque el avión
                                // saltó del piso y volvió a tocar.
                                // LandingToast lo señala como
                                // "Bouncing Detected". Persistimos a
                                // DB para que UI lo dibuje al lado
                                // del FPM (Q del usuario v3.13.3).
                                let is_bounce = captured_landing_fpm.is_some();
                                if is_bounce {
                                    let id_opt = current_flight_id.lock().ok().and_then(|g| *g);
                                    if let Some(id) = id_opt {
                                        let pool_c = pool.clone();
                                        tokio::spawn(async move {
                                            let _ = sqlx::query(
                                                "UPDATE flight_log SET bounced = 1 WHERE id = ?1"
                                            )
                                            .bind(id)
                                            .execute(&pool_c)
                                            .await;
                                        });
                                    }
                                }

                                tracing::info!(
                                    target: "simconnect",
                                    "STREAM-B TOUCHDOWN: live_vs={:.1}fpm radio_alt={:.1}ft g={:.2} → candidate={} (prev_captured={:?} update={} bounce={})",
                                    vs_now, radio_alt, g, fpm_val, captured_landing_fpm, should_update, is_bounce
                                );

                                if should_update {
                                    captured_landing_fpm = Some(fpm_val);

                                    let id_opt = current_flight_id.lock().ok().and_then(|g| *g);
                                    if let Some(id) = id_opt {
                                        let pool_c = pool.clone();
                                        tokio::spawn(async move {
                                            let _ = crate::flight_log::touch_landing(
                                                &pool_c, id, fpm_val,
                                            )
                                            .await;
                                        });
                                    }
                                }
                            } else {
                                tracing::warn!(
                                    target: "simconnect",
                                    "STREAM-B TOUCHDOWN sin VS válido (live={:.1})",
                                    vs_now
                                );
                            }
                        }
                        prev_on_ground = Some(on_ground_now);
                        // No procesar nada más para REQUEST_ID_TOUCHDOWN
                        continue;
                    }

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
                        // (v4.23.0) Extracción INTELIGENTE: los creadores de
                        // liveries rellenan ATC AIRLINE con el dev ("Fly By
                        // Wire") y ATC ID con basura ("'"), pero el TITLE trae
                        // la aerolínea y matrícula reales. `livery_meta` valida
                        // el raw y, si es placeholder, las extrae del TITLE
                        // (diccionario de ~90 aerolíneas + regex de matrícula).
                        let raw_airline = read_cstr(&meta.atc_airline);
                        let raw_reg = read_cstr(&meta.atc_id);
                        let airline = crate::livery_meta::smart_airline(
                            raw_airline.as_deref(),
                            title.as_deref(),
                        );
                        let registration = crate::livery_meta::smart_registration(
                            raw_reg.as_deref(),
                            title.as_deref(),
                        );
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
                        // (v4.23.0) Comparamos contra los valores RAW del
                        // cache (no los sanitizados) para que el throttle del
                        // log siga funcionando.
                        let changed = match &aircraft_meta_cache {
                            None => true,
                            Some(prev) => {
                                read_cstr(&prev.title) != title
                                    || read_cstr(&prev.atc_type) != atc_type
                                    || read_cstr(&prev.atc_airline) != raw_airline
                                    || read_cstr(&prev.atc_id) != raw_reg
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

                    // (v4.23.0) SMART RESTORE CHECK — un solo intento con la
                    // primera muestra real. Si el avión apareció lejos del
                    // último punto del vuelo restaurado (>20 nm) o el último
                    // tick es viejo (>6 h), el vuelo anterior quedó huérfano
                    // (PC apagada / sim cerrado a mitad de vuelo): se cierra
                    // como 'partial' y se resetea el estado para empezar
                    // limpio. Antes ese vuelo se "completaba" falsamente al
                    // detectar engines-off en el nuevo spawn (bug reportado).
                    if let Some((rid, rlat, rlon, rts)) = restore_check.take() {
                        let dist = crate::flight_log::haversine_nm(
                            rlat, rlon, data.latitude_deg, data.longitude_deg,
                        );
                        let stale_by_time = rts
                            .as_deref()
                            .and_then(|ts| {
                                chrono::NaiveDateTime::parse_from_str(
                                    ts, "%Y-%m-%d %H:%M:%S",
                                )
                                .or_else(|_| {
                                    chrono::NaiveDateTime::parse_from_str(
                                        ts.trim_end_matches('Z'),
                                        "%Y-%m-%dT%H:%M:%S",
                                    )
                                })
                                .ok()
                            })
                            .map(|dt| {
                                chrono::Utc::now()
                                    .signed_duration_since(dt.and_utc())
                                    .num_seconds()
                                    > 6 * 60 * 60
                            })
                            .unwrap_or(false);
                        if dist > 20.0 || stale_by_time {
                            tracing::warn!(
                                target: "simconnect",
                                "restore HUÉRFANO: flight {} a {:.1} nm del último punto (stale_time={}) — cerrando como 'partial'",
                                rid, dist, stale_by_time
                            );
                            if let Ok(mut g) = current_flight_id.lock() {
                                *g = None;
                            }
                            phase = if data.on_ground >= 0.5 {
                                FlightPhase::OnGround
                            } else {
                                FlightPhase::Airborne
                            };
                            max_alt_ft = 0;
                            max_gs_kt = 0;
                            max_tas_kt = 0;
                            engines_seen_running = false;
                            let pool_c = pool.clone();
                            let app_c = app.clone();
                            std::thread::spawn(move || {
                                let rt = tokio::runtime::Builder::new_current_thread()
                                    .enable_all()
                                    .build()
                                    .ok();
                                if let Some(rt) = rt {
                                    let _ = rt.block_on(
                                        crate::flight_log::close_flight_as_partial(
                                            &pool_c, rid,
                                        ),
                                    );
                                    let _ = app_c.emit("flightlog://changed", ());
                                    let _ = app_c.emit("flightlog://incomplete", ());
                                }
                            });
                        } else {
                            tracing::info!(
                                target: "simconnect",
                                "restore VALIDADO: flight {} a {:.1} nm del último punto — continuamos el vuelo",
                                rid, dist
                            );
                        }
                    }

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
                        &mut post_touchdown_ticks_remaining,
                        &mut engines_seen_running,
                        &mut idle_ticks_in_landed,
                        &mut initial_fuel_lb,
                        &mut paused_seconds_total,
                        &mut passed_taxi_threshold,
                        &mut in_replay_mode,
                        &mut replay_stable_ticks_normal,
                        &mut prev_absolute_time_s,
                        &mut predeparture_buffer,
                    );
                    // (v4.1.1 FIX) La detección de gate usa el INI de GSX
                    // en disco (`gsx_parking::find_nearest_parking`), que
                    // NO depende de SimConnect Facility Data. Antes todo
                    // este bloque estaba envuelto en `if facility_def_ok`,
                    // así que si la API de Facility Data fallaba —
                    // justamente lo que ocurre en MSFS 2020, motivo por el
                    // que la abandonamos— TODA la detección de gate moría en
                    // silencio aunque el INI de GSX existiera. Desacoplado:
                    // los triggers corren siempre; el INI hace el trabajo.
                    let _ = facility_def_ok;

                    // (v4.6.3 FIX) Al DESPEGAR (flanco a Airborne):
                    //   1. Si el gate de SALIDA no se persistió pero lo
                    //      detectamos en preflight (SharedState.current_gate),
                    //      lo guardamos AHORA en departure_gate — así no se
                    //      pierde aunque el trigger del OUT no lo escribiera
                    //      (p. ej. el avión se alejó >75 m del parking en el
                    //      pushback antes del OUT).
                    //   2. Limpiamos current_gate y la caché del menú GSX. El
                    //      gate de salida NO debe reaparecer como gate de
                    //      LLEGADA en el destino — ese era el bug: como
                    //      current_gate (de la salida) nunca se limpiaba,
                    //      finish_flight lo reusaba de fallback para arrival.
                    //      v4.5.0 solo limpió la caché del MENÚ; faltaba esto.
                    if !matches!(prev_phase, FlightPhase::Airborne)
                        && matches!(phase, FlightPhase::Airborne)
                    {
                        last_menu_gate = None;
                        last_applied_menu_gate = None;
                        let fid =
                            current_flight_id.lock().ok().and_then(|g| *g);
                        let state_c = state.clone();
                        let pool_c = pool.clone();
                        let app_c = app.clone();
                        tokio::spawn(async move {
                            // Tomar (y limpiar) el gate de salida del
                            // SharedState para que no contamine la llegada.
                            let dep_gate = {
                                let mut guard = state_c.lock().await;
                                guard.status.current_gate.take()
                            };
                            if let (Some(id), Some(g)) = (fid, dep_gate) {
                                // Solo rellena si departure_gate está vacío
                                // (no pisa un valor del INI ya guardado).
                                let r = sqlx::query(
                                    "UPDATE flight_log SET departure_gate = ?1 \
                                     WHERE id = ?2 AND (departure_gate IS NULL OR departure_gate = '')",
                                )
                                .bind(g)
                                .bind(id)
                                .execute(&pool_c)
                                .await;
                                if matches!(r, Ok(res) if res.rows_affected() > 0) {
                                    let _ = app_c.emit("flightlog://changed", ());
                                }
                            }
                        });
                    }

                    // (v4.3.0) Leer el gate que GSX muestra en su menú
                    // in-sim (archivo en disco; NO toca la API del
                    // simulador). Cubre aeropuertos SIN perfil de GSX.
                    // Mientras el avión está parado en tierra leemos el
                    // archivo cada ~1 s; al ver un gate NUEVO disparamos la
                    // detección con el rol correcto (preflight a la salida,
                    // arrival a la llegada). `request_gate_facility` da
                    // prioridad al INI de GSX, así que esto NUNCA
                    // sobrescribe un perfil GSX.
                    {
                        let m_lat = data.latitude_deg;
                        let m_lon = data.longitude_deg;
                        let m_pos_real =
                            m_lat.abs() > 0.01 || m_lon.abs() > 0.01;
                        let m_stationary = m_pos_real
                            && data.ground_velocity_kt < 1.0
                            && (matches!(phase, FlightPhase::OnGround)
                                || matches!(phase, FlightPhase::Landed));
                        if m_stationary
                            && last_menu_read_at.elapsed()
                                >= Duration::from_secs(1)
                        {
                            last_menu_read_at = std::time::Instant::now();
                            if let Some(menu_g) = crate::gsx_menu::gate_from_menu(
                                latest_gsx_subtitle.as_deref(),
                            ) {
                                last_menu_gate = Some((
                                    menu_g.clone(),
                                    std::time::Instant::now(),
                                ));
                                if last_applied_menu_gate.as_deref()
                                    != Some(menu_g.as_str())
                                {
                                    last_applied_menu_gate =
                                        Some(menu_g.clone());
                                    let role = if matches!(
                                        phase,
                                        FlightPhase::Landed
                                    ) {
                                        "arrival"
                                    } else {
                                        "preflight"
                                    };
                                    tracing::info!(
                                        target: "simconnect",
                                        "GSX menú muestra gate \"{}\" → trigger {} (fallback sin perfil GSX)",
                                        menu_g, role
                                    );
                                    let seq = if role == "arrival" {
                                        &mut next_gate_seq_arr
                                    } else {
                                        &mut next_gate_seq_pre
                                    };
                                    request_gate_facility(
                                        lib,
                                        handle,
                                        pool,
                                        app,
                                        state,
                                        role,
                                        m_lat,
                                        m_lon,
                                        &current_flight_id,
                                        seq,
                                        &mut pending_gates,
                                        Some(menu_g.as_str()),
                                    );
                                }
                            }
                        }
                    }
                    // Gate del menú GSX más reciente que siga fresco
                    // (≤30 min). Se pasa a los triggers de gate de abajo
                    // como fallback cuando el INI no da match — p. ej. al
                    // OUT, con el menú ya cerrado, reusamos el que vimos
                    // mientras el usuario pedía servicios en el preflight.
                    let menu_gate_fresh: Option<String> = last_menu_gate
                        .as_ref()
                        .filter(|(_, t)| {
                            t.elapsed() < Duration::from_secs(30 * 60)
                        })
                        .map(|(g, _)| g.clone());

                    {
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
                                menu_gate_fresh.as_deref(),
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
                                menu_gate_fresh.as_deref(),
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
                                        menu_gate_fresh.as_deref(),
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
                                    menu_gate_fresh.as_deref(),
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

                    // (v4.0.0 P7.4b) ¿Es el stream de brake temps del
                    // puente LVar de MobiFlight? Si sí, leemos el máximo
                    // y lo persistimos cuando crece y hay vuelo abierto.
                    if let Some(bridge) = brake_bridge.as_ref() {
                        if request_id == bridge.request_id {
                            if let Some(t) = unsafe {
                                crate::mobiflight_lvars::parse_max_brake_temp(
                                    p_data as *const sc::SIMCONNECT_RECV_CLIENT_DATA,
                                    bridge.brake_count,
                                )
                            } {
                                let grew =
                                    max_brake_temp_c.map_or(true, |m| t > m + 0.5);
                                if max_brake_temp_c.map_or(true, |m| t > m) {
                                    max_brake_temp_c = Some(t);
                                }
                                if grew {
                                    let fid = current_flight_id
                                        .lock()
                                        .ok()
                                        .and_then(|g| *g);
                                    if let Some(fid) = fid {
                                        let pool2 = pool.clone();
                                        tokio::runtime::Handle::current().spawn(
                                            async move {
                                                if let Err(e) =
                                                    crate::flight_log::touch_max_brake_temp(
                                                        &pool2, fid, t,
                                                    )
                                                    .await
                                                {
                                                    tracing::warn!(
                                                        target: "lvar_bridge",
                                                        "touch_max_brake_temp falló: {e:#}"
                                                    );
                                                }
                                            },
                                        );
                                    }
                                }
                            }
                            // (v4.4.0) El subtítulo del menú de GSX = el
                            // gate cuando el menú principal está abierto.
                            // Lo decodificamos del slot LVar y lo guardamos;
                            // el bloque de gate (SIMOBJECT_DATA) lo combina
                            // con el título del archivo `menu` y lo aplica.
                            latest_gsx_subtitle = unsafe {
                                crate::mobiflight_lvars::parse_gate_subtitle(
                                    p_data as *const sc::SIMCONNECT_RECV_CLIENT_DATA,
                                    bridge.subtitle_len_index,
                                    bridge.subtitle_chunk_count,
                                )
                            };
                            continue;
                        }
                    }

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
        post_touchdown_ticks_remaining: &mut u32,
        engines_seen_running: &mut bool,
        idle_ticks_in_landed: &mut u32,
        initial_fuel_lb: &mut Option<f64>,
        paused_seconds_total: &mut u64,
        passed_taxi_threshold: &mut bool,
        in_replay_mode: &mut bool,
        replay_stable_ticks_normal: &mut u32,
        prev_absolute_time_s: &mut Option<f64>,
        predeparture_buffer: &mut std::collections::VecDeque<(
            String,
            crate::flight_log::TrackPointInsert,
        )>,
    ) {
        // (v4.0.0 P7.7) Replay/Slew detection FIRST — antes de
        // procesar cualquier otra lógica del state machine.
        //
        // Tres señales:
        //   1. SIMULATION RATE != 1.0 (replay forward/backward o
        //      tiempo acelerado). Tolerancia: 0.95..1.05 normal.
        //   2. IS SLEW ACTIVE == 1 (Q14: slew cuenta como replay).
        //   3. ABSOLUTE TIME retrocede vs sample anterior.
        //      → la señal más fuerte; con replay forward se puede
        //      confundir con un blip pero el retroceso es indiscutible.
        //
        // Entrada al replay mode: instantánea (1 tick basta).
        // Salida del replay mode: 20 ticks (~5s a 4Hz) sostenidos
        //   de condiciones normales. Sin esto, podríamos salir
        //   prematuramente en un tick aislado de SIMULATION RATE=1.
        let sim_rate = data.simulation_rate;
        let slew_active = data.is_slew_active >= 0.5;
        let abs_time = data.absolute_time_s;
        let time_regressed = match *prev_absolute_time_s {
            Some(prev) if abs_time.is_finite() && prev.is_finite() => {
                abs_time + 0.001 < prev
            }
            _ => false,
        };
        *prev_absolute_time_s = if abs_time.is_finite() { Some(abs_time) } else { None };
        let rate_abnormal = sim_rate.is_finite() && (sim_rate < 0.95 || sim_rate > 1.05);
        // (v4.0.0 P7.7 iter 2) Detección de replay externo.
        let lat_lon_frozen = data.is_lat_lon_freeze >= 0.5;
        let altitude_frozen = data.is_altitude_freeze >= 0.5;
        let attitude_frozen = data.is_attitude_freeze >= 0.5;
        // (v3.20.0) GSX hace el pushback congelando lat/lon + altitud +
        // actitud del avión EN TIERRA, y algunos handlers además activan
        // slew brevemente. Eso disparaba un FALSO "replay/slew" en cada
        // pushback. Un replay real (Flight Recorder, replay nativo de
        // MSFS) reproduce el vuelo EN EL AIRE. Por eso las señales de
        // freeze y slew sólo cuentan como replay cuando el avión está
        // EN VUELO. En tierra, freeze/slew = ops normales (GSX, jetway,
        // reposicionar) y se ignoran. `sim_rate` y `time_regress` siguen
        // contando siempre (GSX no los causa).
        let on_ground = data.on_ground >= 0.5;
        let airborne = !on_ground;
        let any_freeze = lat_lon_frozen || altitude_frozen || attitude_frozen;
        // (v3.29.0 #1) HISTÉRESIS entrar/permanecer en replay.
        //
        // Señales "duras" (rate de sim anormal, tiempo que retrocede) las
        // causa SÓLO un replay/slew — GSX nunca las produce. Cuentan
        // siempre, en aire o en tierra.
        //
        // Señales "blandas" (freeze de lat-lon/alt/actitud, slew activo):
        //   · Para ENTRAR exigimos `airborne`. GSX hace el pushback
        //     congelando el avión EN TIERRA; sin el gate, cada pushback
        //     daría un falso "replay".
        //   · Para PERMANECER NO exigimos airborne. Un replay (nativo o
        //     Flight Recorder) que YA aterrizó sigue congelado en tierra.
        //     El bug previo: al tocar pista el avión pasaba a on_ground,
        //     `airborne` caía a false, las señales blandas se anulaban y
        //     el watcher SALÍA de replay justo en el touchdown → procesaba
        //     un aterrizaje FALSO. Manteniendo las señales blandas activas
        //     en tierra mientras ya estamos en replay, el replay sigue
        //     detectado durante y después del aterrizaje.
        let hard_signal = rate_abnormal || time_regressed;
        let soft_signal = any_freeze || slew_active;
        let is_replay_enter = hard_signal || (airborne && soft_signal);
        let is_replay_stay = hard_signal || soft_signal;
        let is_replay_now = if *in_replay_mode {
            is_replay_stay
        } else {
            is_replay_enter
        };

        // Determinar la causa primaria para el payload UI. En tierra sólo
        // etiquetamos freeze/slew como causa si YA estamos en replay
        // (coherente con la histéresis de arriba).
        let soft_counts = airborne || *in_replay_mode;
        let cause = if any_freeze && soft_counts {
            "external_replay"
        } else if slew_active && soft_counts {
            "slew"
        } else if time_regressed {
            "time_regress"
        } else if rate_abnormal {
            "sim_rate"
        } else {
            "normal"
        };

        if is_replay_now {
            if !*in_replay_mode {
                tracing::warn!(
                    target: "simconnect",
                    "REPLAY/SLEW DETECTADO — cause={} sim_rate={:.2} slew={} time_regressed={} ll_freeze={} alt_freeze={} att_freeze={} — freezing state machine",
                    cause, sim_rate, slew_active, time_regressed,
                    lat_lon_frozen, altitude_frozen, attitude_frozen
                );
                *in_replay_mode = true;
                // (v4.0.0 P7.7 iter 3) Reset captured_landing_fpm.
                // Si Stream B había capturado un FPM antes del replay
                // y todavía no se persistió en finish, no queremos
                // que un dispatch durante el replay reuse ese valor.
                // El próximo touchdown REAL (post-cooldown) tendrá
                // que volver a capturar desde 0.
                *captured_landing_fpm = None;
                *post_touchdown_ticks_remaining = 0;
                #[derive(serde::Serialize, Clone)]
                #[serde(rename_all = "camelCase")]
                struct ReplayPayload {
                    active: bool,
                    sim_rate: f64,
                    is_slew: bool,
                    is_external_replay: bool,
                    cause: String,
                }
                let _ = app.emit(
                    "flight://replay",
                    ReplayPayload {
                        active: true,
                        sim_rate,
                        is_slew: slew_active,
                        is_external_replay: any_freeze,
                        cause: cause.to_string(),
                    },
                );
            }
            *replay_stable_ticks_normal = 0;
        } else if *in_replay_mode {
            *replay_stable_ticks_normal = replay_stable_ticks_normal.saturating_add(1);
            // (v4.0.0 P7.7 iter 3) Threshold ampliado: 20→60 ticks
            // (5s→15s) sostenidos de operación normal. Motivación:
            // tras un replay externo (Flight Recorder), el avión puede
            // quedar en estado físico extraño (aire+vs=0 en pista, o
            // ground+motores OFF). Con solo 5s el state machine
            // resumía y disparaba transiciones espurias (touchdowns
            // falsos, engine shutdowns que cerraban el vuelo abierto).
            // 15s da margen para que el avión "se asiente" o para que
            // el usuario inicie/finalice manualmente.
            if *replay_stable_ticks_normal >= 60 {
                tracing::info!(
                    target: "simconnect",
                    "REPLAY/SLEW desactivado — operación normal sostenida {}s, resumiendo state machine",
                    *replay_stable_ticks_normal / 4
                );
                *in_replay_mode = false;
                *replay_stable_ticks_normal = 0;
                #[derive(serde::Serialize, Clone)]
                #[serde(rename_all = "camelCase")]
                struct ReplayPayload {
                    active: bool,
                    sim_rate: f64,
                    is_slew: bool,
                    is_external_replay: bool,
                    cause: String,
                }
                let _ = app.emit(
                    "flight://replay",
                    ReplayPayload {
                        active: false,
                        sim_rate,
                        is_slew: false,
                        is_external_replay: false,
                        cause: "normal".to_string(),
                    },
                );
            }
        }

        // Mientras estemos en replay, NO procesamos nada. Salimos.
        // Esto previene:
        //   · Persistencia de track samples con coords del replay.
        //   · Transiciones OOOI (OUT/OFF/ON/IN) falsas.
        //   · Updates de max_altitude / max_gs con valores re-vividos.
        //   · Captura de touchdown FPM durante replay (Stream B sigue
        //     pero no escribe porque captured_landing_fpm es leído
        //     desde Stream A, que no ejecuta este path).
        if *in_replay_mode {
            return;
        }

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

        // (v4.0.0 P7.5) Post-touchdown observation window. La
        // transición Airborne→Landed setea `post_touchdown_ticks_remaining`
        // a 12 (3s @ 4Hz). Mientras esté > 0 y sigamos en Landed,
        // releemos el simvar `PLANE TOUCHDOWN NORMAL VELOCITY` y el
        // `vs` en CADA tick, conservando el MIN (más negativo) si
        // mejora el valor capturado. Esto cubre el caso donde el
        // simvar todavía no estaba listo al primer tick post-touch
        // (Fenix A319, FBW A380, otros aviones third-party que
        // actualizan el simvar 1-3 frames más tarde).
        //
        // Persistimos en DB sólo cuando el MIN baja (no spam de
        // touch_landing). Al salir de Landed (touch-and-go o
        // IN/OnGround), no necesitamos reset explícito — la transición
        // del state machine lo hace.
        if matches!(*phase, FlightPhase::Landed) && *post_touchdown_ticks_remaining > 0 {
            *post_touchdown_ticks_remaining -= 1;
            // Lee simvar otra vez.
            let live_simvar = data.touchdown_normal_velocity_fps;
            let live_simvar_fpm = if live_simvar.is_finite() && live_simvar < -0.01 {
                Some((live_simvar * 60.0) as i64)
            } else {
                None
            };
            // Live vs (instantáneo).
            let live_vs_fpm = if vs.is_finite() && vs < -1.0 {
                Some(vs as i64)
            } else {
                None
            };
            let candidate = match (live_simvar_fpm, live_vs_fpm) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (Some(a), None) => Some(a),
                (None, Some(b)) => Some(b),
                (None, None) => None,
            };
            if let Some(new_fpm) = candidate {
                let improved = match *captured_landing_fpm {
                    None => true,
                    Some(prev) => new_fpm < prev,
                };
                if improved {
                    tracing::info!(
                        target: "simconnect",
                        "POST-TOUCHDOWN obs: landing_fpm {:?} → {} (simvar={:.3}fps vs={:.1}fpm ticks_left={})",
                        *captured_landing_fpm,
                        new_fpm,
                        live_simvar,
                        vs,
                        *post_touchdown_ticks_remaining,
                    );
                    *captured_landing_fpm = Some(new_fpm);
                    let id_opt = current_flight_id.lock().ok().and_then(|g| *g);
                    if let Some(id) = id_opt {
                        let pool_c = pool.clone();
                        tokio::spawn(async move {
                            let _ = crate::flight_log::touch_landing(&pool_c, id, new_fpm).await;
                        });
                    }
                }
            }
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
                    // Nota: pasamos `None, None` para aircraft meta —
                    // al OUT el dispatch de meta puede no haber llegado
                    // todavía. `populate_simbrief_async` (que corre
                    // después y lee aircraft_atc del DB tras
                    // update_aircraft_meta) hace el matching real con
                    // validación de aircraft (P7.6).
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
                        // (v4.0.0 P7.5b) populate_simbrief_async hace
                        // scoring multi-factor: reg + aircraft + fuel +
                        // freshness. Pasamos el fuel_lb capturado AL OUT
                        // (justo arriba) para usarlo como discriminador.
                        let pool_b = pool_c.clone();
                        let app_b = app_c.clone();
                        let fuel_lb_at_out = fuel_lb;
                        tokio::spawn(async move {
                            // Delay 2s para que update_aircraft_meta tenga
                            // tiempo de poblar aircraft_atc_type y
                            // aircraft_registration en el DB. El dispatch
                            // de meta llega típicamente 1-3s después del
                            // OUT (FLAG_DEFAULT = 1s polling).
                            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                            populate_simbrief_async(
                                &pool_b,
                                &app_b,
                                id,
                                lat_c,
                                lon_c,
                                Some(fuel_lb_at_out),
                                false, // OUT: pre-populate clásico (badge en vivo)
                            ).await;
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
                    // (v4.6.0) Salida cancelada SIN despegar: el usuario
                    // salió del gate (motores + movimiento) pero volvió a
                    // frenar y apagar motores sin volar. Esto NO es un
                    // vuelo — es lo típico tras ver un replay o reposicionar
                    // el avión en el gate para cambiar de aeronave o
                    // empezar otro vuelo. Antes lo cerrábamos como "vuelo
                    // vacío" (destino = origen) y quedaba un fantasma en el
                    // FlightBook. Ahora BORRAMOS la fila: un vuelo que nunca
                    // estuvo en el aire no cuenta. Los vuelos REALES se
                    // cierran por la rama (Landed → OnGround), nunca por
                    // aquí, así que esto jamás borra un vuelo de verdad.
                    let id_opt = current_flight_id
                        .lock()
                        .ok()
                        .and_then(|mut g| g.take());
                    if let Some(id) = id_opt {
                        let pool_c = pool.clone();
                        let app_c = app.clone();
                        tokio::spawn(async move {
                            if let Err(e) =
                                crate::flight_log::delete_entry(&pool_c, id).await
                            {
                                tracing::error!(
                                    target: "simconnect",
                                    "delete_entry (cancel OUT sin despegar) falló: {e:#}"
                                );
                            } else {
                                tracing::info!(
                                    target: "simconnect",
                                    "BLOCK-OUT CANCELADO — flight_log id={} BORRADO (nunca despegó: replay/reposición en gate)",
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
                    //
                    // (v3.6.7 N2) PRIORIZAR el simvar dedicado
                    // PLANE TOUCHDOWN NORMAL VELOCITY (ft/s × 60 = fpm).
                    // MSFS lo actualiza AT the moment of touchdown con
                    // resolución de frame (60Hz). Es lo que usa
                    // LandingToast y los VA platforms para reportar
                    // landing rate exacto.
                    //
                    // El simvar viene NEGATIVO al descender (convención
                    // SimConnect: Y positivo arriba). Multiplicamos × 60
                    // para convertir ft/s → ft/min.
                    //
                    // (v4.0.0 — P6.3 fix) **Sólo aceptar valores
                    // NEGATIVOS** del simvar. El simvar `PLANE
                    // TOUCHDOWN NORMAL VELOCITY` reporta el VS del
                    // touchdown — para un aterrizaje real es siempre
                    // negativo (descenso). Casos donde sale positivo:
                    //   · Avión rebotando (`bounce`) — segundo contacto
                    //     desde abajo, velocidad neta positiva.
                    //   · Aviones de terceros con modelado dudoso que
                    //     no actualizan el signo correctamente.
                    //   · El simvar no se actualizó (~0) → consideramos
                    //     no-touchdown válido.
                    //
                    // Si el simvar da algo no-negativo, **caemos al
                    // fallback** del min(recent_vs) que SÍ filtra
                    // por signo. Sin este check, el watcher persistía
                    // valores positivos y el frontend los filtraba al
                    // render → el FPM aparecía en logs pero "no se
                    // pintaba en la app" (bug reportado P6.3).
                    let touchdown_fps_raw = data.touchdown_normal_velocity_fps;
                    let touchdown_fpm = if touchdown_fps_raw.is_finite()
                        && touchdown_fps_raw < -0.01
                    {
                        Some((touchdown_fps_raw * 60.0) as i64)
                    } else {
                        None
                    };
                    let fallback_fpm = {
                        let landing_vs = recent_vs
                            .iter()
                            .copied()
                            .fold(f64::INFINITY, f64::min);
                        if landing_vs.is_finite() && landing_vs < 0.0 {
                            Some(landing_vs as i64)
                        } else {
                            None
                        }
                    };
                    let fpm = touchdown_fpm.or(fallback_fpm);
                    if let Some(v) = fpm {
                        tracing::info!(
                            target: "simconnect",
                            "STREAM-A TOUCHDOWN candidate={} (simvar_fps={:.3} → {} fpm) (fallback_min_vs={:?})",
                            v,
                            touchdown_fps_raw,
                            touchdown_fpm.unwrap_or(0),
                            fallback_fpm
                        );
                    }
                    // (v4.0.0 P7.6b iter 2) NO sobrescribir si Stream B
                    // ya capturó algo más negativo. Stream B corre a
                    // SIM_FRAME interval=1 (60Hz) y es frame-exacto.
                    // Stream A corre a interval=15 (~4Hz) y puede
                    // disparar 19s después del touchdown REAL (cuando
                    // gs<50 — fin del landing roll). Si llegamos acá
                    // con captured_landing_fpm ya seteado por Stream B,
                    // ese valor ya es el verdadero.
                    let should_update_from_a = match (*captured_landing_fpm, fpm) {
                        (None, Some(_)) => true,
                        (Some(prev), Some(new_v)) => new_v < prev,
                        _ => false,
                    };
                    if should_update_from_a {
                        *captured_landing_fpm = fpm;
                        let id_opt = current_flight_id.lock().ok().and_then(|g| *g);
                        if let Some(id) = id_opt {
                            if let Some(fpm_val) = fpm {
                                let pool_c = pool.clone();
                                tokio::spawn(async move {
                                    let _ = crate::flight_log::touch_landing(
                                        &pool_c, id, fpm_val,
                                    )
                                    .await;
                                });
                            }
                        }
                    }
                    // (v4.0.0 P7.5) Abrir window de observación post-
                    // touchdown — 12 ticks ≈ 3s a 4Hz. Durante ese
                    // window seguimos leyendo el simvar y `vs` en
                    // cada tick (en el branch `else` de Landed más
                    // abajo) y actualizamos `captured_landing_fpm`
                    // con el MIN observado. Cubre casos donde el
                    // simvar todavía no estaba listo al ingresar a
                    // Landed (Fenix A319 / aviones third-party).
                    *post_touchdown_ticks_remaining = 12;
                    tracing::info!(
                        target: "simconnect",
                        "TOUCHDOWN (phase transition Airborne→Landed) — captured_landing_fpm={:?} (stream_a_proposed={:?} updated={})",
                        *captured_landing_fpm, fpm, should_update_from_a
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
                                    // (v4.0.0 P7.5b) Pasamos None para fuel
                                    // — al IN ya el fuel cambió por consumo,
                                    // no es comparable con plan_ramp. El
                                    // scorer cae al match por reg+type+
                                    // freshness, que ya es robusto.
                                    // (v4.17.0) confirm_mode=true: al apagado
                                    // de motores NO se auto-linkea — se emite
                                    // simbrief://confirm con los candidatos y
                                    // el usuario decide en el modal.
                                    populate_simbrief_async(&pool_c, &app_c, id, lat_c, lon_c, None, true).await;
                                    let _ = app_c.emit("flightlog://changed", ());

                                    // (v3.6.0 Phase H — H13) Cierra el
                                    // último flight_log_phase pendiente
                                    // (exited_at = NULL), luego corre
                                    // scoring + auto-upload al cloud
                                    // (Q15 del kickoff). NO bloquea —
                                    // si el cloud falla, el vuelo queda
                                    // local OK y los eventos emiten el
                                    // detalle al frontend.
                                    let _ = sqlx::query(
                                        r#"
                                        UPDATE flight_log_phase
                                        SET exited_at = ?1
                                        WHERE flight_id = ?2 AND exited_at IS NULL
                                        "#,
                                    )
                                    .bind(chrono::Utc::now()
                                        .format("%Y-%m-%dT%H:%M:%SZ")
                                        .to_string())
                                    .bind(id)
                                    .execute(&pool_c)
                                    .await;

                                    crate::scoring::finalize_after_finish(
                                        &pool_c, &app_c, id,
                                    )
                                    .await;
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
        // (v4.18.0) Cadencia adaptativa: rodando en tierra (gs ≥ 2 kt) un
        // punto cada 2 s para que el track siga las taxiways con precisión;
        // en el aire o parado, los 10 s de siempre.
        let persist_interval = if on_ground && gs >= 2.0 {
            GROUND_MOVING_PERSIST_TICKS
        } else {
            PERSIST_INTERVAL_TICKS
        };
        if *ticks_since_persist >= persist_interval {
            *ticks_since_persist = 0;
            let id_opt = current_flight_id.lock().ok().and_then(|g| *g);
            // (v3.30.0 #2) Captura/flush PRE-DEPARTURE (cold-and-dark).
            match id_opt {
                None => {
                    // Sin vuelo todavía. Bufferizamos un snapshot MÍNIMO
                    // (lo que leen las reglas pre-departure) SÓLO en tierra
                    // y con posición plausible (evita basura de menús /
                    // lat-lon 0,0). No crea filas en DB — sólo memoria.
                    if on_ground
                        && lat.is_finite()
                        && lon.is_finite()
                        && !(lat.abs() < 0.01 && lon.abs() < 0.01)
                    {
                        const PREDEP_BUFFER_CAP: usize = 240; // ~40min @ 10s
                        let now_ts = chrono::Utc::now()
                            .format("%Y-%m-%dT%H:%M:%SZ")
                            .to_string();
                        let n1_max = [
                            data.eng_n1_1,
                            data.eng_n1_2,
                            data.eng_n1_3,
                            data.eng_n1_4,
                        ]
                        .into_iter()
                        .filter(|v| v.is_finite())
                        .fold(None, |acc: Option<f64>, v| {
                            Some(acc.map_or(v, |a| a.max(v)))
                        });
                        let pt = crate::flight_log::TrackPointInsert {
                            lat,
                            lon,
                            alt_ft: Some(alt as i64),
                            gs_kt: Some(gs as i64),
                            parking_brake: Some(data.parking_brake >= 0.5),
                            flaps_pct: Some(
                                (data.flaps_handle_pct * 100.0).round() as i64,
                            ),
                            spoilers_pct: Some(
                                (data.spoilers_handle_pct * 100.0).round() as i64,
                            ),
                            eng_n1: [n1_max, None, None, None],
                            scoring_phase: Some("pre_departure".to_string()),
                            ..Default::default()
                        };
                        if predeparture_buffer.len() >= PREDEP_BUFFER_CAP {
                            predeparture_buffer.pop_front();
                        }
                        predeparture_buffer.push_back((now_ts, pt));
                    }
                }
                Some(id) => {
                    // Ya hay flight_id (post-OUT). Volcamos el buffer
                    // cold-and-dark conservando los ts ORIGINALES y
                    // registramos UN rango de fase pre_departure cerrado
                    // (para la regla "ample pre-departure time"). drain()
                    // garantiza que se hace una sola vez.
                    if !predeparture_buffer.is_empty() {
                        let drained: Vec<(String, crate::flight_log::TrackPointInsert)> =
                            predeparture_buffer.drain(..).collect();
                        let pool_f = pool.clone();
                        tokio::spawn(async move {
                            let first_ts = drained.first().map(|(t, _)| t.clone());
                            let last_ts = drained.last().map(|(t, _)| t.clone());
                            let n = drained.len();
                            for (ts, pt) in drained {
                                if let Err(e) =
                                    crate::flight_log::insert_track_point_full_at(
                                        &pool_f, id, &ts, pt,
                                    )
                                    .await
                                {
                                    tracing::warn!(
                                        target: "simconnect",
                                        "flush pre-departure: insert falló: {e:#}"
                                    );
                                }
                            }
                            if let (Some(a), Some(b)) = (first_ts, last_ts) {
                                if let Err(e) = sqlx::query(
                                    "INSERT INTO flight_log_phase (flight_id, phase, entered_at, exited_at) VALUES (?1, 'pre_departure', ?2, ?3)",
                                )
                                .bind(id)
                                .bind(&a)
                                .bind(&b)
                                .execute(&pool_f)
                                .await
                                {
                                    tracing::warn!(
                                        target: "simconnect",
                                        "flush pre-departure: phase range falló: {e:#}"
                                    );
                                }
                            }
                            tracing::info!(
                                target: "simconnect",
                                "pre-departure: volcadas {} muestras cold-and-dark al vuelo {}",
                                n, id
                            );
                        });
                    }
                }
            }
            if let Some(id) = id_opt {
                let pool_c = pool.clone();
                let lat_c = lat;
                let lon_c = lon;
                // (v4.0.0 P7.8c) Calculamos el scoring_phase de ESTE
                // sample y lo escribimos EN el track point — no en una
                // tabla separada. Esto elimina el race de timestamps que
                // causaba "No data to evaluate" en fases cortas.
                let scoring_phase_c = derive_scoring_phase(
                    *phase,
                    on_ground,
                    gs,
                    data.alt_above_ground_ft,
                    vs,
                    any_engine_running,
                    *passed_taxi_threshold,
                    in_pushback,
                    *engines_seen_running,
                );
                // (v4.0.0 P7.9) Snapshot weather AMBIENT del sim. NaN
                // guard: SimConnect ocasionalmente emite NaN durante
                // carga; lo convertimos a None para no escribir basura.
                let fin = |v: f64| if v.is_finite() { Some(v) } else { None };
                let wind_dir_c = fin(data.ambient_wind_dir_deg).map(|v| v.round() as i64);
                let wind_speed_c = fin(data.ambient_wind_vel_kt).map(|v| v.round() as i64);
                let oat_c_c = fin(data.ambient_temp_c);
                let baro_hpa_c = fin(data.ambient_pressure_mbar).map(|v| v.round() as i64);
                let visibility_c = fin(data.ambient_visibility_m).map(|v| v.round() as i64);
                let precip_c = fin(data.ambient_precip_state).map(|v| v as i64);
                // (v3.19.0 P7.9c) Nubes de la VIDA REAL (Open-Meteo) en la
                // posición actual. MSFS 2020 no las expone por SimConnect,
                // así que tomamos el dato real cacheado (refresco en
                // background cada ~3 min / 20 NM — no bloquea el watcher) y
                // lo estampamos en este sample para mostrarlo como histórico.
                let cloud = crate::openmeteo::cloud_for_position(lat, lon);
                let cloud_cover_c = cloud.total_pct;
                let cloud_low_c = cloud.low_pct;
                let cloud_mid_c = cloud.mid_pct;
                let cloud_high_c = cloud.high_pct;
                // (v3.21.0) Stall warning del sim (bool) para el scoring.
                let stall_warning_c = data.stall_warning >= 0.5;
                // (v3.26.0 P7.10) Warnings de daño/forzado (bool).
                let overspeed_warning_c = data.overspeed_warning >= 0.5;
                let oil_press_warning_c = data.oil_press_warning >= 0.5;
                let alt_c = alt as i64;
                let gs_c = gs as i64;
                // (v3.7.0 — Phase O) Snapshot de todos los simvars
                // VAS-aligned para persistir junto al punto. Algunos
                // necesitan conversión de unidad — radians→degrees,
                // fracción→pct, f64→bool.
                let pitch_deg_c = data.pitch_radians.to_degrees();
                let bank_deg_c = data.bank_radians.to_degrees();
                let g_force_c = data.g_force;
                let ias_kt_c = data.indicated_airspeed_kt as i64;
                let vs_fpm_c = vs as i64;
                let flaps_pct_c = (data.flaps_handle_pct * 100.0).round() as i64;
                let spoilers_pct_c = (data.spoilers_handle_pct * 100.0).round() as i64;
                let gear_down_c = data.gear_handle >= 0.5;
                let light_nav_c = data.light_nav >= 0.5;
                let light_beacon_c = data.light_beacon >= 0.5;
                let light_taxi_c = data.light_taxi >= 0.5;
                let light_landing_c = data.light_landing >= 0.5;
                let light_strobe_c = data.light_strobe >= 0.5;
                let parking_brake_c = parking_brake_set;
                let transponder_c = data.transponder_code as i64;
                // (v4.0.0 — P4) Snapshot de motor por slot 1..4. Para
                // aviones con menos motores los slots sobrantes leen
                // ~0 desde SimConnect — el frontend (P5) los descarta
                // si todos los samples del slot son 0/NULL.
                let eng_n1_c = [
                    data.eng_n1_1,
                    data.eng_n1_2,
                    data.eng_n1_3,
                    data.eng_n1_4,
                ];
                let eng_n2_c = [
                    data.eng_n2_1,
                    data.eng_n2_2,
                    data.eng_n2_3,
                    data.eng_n2_4,
                ];
                let eng_egt_c = [
                    data.eng_egt_1,
                    data.eng_egt_2,
                    data.eng_egt_3,
                    data.eng_egt_4,
                ];
                let eng_ff_pph_c = [
                    data.eng_ff_pph_1,
                    data.eng_ff_pph_2,
                    data.eng_ff_pph_3,
                    data.eng_ff_pph_4,
                ];
                let eng_oil_temp_c = [
                    data.eng_oil_temp_1,
                    data.eng_oil_temp_2,
                    data.eng_oil_temp_3,
                    data.eng_oil_temp_4,
                ];
                let eng_oil_press_c = [
                    data.eng_oil_press_1,
                    data.eng_oil_press_2,
                    data.eng_oil_press_3,
                    data.eng_oil_press_4,
                ];
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
                    // los puntos. Incluye TODOS los simvars VAS-aligned
                    // para que el scoring engine los lea por phase.
                    let pt = crate::flight_log::TrackPointInsert {
                        lat: lat_c,
                        lon: lon_c,
                        alt_ft: Some(alt_c),
                        gs_kt: Some(gs_c),
                        ias_kt: Some(ias_kt_c),
                        vs_fpm: Some(vs_fpm_c),
                        pitch_deg: Some(pitch_deg_c),
                        bank_deg: Some(bank_deg_c),
                        g_force: Some(g_force_c),
                        flaps_pct: Some(flaps_pct_c),
                        gear_down: Some(gear_down_c),
                        spoilers_pct: Some(spoilers_pct_c),
                        light_nav: Some(light_nav_c),
                        light_beacon: Some(light_beacon_c),
                        light_taxi: Some(light_taxi_c),
                        light_landing: Some(light_landing_c),
                        light_strobe: Some(light_strobe_c),
                        parking_brake: Some(parking_brake_c),
                        transponder_code: Some(transponder_c),
                        eng_n1: eng_n1_c.map(Some),
                        eng_n2: eng_n2_c.map(Some),
                        eng_egt: eng_egt_c.map(Some),
                        eng_ff_pph: eng_ff_pph_c.map(Some),
                        eng_oil_temp: eng_oil_temp_c.map(Some),
                        eng_oil_press: eng_oil_press_c.map(Some),
                        scoring_phase: scoring_phase_c,
                        wind_dir_deg: wind_dir_c,
                        wind_speed_kt: wind_speed_c,
                        oat_c: oat_c_c,
                        baro_hpa: baro_hpa_c,
                        visibility_m: visibility_c,
                        precip_state: precip_c,
                        cloud_cover_pct: cloud_cover_c,
                        cloud_low_pct: cloud_low_c,
                        cloud_mid_pct: cloud_mid_c,
                        cloud_high_pct: cloud_high_c,
                        stall_warning: Some(stall_warning_c),
                        overspeed_warning: Some(overspeed_warning_c),
                        oil_press_warning: Some(oil_press_warning_c),
                    };
                    if let Err(e) = crate::flight_log::insert_track_point_full(
                        &pool_c, id, pt,
                    )
                    .await
                    {
                        tracing::warn!(
                            target: "simconnect",
                            "insert_track_point_full falló: {e:#}"
                        );
                    }
                });
            }
        }

        // (v3.6.0 Phase H — H11) Persist phase transitions a
        // `flight_log_phase` para que el scoring engine tenga ventanas
        // temporales por phase. La query es idempotente: cierra el
        // phase anterior abierto si difiere del nuevo, y crea el nuevo
        // sólo si no existe ya un row abierto con el mismo phase.
        //
        // Se ejecuta junto al persist del track (cada ~10s) — esa
        // resolución cubre el caso "phase cambia y dura segundos"
        // (pushback → taxi → takeoff suelen tomar varios segundos cada
        // uno, así que el muestreo es suficiente).
        //
        // (v4.0.0 P7.8) Persistimos el SCORING phase (nombres del
        // rubric, bandas AGL) — NO el label del badge UI. Antes
        // persistíamos `phase_label` ("climbing"/"approach") que el
        // rubric no reconocía → "No data to evaluate" en Cruise /
        // Final Approach. Ahora derive_scoring_phase genera
        // initial_climb / final_approach / pre_departure / etc.
        if *ticks_since_persist == 0 {
            let id_opt = current_flight_id.lock().ok().and_then(|g| *g);
            let scoring_phase = derive_scoring_phase(
                *phase,
                on_ground,
                gs,
                data.alt_above_ground_ft,
                vs,
                any_engine_running,
                *passed_taxi_threshold,
                in_pushback,
                *engines_seen_running,
            );
            if let (Some(id), Some(label)) = (id_opt, scoring_phase) {
                let pool_c = pool.clone();
                let label_c = label.clone();
                tokio::spawn(async move {
                    let now = chrono::Utc::now()
                        .format("%Y-%m-%dT%H:%M:%SZ")
                        .to_string();
                    if let Err(e) = sqlx::query(
                        r#"
                        UPDATE flight_log_phase
                        SET exited_at = ?1
                        WHERE flight_id = ?2
                          AND exited_at IS NULL
                          AND phase != ?3
                        "#,
                    )
                    .bind(&now)
                    .bind(id)
                    .bind(&label_c)
                    .execute(&pool_c)
                    .await
                    {
                        tracing::warn!(
                            target: "simconnect",
                            "flight_log_phase close failed: {e:#}"
                        );
                    }
                    if let Err(e) = sqlx::query(
                        r#"
                        INSERT INTO flight_log_phase
                            (flight_id, phase, entered_at)
                        SELECT ?2, ?3, ?1
                        WHERE NOT EXISTS (
                            SELECT 1 FROM flight_log_phase
                            WHERE flight_id = ?2
                              AND phase = ?3
                              AND exited_at IS NULL
                        )
                        "#,
                    )
                    .bind(&now)
                    .bind(id)
                    .bind(&label_c)
                    .execute(&pool_c)
                    .await
                    {
                        tracing::warn!(
                            target: "simconnect",
                            "flight_log_phase insert failed: {e:#}"
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
                // (v4.5.1) Log del emit SÓLO cuando cambia el resumen
                // (sim/conn/gate/fase). Antes lo escribía cada ~10s aunque
                // no hubiera novedad, inflando el log. El emit a la UI sí
                // sigue saliendo cada intervalo; solo se calla el log.
                static LAST_EMIT_SIG: std::sync::OnceLock<std::sync::Mutex<Option<String>>> =
                    std::sync::OnceLock::new();
                let sig = format!(
                    "{} {} {:?} {:?}",
                    snapshot.sim_running,
                    snapshot.simconnect_connected,
                    snapshot.current_gate,
                    snapshot.phase_label
                );
                let cell = LAST_EMIT_SIG.get_or_init(|| std::sync::Mutex::new(None));
                if let Ok(mut last) = cell.lock() {
                    if last.as_deref() != Some(sig.as_str()) {
                        tracing::info!(
                            target: "simconnect",
                            "emit flight://current sim_running={} sc_connected={} gate={:?} phase={:?}",
                            snapshot.sim_running,
                            snapshot.simconnect_connected,
                            snapshot.current_gate,
                            snapshot.phase_label
                        );
                        *last = Some(sig);
                    }
                }
                let _ = app.emit("flight://current", &snapshot);
                *ticks_since_emit = 0;
            }
        }
    }

    /// (v4.5.0) Loguea el resultado de detección de gate SÓLO cuando
    /// cambia respecto al último logueado — evita repetir la misma línea
    /// cada vez que el trigger periódico (preflight cada ~15 min, etc.)
    /// re-evalúa el mismo gate sin cambios. Dedup global por el texto.
    fn log_gate_outcome_once(msg: &str) {
        use std::sync::Mutex;
        static LAST: Mutex<Option<String>> = Mutex::new(None);
        if let Ok(mut g) = LAST.lock() {
            if g.as_deref() == Some(msg) {
                return;
            }
            *g = Some(msg.to_string());
        }
        tracing::info!(target: "simconnect", "{}", msg);
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
        // (v4.3.0) Gate que GSX muestra en su menú in-sim (leído de disco
        // por `crate::gsx_menu`). Sólo se usa como FALLBACK cuando el INI
        // de GSX no da match — NUNCA sobrescribe un perfil GSX.
        menu_gate: Option<&str>,
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

        // Helper local: aplica un gate YA resuelto — emite a la UI
        // (SharedState.current_gate) y, para departure/arrival, lo persiste
        // en DB. Reutilizado por el INI de GSX y por el menú de GSX.
        let apply_gate = |name: String, source: &'static str| {
            log_gate_outcome_once(&format!(
                "gate {} {} → \"{}\" ({})",
                role, icao, name, source
            ));
            // 1) SharedState + emit inmediato (incluye preflight, para que
            //    el chip "Volando ahora" pinte el gate ANTES del OUT).
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
            // 2) Persistir a DB para departure/arrival. Preflight no lo
            //    necesita: el gate viaja por SharedState.current_gate;
            //    finish_flight lo usa como fallback_arrival_gate y el
            //    trigger "departure" del OUT lo persiste a departure_gate.
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
                    let _ =
                        crate::flight_log::update_entry(&pool_c, fid, &input)
                            .await;
                });
            }
        };

        // **PRIORIDAD: GSX INI parser en disco (preciso).** Si hay match, lo
        // usamos y terminamos — el fallback NO corre, así que nunca
        // sobrescribe el gate de un perfil GSX.
        if let Some(parking) = crate::gsx_parking::find_nearest_parking(
            &icao, player_lat, player_lon,
        ) {
            apply_gate(parking.name.clone(), "GSX INI");
            return;
        }

        // (v4.3.0) FALLBACK sin perfil GSX → usamos el gate que GSX muestra
        // en su MENÚ in-sim (leído de disco por `crate::gsx_menu`). GSX
        // SIEMPRE sabe el gate aunque no haya perfil. NO usamos la API de
        // Facility Data del simulador (TAXI_PARKING): en MSFS 2020 no
        // devuelve nada — confirmado por el usuario. Esto NUNCA sobrescribe
        // un gate de GSX porque el INI tiene prioridad y ya hizo `return`.
        if let Some(gate) = menu_gate {
            apply_gate(gate.to_string(), "GSX menú");
            return;
        }

        log_gate_outcome_once(&format!(
            "gate {} {}: sin perfil GSX ni menú GSX abierto → gate vacío (editable a mano)",
            role, icao
        ));
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
        app: &AppHandle,
        flight_id: i64,
        lat: f64,
        lon: f64,
        fuel_loaded_lb: Option<f64>,
        // (v4.17.0) `true` cuando se invoca al CIERRE del vuelo (apagado de
        // motores): en ese modo NUNCA se auto-linkea — se emiten los OFP
        // candidatos para que el usuario confirme en el modal (cuenta
        // SimBrief compartida). `false` en el OUT: comportamiento clásico
        // (pre-populate del badge en vivo).
        confirm_mode: bool,
    ) {
        // (v4.0.0 P7.5b) Leemos meta del vuelo incluyendo
        // aircraft_registration que update_aircraft_meta poblaró por
        // dispatch SimConnect (ATC ID simvar).
        let row: Option<(
            Option<String>,    // origin_icao
            Option<String>,    // aircraft_atc_type
            Option<String>,    // aircraft_registration
            Option<String>,    // simbrief_ofp_id
        )> = sqlx::query_as(
            "SELECT origin_icao, aircraft_atc_type, aircraft_registration, simbrief_ofp_id FROM flight_log WHERE id = ?1"
        )
        .bind(flight_id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();
        let (origin_from_db, aircraft_atc_from_db, aircraft_reg_from_db, ofp_id_from_db) = match row {
            Some((a, b, c, d)) => (a, b, c, d),
            None => (None, None, None, None),
        };

        // (v4.0.0 P7.5b) Si ya hay OFP linked, hacemos re-scoring
        // (no solo aircraft type) — si el score no llega a viable,
        // lo deslinkamos. Esto cubre el caso de start_flight pre-v3.11
        // que pudo haber linkeado por matching legacy.
        // (v4.17.0) En confirm_mode este bloque se SALTA por completo
        // (filter !confirm_mode): al cierre el usuario decide en el modal;
        // no re-validamos ni deslinkeamos por nuestra cuenta.
        if let Some(linked_ofp_id) =
            ofp_id_from_db.as_deref().filter(|s| !s.is_empty() && !confirm_mode)
        {
            // Re-validamos: si la decisión inicial sigue siendo el top
            // ganador del scorer actual, la respetamos. Si no, deslinkamos.
            let candidates_result = crate::simbrief::score_simbrief_candidates(
                pool,
                origin_from_db.as_deref().unwrap_or(""),
                aircraft_atc_from_db.as_deref(),
                aircraft_reg_from_db.as_deref(),
                fuel_loaded_lb,
                7 * 24,
                Some(flight_id),
            ).await;
            let still_valid = match &candidates_result {
                Ok(crate::simbrief::MatchResult::Auto { ofp, .. }) => {
                    ofp.ofp_id == linked_ofp_id
                }
                Ok(crate::simbrief::MatchResult::Ambiguous { candidates }) => {
                    candidates.iter().any(|c| c.ofp.ofp_id == linked_ofp_id)
                }
                _ => false,
            };
            if still_valid {
                tracing::info!(
                    target: "simbrief",
                    "linked ofp_id={} validado para flight_id={} — manteniendo link",
                    linked_ofp_id, flight_id
                );
                return;
            }
            tracing::warn!(
                target: "simbrief",
                "linked ofp_id={} no pasa scoring actual para flight_id={} — desligando",
                linked_ofp_id, flight_id
            );
            let _ = sqlx::query(
                "UPDATE flight_log SET simbrief_ofp_id = NULL, flight_number = NULL, callsign = NULL, airline_icao = NULL WHERE id = ?1"
            )
            .bind(flight_id)
            .execute(pool)
            .await;
        }

        // Resolver origen para el query.
        let icao = match origin_from_db.clone() {
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

        // Score multi-factor (P7.5b).
        let scored = match crate::simbrief::score_simbrief_candidates(
            pool,
            &icao,
            aircraft_atc_from_db.as_deref(),
            aircraft_reg_from_db.as_deref(),
            fuel_loaded_lb,
            7 * 24,
            Some(flight_id),
        )
        .await
        {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(
                    target: "simbrief",
                    "score_simbrief_candidates falló: {e:#}"
                );
                return;
            }
        };

        // (v4.17.0) Apagado de motores con cuenta SimBrief compartida:
        // NUNCA auto-linkeamos. Construimos la lista de OFP candidatos —
        // el mejor puntuado primero, luego el resto de OFPs recientes del
        // caché local (simbrief_flights) — y la emitimos para que el
        // usuario confirme en el modal (carrusel SÍ/NO). La doble
        // validación por matrícula ocurre al confirmar (link_simbrief_ofp).
        if confirm_mode {
            let mut ordered: Vec<crate::simbrief::SimBriefFlight> = Vec::new();
            match scored {
                crate::simbrief::MatchResult::Auto { ofp, .. } => ordered.push(ofp),
                crate::simbrief::MatchResult::Ambiguous { candidates } => {
                    ordered.extend(candidates.into_iter().map(|c| c.ofp));
                }
                crate::simbrief::MatchResult::NoMatch => {}
            }
            if let Ok(all) = crate::simbrief::list_flights(pool).await {
                for f in all {
                    if ordered.len() >= 10 {
                        break;
                    }
                    if !ordered.iter().any(|o| o.ofp_id == f.ofp_id) {
                        ordered.push(f);
                    }
                }
            }
            if ordered.is_empty() {
                tracing::info!(
                    target: "simbrief",
                    "confirm: caché de OFPs vacío para flight {} — nada que confirmar",
                    flight_id
                );
                return;
            }
            #[derive(serde::Serialize, Clone)]
            #[serde(rename_all = "camelCase")]
            struct ConfirmPayload {
                flight_id: i64,
                candidates: Vec<crate::simbrief::SimBriefFlight>,
            }
            tracing::info!(
                target: "simbrief",
                "confirm: emitiendo {} OFP candidatos para flight {} (modal de confirmación)",
                ordered.len(),
                flight_id
            );
            let _ = app.emit(
                "simbrief://confirm",
                ConfirmPayload { flight_id, candidates: ordered },
            );
            return;
        }

        match scored {
            crate::simbrief::MatchResult::Auto { ofp, score, breakdown } => {
                let derived_airline = crate::flight_log::derive_airline_icao(ofp.callsign.as_deref());
                let res = sqlx::query(
                    r#"
                    UPDATE flight_log
                    SET simbrief_ofp_id  = ?1,
                        flight_number    = COALESCE(flight_number, ?2),
                        callsign         = COALESCE(callsign, ?3),
                        airline_icao     = COALESCE(airline_icao, ?4),
                        passengers       = COALESCE(passengers, ?5),
                        cargo_kg         = COALESCE(cargo_kg, ?6),
                        fuel_used_kg     = COALESCE(fuel_used_kg, ?7),
                        route_fixes      = COALESCE(NULLIF(route_fixes, '[]'), ?9)
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
                .bind(
                    serde_json::to_string(&ofp.route_fixes)
                        .unwrap_or_else(|_| "[]".to_string()),
                )
                .execute(pool)
                .await;
                match res {
                    Ok(_) => {
                        tracing::info!(
                            target: "simbrief",
                            "AUTO-LINK ofp_id={} → flight_id={} score={} breakdown={:?} reg_sim={:?} ac_sim={:?} fuel_sim_lb={:?} fn={:?} cs={:?}",
                            ofp.ofp_id, flight_id, score, breakdown,
                            aircraft_reg_from_db, aircraft_atc_from_db, fuel_loaded_lb,
                            ofp.flight_number, ofp.callsign
                        );
                        let _ = app.emit("flightlog://changed", ());
                    }
                    Err(e) => tracing::warn!(
                        target: "simbrief",
                        "AUTO-LINK UPDATE falló para ofp_id={} flight_id={}: {e:#}",
                        ofp.ofp_id, flight_id
                    ),
                }
            }
            crate::simbrief::MatchResult::Ambiguous { candidates } => {
                // (v4.19.1) Ya NO se emite `simbrief://ambiguous` ni existe
                // el toast de elección en pleno vuelo — el usuario decidió
                // que el ÚNICO flujo de atribución es el modal de
                // confirmación al apagar motores (confirm_mode arriba).
                // El vuelo queda sin atribuir hasta entonces.
                tracing::info!(
                    target: "simbrief",
                    "AMBIGUOUS — {} candidatos viables para flight_id={} reg_sim={:?} ac_sim={:?} — se resolverá en el modal de confirmación al cierre",
                    candidates.len(), flight_id, aircraft_reg_from_db, aircraft_atc_from_db
                );
            }
            crate::simbrief::MatchResult::NoMatch => {
                tracing::info!(
                    target: "simbrief",
                    "NO MATCH para flight_id={} icao={} reg_sim={:?} ac_sim={:?} — VA meta queda NULL",
                    flight_id, icao, aircraft_reg_from_db, aircraft_atc_from_db
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
        // (v4.24.0) ROLLOUT del aterrizaje: si venimos del aire (enum aún
        // Airborne) y ya estamos EN TIERRA, eso es un aterrizaje — jamás
        // "takeoff". El bug (visto en el vuelo 150 en SBGR): el branch
        // on_ground del takeoff atrapaba el rollout (gs 139→69
        // decelerando) porque el enum tarda unos ticks en pasar a Landed,
        // y el badge mostraba TAKEOFF justo al tocar pista.
        if matches!(phase, FlightPhase::Airborne) && on_ground {
            return if gs >= 30.0 {
                "landed_rollout".to_string()
            } else {
                "taxi_in".to_string()
            };
        }

        // Takeoff roll: SOLO desde BlockOut en tierra (carrera de
        // despegue real), o recién levantado con VS POSITIVO (v4.22.0 —
        // el corto final descendiendo no cuenta; un touch-and-go sí).
        if gs >= 60.0
            && ((matches!(phase, FlightPhase::BlockOut) && on_ground)
                || (matches!(phase, FlightPhase::BlockOut | FlightPhase::Airborne)
                    && !on_ground
                    && alt_ft < 500.0
                    && vs_fpm > 100.0))
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

    /// (v4.0.0 P7.8) Deriva la fase de SCORING — los nombres EXACTOS
    /// que el rubric (`scoring::rubric::Phase::as_str`) busca, usando
    /// bandas de altitud AGL.
    ///
    /// Esto es DISTINTO de `derive_phase_label` (que alimenta el badge
    /// UI con labels amigables "climbing"/"approach"). El bug que esto
    /// arregla: el watcher persistía el label del badge a
    /// `flight_log_phase`, pero el rubric busca nombres diferentes
    /// (`initial_climb`, `final_approach`, `pre_departure`, etc.) y por
    /// bandas AGL — resultado: "No data to evaluate" en Cruise / Final
    /// Approach / etc.
    ///
    /// Nombres generados (deben coincidir 1:1 con `Phase::as_str`):
    ///   pre_departure | pushback | taxi_out | takeoff | takeoff_accel |
    ///   initial_climb | climb | cruise | descent | initial_approach |
    ///   final_approach | landing | taxi_in | arrived
    ///
    /// Bandas AGL (del rubric):
    ///   takeoff_accel    0..1000 ft AGL  (subiendo)
    ///   initial_climb    1000..10000 ft AGL (subiendo)
    ///   final_approach   < 1800 ft AGL (bajando)
    ///   initial_approach 1800..10000 ft AGL (bajando)
    #[allow(clippy::too_many_arguments)]
    fn derive_scoring_phase(
        phase: FlightPhase,
        on_ground: bool,
        gs: f64,
        agl_ft: f64,
        vs_fpm: f64,
        any_engine_running: bool,
        passed_taxi_threshold: bool,
        in_pushback: bool,
        engines_seen_running: bool,
    ) -> Option<String> {
        // (v4.24.0) ROLLOUT del aterrizaje: viniendo del aire (enum aún
        // Airborne) + en tierra = fase "landing" (gs alto) o taxi_in.
        // Antes el branch on_ground del takeoff etiquetaba el rollout
        // como "takeoff" (vuelo 150 en SBGR: 23 s de rollout robados a
        // la fase landing — afectaba la regla de spoilers desplegados).
        if matches!(phase, FlightPhase::Airborne) && on_ground {
            return Some(if gs >= 40.0 {
                "landing".to_string()
            } else {
                "taxi_in".to_string()
            });
        }

        // Takeoff roll: SOLO desde BlockOut en tierra, o recién
        // levantado con VS positivo (v4.22.0).
        if gs >= 60.0
            && ((matches!(phase, FlightPhase::BlockOut) && on_ground)
                || (!on_ground && agl_ft < 50.0 && vs_fpm > 100.0))
        {
            return Some("takeoff".to_string());
        }

        let gsx_towing = matches!(phase, FlightPhase::OnGround)
            && on_ground
            && !any_engine_running
            && gs > 0.3;
        if (in_pushback || gsx_towing)
            && matches!(phase, FlightPhase::OnGround | FlightPhase::BlockOut)
        {
            return Some("pushback".to_string());
        }

        match phase {
            // Antes de despegar: pre_departure (incluye preflight,
            // engine start, gate). El rubric agrupa todo lo previo al
            // pushback acá.
            FlightPhase::Disconnected => None,
            FlightPhase::OnGround => {
                // Si ya voló (engines_seen + Landed antes) → arrived.
                if engines_seen_running && !any_engine_running {
                    Some("arrived".to_string())
                } else {
                    Some("pre_departure".to_string())
                }
            }
            FlightPhase::BlockOut => {
                if passed_taxi_threshold {
                    Some("taxi_out".to_string())
                } else {
                    Some("pushback".to_string())
                }
            }
            FlightPhase::Airborne => {
                let climbing = vs_fpm > 300.0;
                let descending = vs_fpm < -300.0;
                if climbing {
                    if agl_ft < 1000.0 {
                        Some("takeoff_accel".to_string())
                    } else if agl_ft < 10_000.0 {
                        Some("initial_climb".to_string())
                    } else {
                        Some("climb".to_string())
                    }
                } else if descending {
                    if agl_ft < 1800.0 {
                        Some("final_approach".to_string())
                    } else if agl_ft < 10_000.0 {
                        Some("initial_approach".to_string())
                    } else {
                        Some("descent".to_string())
                    }
                } else {
                    // vs ≈ 0
                    if agl_ft < 1800.0 {
                        // Nivelado bajo = final approach (steady glide).
                        Some("final_approach".to_string())
                    } else if agl_ft >= 10_000.0 {
                        Some("cruise".to_string())
                    } else {
                        // 1800..10000 nivelado — transición, lo más
                        // útil para scoring es initial_approach si
                        // veníamos bajando, pero sin historial lo
                        // dejamos cruise (banda intermedia).
                        Some("cruise".to_string())
                    }
                }
            }
            FlightPhase::Landed => {
                if gs >= 40.0 {
                    // Rollout justo tras el touchdown.
                    Some("landing".to_string())
                } else if gs >= 3.0 {
                    Some("taxi_in".to_string())
                } else {
                    Some("arrived".to_string())
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
