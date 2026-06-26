//! Registro de vuelos reales — capa de persistencia y dominio.
//!
//! El watcher (`simconnect_watcher.rs`) llama a `start_flight` al
//! detectar despegue y a `finish_flight` al detectar aterrizaje.
//! Esta capa abstrae el SQL y el lookup de aeropuerto cercano para
//! que el watcher se concentre en la state-machine.
//!
//! La pareja origen/destino se rellena por **proximidad** contra la
//! tabla `airports` (haversine simple, ventana 3 nm). Si el punto
//! está fuera de OurAirports (helipuertos privados, runways
//! deshabilitados), guardamos sólo lat/lon — el ICAO queda NULL.

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

/// Registro completo tal como vive en `flight_log`. `ended_at`
/// `None` indica un vuelo en curso (o interrumpido si la app cerró
/// sin que llegara aterrizaje).
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct FlightLogEntry {
    pub id: i64,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub origin_lat: f64,
    pub origin_lon: f64,
    pub origin_icao: Option<String>,
    pub origin_name: Option<String>,
    pub destination_lat: Option<f64>,
    pub destination_lon: Option<f64>,
    pub destination_icao: Option<String>,
    pub destination_name: Option<String>,
    pub aircraft_title: Option<String>,
    pub aircraft_atc_type: Option<String>,
    /// (v3.5.0) `ATC MODEL` — referencia interna del addon (ej.
    /// `TT:ATCCOM.AC_MODEL_A20N.0.text`). Permite agrupar vuelos por
    /// tipo de avión aunque la livery cambie.
    pub aircraft_model: Option<String>,
    /// (v3.5.0) `ATC AIRLINE` — aerolínea visible (configurada en el
    /// MSFS Hangar / livery).
    pub aircraft_airline: Option<String>,
    /// (v3.5.0) `ATC ID` — matrícula real del avión ("EC-MXY"). En
    /// MSFS 2024 `ATC TAIL NUMBER` está deprecada — esta es la
    /// canónica.
    pub aircraft_registration: Option<String>,
    pub distance_nm: Option<f64>,
    pub flight_time_s: Option<i64>,
    pub max_altitude_ft: Option<i64>,
    /// Vertical speed (FPM) capturado en el momento del touchdown.
    /// Negativo = descenso (lo normal); positivo = subida (raro).
    /// Una "buen aterrizaje" suele estar entre -100 y -500 FPM;
    /// arriba de -600 ya es duro, arriba de -1000 abusivo.
    pub landing_fpm: Option<i64>,
    /// Velocidad máxima sobre tierra durante el vuelo (knots).
    pub max_ground_speed_kt: Option<i64>,
    /// Velocidad máxima true airspeed (knots) — útil para distinguir
    /// performance real del avión vs. lo que el viento añade/quita.
    pub max_true_airspeed_kt: Option<i64>,
    /// Parking spot / gate de salida — formato "GATE A12", "RAMP 3"
    /// o `Position: lat,lon` cuando no detectamos parking conocido.
    pub departure_gate: Option<String>,
    /// Parking spot / gate de llegada.
    pub arrival_gate: Option<String>,
    /// Pasajeros transportados — manual (la integración GSX queda
    /// para una release futura). Editable vía `update_flight_log_entry`.
    pub passengers: Option<i64>,
    /// Carga útil (kg) — manual, editable.
    pub cargo_kg: Option<i64>,
    /// Combustible consumido (kg) — automático cuando SimConnect
    /// está conectado: diferencia entre FUEL TOTAL QUANTITY WEIGHT al
    /// OUT (pushback) y al IN (engine shutdown). Manualmente
    /// editable.
    pub fuel_used_kg: Option<i64>,
    /// Segundos totales con el sim pausado durante el vuelo. Lo
    /// usamos para restar al raw `flight_time_s` y devolver block
    /// time REAL (sin contar pausas).
    pub paused_seconds: i64,
    pub source: String,
    /// (v3.6.0 Phase H) Número de vuelo VA-style ("DL115", "IB2741").
    /// Origen: parser de VAS-ACARS `data.db` (campo f10 de FlightInfo),
    /// o cross-match de SimBrief OFP al iniciar un vuelo SimConnect.
    pub flight_number: Option<String>,
    /// (v3.6.0) Callsign ATC ("DAL115"). Complementa flight_number —
    /// flight_number usa código IATA (2 letras), callsign usa ICAO (3 letras).
    pub callsign: Option<String>,
    /// (v3.6.0) Código ICAO de aerolínea (3 letras: "DAL", "IBE").
    /// Usado para agrupar por aerolínea en el dashboard. Más robusto
    /// que `aircraft_airline` (string libre con variantes).
    pub airline_icao: Option<String>,
    /// (v3.6.0) Estado del vuelo. `completed` = visible en UI. `partial`
    /// = importado pero falta data OOOI o duplicado descartado;
    /// se preserva en DB pero la UI lo oculta por defecto.
    pub status: String,
    /// (v3.6.0 Epic E) Puntaje total alcanzado.
    pub score_total: Option<i64>,
    /// (v3.6.0) Puntaje máximo posible (la suma de `points_max` de
    /// todas las reglas evaluadas para este vuelo).
    pub score_max: Option<i64>,
    /// (v3.6.0) Grade derivada — 'A' (95-100%), 'B' (85-95%),
    /// 'C' (70-85%), 'D' (50-70%), 'F' (<50%).
    pub score_grade: Option<String>,
    /// (v4.0.0 P7.6b iter 3) Flag de rebote — `true` si el avión
    /// rebotó en el aterrizaje (Stream B detectó 2+ flancos 0→1 de
    /// SIM_ON_GROUND con radio_alt < 50ft). La UI lo marca como
    /// "Bouncing Detected" al lado del FPM.
    pub bounced: bool,
    /// (v4.0.0 P7.4b) Temperatura máxima de frenos (°C) alcanzada
    /// durante el vuelo. Solo se puebla cuando el avión publica el
    /// LVar de brake temp (FBW A32NX, etc.) Y el usuario tiene el
    /// módulo WASM de MobiFlight instalado (puente LVar opcional).
    /// `None` en la inmensa mayoría de vuelos. `#[sqlx(default)]`
    /// para que cualquier SELECT que no incluya la columna siga
    /// mapeando sin error.
    #[sqlx(default)]
    pub max_brake_temp_c: Option<f64>,
    /// (v6.1 Layer 2) Desgaste de neumático máximo (%) leído del addon
    /// (iniBuilds A350 vía `INI_TIREx_WEAR`). `None` si el addon no lo publica
    /// o no hay módulo WASM de MobiFlight. Lo usa el desgaste de neumáticos del
    /// panel de mantenimiento como fuente REAL cuando existe.
    #[sqlx(default)]
    pub max_tire_wear_pct: Option<f64>,
    /// (v4.16.1 #5b) METAR REAL de salida/llegada (vida real) capturado del
    /// OFP de SimBrief al finalizar el vuelo, para consulta histórica en el
    /// FlightBook. `#[sqlx(default)]` → SELECTs que no incluyan la columna
    /// siguen mapeando. `None` para imports VAS y vuelos sin OFP linkeado.
    #[sqlx(default)]
    pub metar_origin: Option<String>,
    #[sqlx(default)]
    pub metar_dest: Option<String>,
}

/// Resultado del lookup de aeropuerto más cercano. Incluye
/// coordenadas para que `format_gate_fallback` pueda calcular
/// bearing y distancia desde el centro del aeropuerto.
#[derive(Debug, Clone)]
pub struct NearestAirportFull {
    pub icao: String,
    pub name: String,
    pub latitude: f64,
    pub longitude: f64,
}

const NEAREST_THRESHOLD_NM: f64 = 3.0;

/// Crea una fila nueva con `ended_at = NULL`. Devuelve el id
/// para que el watcher lo guarde en su state — al aterrizar se
/// hará UPDATE sobre esa fila.
///
/// **v3.0.0** — `departure_gate` se queda como `NULL` hasta que la
/// `RequestFacilityData` de SimConnect devuelve el nombre real del
/// parking (ej. "A34", "B12", "Ramp 4"). Antes guardábamos un
/// fallback `Stand · 341° 855m de LGKO` que el usuario reportó como
/// ruidoso y confuso. Si la API falla, el campo queda NULL y la UI
/// simplemente no muestra el chip de gate.
pub async fn start_flight(
    pool: &SqlitePool,
    lat: f64,
    lon: f64,
    aircraft_title: Option<&str>,
    aircraft_atc: Option<&str>,
) -> anyhow::Result<i64> {
    let nearest = nearest_airport_with_coords(pool, lat, lon).await?;
    // (v1.1.1) Defensa contra "vuelos fantasma" cuando MSFS reporta
    // posición (0, 0) durante el splash/menú antes de que el
    // escenario cargue. Si Y no hay airport cercano (esa zona del
    // Atlántico ecuatorial no tiene ninguno), rechazamos la creación.
    // El state machine del watcher YA filtra esto upstream, pero
    // esta guard cubre cualquier camino que llegue aquí (ej. el
    // comando debug_seed_flight_log o llamadas manuales).
    if nearest.is_none() && lat.abs() < 0.01 && lon.abs() < 0.01 {
        anyhow::bail!(
            "posición (0, 0) sin airport cercano — probablemente MSFS aún cargando el escenario, vuelo no creado"
        );
    }
    let started_at = chrono::Utc::now()
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string();

    // (v4.0.0 — P7.5b) start_flight ya NO hace cross-match con SimBrief.
    //
    // Motivación: en cuenta SimBrief compartida ("vuelan juntos"), el
    // matching requiere `aircraft_reg` y `fuel_loaded_lb` para discriminar
    // — datos que al OUT no están todavía (el dispatch de meta puede no
    // haber llegado, y `FUEL TOTAL QUANTITY WEIGHT` lo capturamos al
    // OUT pero el watcher no pasa por start_flight directamente).
    //
    // El matching definitivo lo hace `populate_simbrief_async` 2s después
    // del OUT con la info completa (aircraft_atc + aircraft_reg +
    // fuel_loaded). Eso permite scoring multi-factor real.
    //
    // start_flight crea la fila con VA meta NULL — la UI muestra "—" en
    // flight_number por ~2s hasta que el populate corre. Trade-off
    // aceptable contra matching incorrecto.
    let result = sqlx::query(
        r#"
        INSERT INTO flight_log (
            started_at, origin_lat, origin_lon, origin_icao, origin_name,
            aircraft_title, aircraft_atc_type, departure_gate, source
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, 'simconnect')
        "#,
    )
    .bind(&started_at)
    .bind(lat)
    .bind(lon)
    .bind(nearest.as_ref().map(|n| n.icao.as_str()))
    .bind(nearest.as_ref().map(|n| n.name.as_str()))
    .bind(aircraft_title)
    .bind(aircraft_atc)
    .execute(pool)
    .await?;

    Ok(result.last_insert_rowid())
}

