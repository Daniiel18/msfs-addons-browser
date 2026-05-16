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
    pub source: String,
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
/// El `departure_gate` queda como `<bearing>° / <dist>m de <ICAO>`
/// cuando no tenemos data del parking real. La detección de nombre
/// de gate exacto ("GATE A12") requiere la SimConnect Facility Data
/// API que aún no está implementada — el offset desde el centro del
/// aeropuerto es lo más útil hasta entonces.
pub async fn start_flight(
    pool: &SqlitePool,
    lat: f64,
    lon: f64,
    aircraft_title: Option<&str>,
    aircraft_atc: Option<&str>,
) -> anyhow::Result<i64> {
    let nearest = nearest_airport_with_coords(pool, lat, lon).await?;
    let started_at = chrono::Utc::now()
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string();
    let dep_gate = format_gate_fallback(lat, lon, nearest.as_ref());

    let result = sqlx::query(
        r#"
        INSERT INTO flight_log (
            started_at, origin_lat, origin_lon, origin_icao, origin_name,
            aircraft_title, aircraft_atc_type, departure_gate, source
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'simconnect')
        "#,
    )
    .bind(&started_at)
    .bind(lat)
    .bind(lon)
    .bind(nearest.as_ref().map(|n| n.icao.as_str()))
    .bind(nearest.as_ref().map(|n| n.name.as_str()))
    .bind(aircraft_title)
    .bind(aircraft_atc)
    .bind(&dep_gate)
    .execute(pool)
    .await?;

    Ok(result.last_insert_rowid())
}

/// Format del gate fallback. Si tenemos aeropuerto cercano, lo
/// expresamos como `Stand · 320° 280m de EBBR` — al menos el usuario
/// puede ubicar el parking en el plano del airport. Si no hay
/// aeropuerto, caemos a coords crudas.
fn format_gate_fallback(lat: f64, lon: f64, nearest: Option<&NearestAirportFull>) -> String {
    match nearest {
        Some(n) => {
            let dist_nm = haversine_nm(n.latitude, n.longitude, lat, lon);
            let bearing = bearing_deg(n.latitude, n.longitude, lat, lon);
            // Convertir nm → metros para que sea más legible (gates
            // están a decenas/cientos de metros del centro de pista).
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
#[derive(Debug, Clone, Copy, Default)]
pub struct FlightFinishMetrics {
    pub max_altitude_ft: Option<i64>,
    /// FPM en el touchdown — negativo = descenso (lo normal).
    pub landing_fpm: Option<i64>,
    /// Ground speed máxima durante el vuelo.
    pub max_ground_speed_kt: Option<i64>,
    /// True airspeed máxima durante el vuelo.
    pub max_true_airspeed_kt: Option<i64>,
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
    let arr_gate = format_gate_fallback(lat, lon, nearest.as_ref());

    // Duración = ahora - started_at. Si el parse falla por algún
    // motivo dejamos NULL en flight_time_s; no es crítico.
    let flight_time_s = chrono::DateTime::parse_from_rfc3339(&started_at)
        .ok()
        .map(|dt| (now.signed_duration_since(dt.with_timezone(&chrono::Utc))).num_seconds());

    sqlx::query(
        r#"
        UPDATE flight_log
        SET ended_at             = ?1,
            destination_lat      = ?2,
            destination_lon      = ?3,
            destination_icao     = ?4,
            destination_name     = ?5,
            distance_nm          = ?6,
            flight_time_s        = ?7,
            max_altitude_ft      = COALESCE(?8, max_altitude_ft),
            landing_fpm          = ?9,
            max_ground_speed_kt  = ?10,
            max_true_airspeed_kt = ?11,
            arrival_gate         = ?12
        WHERE id = ?13
        "#,
    )
    .bind(&ended_at)
    .bind(lat)
    .bind(lon)
    .bind(nearest.as_ref().map(|n| n.icao.as_str()))
    .bind(nearest.as_ref().map(|n| n.name.as_str()))
    .bind(distance_nm)
    .bind(flight_time_s)
    .bind(metrics.max_altitude_ft)
    .bind(metrics.landing_fpm)
    .bind(metrics.max_ground_speed_kt)
    .bind(metrics.max_true_airspeed_kt)
    .bind(&arr_gate)
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
}

/// Devuelve el vuelo abierto más reciente, si existe. El watcher
/// lo invoca al arrancar — si hay uno con `last_position_at` < 1h,
/// asume que el usuario sigue volando y restaura el state.
pub async fn latest_open_flight(pool: &SqlitePool) -> anyhow::Result<Option<OpenFlight>> {
    let row = sqlx::query_as::<_, OpenFlight>(
        r#"
        SELECT id, started_at, origin_lat, origin_lon, origin_icao, max_altitude_ft
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
    let r = sqlx::query(
        r#"
        UPDATE flight_log
        SET ended_at = COALESCE(last_position_at, started_at)
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

pub async fn list_entries(pool: &SqlitePool) -> anyhow::Result<Vec<FlightLogEntry>> {
    let rows = sqlx::query_as::<_, FlightLogEntry>(
        r#"
        SELECT id, started_at, ended_at,
               origin_lat, origin_lon, origin_icao, origin_name,
               destination_lat, destination_lon, destination_icao, destination_name,
               aircraft_title, aircraft_atc_type, distance_nm, flight_time_s,
               max_altitude_ft,
               landing_fpm, max_ground_speed_kt, max_true_airspeed_kt,
               departure_gate, arrival_gate,
               source
        FROM flight_log
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
fn haversine_nm(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
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
