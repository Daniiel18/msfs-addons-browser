//! (v6.2.38) GSX remoto — GSX Pro puede exponer su estado en la LAN por
//! HTTP en el puerto 8744, pero la IP rota. Por eso lo DESCUBRIMOS por
//! sondeo: probamos la última IP buena y, si falla, escaneamos la subred
//! /24 local buscando quién responde en 8744. De ahí sacamos el estado
//! (fase: boarding/deboarding/…) y el porcentaje para mostrarlo arriba.
//!
//! El puerto es fijo (8744). El FORMATO de la respuesta se resuelve por
//! sondeo: intentamos parsear JSON con nombres de campo comunes y, si no,
//! devolvemos el crudo (`raw`) para poder afinar el parser con una
//! muestra real.

use std::sync::Mutex;
use std::time::Duration;

use futures_util::{stream, StreamExt};
use once_cell::sync::Lazy;
use serde::Serialize;

const GSX_PORT: u16 = 8744;

/// Última IP donde encontramos GSX — se prueba primero para no reescanear.
static LAST_HOST: Lazy<Mutex<Option<String>>> = Lazy::new(|| Mutex::new(None));

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct GsxRemoteStatus {
    /// ¿Respondió algún host en 8744?
    pub connected: bool,
    /// IP:puerto donde respondió.
    pub host: Option<String>,
    /// Fase/estado detectado (boarding, deboarding, pushback, …).
    pub status: Option<String>,
    /// Porcentaje 0..100 si se pudo extraer.
    pub percent: Option<i64>,
    /// Cuerpo crudo (recortado) — para afinar el parser con datos reales.
    pub raw: Option<String>,
}

/// IP IPv4 de la interfaz LAN (truco del UDP connect — no envía nada).
fn local_ipv4() -> Option<[u8; 4]> {
    let sock = std::net::UdpSocket::bind(("0.0.0.0", 0)).ok()?;
    sock.connect(("8.8.8.8", 80)).ok()?;
    match sock.local_addr().ok()?.ip() {
        std::net::IpAddr::V4(v4) => Some(v4.octets()),
        _ => None,
    }
}

/// ¿Hay algo escuchando en `ip:8744`? Connect TCP con timeout corto.
async fn port_open(ip: String) -> Option<String> {
    let addr = format!("{ip}:{GSX_PORT}");
    match tokio::time::timeout(
        Duration::from_millis(350),
        tokio::net::TcpStream::connect(&addr),
    )
    .await
    {
        Ok(Ok(_)) => Some(ip),
        _ => None,
    }
}

/// Escanea la /24 local buscando el primer host con 8744 abierto.
async fn discover_host() -> Option<String> {
    let octets = local_ipv4()?;
    let base = [octets[0], octets[1], octets[2]];
    let candidates: Vec<String> =
        (1..=254u16).map(|h| format!("{}.{}.{}.{}", base[0], base[1], base[2], h)).collect();
    // `collect` no exige `Unpin` (a diferencia de `next`), y a 64 en
    // paralelo con timeout de 350ms el barrido tarda ~1-1.5 s.
    let results: Vec<Option<String>> = stream::iter(candidates)
        .map(port_open)
        .buffer_unordered(64)
        .collect()
        .await;
    results.into_iter().flatten().next()
}

/// Descarga el estado de GSX desde `ip` probando rutas comunes.
async fn fetch_status(client: &reqwest::Client, ip: &str) -> Option<GsxRemoteStatus> {
    for path in ["/", "/status", "/api/status", "/gsx", "/state"] {
        let url = format!("http://{ip}:{GSX_PORT}{path}");
        let resp = client
            .get(&url)
            .timeout(Duration::from_millis(1500))
            .send()
            .await;
        let Ok(resp) = resp else { continue };
        if !resp.status().is_success() {
            continue;
        }
        let body = resp.text().await.unwrap_or_default();
        if body.trim().is_empty() {
            continue;
        }
        let (status, percent) = parse_status(&body);
        return Some(GsxRemoteStatus {
            connected: true,
            host: Some(format!("{ip}:{GSX_PORT}")),
            status,
            percent,
            raw: Some(body.chars().take(2000).collect()),
        });
    }
    None
}

/// Parser best-effort: intenta JSON con nombres de campo comunes; si no,
/// busca palabras clave de fase y un número %. Se afinará con datos reales.
fn parse_status(body: &str) -> (Option<String>, Option<i64>) {
    // 1) JSON.
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(body) {
        let status = ["status", "state", "phase", "service", "activity"]
            .iter()
            .find_map(|k| v.get(*k).and_then(|x| x.as_str()).map(|s| s.to_string()));
        let percent = ["percent", "progress", "percentage", "pct", "value"]
            .iter()
            .find_map(|k| {
                v.get(*k).and_then(|x| x.as_i64().or_else(|| x.as_f64().map(|f| f as i64)))
            })
            .filter(|p| (0..=100).contains(p));
        if status.is_some() || percent.is_some() {
            return (status, percent);
        }
    }
    // 2) Heurística sobre texto plano.
    let lower = body.to_lowercase();
    let phase = [
        "deboarding", "boarding", "pushback", "refuel", "refuelling", "catering",
        "loading", "unloading", "parked", "departure", "arrival",
    ]
    .iter()
    .find(|kw| lower.contains(**kw))
    .map(|kw| kw.to_string());
    let percent = {
        // primer entero 0..100 seguido de '%'
        let re = regex::Regex::new(r"(\d{1,3})\s*%").ok();
        re.and_then(|re| {
            re.captures(body)
                .and_then(|c| c.get(1))
                .and_then(|m| m.as_str().parse::<i64>().ok())
        })
        .filter(|p| (0..=100).contains(p))
    };
    (phase, percent)
}

#[tauri::command]
pub async fn gsx_remote_status(
    state: tauri::State<'_, crate::AppState>,
) -> Result<GsxRemoteStatus, String> {
    let client = &state.http;
    // 1) Prueba la última IP buena.
    let last = LAST_HOST.lock().ok().and_then(|g| g.clone());
    if let Some(ip) = last {
        if port_open(ip.clone()).await.is_some() {
            if let Some(st) = fetch_status(client, &ip).await {
                return Ok(st);
            }
        }
    }
    // 2) Descubre por sondeo.
    if let Some(ip) = discover_host().await {
        if let Ok(mut g) = LAST_HOST.lock() {
            *g = Some(ip.clone());
        }
        if let Some(st) = fetch_status(client, &ip).await {
            return Ok(st);
        }
        // Puerto abierto pero sin cuerpo parseable — reporta conectado.
        return Ok(GsxRemoteStatus {
            connected: true,
            host: Some(format!("{ip}:{GSX_PORT}")),
            ..Default::default()
        });
    }
    Ok(GsxRemoteStatus::default())
}
