//! (26/10) Pistas de cada aeropuerto para la tarjeta del FlightBook.
//!
//! Origen: OurAirports (CC0), la misma fuente que `airports.rs`:
//! `https://davidmegginson.github.io/ourairports-data/runways.csv` (~3,5 MB,
//! ~45k pistas). Se baja una vez, en segundo plano (o al abrir la primera
//! tarjeta si aún no estaba), y se renueva cada 30 días. Las cerradas se
//! descartan; los helipuertos vienen como pistas `H1`, `H2`…

use std::time::Duration;

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

const SOURCE_URL: &str = "https://davidmegginson.github.io/ourairports-data/runways.csv";
const SETTING_KEY: &str = "runways_dataset_fetched_at";
const TTL_DAYS: i64 = 30;

/// Una pista (o helipuerto) de un aeropuerto.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Runway {
    /// "08/26", "H1"…
    pub ident: String,
    pub length_ft: Option<i64>,
    pub length_m: Option<i64>,
    pub width_m: Option<i64>,
    pub surface: Option<String>,
    pub lighted: bool,
    pub helipad: bool,
    /// Cabeceras `[lat, lon]` (no todas las pistas las tienen).
    pub le: Option<[f64; 2]>,
    pub he: Option<[f64; 2]>,
}

#[derive(Debug, Deserialize)]
struct Raw {
    airport_ident: String,
    length_ft: String,
    width_ft: String,
    surface: String,
    lighted: String,
    closed: String,
    le_ident: String,
    le_latitude_deg: String,
    le_longitude_deg: String,
    he_ident: String,
    he_latitude_deg: String,
    he_longitude_deg: String,
}

/// Fila ya limpia para la tabla.
#[derive(Debug, Clone, PartialEq)]
struct Row {
    airport: String,
    le_ident: Option<String>,
    he_ident: Option<String>,
    length_ft: Option<i64>,
    width_ft: Option<i64>,
    surface: Option<String>,
    lighted: bool,
    le: Option<(f64, f64)>,
    he: Option<(f64, f64)>,
}

fn opt(s: &str) -> Option<String> {
    let t = s.trim();
    (!t.is_empty()).then(|| t.to_string())
}

fn coord(lat: &str, lon: &str) -> Option<(f64, f64)> {
    let (la, lo) = (lat.trim().parse::<f64>().ok()?, lon.trim().parse::<f64>().ok()?);
    ((-90.0..=90.0).contains(&la) && (-180.0..=180.0).contains(&lo)).then_some((la, lo))
}

fn parse_csv(bytes: &[u8]) -> Vec<Row> {
    let mut rdr = csv::ReaderBuilder::new().has_headers(true).flexible(true).from_reader(bytes);
    let mut out = Vec::with_capacity(48_000);
    for rec in rdr.deserialize::<Raw>() {
        let Ok(r) = rec else { continue };
        let airport = r.airport_ident.trim().to_ascii_uppercase();
        if airport.len() != 4 || !airport.bytes().all(|b| b.is_ascii_alphanumeric()) || r.closed.trim() == "1" {
            continue;
        }
        out.push(Row {
            airport,
            le_ident: opt(&r.le_ident),
            he_ident: opt(&r.he_ident),
            length_ft: r.length_ft.trim().parse().ok(),
            width_ft: r.width_ft.trim().parse().ok(),
            surface: opt(&r.surface),
            lighted: r.lighted.trim() == "1",
            le: coord(&r.le_latitude_deg, &r.le_longitude_deg),
            he: coord(&r.he_latitude_deg, &r.he_longitude_deg),
        });
    }
    out
}

async fn needs_refresh(pool: &SqlitePool) -> bool {
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM runways").fetch_one(pool).await.unwrap_or(0);
    if count == 0 {
        return true;
    }
    let at: Option<String> = sqlx::query_scalar("SELECT value FROM settings WHERE key = ?1")
        .bind(SETTING_KEY)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();
    at.and_then(|v| chrono::DateTime::parse_from_rfc3339(&v).ok())
        .map_or(true, |t| chrono::Utc::now() - t.with_timezone(&chrono::Utc) >= chrono::Duration::days(TTL_DAYS))
}

