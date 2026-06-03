//! (v3.19.0 — P7.9c) Captura de cobertura de NUBES de la vida real
//! durante el vuelo, vía Open-Meteo (gratis, sin API key).
//!
//! MSFS 2020 NO expone la cobertura de nubes por SimConnect de forma
//! fiable (los simvars de nubes están deprecados; `ENV CLOUD DENSITY`
//! existe sólo en MSFS 2024). Como el usuario quiere ver las nubes
//! REALES del momento/lugar del vuelo —no las del simulador— tomamos el
//! dato de Open-Meteo en la posición actual del avión y lo guardamos
//! por sample en `flight_log_track`. Luego el Weather modal lo muestra
//! como histórico, sin depender de ninguna API de pago ni de tener
//! internet al revisar el vuelo.
//!
//! ## Estrategia anti-spam
//!
//! La cobertura de nubes cambia despacio, así que cacheamos el último
//! valor y sólo refrescamos cada ~3 min o cuando el avión se movió
//! >20 NM. El watcher estampa el valor cacheado en CADA track sample
//! (no hace HTTP por sample). Un flag `in_flight` evita fetches
//! solapados. Si Open-Meteo falla o no hay internet, conservamos el
//! último valor (o `None`) sin romper nada.

use once_cell::sync::Lazy;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Snapshot de cobertura de nubes (porcentajes 0..100). `None` cuando
/// todavía no se obtuvo dato (primeros segundos del vuelo, sin internet,
/// o API caída).
#[derive(Clone, Copy, Default, Debug)]
pub struct CloudSnapshot {
    pub total_pct: Option<i64>,
    pub low_pct: Option<i64>,
    pub mid_pct: Option<i64>,
    pub high_pct: Option<i64>,
}

struct CloudCache {
    snap: CloudSnapshot,
    last_fetch: Option<Instant>,
    last_lat: f64,
    last_lon: f64,
    in_flight: bool,
}

static CACHE: Lazy<Mutex<CloudCache>> = Lazy::new(|| {
    Mutex::new(CloudCache {
        snap: CloudSnapshot::default(),
        last_fetch: None,
        last_lat: 0.0,
        last_lon: 0.0,
        in_flight: false,
    })
});

const REFRESH_SECS: u64 = 180;
const MOVE_NM: f64 = 20.0;

/// Devuelve el snapshot de nubes cacheado y, si está viejo o el avión
/// se movió >20 NM, dispara un refresh en background (no bloquea). Lo
/// llama el watcher en el momento de escribir cada track sample.
pub fn cloud_for_position(lat: f64, lon: f64) -> CloudSnapshot {
    if !lat.is_finite() || !lon.is_finite() {
        return CloudSnapshot::default();
    }
    let mut spawn_fetch = false;
    let snap = {
        let mut c = match CACHE.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        let stale = c
            .last_fetch
            .map(|t| t.elapsed() >= Duration::from_secs(REFRESH_SECS))
            .unwrap_or(true);
        let moved = c.last_fetch.is_some()
            && haversine_nm(c.last_lat, c.last_lon, lat, lon) >= MOVE_NM;
        if !c.in_flight && (stale || moved) {
            c.in_flight = true;
            spawn_fetch = true;
        }
        c.snap
    };
    if spawn_fetch {
        tokio::spawn(async move {
            let result = fetch_open_meteo(lat, lon).await;
            let mut c = match CACHE.lock() {
                Ok(g) => g,
                Err(p) => p.into_inner(),
            };
            c.in_flight = false;
            c.last_fetch = Some(Instant::now());
            c.last_lat = lat;
            c.last_lon = lon;
            match result {
                Ok(s) => {
                    tracing::info!(
                        target: "weather",
                        "cloud capture: total={:?}% low={:?} mid={:?} high={:?} @ ({:.3},{:.3})",
                        s.total_pct, s.low_pct, s.mid_pct, s.high_pct, lat, lon
                    );
                    c.snap = s;
                }
                Err(e) => {
                    tracing::warn!(
                        target: "weather",
                        "open-meteo cloud fetch falló @ ({:.3},{:.3}): {e:#}",
                        lat, lon
                    );
                }
            }
        });
    }
    snap
}

