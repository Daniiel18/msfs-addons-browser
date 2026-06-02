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
