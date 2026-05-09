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
    pub source: String,
}

/// Resultado del lookup de aeropuerto más cercano.
#[derive(Debug, Clone)]
pub struct NearestAirport {
    pub icao: String,
    pub name: String,
}

const NEAREST_THRESHOLD_NM: f64 = 3.0;

/// Crea una fila nueva con `ended_at = NULL`. Devuelve el id
/// para que el watcher lo guarde en su state — al aterrizar se
/// hará UPDATE sobre esa fila.
pub async fn start_flight(
    pool: &SqlitePool,
    lat: f64,
    lon: f64,
    aircraft_title: Option<&str>,
    aircraft_atc: Option<&str>,
) -> anyhow::Result<i64> {
    let nearest = nearest_airport(pool, lat, lon).await?;
    let started_at = chrono::Utc::now()
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string();

    let result = sqlx::query(
        r#"
        INSERT INTO flight_log (
            started_at, origin_lat, origin_lon, origin_icao, origin_name,
            aircraft_title, aircraft_atc_type, source
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'simconnect')
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

/// Cierra un vuelo abierto: rellena destino, distancia, duración
/// y altitud máxima.
pub async fn finish_flight(
    pool: &SqlitePool,
    id: i64,
    lat: f64,
    lon: f64,
    max_altitude_ft: Option<i64>,
) -> anyhow::Result<()> {
    let nearest = nearest_airport(pool, lat, lon).await?;

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

    // Duración = ahora - started_at. Si el parse falla por algún
    // motivo dejamos NULL en flight_time_s; no es crítico.
    let flight_time_s = chrono::DateTime::parse_from_rfc3339(&started_at)
        .ok()
        .map(|dt| (now.signed_duration_since(dt.with_timezone(&chrono::Utc))).num_seconds());

    sqlx::query(
        r#"
        UPDATE flight_log
        SET ended_at         = ?1,
            destination_lat  = ?2,
            destination_lon  = ?3,
            destination_icao = ?4,
            destination_name = ?5,
            distance_nm      = ?6,
            flight_time_s    = ?7,
            max_altitude_ft  = COALESCE(?8, max_altitude_ft)
        WHERE id = ?9
        "#,
    )
    .bind(&ended_at)
    .bind(lat)
    .bind(lon)
    .bind(nearest.as_ref().map(|n| n.icao.as_str()))
    .bind(nearest.as_ref().map(|n| n.name.as_str()))
    .bind(distance_nm)
    .bind(flight_time_s)
    .bind(max_altitude_ft)
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

pub async fn list_entries(pool: &SqlitePool) -> anyhow::Result<Vec<FlightLogEntry>> {
    let rows = sqlx::query_as::<_, FlightLogEntry>(
        r#"
        SELECT id, started_at, ended_at,
               origin_lat, origin_lon, origin_icao, origin_name,
               destination_lat, destination_lon, destination_icao, destination_name,
               aircraft_title, aircraft_atc_type, distance_nm, flight_time_s,
               max_altitude_ft, source
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
/// luego haversine en Rust.
pub async fn nearest_airport(
    pool: &SqlitePool,
    lat: f64,
    lon: f64,
) -> anyhow::Result<Option<NearestAirport>> {
    // Bounding box: 1 grado lat ≈ 60 nm, 1 grado lon ≈ 60 nm * cos(lat).
    // Ampliamos a ~10 nm de margen para no perder candidatos en el borde.
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
            (icao, name, d)
        })
        .filter(|(_, _, d)| *d < NEAREST_THRESHOLD_NM)
        .min_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal));

    Ok(best.map(|(icao, name, _)| NearestAirport { icao, name }))
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
