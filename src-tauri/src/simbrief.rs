//! Integración mínima con SimBrief.
//!
//! La API pública es:
//!   `https://www.simbrief.com/api/xml.fetcher.php?userid={pilotId}`
//!
//! Devuelve XML del **último OFP** (Operational Flight Plan)
//! generado por ese piloto. NO hay endpoint público de historial,
//! así que la app va construyendo su propia tabla local: cada
//! refresh descarga el OFP actual y lo persiste si es nuevo.
//!
//! Decisión deliberada: parseamos el XML con regex en lugar de
//! añadir una dependencia (`quick-xml` o `serde-xml-rs`). Sólo
//! necesitamos ~10 campos top-level y los selectores de SimBrief
//! son estables — un parser completo es overkill y aumenta el
//! tamaño del binario sin beneficio.

use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

/// Vuelo persistido en `simbrief_flights`. Lo expone el comando
/// `list_simbrief_flights` para que el mapa pinte una LineString
/// por cada uno.
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct SimBriefFlight {
    pub ofp_id: String,
    pub pilot_id: String,
    pub flight_number: Option<String>,
    pub callsign: Option<String>,
    pub aircraft_icao: Option<String>,
    pub origin_icao: String,
    pub origin_name: Option<String>,
    pub origin_lat: f64,
    pub origin_lon: f64,
    pub destination_icao: String,
    pub destination_name: Option<String>,
    pub destination_lat: f64,
    pub destination_lon: f64,
    pub route: Option<String>,
    pub distance_nm: Option<i64>,
    pub est_time_enroute_s: Option<i64>,
    pub generated_at: Option<String>,
    pub fetched_at: String,
}

/// Resultado del refresh manual: cuántos OFPs nuevos se añadieron
/// y la URL/error si algo falló. La UI muestra esto como toast.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SimBriefRefreshResult {
    pub added: usize,
    pub already_known: bool,
    pub flight: Option<SimBriefFlight>,
}

const API_BASE: &str = "https://www.simbrief.com/api/xml.fetcher.php";

/// Descarga el último OFP del piloto y lo persiste. Devuelve si
/// fue una entrada nueva o si ya estaba en la tabla (mismo OFP id).
pub async fn refresh_latest(
    pool: &SqlitePool,
    http: &reqwest::Client,
    pilot_id: &str,
) -> anyhow::Result<SimBriefRefreshResult> {
    let url = format!("{}?userid={}", API_BASE, pilot_id);
    tracing::info!("simbrief: fetching {}", url);

    let xml = http
        .get(&url)
        .header("User-Agent", "MSFSAddonsBrowser/0.1")
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;

    let flight = parse_ofp_xml(&xml, pilot_id).ok_or_else(|| {
        anyhow::anyhow!(
            "no se pudo parsear el OFP de SimBrief para pilot_id={}",
            pilot_id
        )
    })?;

    // ¿Existe ya el ofp_id?
    let existing: Option<(String,)> = sqlx::query_as(
        "SELECT ofp_id FROM simbrief_flights WHERE ofp_id = ?1",
    )
    .bind(&flight.ofp_id)
    .fetch_optional(pool)
    .await?;
    let already_known = existing.is_some();

    sqlx::query(
        r#"
        INSERT INTO simbrief_flights (
            ofp_id, pilot_id, flight_number, callsign, aircraft_icao,
            origin_icao, origin_name, origin_lat, origin_lon,
            destination_icao, destination_name, destination_lat, destination_lon,
            route, distance_nm, est_time_enroute_s, generated_at, fetched_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, datetime('now'))
        ON CONFLICT(ofp_id) DO UPDATE SET fetched_at = datetime('now')
        "#,
    )
    .bind(&flight.ofp_id)
    .bind(&flight.pilot_id)
    .bind(&flight.flight_number)
    .bind(&flight.callsign)
    .bind(&flight.aircraft_icao)
    .bind(&flight.origin_icao)
    .bind(&flight.origin_name)
    .bind(flight.origin_lat)
    .bind(flight.origin_lon)
    .bind(&flight.destination_icao)
    .bind(&flight.destination_name)
    .bind(flight.destination_lat)
    .bind(flight.destination_lon)
    .bind(&flight.route)
    .bind(flight.distance_nm)
    .bind(flight.est_time_enroute_s)
    .bind(&flight.generated_at)
    .execute(pool)
    .await?;

    Ok(SimBriefRefreshResult {
        added: if already_known { 0 } else { 1 },
        already_known,
        flight: Some(flight),
    })
}