/// (v3.6.0 Phase H) Deriva el código ICAO de aerolínea (3 letras)
/// del callsign. Convenciones que reconocemos:
///
/// · `"DAL115"` → `"DAL"` — formato estándar (3 letras + número).
/// · `"DLH4U"` → `"DLH"` — número alfanumérico no rompe el match.
/// · `"N123AB"` → `None` — registration personal, no es callsign de
///   aerolínea (no empieza con 3 letras del alfabeto seguidas de un
///   dígito o letra que NO sea letra inicial de registration).
///
/// Heurística: 3 primeros chars deben ser letras A-Z, el 4to debe
/// existir y ser dígito O letra (algunos códigos tienen sufijos). Si
/// el callsign empieza con un dígito o tiene < 4 chars, devolvemos
/// None — no parece un callsign comercial.
pub fn derive_airline_icao(callsign: Option<&str>) -> Option<String> {
    let cs = callsign?.trim();
    if cs.len() < 4 {
        return None;
    }
    let bytes = cs.as_bytes();
    let first3_letters = bytes
        .iter()
        .take(3)
        .all(|c| c.is_ascii_alphabetic());
    let fourth_is_alnum = bytes
        .get(3)
        .is_some_and(|c| c.is_ascii_alphanumeric());
    if !first3_letters || !fourth_is_alnum {
        return None;
    }
    // Registraciones tipo N123AB / D-ABCD también empiezan con letras
    // pero suelen tener un guion o pocos chars; el `find_recent_for_origin`
    // viene de SimBrief y filename `callsign` siempre es de aerolínea,
    // así que aceptamos.
    Some(cs[..3].to_ascii_uppercase())
}

/// (v3.0.0 — deprecado) Formateo del gate fallback estilo
/// `Stand · 320° 280m de EBBR`. Ya no se usa porque el usuario
/// reportó la string como confusa; ahora dejamos los gates NULL hasta
/// tener el nombre real de SimConnect Facility Data API. Se conserva
/// la función por compat con código legado pero ningún call site
/// activo la invoca.
#[allow(dead_code)]
fn format_gate_fallback(lat: f64, lon: f64, nearest: Option<&NearestAirportFull>) -> String {
    match nearest {
        Some(n) => {
            let dist_nm = haversine_nm(n.latitude, n.longitude, lat, lon);
            let bearing = bearing_deg(n.latitude, n.longitude, lat, lon);
            let dist_m = (dist_nm * 1852.0).round() as i64;
            format!(
                "Stand · {:03.0}° {}m de {}",
                bearing, dist_m, n.icao
            )
        }
        None => format!("Position: {:.4}, {:.4}", lat, lon),
    }
}

/// Métricas extra capturadas al cerrar el vuelo. Las pasamos en un
/// struct para evitar arguments-explosion en la signatura.
#[derive(Debug, Clone, Default)]
pub struct FlightFinishMetrics {
    pub max_altitude_ft: Option<i64>,
    /// FPM en el touchdown — negativo = descenso (lo normal).
    pub landing_fpm: Option<i64>,
    /// Ground speed máxima durante el vuelo.
    pub max_ground_speed_kt: Option<i64>,
    /// True airspeed máxima durante el vuelo.
    pub max_true_airspeed_kt: Option<i64>,
    /// Combustible consumido (kg) — diferencia entre fuel inicial
    /// (capturado al OUT) y fuel final (al IN). Si SimConnect no
    /// estaba conectado al OUT, queda `None`.
    pub fuel_used_kg: Option<i64>,
    /// Total de segundos con el sim pausado durante el vuelo.
    /// `finish_flight` resta esto del raw duration → block time real.
    pub paused_seconds: i64,
    /// (v3.1.0) Fallback de arrival_gate — usado si SimConnect
    /// Facility Data no devolvió un gate real durante el taxi-in.
    /// Típicamente el `current_gate` del SharedState al momento del
    /// IN (que viene del último request "preflight"/"arrival" o se
    /// resolvió por proximidad). Si el query del watcher ya tenía
    /// algo en la columna, COALESCE lo conserva.
    pub fallback_arrival_gate: Option<String>,
}

/// Cierra un vuelo abierto: rellena destino, distancia, duración,
/// altitud máxima y las nuevas métricas (landing FPM + max speeds).
pub async fn finish_flight(
    pool: &SqlitePool,
    id: i64,
    lat: f64,
    lon: f64,
    metrics: FlightFinishMetrics,
) -> anyhow::Result<()> {
    let nearest = nearest_airport_with_coords(pool, lat, lon).await?;

    // Cargamos origen para calcular distancia y duración.
    let origin: Option<(String, f64, f64)> = sqlx::query_as(
        "SELECT started_at, origin_lat, origin_lon FROM flight_log WHERE id = ?1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;

    let Some((started_at, origin_lat, origin_lon)) = origin else {
        anyhow::bail!("flight_log id={} no existe", id);
    };

    let distance_nm = haversine_nm(origin_lat, origin_lon, lat, lon);
    let now = chrono::Utc::now();
    let ended_at = now.format("%Y-%m-%dT%H:%M:%SZ").to_string();

    // Duración = ahora - started_at MENOS el tiempo pausado. Eso
    // refleja block-time REAL (sin las pausas que el usuario hizo
    // para preparar charts, comer, etc). Si el parse falla, NULL.
    let raw_seconds = chrono::DateTime::parse_from_rfc3339(&started_at)
        .ok()
        .map(|dt| now.signed_duration_since(dt.with_timezone(&chrono::Utc)).num_seconds());
    let flight_time_s = raw_seconds.map(|s| (s - metrics.paused_seconds).max(0));

    // (v3.1.0) destination_icao: si `nearest` falló (no airport en 3 nm)
    // pero teníamos un OFP de SimBrief que matchea el origen, intentamos
    // usar su destinationIcao como fallback. Mejor un destino "del plan"
    // que un destino NULL ruidoso en la UI.
    let dest_icao_resolved: Option<String> = match nearest.as_ref() {
        Some(n) => Some(n.icao.clone()),
        None => {
            // Lookup OFP por origin_icao del flight para fallback.
            let origin_icao_opt: Option<String> = sqlx::query_scalar(
                "SELECT origin_icao FROM flight_log WHERE id = ?1"
            )
            .bind(id)
            .fetch_optional(pool)
            .await
            .ok()
            .flatten();
            if let Some(orig) = origin_icao_opt {
                sqlx::query_scalar::<_, String>(
                    r#"
                    SELECT destination_icao FROM simbrief_flights
                    WHERE origin_icao = ?1
                    ORDER BY CAST(generated_at AS INTEGER) DESC
                    LIMIT 1
                    "#
                )
                .bind(&orig)
                .fetch_optional(pool)
                .await
                .ok()
                .flatten()
            } else {
                None
            }
        }
    };
    let dest_name_resolved: Option<String> = nearest.as_ref().map(|n| n.name.clone());

    // (v3.1.0) arrival_gate: COALESCE con el `fallback_arrival_gate`
    // que pasa el watcher (típicamente current_gate del SharedState
    // al momento del IN). Esto cubre el caso en que SimConnect
    // Facility Data falla en el aeropuerto destino (scenery custom
    // sin TAXI_PARKING definido).
    // (v2.2.0) fuel_used_kg: PRIORIZAR el valor existente (de
    // SimBrief vía pre-populate al OUT). Sólo usar el cálculo de
    // SimConnect si la fila no tiene fuel todavía.
    sqlx::query(
        r#"
        UPDATE flight_log
        SET ended_at             = ?1,
            destination_lat      = ?2,
            destination_lon      = ?3,
            destination_icao     = COALESCE(destination_icao, ?4),
            destination_name     = COALESCE(destination_name, ?5),
            distance_nm          = ?6,
            flight_time_s        = ?7,
            max_altitude_ft      = COALESCE(?8, max_altitude_ft),
            landing_fpm          = ?9,
            max_ground_speed_kt  = ?10,
            max_true_airspeed_kt = ?11,
            arrival_gate         = COALESCE(arrival_gate, ?12),
            fuel_used_kg         = COALESCE(fuel_used_kg, ?13),
            paused_seconds       = ?14
        WHERE id = ?15
        "#,
    )
    .bind(&ended_at)
    .bind(lat)
    .bind(lon)
    .bind(dest_icao_resolved.as_deref())
    .bind(dest_name_resolved.as_deref())
    .bind(distance_nm)
    .bind(flight_time_s)
    .bind(metrics.max_altitude_ft)
    .bind(metrics.landing_fpm)
    .bind(metrics.max_ground_speed_kt)
    .bind(metrics.max_true_airspeed_kt)
    .bind(metrics.fallback_arrival_gate.as_deref())
    .bind(metrics.fuel_used_kg)
    .bind(metrics.paused_seconds)
    .bind(id)
    .execute(pool)
    .await?;

    Ok(())
}

/// Persiste el `fuel_initial_kg` recién captado al OUT (transición
/// OnGround → BlockOut). El watcher recuerda este valor en su state
/// local; lo persistimos también en DB en una columna virtual via
/// el campo `fuel_used_kg` con signo negativo TEMPORAL — no, mejor
/// usamos una struct sólo en memoria del watcher para no contaminar
/// el schema. Esta función queda como helper para el futuro si
/// queremos persistir el inicial separado del usado.
///
/// Por ahora el watcher hace el delta en memoria y al cerrar el
/// vuelo pasa `fuel_used_kg` ya calculado dentro de
/// `FlightFinishMetrics`.

/// Actualiza campos editables de un vuelo ya cerrado — usado por
/// el botón "Editar" del panel de detalle. Todos los argumentos son
/// `Option<>`: `None` significa "no tocar este campo". Permite al
/// usuario corregir lo que el watcher detectó mal (gates, pasajeros,
/// carga, block time si el sim no detectó bien una pausa).
#[derive(Debug, Clone, Default)]
pub struct UpdateEntryInput {
    pub flight_time_s: Option<i64>,
    pub passengers: Option<i64>,
    pub cargo_kg: Option<i64>,
    pub fuel_used_kg: Option<i64>,
    pub departure_gate: Option<String>,
    pub arrival_gate: Option<String>,
    /// Matrícula del avión (ej. "N404DX", "D-AIZA"). El usuario la
    /// edita cuando el watcher no la capturó (vuelos imported desde
    /// VAS-ACARS donde el livery no contenía un tail real, o vuelos
    /// viejos pre-v3.5.0 sin captura SimConnect de `ATC ID`).
    pub aircraft_registration: Option<String>,
    /// (v4.0.0 — P6.2) Origen y destino editables. Útil cuando el
    /// watcher detectó un ICAO erróneo (spawn en aeropuerto cercano
    /// al real, vuelos VAS importados con ICAOs mal parseados, etc.).
    /// El frontend manda strings de 3-4 letras; backend NO valida
    /// contra la tabla airports — confiamos en el usuario, una
    /// validación interactiva con sugerencias sería un esfuerzo
    /// futuro (autocomplete). Se uppercase-ean antes de persistir.
    pub origin_icao: Option<String>,
    pub destination_icao: Option<String>,
}

pub async fn update_entry(
    pool: &SqlitePool,
    id: i64,
    input: &UpdateEntryInput,
) -> anyhow::Result<()> {
    // Construimos el UPDATE dinámicamente para no sobreescribir con
    // NULL los campos que el caller no quiere tocar. SQLite no tiene
    // un "DEFAULT existing_value" syntax conciso para todos los
    // campos, así que armamos el SET a mano.
    let mut sets: Vec<&str> = Vec::new();
    if input.flight_time_s.is_some() {
        sets.push("flight_time_s = ?");
    }
    if input.passengers.is_some() {
        sets.push("passengers = ?");
    }
    if input.cargo_kg.is_some() {
        sets.push("cargo_kg = ?");
    }
    if input.fuel_used_kg.is_some() {
        sets.push("fuel_used_kg = ?");
    }
    if input.departure_gate.is_some() {
        sets.push("departure_gate = ?");
    }
    if input.arrival_gate.is_some() {
        sets.push("arrival_gate = ?");
    }
    if input.aircraft_registration.is_some() {
        sets.push("aircraft_registration = ?");
    }
    if input.origin_icao.is_some() {
        sets.push("origin_icao = ?");
    }
    if input.destination_icao.is_some() {
        sets.push("destination_icao = ?");
    }
    if sets.is_empty() {
        return Ok(());
    }
    let sql = format!(
        "UPDATE flight_log SET {} WHERE id = ?",
        sets.join(", ")
    );
    let mut q = sqlx::query(&sql);
    if let Some(v) = input.flight_time_s {
        q = q.bind(v);
    }
    if let Some(v) = input.passengers {
        q = q.bind(v);
    }
    if let Some(v) = input.cargo_kg {
        q = q.bind(v);
    }
    if let Some(v) = input.fuel_used_kg {
        q = q.bind(v);
    }
    if let Some(v) = input.departure_gate.as_ref() {
        q = q.bind(v);
    }
    if let Some(v) = input.arrival_gate.as_ref() {
        q = q.bind(v);
    }
    if let Some(v) = input.aircraft_registration.as_ref() {
        q = q.bind(v);
    }
    if let Some(v) = input.origin_icao.as_ref() {
        // Uppercase para consistencia con la convención del schema
        // (ICAOs son siempre uppercase en aviación civil).
        q = q.bind(v.to_uppercase());
    }
    if let Some(v) = input.destination_icao.as_ref() {
        q = q.bind(v.to_uppercase());
    }
    q = q.bind(id);
    q.execute(pool).await?;
    Ok(())
}

/// Borra todos los vuelos de una fuente específica (ej. `"vas-acars"`,
/// `"msfs-logbook"`). Usado por el botón "Borrar todos" del UI tras
/// import erróneos durante prueba y error. NO toca los SimConnect
/// captures (source='simconnect') a menos que se le pase explícitamente.
///
/// Devuelve `(deleted_flights, deleted_tracks)` — los tracks se
/// borran por CASCADE en la migración 015 (FK con ON DELETE CASCADE),
/// pero contamos antes del DELETE para feedback al usuario.
pub async fn delete_by_source(
    pool: &SqlitePool,
    source: &str,
) -> anyhow::Result<(u64, u64)> {
    let track_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM flight_log_track
         WHERE flight_id IN (SELECT id FROM flight_log WHERE source = ?1)",
    )
    .bind(source)
    .fetch_one(pool)
    .await
    .unwrap_or(0);

    let result = sqlx::query("DELETE FROM flight_log WHERE source = ?1")
        .bind(source)
        .execute(pool)
        .await?;
    Ok((result.rows_affected(), track_count as u64))
}