async fn fetch_open_meteo(lat: f64, lon: f64) -> anyhow::Result<CloudSnapshot> {
    let url = format!(
        "https://api.open-meteo.com/v1/forecast?latitude={:.4}&longitude={:.4}\
         &current=cloud_cover,cloud_cover_low,cloud_cover_mid,cloud_cover_high",
        lat, lon
    );
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()?;
    let resp = client.get(&url).send().await?.error_for_status()?;
    let json: serde_json::Value = resp.json().await?;
    let cur = json.get("current").cloned().unwrap_or(serde_json::Value::Null);
    let get = |k: &str| -> Option<i64> {
        cur.get(k)
            .and_then(|v| v.as_f64())
            .filter(|v| v.is_finite())
            .map(|v| v.round().clamp(0.0, 100.0) as i64)
    };
    Ok(CloudSnapshot {
        total_pct: get("cloud_cover"),
        low_pct: get("cloud_cover_low"),
        mid_pct: get("cloud_cover_mid"),
        high_pct: get("cloud_cover_high"),
    })
}

/// Distancia aproximada en millas náuticas (haversine).
fn haversine_nm(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    const R_NM: f64 = 3440.065; // radio terrestre medio en NM
    let (p1, p2) = (lat1.to_radians(), lat2.to_radians());
    let dlat = (lat2 - lat1).to_radians();
    let dlon = (lon2 - lon1).to_radians();
    let a = (dlat / 2.0).sin().powi(2)
        + p1.cos() * p2.cos() * (dlon / 2.0).sin().powi(2);
    2.0 * R_NM * a.sqrt().asin()
}

// ===========================================================================
// (v3.25.0) FALLBACK: reconstrucción de weather histórico de la VIDA REAL
// ===========================================================================

/// Sample de weather reconstruido desde Open-Meteo **Archive**. Mismos
/// campos que `flight_log::WeatherSample` — se mapea 1:1 en el comando
/// `get_flight_weather`.
#[derive(Debug, Clone)]
pub struct ArchiveWxSample {
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
    pub cloud_cover_pct: Option<i64>,
    pub cloud_low_pct: Option<i64>,
    pub cloud_mid_pct: Option<i64>,
    pub cloud_high_pct: Option<i64>,
}

/// Parser tolerante de timestamps del track (RFC3339, ISO con/sin `Z`,
/// o separados por espacio). Devuelve UTC.
fn parse_ts_utc(s: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    use chrono::{DateTime, NaiveDateTime, Utc};
    if let Ok(d) = DateTime::parse_from_rfc3339(s) {
        return Some(d.with_timezone(&Utc));
    }
    for fmt in [
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%d %H:%M:%S",
    ] {
        if let Ok(ndt) = NaiveDateTime::parse_from_str(s, fmt) {
            return Some(ndt.and_utc());
        }
    }
    None
}