pub async fn list_flights(pool: &SqlitePool) -> anyhow::Result<Vec<SimBriefFlight>> {
    let rows = sqlx::query_as::<_, SimBriefFlight>(
        r#"
        SELECT ofp_id, pilot_id, flight_number, callsign, aircraft_icao,
               origin_icao, origin_name, origin_lat, origin_lon,
               destination_icao, destination_name, destination_lat, destination_lon,
               route, distance_nm, est_time_enroute_s, generated_at, fetched_at
        FROM simbrief_flights
        ORDER BY COALESCE(generated_at, fetched_at) DESC
        "#,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn delete_flight(pool: &SqlitePool, ofp_id: &str) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM simbrief_flights WHERE ofp_id = ?1")
        .bind(ofp_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn get_pilot_id(pool: &SqlitePool) -> anyhow::Result<Option<String>> {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT value FROM settings WHERE key = 'simbrief_pilot_id'")
            .fetch_optional(pool)
            .await?;
    Ok(row.map(|(v,)| v).filter(|s| !s.trim().is_empty()))
}

pub async fn set_pilot_id(pool: &SqlitePool, pilot_id: &str) -> anyhow::Result<()> {
    sqlx::query(
        r#"
        INSERT INTO settings (key, value, updated_at)
        VALUES ('simbrief_pilot_id', ?1, datetime('now'))
        ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at
        "#,
    )
    .bind(pilot_id)
    .execute(pool)
    .await?;
    Ok(())
}

// ---- Parser XML ------------------------------------------------------------

/// Selectores precompilados para los campos que necesitamos.
/// Los nombres de tag de SimBrief son estables desde 2018+;
/// si cambian, el `Regex::captures` devuelve `None` y el campo
/// se queda en `Option::None` — degradación elegante.
fn tag_re(tag: &str) -> Regex {
    // Acepta atributos en la apertura: `<icao_code>` o `<icao_code attr="…">`.
    // Captura todo entre apertura y cierre, sin importar saltos de línea.
    Regex::new(&format!(r"(?s)<{tag}(?:\s[^>]*)?>(.*?)</{tag}>")).unwrap()
}

static TAG_OFP_ID: Lazy<Regex> = Lazy::new(|| tag_re("request_id"));
static TAG_FLIGHT_NUMBER: Lazy<Regex> = Lazy::new(|| tag_re("flight_number"));
static TAG_CALLSIGN: Lazy<Regex> = Lazy::new(|| tag_re("callsign"));
// `TAG_AIRCRAFT_ICAO` se quitó: usamos `inner_tag(aircraft_block, "icao_code")`
// para no chocar con los `<icao_code>` de origin/destination.
static TAG_ROUTE: Lazy<Regex> = Lazy::new(|| tag_re("route"));
static TAG_DISTANCE: Lazy<Regex> = Lazy::new(|| tag_re("air_distance"));
static TAG_EST_TIME: Lazy<Regex> = Lazy::new(|| tag_re("est_time_enroute"));
static TAG_TIME_GENERATED: Lazy<Regex> = Lazy::new(|| tag_re("time_generated"));
static TAG_ORIGIN: Lazy<Regex> = Lazy::new(|| tag_re("origin"));
static TAG_DEST: Lazy<Regex> = Lazy::new(|| tag_re("destination"));
static TAG_AIRCRAFT: Lazy<Regex> = Lazy::new(|| tag_re("aircraft"));

/// Parser principal. Devuelve `None` si faltan los campos
/// imprescindibles (origen, destino con lat/lon).
pub fn parse_ofp_xml(xml: &str, pilot_id: &str) -> Option<SimBriefFlight> {
    let origin_block = TAG_ORIGIN.captures(xml)?.get(1)?.as_str();
    let dest_block = TAG_DEST.captures(xml)?.get(1)?.as_str();

    let origin_icao = inner_tag(origin_block, "icao_code")?;
    let origin_lat: f64 = inner_tag(origin_block, "pos_lat")?.parse().ok()?;
    let origin_lon: f64 = inner_tag(origin_block, "pos_long")?.parse().ok()?;
    let origin_name = inner_tag(origin_block, "name");

    let destination_icao = inner_tag(dest_block, "icao_code")?;
    let destination_lat: f64 = inner_tag(dest_block, "pos_lat")?.parse().ok()?;
    let destination_lon: f64 = inner_tag(dest_block, "pos_long")?.parse().ok()?;
    let destination_name = inner_tag(dest_block, "name");

    // Aircraft block puede no estar — fallback a None.
    let aircraft_icao = TAG_AIRCRAFT
        .captures(xml)
        .and_then(|c| c.get(1))
        .and_then(|m| inner_tag(m.as_str(), "icao_code"));

    let ofp_id = first_tag(&TAG_OFP_ID, xml).unwrap_or_else(|| {
        // Sin request_id no podemos deduplicar — usamos hash del
        // contenido como fallback.
        format!("nohash-{}", xml.len())
    });

    Some(SimBriefFlight {
        ofp_id,
        pilot_id: pilot_id.to_string(),
        flight_number: first_tag(&TAG_FLIGHT_NUMBER, xml),
        callsign: first_tag(&TAG_CALLSIGN, xml),
        aircraft_icao,
        origin_icao,
        origin_name,
        origin_lat,
        origin_lon,
        destination_icao,
        destination_name,
        destination_lat,
        destination_lon,
        route: first_tag(&TAG_ROUTE, xml),
        distance_nm: first_tag(&TAG_DISTANCE, xml).and_then(|s| s.trim().parse().ok()),
        est_time_enroute_s: first_tag(&TAG_EST_TIME, xml)
            .and_then(|s| s.trim().parse().ok()),
        generated_at: first_tag(&TAG_TIME_GENERATED, xml),
        fetched_at: chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string(),
    })
}

fn first_tag(re: &Regex, s: &str) -> Option<String> {
    re.captures(s)
        .and_then(|c| c.get(1))
        .map(|m| decode_entities(m.as_str().trim()))
        .filter(|s| !s.is_empty())
}

fn inner_tag(block: &str, tag: &str) -> Option<String> {
    let re = tag_re(tag);
    re.captures(block)
        .and_then(|c| c.get(1))
        .map(|m| decode_entities(m.as_str().trim()))
        .filter(|s| !s.is_empty())
}

/// Decodifica las pocas entidades XML que SimBrief puede emitir
/// (`&amp;`, `&lt;`, `&gt;`). Suficiente para los campos que
/// extraemos.
fn decode_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<OFP>
  <fetch>
    <request_id>123456</request_id>
    <userid>50956</userid>
  </fetch>
  <general>
    <flight_number>BEL341</flight_number>
    <icao_airline>BEL</icao_airline>
    <route>EBBR DCT KOK DCT MAD</route>
    <air_distance>720</air_distance>
  </general>
  <origin>
    <icao_code>EBBR</icao_code>
    <pos_lat>50.901</pos_lat>
    <pos_long>4.484</pos_long>
    <name>Brussels Airport</name>
  </origin>
  <destination>
    <icao_code>LEMD</icao_code>
    <pos_lat>40.471</pos_lat>
    <pos_long>-3.564</pos_long>
    <name>Madrid Barajas</name>
  </destination>
  <aircraft>
    <icao_code>A320</icao_code>
  </aircraft>
  <times>
    <est_time_enroute>5400</est_time_enroute>
  </times>
  <api_params>
    <time_generated>1735689600</time_generated>
  </api_params>
</OFP>"#;

    #[test]
    fn parses_real_simbrief_xml() {
        let f = parse_ofp_xml(SAMPLE_XML, "50956").expect("parse should succeed");
        assert_eq!(f.ofp_id, "123456");
        assert_eq!(f.pilot_id, "50956");
        assert_eq!(f.origin_icao, "EBBR");
        assert_eq!(f.destination_icao, "LEMD");
        assert!((f.origin_lat - 50.901).abs() < 0.01);
        assert!((f.destination_lon - -3.564).abs() < 0.01);
        assert_eq!(f.aircraft_icao.as_deref(), Some("A320"));
        assert_eq!(f.distance_nm, Some(720));
        assert_eq!(f.est_time_enroute_s, Some(5400));
    }

    #[test]
    fn returns_none_when_origin_missing() {
        let xml = "<OFP><general/></OFP>";
        assert!(parse_ofp_xml(xml, "1").is_none());
    }
}
