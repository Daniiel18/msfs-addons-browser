//! (v4.26.0) Descarga de imágenes para aeropuertos sin thumbnail.
//!
//! Muchos packs de scenery traen el PNG "PLACEHOLDER" del SDK o
//! directamente ninguna imagen. Este módulo busca una foto real en
//! **Wikipedia** (REST API `page/summary`, que resuelve redirects:
//! "SUMU" → artículo del Aeropuerto de Carrasco) y la guarda como
//! `simfleet-thumbnail.jpg` EN LA RAÍZ del paquete — exactamente
//! donde `find_thumbnail` busca primero, así el card/popover la usan
//! sin tocar nada más.
//!
//! Reglas:
//!   · Solo SCENERY con ICAO resuelto y que no sea library pack.
//!   · Se salta paquetes que ya tienen `simfleet-thumbnail.jpg` o un
//!     thumbnail "real" (no llamado placeholder y ≥30 KB).
//!   · Si Wikipedia no tiene foto, escribe un marcador
//!     `simfleet-thumbnail.none` para no re-consultar en cada scan.
//!   · Secuencial con pausa de 250 ms — amable con la API.
//!   · Al terminar emite `thumbs://updated` con el conteo para que el
//!     frontend invalide su cache de thumbnails.

use std::path::Path;

use tauri::Emitter;

use crate::commands::community::find_thumbnail;
use crate::db::repo;

/// Punto de entrada — best-effort, pensado para correr en background
/// después de cada scan. Devuelve cuántas imágenes nuevas se bajaron.
pub async fn fetch_missing(
    pool: &sqlx::SqlitePool,
    http: &reqwest::Client,
    app: &tauri::AppHandle,
) -> anyhow::Result<usize> {
    let pkgs = repo::list_community_packages(pool).await?;
    let candidates: Vec<_> = pkgs
        .iter()
        .filter(|p| {
            p.icao.is_some()
                && p.airport_name.is_some()
                && !p.is_library_pack
                && p.content_type
                    .as_deref()
                    .map(|c| c.trim().eq_ignore_ascii_case("SCENERY"))
                    .unwrap_or(false)
        })
        .collect();

    let mut downloaded = 0usize;
    for pkg in candidates {
        let root = Path::new(&pkg.install_path);
        if !needs_thumbnail(root) {
            continue;
        }
        let icao = pkg.icao.as_deref().unwrap_or_default();
        let name = pkg.airport_name.as_deref().unwrap_or_default();
        match fetch_one(http, icao, name).await {
            Ok(Some(bytes)) => {
                let out = root.join("simfleet-thumbnail.jpg");
                if let Err(e) = std::fs::write(&out, &bytes) {
                    tracing::debug!(target: "scan", "thumbs: no pude escribir {}: {e}", out.display());
                } else {
                    tracing::info!(
                        target: "scan",
                        "thumbs: {} ({}) ← Wikipedia ({} KB)",
                        pkg.folder_name,
                        icao,
                        bytes.len() / 1024
                    );
                    downloaded += 1;
                }
            }
            Ok(None) => {
                // Sin artículo/foto — marcador para no reintentar en
                // cada scan. Se borra a mano si el usuario quiere
                // forzar otro intento.
                let _ = std::fs::write(root.join("simfleet-thumbnail.none"), b"");
            }
            Err(e) => {
                // Error transitorio (red caída, rate limit): NO
                // escribimos marcador — se reintenta al próximo scan.
                tracing::debug!(target: "scan", "thumbs: {} falló: {e:#}", pkg.folder_name);
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }

    if downloaded > 0 {
        let _ = app.emit("thumbs://updated", downloaded);
    }
    Ok(downloaded)
}

/// ¿Hace falta bajar imagen? Sí cuando no hay `simfleet-thumbnail.*`
/// ni marcador, y el thumbnail local es inexistente o sospechoso de
/// placeholder (nombre lo delata, o pesa <30 KB — los JPG grises del
/// SDK comprimen muy chico; una foto real de aeropuerto no).
fn needs_thumbnail(root: &Path) -> bool {
    if root.join("simfleet-thumbnail.jpg").is_file()
        || root.join("simfleet-thumbnail.none").is_file()
    {
        return false;
    }
    match find_thumbnail(root, 0) {
        None => true,
        Some(found) => {
            let p = Path::new(&found);
            let name = p
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_lowercase();
            if name.contains("placeholder") {
                return true;
            }
            std::fs::metadata(p).map(|m| m.len() < 30 * 1024).unwrap_or(true)
        }
    }
}

/// Busca el artículo en Wikipedia (inglés) — primero por ICAO (hay
/// redirects para casi todos), luego por nombre del aeropuerto — y
/// descarga la imagen principal. Devuelve Ok(None) si no hay artículo
/// o el artículo no parece de un aeropuerto (guard anti-ambigüedad).
async fn fetch_one(
    http: &reqwest::Client,
    icao: &str,
    airport_name: &str,
) -> anyhow::Result<Option<Vec<u8>>> {
    for term in [icao, airport_name] {
        if term.is_empty() {
            continue;
        }
        let url = format!(
            "https://en.wikipedia.org/api/rest_v1/page/summary/{}?redirect=true",
            urlencoding::encode(&term.replace(' ', "_"))
        );
        let resp = match http
            .get(&url)
            .header("accept", "application/json")
            .timeout(std::time::Duration::from_secs(12))
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => return Err(e.into()), // red — transitorio, reintentar luego
        };
        if !resp.status().is_success() {
            continue; // 404 → probar siguiente término
        }
        let body: serde_json::Value = resp.json().await?;
        // Guard: el redirect del ICAO debe aterrizar en un artículo de
        // aeropuerto, no en una sigla cualquiera.
        let hay = format!(
            "{} {}",
            body.get("title").and_then(|v| v.as_str()).unwrap_or(""),
            body.get("description").and_then(|v| v.as_str()).unwrap_or("")
        )
        .to_lowercase();
        let looks_airport = ["airport", "aerodrome", "airfield", "air base", "airpark"]
            .iter()
            .any(|k| hay.contains(k));
        if !looks_airport {
            continue;
        }
        let img_url = body
            .pointer("/originalimage/source")
            .or_else(|| body.pointer("/thumbnail/source"))
            .and_then(|v| v.as_str());
        let Some(img_url) = img_url else {
            continue; // artículo sin foto
        };
        let img = http
            .get(img_url)
            .timeout(std::time::Duration::from_secs(20))
            .send()
            .await?;
        if !img.status().is_success() {
            continue;
        }
        let ct = img
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        if !ct.starts_with("image/") {
            continue;
        }
        let bytes = img.bytes().await?;
        // Caps defensivos: ni iconitos (<5 KB) ni murales (>4 MB).
        if bytes.len() < 5 * 1024 || bytes.len() > 4 * 1024 * 1024 {
            continue;
        }
        return Ok(Some(bytes.to_vec()));
    }
    Ok(None)
}