/// (v3.25.0) Reconstruye el clima REAL del día del vuelo desde Open-Meteo
/// **Archive** (gratis, sin API key, histórico desde 1940), para vuelos que
/// NO capturaron weather AMBIENT durante el vuelo (vuelos viejos / imports
/// VAS). El usuario quiere ver el clima de la **vida real**, no el del
/// simulador — Archive da exactamente eso: viento, temperatura, nubes y
/// precipitación reales en la posición y fecha del vuelo.
///
/// Toma una muestra de hasta 16 puntos del track (espaciados) y los consulta
/// en UNA sola request multi-coordenada. Para cada punto busca la hora del
/// archivo más cercana a su timestamp.
///
/// `track`: lista `(lat, lon, ts)` en orden cronológico.
pub async fn reconstruct_weather_from_archive(
    track: &[(f64, f64, String)],
) -> anyhow::Result<Vec<ArchiveWxSample>> {
    use chrono::NaiveDateTime;

    // 1) Filtrar puntos con coords y ts válidos.
    let valid: Vec<(f64, f64, chrono::DateTime<chrono::Utc>, String)> = track
        .iter()
        .filter_map(|(lat, lon, ts)| {
            if !lat.is_finite() || !lon.is_finite() {
                return None;
            }
            parse_ts_utc(ts).map(|dt| (*lat, *lon, dt, ts.clone()))
        })
        .collect();
    if valid.is_empty() {
        anyhow::bail!("track sin puntos válidos (coords/ts) para reconstruir weather");
    }

    // 2) Muestrear hasta 16 puntos espaciados + asegurar el último.
    const MAX_PTS: usize = 16;
    let step = ((valid.len() as f64) / (MAX_PTS as f64)).ceil().max(1.0) as usize;
    let mut sampled: Vec<&(f64, f64, chrono::DateTime<chrono::Utc>, String)> =
        valid.iter().step_by(step).collect();
    if let (Some(last), Some(cur_last)) = (valid.last(), sampled.last()) {
        if cur_last.3 != last.3 {
            sampled.push(last);
        }
    }

    // 3) Rango de fechas (UTC) del vuelo.
    let d0 = valid.first().unwrap().2.format("%Y-%m-%d").to_string();
    let d1 = valid.last().unwrap().2.format("%Y-%m-%d").to_string();

    // 4) CSV de coords.
    let lats: Vec<String> = sampled.iter().map(|p| format!("{:.4}", p.0)).collect();
    let lons: Vec<String> = sampled.iter().map(|p| format!("{:.4}", p.1)).collect();

    let url = format!(
        "https://archive-api.open-meteo.com/v1/archive?latitude={lats}&longitude={lons}\
&start_date={d0}&end_date={d1}\
&hourly=temperature_2m,wind_speed_10m,wind_direction_10m,cloud_cover,cloud_cover_low,\
cloud_cover_mid,cloud_cover_high,precipitation,surface_pressure,visibility\
&wind_speed_unit=kn&timezone=GMT",
        lats = lats.join(","),
        lons = lons.join(","),
        d0 = d0,
        d1 = d1,
    );

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(25))
        .build()?;
    let resp = client.get(&url).send().await?.error_for_status()?;
    let json: serde_json::Value = resp.json().await?;

    // La API devuelve un objeto si pides 1 coord, o un array si pides >1.
    let blocks: Vec<serde_json::Value> = if json.is_array() {
        json.as_array().cloned().unwrap_or_default()
    } else {
        vec![json]
    };
    if blocks.is_empty() {
        anyhow::bail!("Open-Meteo Archive devolvió 0 bloques");
    }

    // 5) Para cada punto muestreado, buscar la hora más cercana a su ts.
    let mut out: Vec<ArchiveWxSample> = Vec::new();
    for (i, p) in sampled.iter().enumerate() {
        let block = match blocks.get(i).or_else(|| blocks.first()) {
            Some(b) => b,
            None => continue,
        };
        let hourly = match block.get("hourly") {
            Some(h) => h,
            None => continue,
        };
        let times = match hourly.get("time").and_then(|v| v.as_array()) {
            Some(t) => t,
            None => continue,
        };
        let target = p.2.timestamp();
        let mut best_idx = 0usize;
        let mut best_diff = i64::MAX;
        for (idx, tv) in times.iter().enumerate() {
            if let Some(s) = tv.as_str() {
                if let Ok(ndt) = NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M") {
                    let diff = (ndt.and_utc().timestamp() - target).abs();
                    if diff < best_diff {
                        best_diff = diff;
                        best_idx = idx;
                    }
                }
            }
        }
        let at = |key: &str| -> Option<f64> {
            hourly
                .get(key)
                .and_then(|v| v.as_array())
                .and_then(|a| a.get(best_idx))
                .and_then(|v| v.as_f64())
                .filter(|v| v.is_finite())
        };
        let oat = at("temperature_2m");
        let precip_mm = at("precipitation");
        // precip_state estilo SimConnect: 2 none, 4 rain, 8 snow.
        let precip_state = match precip_mm {
            Some(mm) if mm > 0.05 => {
                if oat.map(|t| t <= 0.5).unwrap_or(false) {
                    Some(8)
                } else {
                    Some(4)
                }
            }
            Some(_) => Some(2),
            None => None,
        };
        let clamp_pct = |v: f64| v.round().clamp(0.0, 100.0) as i64;

        out.push(ArchiveWxSample {
            ts: p.3.clone(),
            lat: p.0,
            lon: p.1,
            alt_ft: None,
            wind_dir_deg: at("wind_direction_10m").map(|v| v.round() as i64),
            wind_speed_kt: at("wind_speed_10m").map(|v| v.round() as i64),
            oat_c: oat,
            baro_hpa: at("surface_pressure").map(|v| v.round() as i64),
            visibility_m: at("visibility").map(|v| v.round() as i64),
            precip_state,
            cloud_cover_pct: at("cloud_cover").map(clamp_pct),
            cloud_low_pct: at("cloud_cover_low").map(clamp_pct),
            cloud_mid_pct: at("cloud_cover_mid").map(clamp_pct),
            cloud_high_pct: at("cloud_cover_high").map(clamp_pct),
        });
    }

    if out.is_empty() {
        anyhow::bail!("no se pudo mapear ningún sample del archivo");
    }
    tracing::info!(
        target: "weather",
        "archive reconstruido: {} samples de {} puntos ({}..{})",
        out.len(),
        sampled.len(),
        d0,
        d1
    );
    Ok(out)
}
