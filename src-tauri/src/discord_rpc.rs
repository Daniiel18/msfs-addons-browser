//! (v7) **Discord Rich Presence** — muestra en tu perfil de Discord qué estás
//! haciendo en SimFleet: "EHAM → KJFK · PMDG 737-800" con el cronómetro del
//! vuelo, o un estado genérico cuando no estás volando.
//!
//! Diseño:
//!   · Vive en un `std::thread` propio (el cliente de `discord-rich-presence`
//!     es SÍNCRONO) → no toca el runtime async ni el hilo de UI.
//!   · Lee el estado en vivo del watcher (`SharedState`) y los ajustes desde
//!     SQLite en cada tick, así el toggle surte efecto **sin reiniciar**.
//!   · Respeta el rate-limit de Discord (~1 update/15 s) y sólo envía cuando
//!     el texto CAMBIA.
//!   · Si Discord no está abierto, falla en silencio y reintenta más tarde:
//!     nunca bloquea ni molesta al usuario.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use discord_rich_presence::{
    activity::{Activity, Assets, Timestamps},
    DiscordIpc, DiscordIpcClient,
};
use sqlx::SqlitePool;
use tauri::Manager;

/// Cada cuánto miramos el estado (Discord limita ~1 update cada 15 s).
const TICK: Duration = Duration::from_secs(15);
/// Espera antes de reintentar cuando Discord no está disponible.
const RETRY: Duration = Duration::from_secs(60);
/// Clave del asset de imagen grande subido en el portal de Discord.
const LARGE_IMAGE: &str = "simfleet";
/// Application ID de SimFleet en el portal de Discord. Va en el código para
/// que funcione **sin configurar nada** en cualquier PC; el ajuste
/// `discord_app_id` sigue existiendo por si algún día se quiere sobrescribir.
const DEFAULT_APP_ID: &str = "1531802142648569926";

/// Lo que queremos mostrar en Discord ahora mismo.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct Presence {
    details: String,
    state: String,
    /// Epoch (s) de inicio, para el cronómetro "hace X min".
    start: Option<i64>,
}

/// Arranca el hilo del Rich Presence. Idempotente por proceso.
pub fn spawn(pool: SqlitePool, app: tauri::AppHandle) {
    std::thread::Builder::new()
        .name("discord-rpc".into())
        .spawn(move || run(pool, app))
        .ok();
}

fn run(pool: SqlitePool, app: tauri::AppHandle) {
    let mut client: Option<DiscordIpcClient> = None;
    let mut current_id = String::new();
    let mut last: Option<Presence> = None;
    let mut next_retry = SystemTime::now();

    loop {
        std::thread::sleep(TICK);

        let (enabled, app_id) = read_settings(&pool);
        // Apagado (o sin App ID) → soltar el cliente y no hacer nada.
        if !enabled || app_id.trim().is_empty() {
            if let Some(mut c) = client.take() {
                let _ = c.clear_activity();
                let _ = c.close();
            }
            last = None;
            continue;
        }
        // Cambió el App ID en Ajustes → reconectar con el nuevo.
        if app_id != current_id {
            if let Some(mut c) = client.take() {
                let _ = c.close();
            }
            current_id = app_id.clone();
            last = None;
        }
        // (Re)conectar, respetando el backoff si Discord no está abierto.
        if client.is_none() {
            if SystemTime::now() < next_retry {
                continue;
            }
            let mut c = DiscordIpcClient::new(current_id.clone());
            match c.connect() {
                Ok(()) => {
                    tracing::info!(target: "discord", "Rich Presence conectado");
                    client = Some(c);
                }
                Err(e) => {
                    tracing::debug!(target: "discord", "Discord no disponible: {e}");
                    next_retry = SystemTime::now() + RETRY;
                    continue;
                }
            }
        }

        let want = build_presence(&app);
        if last.as_ref() == Some(&want) {
            continue; // sin cambios → no gastamos rate-limit
        }
        let Some(c) = client.as_mut() else { continue };

        let mut activity = Activity::new()
            .details(want.details.clone())
            .state(want.state.clone())
            .assets(
                Assets::new()
                    .large_image(LARGE_IMAGE)
                    .large_text("SimFleet"),
            );
        if let Some(start) = want.start {
            activity = activity.timestamps(Timestamps::new().start(start));
        }

        match c.set_activity(activity) {
            Ok(()) => last = Some(want),
            Err(e) => {
                // Discord se cerró → soltar y reintentar más tarde.
                tracing::debug!(target: "discord", "set_activity falló: {e}");
                let _ = c.close();
                client = None;
                last = None;
                next_retry = SystemTime::now() + RETRY;
            }
        }
    }
}

