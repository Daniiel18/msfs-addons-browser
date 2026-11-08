//! (06/11) METAR de un aeropuerto para el globo del mapa: el clima real, de
//! aviationweather.gov. Solo se pide el del pin señalado y dura 10 minutos.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use anyhow::Result;
use serde::Serialize;
use serde_json::Value;

const API: &str = "https://aviationweather.gov/api/data/metar";
const FRESH_FOR: Duration = Duration::from_secs(600);
const TIMEOUT: Duration = Duration::from_secs(8);

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AirportMetar {
    pub icao: String,
    pub raw: String,
    /// `VFR`, `MVFR`, `IFR` o `LIFR`.
    pub category: Option<String>,
    pub temp_c: Option<f64>,
    /// Grados de tres cifras, o `VRB`.
    pub wind_dir: Option<String>,
    pub wind_kt: Option<f64>,
    pub gust_kt: Option<f64>,
    /// Hora de la observación, en segundos.
    pub observed_at: Option<i64>,
}

/// Sin datos la API responde vacío, no con un error.
fn parse(body: &str) -> Option<AirportMetar> {
    let list: Vec<Value> = serde_json::from_str(body).ok()?;
    let report = list.first()?;
    let raw = report.get("rawOb")?.as_str()?.trim();
    if raw.is_empty() {
        return None;
    }
    let text = |key: &str| report.get(key).and_then(Value::as_str).map(str::to_string);
    let number = |key: &str| report.get(key).and_then(Value::as_f64);
    let wind_dir = match report.get("wdir") {
        Some(Value::Number(n)) => n.as_i64().map(|deg| format!("{deg:03}")),
        Some(Value::String(s)) => Some(s.clone()),
        _ => None,
    };
    Some(AirportMetar {
        icao: text("icaoId")?,
        raw: raw.to_string(),
        category: text("fltCat").filter(|c| !c.is_empty()),
        temp_c: number("temp"),
        wind_dir,
        wind_kt: number("wspd"),
        gust_kt: number("wgst"),
        observed_at: report.get("obsTime").and_then(Value::as_i64),
    })
}

fn valid_code(code: &str) -> bool {
    (3..=5).contains(&code.len()) && code.chars().all(|c| c.is_ascii_alphanumeric())
}

type Cache = Mutex<HashMap<String, (Instant, Option<AirportMetar>)>>;

fn cache() -> &'static Cache {
    static CACHE: OnceLock<Cache> = OnceLock::new();
    CACHE.get_or_init(Cache::default)
}

/// `Ok(None)`: ese aeropuerto no publica METAR. `Err`: no hay red o la API falló,
/// y eso no se guarda: el siguiente intento vuelve a preguntar.
pub async fn fetch(http: &reqwest::Client, icao: &str) -> Result<Option<AirportMetar>> {
    let code = icao.trim().to_ascii_uppercase();
    if !valid_code(&code) {
        return Ok(None);
    }
    if let Ok(saved) = cache().lock() {
        if let Some((at, found)) = saved.get(&code).filter(|(at, _)| at.elapsed() < FRESH_FOR) {
            tracing::debug!(target: "metar", "{code} en caché desde hace {:?}", at.elapsed());
            return Ok(found.clone());
        }
    }
    let body = http
        .get(API)
        .query(&[("ids", code.as_str()), ("format", "json")])
        .timeout(TIMEOUT)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    let found = parse(&body);
    if let Ok(mut saved) = cache().lock() {
        saved.insert(code, (Instant::now(), found.clone()));
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CYOW: &str = r#"[{"icaoId":"CYOW","obsTime":1794002400,"temp":12,"dewp":11,"wdir":50,"wspd":4,"visib":15,
        "rawOb":"METAR CYOW 062200Z 05004KT 15SM BKN075 BKN110 12/11 A2988 RMK AC5AC2 SLP123","fltCat":"VFR"}]"#;

    #[test]
    fn a_report_is_read() {
        let metar = parse(CYOW).unwrap();
        assert_eq!(metar.icao, "CYOW");
        assert!(metar.raw.starts_with("METAR CYOW"));
        assert_eq!(metar.category.as_deref(), Some("VFR"));
        assert_eq!(metar.temp_c, Some(12.0));
        assert_eq!(metar.wind_dir.as_deref(), Some("050"));
        assert_eq!(metar.wind_kt, Some(4.0));
        assert_eq!(metar.gust_kt, None);
        assert_eq!(metar.observed_at, Some(1794002400));
    }

    #[test]
    fn variable_wind_and_missing_fields_do_not_break_it() {
        let metar = parse(r#"[{"icaoId":"SKCL","rawOb":"METAR SKCL 062200Z VRB02KT","wdir":"VRB","wspd":2,"wgst":null}]"#).unwrap();
        assert_eq!(metar.wind_dir.as_deref(), Some("VRB"));
        assert_eq!(metar.category, None);
        assert_eq!(metar.temp_c, None);
    }

    #[test]
    fn no_report_means_none() {
        assert_eq!(parse(""), None, "la API responde 204 sin cuerpo");
        assert_eq!(parse("[]"), None);
        assert_eq!(parse(r#"[{"icaoId":"X","rawOb":"  "}]"#), None);
        assert_eq!(parse("<html>"), None);
    }

    #[test]
    fn only_airport_codes_reach_the_url() {
        assert!(valid_code("CYOW"));
        assert!(valid_code("C53"));
        assert!(!valid_code("CY"));
        assert!(!valid_code("CYOW&x=1"));
    }
}