/// Persiste el landing_fpm en el momento exacto del touchdown
/// (v0.1.22). Antes el FPM se guardaba sólo al `finish_flight()`
/// (es decir, al cerrar la fila); ahora el cierre puede demorarse
/// varios minutos esperando el engine shutdown OOOI, y queremos no
/// perder el valor si la app crashea durante el taxi-in.
///
/// Se llama UNA vez por vuelo, en la transición Airborne→Landed.
/// Llamarla múltiples veces (touch-and-go) sobreescribiría con el
/// FPM más reciente — el watcher evita ese caso conservando sólo
/// el primer FPM en memoria.
pub async fn touch_landing(
    pool: &SqlitePool,
    id: i64,
    landing_fpm: i64,
) -> anyhow::Result<()> {
    sqlx::query("UPDATE flight_log SET landing_fpm = ?1 WHERE id = ?2")
        .bind(landing_fpm)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// (v3.5.0) Persiste los simvars STRING256 de SimConnect en cuanto
/// llega la primera respuesta del `DEFINE_ID_AIRCRAFT_META`. Se
/// llama UNA vez por vuelo, al recibir el primer
/// `RECV_ID_SIMOBJECT_DATA` del meta request. Usamos COALESCE para
/// no pisar valores que ya hayan sido editados manualmente por el
/// usuario vía `update_entry`.
///
/// Campos vacíos (`""`) se mapean a NULL para que la UI muestre "—"
/// en vez de un string vacío visible. Algunos addons (CRJ Aerosoft,
/// Asobo defaults) dejan `ATC ID` vacío si el usuario no configuró
/// matrícula — preferimos NULL antes que un string vacío.
pub async fn update_aircraft_meta(
    pool: &SqlitePool,
    id: i64,
    title: Option<&str>,
    atc_type: Option<&str>,
    model: Option<&str>,
    airline: Option<&str>,
    registration: Option<&str>,
) -> anyhow::Result<()> {
    fn nz(s: Option<&str>) -> Option<&str> {
        s.filter(|v| !v.trim().is_empty())
    }
    sqlx::query(
        r#"
        UPDATE flight_log
        SET aircraft_title         = COALESCE(aircraft_title, ?1),
            aircraft_atc_type      = COALESCE(aircraft_atc_type, ?2),
            aircraft_model         = COALESCE(aircraft_model, ?3),
            aircraft_airline       = COALESCE(aircraft_airline, ?4),
            aircraft_registration  = COALESCE(aircraft_registration, ?5)
        WHERE id = ?6
        "#,
    )
    .bind(nz(title))
    .bind(nz(atc_type))
    .bind(nz(model))
    .bind(nz(airline))
    .bind(nz(registration))
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Actualiza el max_altitude_ft sin tocar el resto del vuelo. Lo
/// llama el watcher en cada tick mientras la aeronave esté en
/// vuelo, para tener un valor razonable aunque la app cierre antes
/// del aterrizaje.
pub async fn touch_max_altitude(
    pool: &SqlitePool,
    id: i64,
    altitude_ft: i64,
) -> anyhow::Result<()> {
    sqlx::query(
        r#"
        UPDATE flight_log
        SET max_altitude_ft = MAX(COALESCE(max_altitude_ft, 0), ?1)
        WHERE id = ?2
        "#,
    )
    .bind(altitude_ft)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

/// (v4.0.0 P7.4b) Actualiza el `max_brake_temp_c` con el máximo
/// histórico, sin tocar el resto del vuelo. Lo llama el watcher cada
/// vez que el puente LVar (MobiFlight) reporta una lectura de
/// temperatura de frenos válida para el vuelo abierto.
pub async fn touch_max_brake_temp(
    pool: &SqlitePool,
    id: i64,
    brake_temp_c: f64,
) -> anyhow::Result<()> {
    sqlx::query(
        r#"
        UPDATE flight_log
        SET max_brake_temp_c = MAX(COALESCE(max_brake_temp_c, 0), ?1)
        WHERE id = ?2
        "#,
    )
    .bind(brake_temp_c)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

/// (v6.1 Layer 2) Actualiza `max_tire_wear_pct` con el máximo histórico del
/// vuelo. Lo llama el watcher cuando el puente LVar reporta el desgaste de
/// neumático real del addon (iniBuilds, etc.). Espejo de `touch_max_brake_temp`.
pub async fn touch_max_tire_wear(
    pool: &SqlitePool,
    id: i64,
    tire_wear_pct: f64,
) -> anyhow::Result<()> {
    sqlx::query(
        r#"
        UPDATE flight_log
        SET max_tire_wear_pct = MAX(COALESCE(max_tire_wear_pct, 0), ?1)
        WHERE id = ?2
        "#,
    )
    .bind(tire_wear_pct)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

/// **ACARS-like persistent tracking**: cada tick el watcher escribe
/// la posición + altitud + groundspeed actuales en la fila del
/// vuelo abierto. Sirve dos propósitos:
///
///   1. Si la app se cierra a mitad de vuelo y se reabre, podemos
///      restaurar el state del watcher sin haber perdido nada.
///   2. Si el avión se estrella / el sim crashea, el último punto
///      queda guardado y la UI muestra "interrumpido en lat,lon".
///
/// Es muy barato: un UPDATE por id, ejecutado max 1 vez por
/// segundo (la frecuencia del SimConnect poll).
pub async fn touch_live_position(
    pool: &SqlitePool,
    id: i64,
    lat: f64,
    lon: f64,
    alt_ft: i64,
    gs_kt: i64,
) -> anyhow::Result<()> {
    let now = chrono::Utc::now()
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string();
    sqlx::query(
        r#"
        UPDATE flight_log
        SET last_position_lat    = ?1,
            last_position_lon    = ?2,
            last_position_alt_ft = ?3,
            last_position_gs_kt  = ?4,
            last_position_at     = ?5
        WHERE id = ?6
        "#,
    )
    .bind(lat)
    .bind(lon)
    .bind(alt_ft)
    .bind(gs_kt)
    .bind(&now)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Vuelo abierto (sin `ended_at`) — usado para restaurar el state
/// del watcher al reabrir la app tras un cierre forzado/intencional.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct OpenFlight {
    pub id: i64,
    pub started_at: String,
    pub origin_lat: f64,
    pub origin_lon: f64,
    pub origin_icao: Option<String>,
    pub max_altitude_ft: Option<i64>,
    /// (v2.0.3) Última posición conocida — usada por el watcher para
    /// decidir si "continuar" el vuelo (mismo lugar, mismo session) o
    /// "abandonar" (player muy lejos = nueva sesión).
    pub last_position_lat: Option<f64>,
    pub last_position_lon: Option<f64>,
    /// Timestamp ISO de la última posición. Si > 24h se considera
    /// fantasma; el `close_stale_open_flights` lo va a cerrar al
    /// próximo startup.
    pub last_position_at: Option<String>,
}

/// Devuelve el vuelo abierto más reciente, si existe. El watcher
/// lo invoca al arrancar — si hay uno con `last_position_at` < 1h,
/// asume que el usuario sigue volando y restaura el state.
pub async fn latest_open_flight(pool: &SqlitePool) -> anyhow::Result<Option<OpenFlight>> {
    let row = sqlx::query_as::<_, OpenFlight>(
        r#"
        SELECT id, started_at, origin_lat, origin_lon, origin_icao,
               max_altitude_ft, last_position_lat, last_position_lon,
               last_position_at
        FROM flight_log
        WHERE ended_at IS NULL
        ORDER BY started_at DESC
        LIMIT 1
        "#,
    )
    .fetch_optional(pool)
    .await?;
    Ok(row)
}


/// Cierra todos los vuelos abiertos que llevan más de N segundos
/// sin update de `last_position_at`. Eso evita acumular "vuelos
/// fantasma" cuando la app crasheó / el sim crasheó / el usuario
/// olvidó cerrar el flight plan. Los marcamos con `ended_at =
/// last_position_at` y los datos de aterrizaje quedan vacíos para
/// indicar que fue un cierre artificial.
pub async fn close_stale_open_flights(
    pool: &SqlitePool,
    max_idle_seconds: i64,
) -> anyhow::Result<u64> {
    // (v4.23.0) Los vuelos abandonados se cierran como 'partial' (NO
    // 'completed') con partial_ack=0 — al abrir la app, el modal de
    // vuelos incompletos pregunta si mantener o eliminar.
    let r = sqlx::query(
        r#"
        UPDATE flight_log
        SET ended_at = COALESCE(last_position_at, started_at),
            status = 'partial',
            partial_ack = 0
        WHERE ended_at IS NULL
          AND (
            last_position_at IS NULL
            OR (strftime('%s', 'now') - strftime('%s', last_position_at)) > ?1
          )
        "#,
    )
    .bind(max_idle_seconds)
    .execute(pool)
    .await?;
    Ok(r.rows_affected())
}

/// (v6.2.1) Cierra como **completed** los vuelos que quedaron ABIERTOS pero que
/// YA ATERRIZARON (tienen `landing_fpm`). El cierre normal se dispara al APAGAR
/// MOTORES; si el piloto sale del vuelo tras tocar pista sin apagar motores en
/// puerta, el vuelo quedaba abierto e INVISIBLE en el FlightBook (aunque el FPM
/// sí se capturó). Aquí lo finalizamos con la última posición como destino, para
/// que aparezca con su aterrizaje. Corre al arrancar el watcher, ANTES de
/// restaurar el state (así no se "reanuda" un vuelo que en realidad ya terminó).
/// Sólo toca vuelos con `landing_fpm` — un vuelo realmente en el aire (sin
/// touchdown) no se cierra aquí.
pub async fn finalize_landed_open_flights(pool: &SqlitePool) -> anyhow::Result<u64> {
    let r = sqlx::query(
        r#"
        UPDATE flight_log
        SET ended_at = COALESCE(last_position_at, datetime('now')),
            status = 'completed',
            destination_lat = COALESCE(destination_lat, last_position_lat),
            destination_lon = COALESCE(destination_lon, last_position_lon),
            flight_time_s = COALESCE(
                flight_time_s,
                CAST(
                    strftime('%s', COALESCE(last_position_at, datetime('now')))
                    - strftime('%s', started_at) AS INTEGER
                )
            )
        WHERE ended_at IS NULL
          AND landing_fpm IS NOT NULL
        "#,
    )
    .execute(pool)
    .await?;
    Ok(r.rows_affected())
}

/// (v4.23.0) Cierra un vuelo restaurado que resultó ser HUÉRFANO (la PC
/// se apagó / el sim murió a mitad de vuelo y al reconectar el avión
/// apareció lejos del último punto). Se marca 'partial' + partial_ack=0
/// para que el modal de vuelos incompletos pregunte mantener/eliminar.
/// NO toca finish_flight ni la captura de FPM — es un cierre artificial.
pub async fn close_flight_as_partial(pool: &SqlitePool, id: i64) -> anyhow::Result<()> {
    sqlx::query(
        r#"
        UPDATE flight_log
        SET ended_at        = COALESCE(last_position_at, datetime('now')),
            status          = 'partial',
            partial_ack     = 0,
            destination_lat = COALESCE(destination_lat, last_position_lat),
            destination_lon = COALESCE(destination_lon, last_position_lon)
        WHERE id = ?1
          AND ended_at IS NULL
        "#,
    )
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

/// (v4.23.0) Vuelos 'partial' aún no revisados por el usuario — el
/// frontend muestra el modal de "vuelo incompleto" para cada uno.
/// Solo vuelos del sim (los partial de imports VAS no aplican).
pub async fn list_unacked_partial(pool: &SqlitePool) -> anyhow::Result<Vec<FlightLogEntry>> {
    let rows = sqlx::query_as::<_, FlightLogEntry>(
        r#"
        SELECT id, started_at, ended_at,
               origin_lat, origin_lon, origin_icao, origin_name,
               destination_lat, destination_lon, destination_icao, destination_name,
               aircraft_title, aircraft_atc_type,
               aircraft_model, aircraft_airline, aircraft_registration,
               distance_nm, flight_time_s,
               max_altitude_ft,
               landing_fpm, max_ground_speed_kt, max_true_airspeed_kt,
               departure_gate, arrival_gate,
               passengers, cargo_kg, fuel_used_kg, paused_seconds,
               source,
               flight_number, callsign, airline_icao, status,
               score_total, score_max, score_grade,
               COALESCE(bounced, 0) AS bounced,
               max_brake_temp_c, max_tire_wear_pct,
               metar_origin, metar_dest
        FROM flight_log
        WHERE status = 'partial'
          AND partial_ack = 0
          AND source = 'simconnect'
        ORDER BY started_at DESC
        "#,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// (v4.23.0) "Mantener" un vuelo incompleto: pasa a 'completed' (aparece
/// en el FlightBook con su track parcial) y se marca como revisado.
pub async fn keep_partial_flight(pool: &SqlitePool, id: i64) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE flight_log SET status = 'completed', partial_ack = 1 WHERE id = ?1",
    )
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

/// (v2.0.3) Cierre forzado de un vuelo concreto por id. Usado por la
/// UI cuando el usuario reconoce que el vuelo está abandonado y
/// quiere marcarlo como cerrado sin esperar al engine shutdown.
///
/// Si el vuelo tiene `last_position_*` poblados, los usamos como
/// destino + tiempo de cierre + métricas mínimas. Si no, sólo
/// marcamos `ended_at = now`. NO inferimos altitudes ni FPM al
/// vuelo — quedan en NULL para indicar "cierre artificial".
pub async fn force_close_flight(pool: &SqlitePool, id: i64) -> anyhow::Result<()> {
    sqlx::query(
        r#"
        UPDATE flight_log
        SET
            ended_at         = COALESCE(last_position_at, datetime('now')),
            destination_lat  = COALESCE(destination_lat, last_position_lat),
            destination_lon  = COALESCE(destination_lon, last_position_lon)
        WHERE id = ?1
          AND ended_at IS NULL
        "#,
    )
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Punto individual de la polyline de un vuelo (v0.1.23). Una fila
/// por muestreo (~cada 10s) en la tabla `flight_log_track`. El
/// frontend los pide para un `flight_id` específico y los pinta como
/// LineString en el mapa de FlightBook.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct FlightTrackPoint {
    pub lat: f64,
    pub lon: f64,
    pub alt_ft: Option<i64>,
    pub gs_kt: Option<i64>,
    pub ts: String,
}

/// (v3.7.0 — Phase O) Payload extendido para `insert_track_point_full`.
/// Cubre todos los simvars VAS-aligned que el rubric necesita.
///
/// Los campos opcionales son los que ACARS no exporta y por tanto
/// quedan NULL para imports VAS — el rubric detecta la columna toda
/// NULL para una phase y devuelve "skipped" sin penalizar.
#[derive(Debug, Clone, Default)]
pub struct TrackPointInsert {
    pub lat: f64,
    pub lon: f64,
    pub alt_ft: Option<i64>,
    pub gs_kt: Option<i64>,
    pub ias_kt: Option<i64>,
    pub vs_fpm: Option<i64>,
    pub pitch_deg: Option<f64>,
    pub bank_deg: Option<f64>,
    pub g_force: Option<f64>,
    pub flaps_pct: Option<i64>,
    pub gear_down: Option<bool>,
    pub spoilers_pct: Option<i64>,
    pub light_nav: Option<bool>,
    pub light_beacon: Option<bool>,
    pub light_taxi: Option<bool>,
    pub light_landing: Option<bool>,
    pub light_strobe: Option<bool>,
    pub parking_brake: Option<bool>,
    pub transponder_code: Option<i64>,
    // (v4.0.0 — P4) Métricas de motor por slot 1..4.
    pub eng_n1: [Option<f64>; 4],
    pub eng_n2: [Option<f64>; 4],
    pub eng_egt: [Option<f64>; 4],
    pub eng_ff_pph: [Option<f64>; 4],
    pub eng_oil_temp: [Option<f64>; 4],
    pub eng_oil_press: [Option<f64>; 4],
    /// (v4.0.0 P7.8c) Fase de scoring asignada a ESTE sample (nombres
    /// del rubric: pre_departure, initial_climb, final_approach, etc.).
    /// El evaluador filtra por esta columna en vez de hacer un JOIN
    /// temporal frágil contra flight_log_phase. None para rutas legacy.
    pub scoring_phase: Option<String>,
    /// (v4.0.0 P7.9) Weather AMBIENT capturado en este sample.
    pub wind_dir_deg: Option<i64>,
    pub wind_speed_kt: Option<i64>,
    pub oat_c: Option<f64>,
    pub baro_hpa: Option<i64>,
    pub visibility_m: Option<i64>,
    pub precip_state: Option<i64>,
    /// (v3.19.0 P7.9c) Cobertura de nubes REAL (Open-Meteo) %, 0..100.
    /// Total + capas baja/media/alta. None si no se obtuvo dato.
    pub cloud_cover_pct: Option<i64>,
    pub cloud_low_pct: Option<i64>,
    pub cloud_mid_pct: Option<i64>,
    pub cloud_high_pct: Option<i64>,
    /// (v3.21.0) STALL WARNING del sim (bool) — regla "no stall".
    pub stall_warning: Option<bool>,
    /// (v3.26.0 P7.10) Warnings de daño/forzado del avión (bool):
    /// sobrevelocidad (Vmo/Mmo) y presión de aceite fuera de rango.
    pub overspeed_warning: Option<bool>,
    pub oil_press_warning: Option<bool>,
}

/// Inserta un punto en la traza del vuelo. Compat wrapper —
/// equivalente a `insert_track_point_full` con todos los campos
/// extendidos en `None`. Lo siguen llamando rutas legacy (seed flight,
/// fallback paths del watcher) que no tienen los simvars nuevos.
pub async fn insert_track_point(
    pool: &SqlitePool,
    flight_id: i64,
    lat: f64,
    lon: f64,
    alt_ft: i64,
    gs_kt: i64,
) -> anyhow::Result<()> {
    insert_track_point_full(
        pool,
        flight_id,
        TrackPointInsert {
            lat,
            lon,
            alt_ft: Some(alt_ft),
            gs_kt: Some(gs_kt),
            ..Default::default()
        },
    )
    .await
}

/// Insert con TODOS los campos extendidos. Usado por el watcher
/// SimConnect (que tiene todos los simvars VAS-aligned) y por
/// futuros importers que expongan más datos. No es idempotente —
/// cada muestreo es una fila nueva, el PK es AUTOINCREMENT.
pub async fn insert_track_point_full(
    pool: &SqlitePool,
    flight_id: i64,
    pt: TrackPointInsert,
) -> anyhow::Result<()> {
    let now = chrono::Utc::now()
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string();
    insert_track_point_full_at(pool, flight_id, &now, pt).await
}

/// (v3.30.0 #2) Igual que `insert_track_point_full` pero con `ts`
/// explícito. Lo usa el flush del buffer pre-departure del watcher: las
/// muestras cold-and-dark se capturan ANTES de que exista la fila del
/// vuelo y se vuelcan al crearse (OUT), conservando su timestamp
/// ORIGINAL — no el del flush — para preservar el orden cronológico y
/// permitir que la regla "ample pre-departure time" mida la duración real.
pub async fn insert_track_point_full_at(
    pool: &SqlitePool,
    flight_id: i64,
    ts: &str,
    pt: TrackPointInsert,
) -> anyhow::Result<()> {
    let b = |o: Option<bool>| o.map(|v| if v { 1_i64 } else { 0_i64 });
    sqlx::query(
        r#"
        INSERT OR IGNORE INTO flight_log_track (
            flight_id, lat, lon, alt_ft, gs_kt, ts,
            ias_kt, vs_fpm, pitch_deg, bank_deg, g_force,
            flaps_pct, gear_down, spoilers_pct,
            light_nav, light_beacon, light_taxi, light_landing, light_strobe,
            parking_brake, transponder_code,
            eng_n1_1, eng_n1_2, eng_n1_3, eng_n1_4,
            eng_n2_1, eng_n2_2, eng_n2_3, eng_n2_4,
            eng_egt_1, eng_egt_2, eng_egt_3, eng_egt_4,
            eng_ff_pph_1, eng_ff_pph_2, eng_ff_pph_3, eng_ff_pph_4,
            eng_oil_temp_1, eng_oil_temp_2, eng_oil_temp_3, eng_oil_temp_4,
            eng_oil_press_1, eng_oil_press_2, eng_oil_press_3, eng_oil_press_4,
            scoring_phase,
            wind_dir_deg, wind_speed_kt, oat_c, baro_hpa, visibility_m, precip_state,
            cloud_cover_pct, cloud_low_pct, cloud_mid_pct, cloud_high_pct,
            stall_warning,
            overspeed_warning, oil_press_warning
        )
        VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6,
            ?7, ?8, ?9, ?10, ?11,
            ?12, ?13, ?14,
            ?15, ?16, ?17, ?18, ?19,
            ?20, ?21,
            ?22, ?23, ?24, ?25,
            ?26, ?27, ?28, ?29,
            ?30, ?31, ?32, ?33,
            ?34, ?35, ?36, ?37,
            ?38, ?39, ?40, ?41,
            ?42, ?43, ?44, ?45,
            ?46,
            ?47, ?48, ?49, ?50, ?51, ?52,
            ?53, ?54, ?55, ?56,
            ?57,
            ?58, ?59
        )
        "#,
    )
    .bind(flight_id)
    .bind(pt.lat)
    .bind(pt.lon)
    .bind(pt.alt_ft)
    .bind(pt.gs_kt)
    .bind(ts)
    .bind(pt.ias_kt)
    .bind(pt.vs_fpm)
    .bind(pt.pitch_deg)
    .bind(pt.bank_deg)
    .bind(pt.g_force)
    .bind(pt.flaps_pct)
    .bind(b(pt.gear_down))
    .bind(pt.spoilers_pct)
    .bind(b(pt.light_nav))
    .bind(b(pt.light_beacon))
    .bind(b(pt.light_taxi))
    .bind(b(pt.light_landing))
    .bind(b(pt.light_strobe))
    .bind(b(pt.parking_brake))
    .bind(pt.transponder_code)
    // (v4.0.0 — P4) Métricas de motor — 24 binds.
    .bind(pt.eng_n1[0])
    .bind(pt.eng_n1[1])
    .bind(pt.eng_n1[2])
    .bind(pt.eng_n1[3])
    .bind(pt.eng_n2[0])
    .bind(pt.eng_n2[1])
    .bind(pt.eng_n2[2])
    .bind(pt.eng_n2[3])
    .bind(pt.eng_egt[0])
    .bind(pt.eng_egt[1])
    .bind(pt.eng_egt[2])
    .bind(pt.eng_egt[3])
    .bind(pt.eng_ff_pph[0])
    .bind(pt.eng_ff_pph[1])
    .bind(pt.eng_ff_pph[2])
    .bind(pt.eng_ff_pph[3])
    .bind(pt.eng_oil_temp[0])
    .bind(pt.eng_oil_temp[1])
    .bind(pt.eng_oil_temp[2])
    .bind(pt.eng_oil_temp[3])
    .bind(pt.eng_oil_press[0])
    .bind(pt.eng_oil_press[1])
    .bind(pt.eng_oil_press[2])
    .bind(pt.eng_oil_press[3])
    .bind(pt.scoring_phase.as_deref())
    // (v4.0.0 P7.9) Weather AMBIENT — 6 binds.
    .bind(pt.wind_dir_deg)
    .bind(pt.wind_speed_kt)
    .bind(pt.oat_c)
    .bind(pt.baro_hpa)
    .bind(pt.visibility_m)
    .bind(pt.precip_state)
    // (v3.19.0 P7.9c) Nubes reales — 4 binds.
    .bind(pt.cloud_cover_pct)
    .bind(pt.cloud_low_pct)
    .bind(pt.cloud_mid_pct)
    .bind(pt.cloud_high_pct)
    // (v3.21.0) Stall warning.
    .bind(b(pt.stall_warning))
    // (v3.26.0 P7.10) Warnings de daño/forzado.
    .bind(b(pt.overspeed_warning))
    .bind(b(pt.oil_press_warning))
    .execute(pool)
    .await?;
    Ok(())
}

/// Lee toda la traza de un vuelo, ordenada cronológicamente. El
/// frontend la convierte en GeoJSON LineString para pintar.
///
/// (v4.23.0) ORDER BY **ts** (no id): el volcado async del buffer
/// pre-departure corre concurrente con el primer insert realtime del
/// pushback, así que los ids pueden quedar intercalados (verificado en
/// el vuelo SBBV id=149: un punto de pushback con id MENOR que el resto
/// del buffer del gate). Ordenando por id, la polyline saltaba
/// gate→pushback→gate→pushback = la "doble línea" del pushback que el
/// usuario reportó tres veces. Por ts el track es cronológico siempre.
pub async fn list_track_for_flight(
    pool: &SqlitePool,
    flight_id: i64,
) -> anyhow::Result<Vec<FlightTrackPoint>> {
    let rows = sqlx::query_as::<_, FlightTrackPoint>(
        r#"
        SELECT lat, lon, alt_ft, gs_kt, ts
        FROM flight_log_track
        WHERE flight_id = ?1
        ORDER BY ts ASC, id ASC
        "#,
    )
    .bind(flight_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// (v4.0.0 P7.9b) Sample de weather para el Weather modal. Sólo las
/// posiciones que tienen AL MENOS un campo de clima poblado — el
/// frontend dibuja barbas de viento + colorea por temp/precip.
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WeatherSample {
    pub ts: String,
    pub lat: f64,
    pub lon: f64,
    pub alt_ft: Option<i64>,
    pub wind_dir_deg: Option<i64>,
    pub wind_speed_kt: Option<i64>,
    pub oat_c: Option<f64>,
    pub baro_hpa: Option<i64>,
    pub visibility_m: Option<i64>,
    pub precip_state: Option<i64>,
    /// (v3.19.0 P7.9c) Cobertura de nubes REAL (Open-Meteo) %, 0..100.
    pub cloud_cover_pct: Option<i64>,
    pub cloud_low_pct: Option<i64>,
    pub cloud_mid_pct: Option<i64>,
    pub cloud_high_pct: Option<i64>,
}

/// (v4.0.0 P7.9b) Lee los samples con weather de un vuelo. Devuelve
/// vacío si el vuelo no capturó weather (vuelos pre-v3.16.0 / VAS
/// imports). Incluye filas con SÓLO nubes (cloud_cover_pct) aunque no
/// haya AMBIENT del sim — clave para que la capa de nubes reales se vea.
pub async fn list_weather_for_flight(
    pool: &SqlitePool,
    flight_id: i64,
) -> anyhow::Result<Vec<WeatherSample>> {
    let rows = sqlx::query_as::<_, WeatherSample>(
        r#"
        SELECT ts, lat, lon, alt_ft,
               wind_dir_deg, wind_speed_kt, oat_c, baro_hpa,
               visibility_m, precip_state,
               cloud_cover_pct, cloud_low_pct, cloud_mid_pct, cloud_high_pct
        FROM flight_log_track
        WHERE flight_id = ?1
          AND (wind_speed_kt IS NOT NULL OR oat_c IS NOT NULL
               OR cloud_cover_pct IS NOT NULL)
        ORDER BY ts ASC, id ASC
        "#,
    )
    .bind(flight_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// (v4.0.0 — P5) Sample completo del track para el Performance modal.
/// Lee todos los campos que el modal puede graficar: posición,
/// altitud, velocidades, attitude, lights, gear/flaps/spoilers y
/// motores 1..4.
///
/// Para vuelos VAS imports muchos campos vienen NULL — el frontend
/// detecta una serie sin datos válidos y oculta esa curva del chart.
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FlightTrackFullPoint {
    pub ts: String,
    pub lat: f64,
    pub lon: f64,
    pub alt_ft: Option<i64>,
    pub gs_kt: Option<i64>,
    pub ias_kt: Option<i64>,
    pub vs_fpm: Option<i64>,
    pub pitch_deg: Option<f64>,
    pub bank_deg: Option<f64>,
    pub g_force: Option<f64>,
    pub flaps_pct: Option<i64>,
    pub gear_down: Option<i64>,
    pub spoilers_pct: Option<i64>,
    // Motor 1..4 (6 métricas cada uno).
    pub eng_n1_1: Option<f64>,
    pub eng_n1_2: Option<f64>,
    pub eng_n1_3: Option<f64>,
    pub eng_n1_4: Option<f64>,
    pub eng_n2_1: Option<f64>,
    pub eng_n2_2: Option<f64>,
    pub eng_n2_3: Option<f64>,
    pub eng_n2_4: Option<f64>,
    pub eng_egt_1: Option<f64>,
    pub eng_egt_2: Option<f64>,
    pub eng_egt_3: Option<f64>,
    pub eng_egt_4: Option<f64>,
    pub eng_ff_pph_1: Option<f64>,
    pub eng_ff_pph_2: Option<f64>,
    pub eng_ff_pph_3: Option<f64>,
    pub eng_ff_pph_4: Option<f64>,
    pub eng_oil_temp_1: Option<f64>,
    pub eng_oil_temp_2: Option<f64>,
    pub eng_oil_temp_3: Option<f64>,
    pub eng_oil_temp_4: Option<f64>,
    pub eng_oil_press_1: Option<f64>,
    pub eng_oil_press_2: Option<f64>,
    pub eng_oil_press_3: Option<f64>,
    pub eng_oil_press_4: Option<f64>,
}

/// (v4.0.0 — P5) Carga el track completo para el Performance modal.
/// Tres queries en una porque sqlx no implementa FromRow para tuplas
/// >16 elementos — usamos el struct dedicado.
pub async fn list_track_full(
    pool: &SqlitePool,
    flight_id: i64,
) -> anyhow::Result<Vec<FlightTrackFullPoint>> {
    let rows = sqlx::query_as::<_, FlightTrackFullPoint>(
        r#"
        SELECT ts, lat, lon, alt_ft, gs_kt,
               ias_kt, vs_fpm, pitch_deg, bank_deg, g_force,
               flaps_pct, gear_down, spoilers_pct,
               eng_n1_1, eng_n1_2, eng_n1_3, eng_n1_4,
               eng_n2_1, eng_n2_2, eng_n2_3, eng_n2_4,
               eng_egt_1, eng_egt_2, eng_egt_3, eng_egt_4,
               eng_ff_pph_1, eng_ff_pph_2, eng_ff_pph_3, eng_ff_pph_4,
               eng_oil_temp_1, eng_oil_temp_2, eng_oil_temp_3, eng_oil_temp_4,
               eng_oil_press_1, eng_oil_press_2, eng_oil_press_3, eng_oil_press_4
        FROM flight_log_track
        WHERE flight_id = ?1
        ORDER BY ts ASC
        "#,
    )
    .bind(flight_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn list_entries(pool: &SqlitePool) -> anyhow::Result<Vec<FlightLogEntry>> {
    // (v3.6.0 Phase H) Filtramos `status = 'completed'` por defecto.
    // Los `partial` se conservan en DB para auditoria/recompute pero
    // no aparecen en el FlightBook (decisión Q1=b del usuario).
    let rows = sqlx::query_as::<_, FlightLogEntry>(
        r#"
        SELECT id, started_at, ended_at,
               origin_lat, origin_lon, origin_icao, origin_name,
               destination_lat, destination_lon, destination_icao, destination_name,
               aircraft_title, aircraft_atc_type,
               aircraft_model, aircraft_airline, aircraft_registration,
               distance_nm, flight_time_s,
               max_altitude_ft,
               landing_fpm, max_ground_speed_kt, max_true_airspeed_kt,
               departure_gate, arrival_gate,
               passengers, cargo_kg, fuel_used_kg, paused_seconds,
               source,
               flight_number, callsign, airline_icao, status,
               score_total, score_max, score_grade,
               COALESCE(bounced, 0) AS bounced,
               max_brake_temp_c, max_tire_wear_pct,
               metar_origin, metar_dest
        FROM flight_log
        WHERE status = 'completed'
        ORDER BY started_at DESC
        "#,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn delete_entry(pool: &SqlitePool, id: i64) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM flight_log WHERE id = ?1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────────
// (v6 #2a) Hangar & Analytics — telemetría agregada por avión/matrícula.
//
// Agregamos en Rust sobre los vuelos `completed` (mismo criterio que el
// FlightBook). El "desgaste/mantenimiento" se deriva del FPM de touchdown:
// un aterrizaje fino ronda -100..-300; bajo -600 es duro; bajo -1000 abusivo.
// ─────────────────────────────────────────────────────────────────────────

/// Clasifica un aterrizaje por su FPM de touchdown (negativo = descenso):
///   · butter:     fpm > -150  (mantequilla)
///   · acceptable: -300 ≤ fpm ≤ -150
///   · hard:       fpm < -300  (duro)
pub fn landing_grade(fpm: i64) -> &'static str {
    if fpm > -150 {
        "butter"
    } else if fpm >= -300 {
        "acceptable"
    } else {
        "hard"
    }
}

/// Un aterrizaje individual — alimenta el gráfico de tendencia FPM y la tira
/// "Best Landings".
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HangarLanding {
    pub fpm: i64,
    /// ICAO del aeropuerto de aterrizaje (destino).
    pub icao: Option<String>,
    pub airport_name: Option<String>,
    /// `started_at` del vuelo (ISO-8601).
    pub date: String,
    pub model: Option<String>,
    pub registration: Option<String>,
    pub flight_time_s: Option<i64>,
    /// "butter" | "acceptable" | "hard".
    pub grade: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HangarAircraft {
    /// Clave de agrupación (matrícula si existe, si no el modelo).
    pub key: String,
    pub registration: Option<String>,
    pub model: Option<String>,
    pub airline_icao: Option<String>,
    pub airline_name: Option<String>,
    pub flights: i64,
    pub total_nm: f64,
    pub total_time_s: i64,
    /// Promedio del FPM de touchdown (negativo = descenso).
    pub avg_landing_fpm: Option<f64>,
    /// Peor (más negativo) FPM de touchdown registrado.
    pub worst_landing_fpm: Option<i64>,
    /// Nº de aterrizajes duros (grade == "hard").
    pub hard_landings: i64,
    /// Salud del tren 0..100 derivada del desgaste por aterrizajes.
    pub health_pct: i64,
    /// Estado derivado: "good" | "watch" | "alert".
    pub health: String,
    pub last_flight_at: Option<String>,
    /// Aeropuerto del vuelo más reciente (para "ubicación" del avión).
    pub last_icao: Option<String>,
    pub last_airport_name: Option<String>,
    /// Últimos ≤10 aterrizajes (orden antiguo→reciente) para la gráfica FPM.
    pub recent_landings: Vec<HangarLanding>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HangarAirport {
    pub icao: String,
    pub name: Option<String>,
    /// Veces visitado (como origen o destino).
    pub visits: i64,
}

/// (v6.1 #29) Conteo genérico etiqueta→nº de vuelos (aerolíneas, tipos…).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HangarCount {
    /// Etiqueta principal a mostrar (nombre de aerolínea, modelo limpio…).
    pub label: String,
    /// Código auxiliar para logo/ícono (ICAO de aerolínea), opcional.
    pub code: Option<String>,
    pub count: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HangarAnalytics {
    pub total_flights: i64,
    pub total_nm: f64,
    pub total_time_s: i64,
    /// Nº de aterrizajes registrados (vuelos con landing_fpm).
    pub total_landings: i64,
    /// FPM promedio global de aterrizaje.
    pub global_avg_fpm: Option<f64>,
    /// (v6.1 #33) Flota COMPLETA ordenada por nº de vuelos. El frontend
    /// muestra el top 10 por defecto y permite buscar el resto.
    pub aircraft: Vec<HangarAircraft>,
    /// Aeropuertos más frecuentados (origen + destino).
    pub airports: Vec<HangarAirport>,
    /// (v6.1 #29) Destinos más visitados (solo destino).
    pub top_destinations: Vec<HangarAirport>,
    /// (v6.1 #29) Aerolíneas más voladas.
    pub top_airlines: Vec<HangarCount>,
    /// (v6.1 #29) Tipos de avión más volados.
    pub top_aircraft_types: Vec<HangarCount>,
    /// Mejores aterrizajes de la flota (FPM más fino primero).
    pub best_landings: Vec<HangarLanding>,
}

/// Salud del tren 0..100 a partir de los conteos de aterrizajes por grado.
fn gear_health_pct(butter: i64, acceptable: i64, hard: i64) -> i64 {
    let total = butter + acceptable + hard;
    if total == 0 {
        return 100; // sin aterrizajes registrados → asumimos nuevo
    }
    // Cada acceptable resta 3, cada hard resta 12 (acotado a 0..100).
    (100 - acceptable * 3 - hard * 12).clamp(0, 100)
}

/// (v6.1) Normaliza el modelo del avión a un nombre CANÓNICO para que el mismo
/// tipo —escrito de mil formas por cada addon— cuente como uno solo:
///   "PMDG 737", "Boeing 737-800", "B738"  → "Boeing 737-800"
///   "iniBuilds A350", "Airbus A350-900"   → "Airbus A350-900"
///   "Fenix A320", "Airbus A320"           → "Airbus A320"
/// Quita el prefijo del desarrollador y canoniza por tokens.
pub fn normalize_model(raw: &str) -> String {
    let mut s = raw.trim().to_string();
    // 1. Quitar prefijos de desarrollador (case-insensitive, al inicio).
    const DEVS: &[&str] = &[
        "PMDG", "FENIX", "INIBUILDS", "INIBUILD", "FLYBYWIRE", "FLY BY WIRE", "FBW",
        "AEROSOFT", "CAPTAIN SIM", "CAPTAINSIM", "LEONARDO", "A2A", "MILVIZ",
        "JUST FLIGHT", "JUSTFLIGHT", "QUALITYWINGS", "QUALITY WINGS", "MADDOG",
        "ROTATE", "TFDI", "BREDOK3D", "BREDOK", "ASOBO", "MICROSOFT", "CARENADO",
        "BLACKBOX", "IFLY", "FLIGHT FACTOR", "FLIGHTFACTOR", "TOLISS", "HEADWIND",
    ];
    loop {
        let up = s.trim().to_uppercase();
        let mut stripped = false;
        for d in DEVS {
            if up.starts_with(d) {
                s = s.trim()[d.len()..].trim_start_matches(['-', ':', ' ']).to_string();
                stripped = true;
                break;
            }
        }
        if !stripped {
            break;
        }
    }
    let u = s.to_uppercase();
    let has = |k: &str| u.contains(k);

    // 2. Canonización por tokens (los más específicos primero).
    // Boeing
    if has("747") {
        return "Boeing 747-400".into();
    }
    if has("737") || has("73M") || (has("MAX") && has("73")) {
        if has("MAX 8") || has("7M8") || has("738M") || (has("MAX") && has("8")) {
            return "Boeing 737 MAX 8".into();
        }
        if has("MAX 9") || has("7M9") || (has("MAX") && has("9")) {
            return "Boeing 737 MAX 9".into();
        }
        if has("900") || has("73W") {
            return "Boeing 737-900".into();
        }
        if has("700") || has("73G") {
            return "Boeing 737-700".into();
        }
        if has("600") {
            return "Boeing 737-600".into();
        }
        return "Boeing 737-800".into();
    }
    if has("777") {
        if has("200") {
            return "Boeing 777-200".into();
        }
        return "Boeing 777-300ER".into();
    }
    if has("787") {
        if has("10") {
            return "Boeing 787-10".into();
        }
        if has("8") {
            return "Boeing 787-8".into();
        }
        return "Boeing 787-9".into();
    }
    if has("767") {
        return "Boeing 767-300".into();
    }
    if has("757") {
        return "Boeing 757-200".into();
    }
    // Airbus
    if has("A380") || has("A388") {
        return "Airbus A380".into();
    }
    if has("A350") || has("A359") || has("A35K") {
        if has("1000") || has("35K") {
            return "Airbus A350-1000".into();
        }
        return "Airbus A350-900".into();
    }
    if has("A330") {
        if has("900") || has("NEO") || has("A339") {
            return "Airbus A330-900neo".into();
        }
        if has("200") {
            return "Airbus A330-200".into();
        }
        return "Airbus A330-300".into();
    }
    if has("A340") {
        return "Airbus A340".into();
    }
    if has("A321") || has("A21") {
        return if has("NEO") || has("NX") || has("A21N") {
            "Airbus A321neo".into()
        } else {
            "Airbus A321".into()
        };
    }
    if has("A319") || has("A19") {
        return "Airbus A319".into();
    }
    if has("A318") {
        return "Airbus A318".into();
    }
    if has("A320") || has("A20") || has("A32N") {
        return if has("NEO") || has("NX") || has("A20N") {
            "Airbus A320neo".into()
        } else {
            "Airbus A320".into()
        };
    }
    // Embraer / regional
    if has("E195") || has("E190") || has("E170") || has("E175") || has("EMB") || has("ERJ") {
        return "Embraer E-Jet".into();
    }
    if has("CRJ") {
        return "Bombardier CRJ".into();
    }
    if has("DH8") || has("DHC8") || has("Q400") || has("DASH 8") {
        return "Dash 8 Q400".into();
    }
    if has("ATR") || has("AT7") || has("AT4") {
        return "ATR 72".into();
    }

    // 3. Sin match: devolvemos el nombre ya sin prefijo de dev (limpio).
    if s.trim().is_empty() {
        raw.trim().to_string()
    } else {
        s.trim().to_string()
    }
}

pub async fn hangar_analytics(pool: &SqlitePool) -> anyhow::Result<HangarAnalytics> {
    use std::collections::HashMap;

    // Las entries vienen ordenadas DESC por started_at → la 1ª vista de cada
    // avión es su vuelo más reciente.
    let entries = list_entries(pool).await?;
    let total_flights = entries.len() as i64;
    // (v6.1) ICAOs aprendidos de los propios vuelos para agrupar aerolíneas
    // sin duplicar (mismo criterio que Finanzas).
    let learned_airlines = crate::airline_economy::learn_airline_icaos(&entries);

    struct Acc {
        registration: Option<String>,
        model: Option<String>,
        airline_icao: Option<String>,
        airline_name: Option<String>,
        flights: i64,
        total_nm: f64,
        total_time_s: i64,
        fpm_sum: i64,
        fpm_count: i64,
        worst_fpm: Option<i64>,
        butter: i64,
        acceptable: i64,
        hard: i64,
        last_flight_at: Option<String>,
        last_icao: Option<String>,
        last_airport_name: Option<String>,
        recent: Vec<HangarLanding>, // newest-first, capado a 10
    }

    let mut by_ac: HashMap<String, Acc> = HashMap::new();
    // icao -> (nombre, visitas)
    let mut airports: HashMap<String, (Option<String>, i64)> = HashMap::new();
    // (v6.1 #29) Agregados para la vista "sin selección".
    // destino icao -> (nombre, visitas)
    let mut destinations: HashMap<String, (Option<String>, i64)> = HashMap::new();
    // clave aerolínea -> (label, icao, vuelos)
    let mut airlines: HashMap<String, (String, Option<String>, i64)> = HashMap::new();
    // modelo (mayúsculas) -> (label, vuelos)
    let mut types: HashMap<String, (String, i64)> = HashMap::new();
    let mut best_landings: Vec<HangarLanding> = Vec::new();
    let mut total_nm = 0.0;
    let mut total_time_s = 0i64;
    let mut global_fpm_sum = 0i64;
    let mut global_fpm_count = 0i64;

    let norm = |s: &Option<String>| -> Option<String> {
        s.as_deref()
            .map(str::trim)
            .filter(|x| !x.is_empty())
            .map(|x| x.to_string())
    };

    for e in &entries {
        let reg = norm(&e.aircraft_registration);
        let model = norm(&e.aircraft_model)
            .or_else(|| norm(&e.aircraft_atc_type))
            .or_else(|| norm(&e.aircraft_title))
            .map(|m| normalize_model(&m));
        let key = match (&reg, &model) {
            (Some(r), _) => format!("reg:{}", r.to_uppercase()),
            (None, Some(m)) => format!("mdl:{}", m.to_uppercase()),
            _ => "unknown".to_string(),
        };
        let dest_icao = norm(&e.destination_icao);
        let dest_name = norm(&e.destination_name);

        let acc = by_ac.entry(key.clone()).or_insert_with(|| Acc {
            registration: reg.clone(),
            model: model.clone(),
            airline_icao: norm(&e.airline_icao),
            airline_name: norm(&e.aircraft_airline),
            flights: 0,
            total_nm: 0.0,
            total_time_s: 0,
            fpm_sum: 0,
            fpm_count: 0,
            worst_fpm: None,
            butter: 0,
            acceptable: 0,
            hard: 0,
            last_flight_at: None,
            last_icao: None,
            last_airport_name: None,
            recent: Vec::new(),
        });

        acc.flights += 1;
        if let Some(d) = e.distance_nm {
            acc.total_nm += d;
            total_nm += d;
        }
        if let Some(ts) = e.flight_time_s {
            acc.total_time_s += ts;
            total_time_s += ts;
        }
        if acc.airline_icao.is_none() {
            acc.airline_icao = norm(&e.airline_icao);
        }
        if acc.airline_name.is_none() {
            acc.airline_name = norm(&e.aircraft_airline);
        }
        if acc.registration.is_none() {
            acc.registration = reg.clone();
        }
        if acc.model.is_none() {
            acc.model = model.clone();
        }
        // El 1er vuelo visto (más reciente) fija la "ubicación" del avión.
        if acc.last_flight_at.is_none() {
            acc.last_flight_at = Some(e.started_at.clone());
            acc.last_icao = dest_icao.clone();
            acc.last_airport_name = dest_name.clone();
        }

        if let Some(fpm) = e.landing_fpm {
            acc.fpm_sum += fpm;
            acc.fpm_count += 1;
            acc.worst_fpm = Some(acc.worst_fpm.map_or(fpm, |w| w.min(fpm)));
            global_fpm_sum += fpm;
            global_fpm_count += 1;
            let grade = landing_grade(fpm);
            match grade {
                "butter" => acc.butter += 1,
                "acceptable" => acc.acceptable += 1,
                _ => acc.hard += 1,
            }
            let landing = HangarLanding {
                fpm,
                icao: dest_icao.clone(),
                airport_name: dest_name.clone(),
                date: e.started_at.clone(),
                model: model.clone(),
                registration: reg.clone(),
                flight_time_s: e.flight_time_s,
                grade: grade.to_string(),
            };
            if acc.recent.len() < 10 {
                acc.recent.push(landing.clone());
            }
            best_landings.push(landing);
        }

        for (icao, name) in [
            (norm(&e.origin_icao), norm(&e.origin_name)),
            (dest_icao.clone(), dest_name.clone()),
        ] {
            if let Some(ic) = icao {
                let ent = airports.entry(ic.to_uppercase()).or_insert((name.clone(), 0));
                ent.1 += 1;
                if ent.0.is_none() {
                    ent.0 = name;
                }
            }
        }

        // (v6.1 #29) Destinos más visitados (SOLO destino).
        if let Some(ic) = &dest_icao {
            let ent = destinations
                .entry(ic.to_uppercase())
                .or_insert((dest_name.clone(), 0));
            ent.1 += 1;
            if ent.0.is_none() {
                ent.0 = dest_name.clone();
            }
        }
        // (v6.1) Aerolíneas más voladas — agrupación CANÓNICA compartida con
        // Finanzas: colapsa ICAO/callsign/livery-con-nombre-parecido en una
        // sola aerolínea (igual que los tags del FlightBook), sin duplicar.
        if let Some(canon) = crate::airline_economy::canonical_airline(
            e.airline_icao.as_deref(),
            e.callsign.as_deref(),
            e.aircraft_airline.as_deref(),
            e.aircraft_title.as_deref(),
            &learned_airlines,
        ) {
            let ent = airlines
                .entry(canon.key)
                .or_insert((canon.name.clone(), canon.icao.clone(), 0));
            ent.2 += 1;
            if ent.1.is_none() {
                ent.1 = canon.icao.clone();
            }
        }
        // (v6.1 #29) Tipos de avión más volados.
        if let Some(m) = &model {
            let ent = types.entry(m.to_uppercase()).or_insert((m.clone(), 0));
            ent.1 += 1;
        }
    }

    let mut aircraft: Vec<HangarAircraft> = by_ac
        .into_iter()
        .map(|(key, mut a)| {
            let avg = if a.fpm_count > 0 {
                Some(a.fpm_sum as f64 / a.fpm_count as f64)
            } else {
                None
            };
            let health_pct = gear_health_pct(a.butter, a.acceptable, a.hard);
            let health = if health_pct >= 80 {
                "good"
            } else if health_pct >= 50 {
                "watch"
            } else {
                "alert"
            }
            .to_string();
            // recent venía newest-first → invertimos a antiguo→reciente.
            a.recent.reverse();
            HangarAircraft {
                key,
                registration: a.registration,
                model: a.model,
                airline_icao: a.airline_icao,
                airline_name: a.airline_name,
                flights: a.flights,
                total_nm: a.total_nm,
                total_time_s: a.total_time_s,
                avg_landing_fpm: avg,
                worst_landing_fpm: a.worst_fpm,
                hard_landings: a.hard,
                health_pct,
                health,
                last_flight_at: a.last_flight_at,
                last_icao: a.last_icao,
                last_airport_name: a.last_airport_name,
                recent_landings: a.recent,
            }
        })
        .collect();
    aircraft.sort_by(|a, b| {
        b.flights.cmp(&a.flights).then(
            b.total_nm
                .partial_cmp(&a.total_nm)
                .unwrap_or(std::cmp::Ordering::Equal),
        )
    });
    // (v6.1 #33) NO truncamos: devolvemos la flota completa ordenada. El
    // frontend pinta el top 10 por defecto y el buscador filtra el resto.

    let mut airports: Vec<HangarAirport> = airports
        .into_iter()
        .map(|(icao, (name, visits))| HangarAirport { icao, name, visits })
        .collect();
    airports.sort_by(|a, b| b.visits.cmp(&a.visits));
    airports.truncate(12);

    // (v6.1 #29) Top destinos / aerolíneas / tipos.
    let mut top_destinations: Vec<HangarAirport> = destinations
        .into_iter()
        .map(|(icao, (name, visits))| HangarAirport { icao, name, visits })
        .collect();
    top_destinations.sort_by(|a, b| b.visits.cmp(&a.visits));
    top_destinations.truncate(8);

    let mut top_airlines: Vec<HangarCount> = airlines
        .into_iter()
        .map(|(_, (label, code, count))| HangarCount { label, code, count })
        .collect();
    top_airlines.sort_by(|a, b| b.count.cmp(&a.count));
    top_airlines.truncate(8);

    let mut top_aircraft_types: Vec<HangarCount> = types
        .into_iter()
        .map(|(_, (label, count))| HangarCount {
            label,
            code: None,
            count,
        })
        .collect();
    top_aircraft_types.sort_by(|a, b| b.count.cmp(&a.count));
    top_aircraft_types.truncate(8);

    // Mejores aterrizajes: FPM más cercano a 0 (más fino) primero.
    best_landings.sort_by(|a, b| b.fpm.cmp(&a.fpm));
    best_landings.truncate(8);

    let global_avg_fpm = if global_fpm_count > 0 {
        Some(global_fpm_sum as f64 / global_fpm_count as f64)
    } else {
        None
    };

    Ok(HangarAnalytics {
        total_flights,
        total_nm,
        total_time_s,
        total_landings: global_fpm_count,
        global_avg_fpm,
        aircraft,
        airports,
        top_destinations,
        top_airlines,
        top_aircraft_types,
        best_landings,
    })
}

// ─────────────────────────────────────────────────────────────────────────
// (v6 #2a) Perfil del piloto + sistema de niveles.
//
// La IDENTIDAD se deriva del correo del Drive conectado (`google_user_email`):
//   · contiene "jose.daniel" → Capitán Daniel (el dueño)
//   · cualquier otro / sin correo → Capitán Héctor
//
// El NIVEL se basa en los puntos de scoring acumulados por vuelo. Para que
// subir no sea tan difícil, cada punto vale `XP_PER_POINT` de experiencia y la
// curva por nivel es suave (`xp_to_advance`). Ambos son tuneables.
// ─────────────────────────────────────────────────────────────────────────

/// XP que otorga cada punto de scoring. Subirlo = niveles más fáciles.
const XP_PER_POINT: f64 = 3.0;

/// XP necesaria para pasar de `level` al siguiente. Crece suave.
fn xp_to_advance(level: i64) -> i64 {
    200 + (level - 1) * 60
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PilotProfile {
    /// "daniel" | "hector".
    pub identity: String,
    /// Nombre visible ("Capitán Daniel" / "Capitán Héctor").
    pub name: String,
    pub email: Option<String>,
    pub total_xp: i64,
    pub level: i64,
    /// XP acumulada dentro del nivel actual.
    pub xp_in_level: i64,
    /// XP necesaria para completar el nivel actual.
    pub xp_for_next: i64,
    pub flights_scored: i64,
    pub total_points: i64,
}

pub async fn pilot_profile(pool: &SqlitePool) -> anyhow::Result<PilotProfile> {
    // Identidad por el correo del Drive conectado.
    let email: Option<String> =
        sqlx::query_as::<_, (String,)>("SELECT value FROM settings WHERE key = 'google_user_email'")
            .fetch_optional(pool)
            .await
            .ok()
            .flatten()
            .map(|(v,)| v)
            .filter(|s| !s.trim().is_empty());
    let is_daniel = email
        .as_deref()
        .map(|e| e.to_lowercase().contains("jose.daniel"))
        .unwrap_or(false);
    let (identity, name) = if is_daniel {
        ("daniel", "Capitán Daniel")
    } else {
        ("hector", "Capitán Héctor")
    };

    // Puntos acumulados de vuelos completados con score.
    let (total_points, flights_scored): (i64, i64) = sqlx::query_as(
        "SELECT CAST(COALESCE(SUM(score_total), 0) AS INTEGER), COUNT(score_total) \
         FROM flight_log WHERE status = 'completed' AND score_total IS NOT NULL",
    )
    .fetch_one(pool)
    .await?;

    let total_xp = ((total_points as f64) * XP_PER_POINT).round() as i64;

    // Nivel + progreso recorriendo la curva.
    let mut level = 1i64;
    let mut remaining = total_xp;
    loop {
        let need = xp_to_advance(level);
        if remaining < need || level > 9999 {
            return Ok(PilotProfile {
                identity: identity.to_string(),
                name: name.to_string(),
                email,
                total_xp,
                level,
                xp_in_level: remaining.max(0),
                xp_for_next: need,
                flights_scored,
                total_points,
            });
        }
        remaining -= need;
        level += 1;
    }
}

/// (v1.1.1) Borra "vuelos fantasma" creados por bugs de versiones
/// anteriores cuando MSFS reportaba el avión en world origin (0, 0)
/// durante el splash. Sólo borra filas con origen claramente
/// imposible — no toca historiales reales.
///
/// Criterios (OR):
///   1. `|origin_lat| < 0.01 && |origin_lon| < 0.01` — world origin,
///      no hay aeropuertos ahí (Golfo de Guinea océano).
///   2. `origin_icao IS NULL AND distance_nm < 2 AND flight_time_s < 60` —
///      "vuelo" microscópico sin aeropuerto que detectamos como junk.
///
/// Devuelve cuántas filas se borraron. La cascada elimina también
/// los `flight_log_track` asociados (ON DELETE CASCADE en
/// migración 015).
pub async fn delete_junk_flights(pool: &SqlitePool) -> anyhow::Result<u64> {
    let r = sqlx::query(
        r#"
        DELETE FROM flight_log
        WHERE (ABS(origin_lat) < 0.01 AND ABS(origin_lon) < 0.01)
           OR (
                origin_icao IS NULL
                AND COALESCE(distance_nm, 0) < 2
                AND COALESCE(flight_time_s, 0) < 60
                AND ended_at IS NOT NULL
              )
        "#,
    )
    .execute(pool)
    .await?;
    Ok(r.rows_affected())
}

/// Busca el aeropuerto en `airports` cuya distancia haversine al
/// punto sea mínima y < `NEAREST_THRESHOLD_NM`. SQLite no tiene
/// trig nativa; pre-filtramos con un bounding box rectangular y
/// luego haversine en Rust. Devuelve coordenadas para que el caller
/// pueda calcular bearing/offset si lo necesita.
pub async fn nearest_airport_with_coords(
    pool: &SqlitePool,
    lat: f64,
    lon: f64,
) -> anyhow::Result<Option<NearestAirportFull>> {
    let lat_margin = 10.0 / 60.0;
    let lon_margin = (10.0 / 60.0) / lat.to_radians().cos().abs().max(0.01);
    let rows: Vec<(String, String, f64, f64)> = sqlx::query_as(
        r#"
        SELECT icao, name, latitude, longitude
        FROM airports
        WHERE latitude  BETWEEN ?1 AND ?2
          AND longitude BETWEEN ?3 AND ?4
        "#,
    )
    .bind(lat - lat_margin)
    .bind(lat + lat_margin)
    .bind(lon - lon_margin)
    .bind(lon + lon_margin)
    .fetch_all(pool)
    .await?;
    let best = rows
        .into_iter()
        .map(|(icao, name, alat, alon)| {
            let d = haversine_nm(lat, lon, alat, alon);
            (icao, name, alat, alon, d)
        })
        .filter(|(_, _, _, _, d)| *d < NEAREST_THRESHOLD_NM)
        .min_by(|a, b| a.4.partial_cmp(&b.4).unwrap_or(std::cmp::Ordering::Equal));
    Ok(best.map(|(icao, name, latitude, longitude, _)| NearestAirportFull {
        icao,
        name,
        latitude,
        longitude,
    }))
}

/// Bearing inicial (great circle) en grados [0, 360) desde
/// (lat1,lon1) hacia (lat2,lon2). 0° = norte, 90° = este, etc.
fn bearing_deg(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let phi1 = lat1.to_radians();
    let phi2 = lat2.to_radians();
    let dlmd = (lon2 - lon1).to_radians();
    let y = dlmd.sin() * phi2.cos();
    let x = phi1.cos() * phi2.sin() - phi1.sin() * phi2.cos() * dlmd.cos();
    let theta = y.atan2(x).to_degrees();
    (theta + 360.0) % 360.0
}

/// Haversine en millas náuticas — radio terrestre 3440.065 nm.
pub fn haversine_nm(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let r = 3440.065_f64;
    let phi1 = lat1.to_radians();
    let phi2 = lat2.to_radians();
    let dphi = (lat2 - lat1).to_radians();
    let dlmd = (lon2 - lon1).to_radians();
    let a = (dphi / 2.0).sin().powi(2)
        + phi1.cos() * phi2.cos() * (dlmd / 2.0).sin().powi(2);
    let c = 2.0 * a.sqrt().atan2((1.0 - a).sqrt());
    r * c
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn haversine_known_distance() {
        // EBBR (50.901, 4.484) → LEMD (40.471, -3.564) ~ 720 nm
        let d = haversine_nm(50.901, 4.484, 40.471, -3.564);
        assert!(
            (d - 720.0).abs() < 25.0,
            "haversine EBBR-LEMD = {d} nm (esperado ~720)"
        );
    }

    #[test]
    fn haversine_zero_for_same_point() {
        let d = haversine_nm(40.0, -73.0, 40.0, -73.0);
        assert!(d < 0.0001);
    }
}