async fn persist(pool: &SqlitePool, rows: &[Row]) -> anyhow::Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM runways").execute(&mut *tx).await?;
    for chunk in rows.chunks(400) {
        let mut sql = String::from(
            "INSERT INTO runways (airport, le_ident, he_ident, length_ft, width_ft, surface, lighted, le_lat, le_lon, he_lat, he_lon) VALUES ",
        );
        sql.push_str(&vec!["(?,?,?,?,?,?,?,?,?,?,?)"; chunk.len()].join(","));
        let mut q = sqlx::query(&sql);
        for r in chunk {
            q = q
                .bind(&r.airport)
                .bind(&r.le_ident)
                .bind(&r.he_ident)
                .bind(r.length_ft)
                .bind(r.width_ft)
                .bind(&r.surface)
                .bind(r.lighted as i64)
                .bind(r.le.map(|c| c.0))
                .bind(r.le.map(|c| c.1))
                .bind(r.he.map(|c| c.0))
                .bind(r.he.map(|c| c.1));
        }
        q.execute(&mut *tx).await?;
    }
    sqlx::query("INSERT OR REPLACE INTO settings (key, value, updated_at) VALUES (?1, ?2, datetime('now'))")
        .bind(SETTING_KEY)
        .bind(chrono::Utc::now().to_rfc3339())
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

/// Una descarga a la vez (arranque y primera tarjeta pueden coincidir).
static LOADING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Baja el listado si falta o caducó. Si la red falla, se queda lo que haya.
pub async fn ensure_dataset(pool: &SqlitePool, http: &reqwest::Client) -> anyhow::Result<()> {
    let _one = LOADING.lock().await;
    if !needs_refresh(pool).await {
        return Ok(());
    }
    tracing::info!(target: "airports", "descargando pistas de {SOURCE_URL}");
    let body = http
        .get(SOURCE_URL)
        .timeout(Duration::from_secs(90))
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;
    let rows = parse_csv(&body);
    persist(pool, &rows).await?;
    tracing::info!(target: "airports", "pistas guardadas: {}", rows.len());
    Ok(())
}

fn ft_to_m(ft: Option<i64>) -> Option<i64> {
    ft.filter(|f| *f > 0).map(|f| (f as f64 * 0.3048).round() as i64)
}

/// Las pistas de `icao` (de más larga a más corta; helipuertos al final).
pub async fn of_airport(pool: &SqlitePool, icao: &str) -> anyhow::Result<Vec<Runway>> {
    type R = (Option<String>, Option<String>, Option<i64>, Option<i64>, Option<String>, i64, Option<f64>, Option<f64>, Option<f64>, Option<f64>);
    let rows: Vec<R> = sqlx::query_as(
        "SELECT le_ident, he_ident, length_ft, width_ft, surface, lighted, le_lat, le_lon, he_lat, he_lon \
         FROM runways WHERE airport = ?1",
    )
    .bind(icao.trim().to_ascii_uppercase())
    .fetch_all(pool)
    .await?;
    let mut out: Vec<Runway> = rows
        .into_iter()
        .map(|(le, he, len, wid, surface, lighted, la1, lo1, la2, lo2)| {
            let le_s = le.unwrap_or_default();
            let helipad = le_s.to_ascii_uppercase().starts_with('H') && he.is_none();
            let ident = match he.as_deref() {
                Some(h) if !le_s.is_empty() => format!("{le_s}/{h}"),
                _ => le_s,
            };
            Runway {
                ident,
                length_ft: len.filter(|f| *f > 0),
                length_m: ft_to_m(len),
                width_m: ft_to_m(wid),
                surface,
                lighted: lighted != 0,
                helipad,
                le: la1.zip(lo1).map(|(a, b)| [a, b]),
                he: la2.zip(lo2).map(|(a, b)| [a, b]),
            }
        })
        .filter(|r| !r.ident.is_empty())
        .collect();
    out.sort_by(|a, b| a.helipad.cmp(&b.helipad).then(b.length_m.cmp(&a.length_m)));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runways_csv_keeps_open_runways_of_icao_airports() {
        let csv = "id,airport_ref,airport_ident,length_ft,width_ft,surface,lighted,closed,le_ident,le_latitude_deg,le_longitude_deg,le_elevation_ft,le_heading_degT,le_displaced_threshold_ft,he_ident,he_latitude_deg,he_longitude_deg,he_elevation_ft,he_heading_degT,he_displaced_threshold_ft\n\
1,1,CYWG,11000,200,ASP,1,0,13,49.9148,-97.2559,775,128,,31,49.8929,-97.2187,783,308,\n\
2,1,CYWG,8700,200,ASP,1,1,07,49.9,-97.25,,,,25,49.9,-97.2,,,\n\
3,2,00AK,60,60,GRVL,0,0,H1,,,,,,,,,,,\n\
4,3,US-0042,2500,70,GRVL,0,0,18,,,,,,36,,,,,\n";
        let rows = parse_csv(csv.as_bytes());
        assert_eq!(rows.len(), 2, "la cerrada y la de ident que no es de 4 letras, fuera");
        assert_eq!(rows[0].le, Some((49.9148, -97.2559)));
        assert_eq!(rows[0].he_ident.as_deref(), Some("31"));
        assert_eq!((rows[1].le_ident.as_deref(), rows[1].he_ident.as_deref(), rows[1].le), (Some("H1"), None, None));
        assert_eq!(ft_to_m(Some(11000)), Some(3353));
    }
}