/// Lee el toggle y el App ID de la tabla `settings` (bloqueante, en este hilo).
fn read_settings(pool: &SqlitePool) -> (bool, String) {
    let fetch = |key: &'static str| -> Option<String> {
        tauri::async_runtime::block_on(async {
            sqlx::query_as::<_, (String,)>("SELECT value FROM settings WHERE key = ?1")
                .bind(key)
                .fetch_optional(pool)
                .await
                .ok()
                .flatten()
                .map(|r| r.0)
        })
    };
    // (v7) APAGADO por defecto: aunque no pisa la presencia de MSFS (Discord
    // apila varias actividades), en la fila única de la lista de miembros sólo
    // se muestra una y el orden no es controlable — el usuario prefiere no
    // arriesgarse. Se enciende a voluntad desde Ajustes → Discord.
    let enabled = fetch("pref_discord_rpc_enabled")
        .map(|v| matches!(v.as_str(), "1" | "true" | "yes"))
        .unwrap_or(false);
    // El ajuste sólo se usa si el usuario puso uno propio; si no, el del código.
    let app_id = fetch("discord_app_id")
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| DEFAULT_APP_ID.to_string());
    (enabled, app_id)
}

/// Construye el texto a mostrar a partir del estado en vivo del sim.
fn build_presence(app: &tauri::AppHandle) -> Presence {
    let status = app
        .try_state::<crate::simconnect_watcher::SharedState>()
        .map(|s| tauri::async_runtime::block_on(async { s.lock().await.status.clone() }));

    let Some(st) = status else {
        return idle();
    };
    // Sin sim abierto → estado genérico.
    if !st.sim_running {
        return idle();
    }

    // Ruta: origen → destino (del OFP), o el aeropuerto actual como respaldo.
    let route = match (st.origin_icao.as_deref(), st.destination_icao.as_deref()) {
        (Some(o), Some(d)) if !o.is_empty() && !d.is_empty() => format!("{o} → {d}"),
        _ => st
            .current_airport_icao
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(|a| crate::tr!("discord.atAirport", airport = a))
            .unwrap_or_else(|| crate::tr!("discord.flying")),
    };

    // Avión: título real del sim, o la matrícula/aerolínea como respaldo.
    let aircraft = st
        .aircraft_title
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .or(st.aircraft_icao.as_deref().filter(|s| !s.trim().is_empty()))
        .or(st
            .aircraft_registration
            .as_deref()
            .filter(|s| !s.trim().is_empty()))
        .unwrap_or("")
        .to_string();

    // Segunda línea: avión + fase (traducida a algo legible) + altitud.
    let mut bits: Vec<String> = Vec::new();
    if !aircraft.is_empty() {
        bits.push(aircraft);
    }
    if let Some(p) = st.phase_label.as_deref().map(phase_text) {
        bits.push(p);
    }
    if let Some(alt) = st.current_alt_ft {
        if alt > 1000 {
            bits.push(format!("FL{:03}", (alt as f64 / 100.0).round() as i64));
        }
    }

    Presence {
        details: route,
        state: bits.join(" · "),
        start: open_flight_start(app),
    }
}

/// Estado cuando no hay sim corriendo.
fn idle() -> Presence {
    Presence {
        details: crate::tr!("discord.idleDetails"),
        state: crate::tr!("discord.idleState"),
        start: None,
    }
}

/// Epoch (s) del inicio del vuelo abierto — alimenta el cronómetro de Discord.
fn open_flight_start(app: &tauri::AppHandle) -> Option<i64> {
    let state = app.try_state::<crate::AppState>()?;
    let pool = state.db.clone();
    let started: Option<String> = tauri::async_runtime::block_on(async move {
        sqlx::query_as::<_, (String,)>(
            "SELECT started_at FROM flight_log WHERE status = 'open' \
             ORDER BY id DESC LIMIT 1",
        )
        .fetch_optional(&pool)
        .await
        .ok()
        .flatten()
        .map(|r| r.0)
    });
    let s = started?;
    chrono::DateTime::parse_from_rfc3339(s.trim())
        .ok()
        .map(|dt| dt.timestamp())
        .or_else(|| SystemTime::now().duration_since(UNIX_EPOCH).ok().map(|d| d.as_secs() as i64))
}

/// Fase del vuelo → texto corto para Discord.
fn phase_text(p: &str) -> String {
    match p {
        "preflight" => crate::tr!("discord.phasePreflight"),
        "engine_running" | "engine_start" => crate::tr!("discord.phaseEngineRunning"),
        "pushback" => crate::tr!("discord.phasePushback"),
        "taxi_out" => crate::tr!("discord.phaseTaxiOut"),
        "takeoff" => crate::tr!("discord.phaseTakeoff"),
        "climbing" => crate::tr!("discord.phaseClimbing"),
        "cruise" => crate::tr!("discord.phaseCruise"),
        "descent" => crate::tr!("discord.phaseDescent"),
        "approach" => crate::tr!("discord.phaseApproach"),
        "landed_rollout" | "landing" => crate::tr!("discord.phaseLanding"),
        "taxi_in" => crate::tr!("discord.phaseTaxiIn"),
        "parking" => crate::tr!("discord.phaseParking"),
        "deboarding" => crate::tr!("discord.phaseDeboarding"),
        _ => crate::tr!("discord.phaseInFlight"),
    }
}
