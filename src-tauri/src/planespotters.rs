//! (v6.2.3) Resolución de la foto del avión por matrícula vía planespotters.
//!
//! ANTES esto se hacía con `fetch()` DESDE EL WEBVIEW. planespotters cambió su
//! política y ahora **rechaza User-Agents genéricos** (exige uno descriptivo con
//! URL/email de contacto). El webview manda su UA de navegador → la API
//! respondía `{"error": "...User-Agent..."}` sin la clave `photos`, así que NO
//! salía NINGUNA foto de avión en toda la app. Peor: `fetch()` en el webview NO
//! permite fijar el header `User-Agent` (es un header prohibido por la spec).
//!
//! Solución: pedir el JSON desde Rust (reqwest), donde SÍ podemos mandar un UA
//! propio con contacto. Devolvemos sólo la URL de la miniatura; el `<img>` del
//! frontend la carga normal (las imágenes en sí no validan el UA).

use std::sync::OnceLock;

/// Cliente reqwest perezoso con User-Agent descriptivo (incluye URL de contacto,
/// que es justo lo que exige planespotters).
fn client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .user_agent(concat!(
                "SimFleet/",
                env!("CARGO_PKG_VERSION"),
                " (+https://github.com/Daniiel18/msfs-addons-browser)"
            ))
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new())
    })
}

/// Devuelve la URL de la miniatura grande de planespotters para una matrícula,
/// o `None` si no hay foto (o la matrícula no es válida / la API falla).
#[tauri::command]
pub async fn aircraft_photo(registration: String) -> Option<String> {
    let reg = registration.trim().to_uppercase();
    // Misma validación que el frontend: 3..=10 alfanumérico + guion.
    if reg.len() < 3
        || reg.len() > 10
        || !reg.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    {
        return None;
    }
    let url = format!(
        "https://api.planespotters.net/pub/photos/reg/{}",
        urlencoding::encode(&reg)
    );
    let resp = client()
        .get(&url)
        .header("accept", "application/json")
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let body: serde_json::Value = resp.json().await.ok()?;
    let first = body.get("photos")?.as_array()?.first()?;
    let src = first
        .pointer("/thumbnail_large/src")
        .or_else(|| first.pointer("/thumbnail/src"))
        .and_then(|v| v.as_str())?;
    Some(src.to_string())
}
