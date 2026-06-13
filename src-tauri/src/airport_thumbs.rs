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
use crate::sources::Source;

/// (v4.27.0) Extrae el modelo de avión del título para buscar en
/// Wikipedia ("Airbus A320", "Boeing 777", "CRJ-700"). Devuelve None
/// si no encuentra patrón conocido — el AddonArt del frontend se
/// encarga del fallback.
fn extract_aircraft_query(title: &str) -> Option<String> {
    use once_cell::sync::Lazy;
    use regex::Regex;
    static RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(
            r"(?i)\b(?:(?P<a>airbus)\s*)?(?P<aa>a3(?:1[89]|2[01]|30|40|50|80)(?:[\s-]?neo)?)
            |\b(?:(?P<b>boeing)\s*)?(?P<bb>7[3-9][0-9](?:[\s-]?(?:max|er|lr|f))?)
            |\b(?P<crj>crj[\s-]?(?:200|700|900|1000))
            |\b(?P<atr>atr[\s-]?(?:42|72))
            |\b(?P<emb>(?:e\s?-?)?(?:170|175|190|195|jet[\s-]?14[05]))
            |\b(?P<cs>cessna\s*\d+|c1?7[2358]|c20[8])
            |\b(?P<tbm>tbm[\s-]?9[34]0)
            |\b(?P<dh>dh[c]?[\s-]?8|q400)",
        )
        .unwrap()
    });
    let caps = RE.captures(title)?;
    if let Some(m) = caps.name("aa") {
        return Some(format!("Airbus {}", m.as_str().to_uppercase()));
    }
    if let Some(m) = caps.name("bb") {
        return Some(format!("Boeing {}", m.as_str().to_uppercase()));
    }
    for k in ["crj", "atr", "emb", "cs", "tbm", "dh"] {
        if let Some(m) = caps.name(k) {
            return Some(m.as_str().to_string());
        }
    }
    None
}

/// Punto de entrada — best-effort, pensado para correr en background
/// después de cada scan. Devuelve cuántas imágenes nuevas se bajaron.
/// `sources` se usa para los addons que no son aeropuerto ni avión:
/// buscamos su título en Simplaza/SceneryAddons y tomamos el
/// `image_url` del primer match.
pub async fn fetch_missing(
    pool: &sqlx::SqlitePool,
    http: &reqwest::Client,
    sources: &[std::sync::Arc<dyn Source>],
    app: &tauri::AppHandle,
) -> anyhow::Result<usize> {
    let pkgs = repo::list_community_packages(pool).await?;
    // (v4.27.0) Dos tipos de candidatos: aeropuertos (busca por
    // ICAO/nombre en Wikipedia) y addons aircraft (busca por modelo
    // de avión extraído del título). El usuario quiere imagen en
    // TODOS, no solo en aeropuertos.
    let is_scenery = |p: &repo::CommunityPackageRow| {
        p.content_type
            .as_deref()
            .map(|c| c.trim().eq_ignore_ascii_case("SCENERY"))
            .unwrap_or(false)
    };
    // (v4.28.0) Todos los paquetes (no solo aeropuertos/aviones): el
    // usuario quiere que TODOS tengan imagen. Para el resto buscamos
    // en Simplaza/SceneryAddons por título.
    let candidates: Vec<_> = pkgs.iter().filter(|p| !p.is_library_pack).collect();

    let simplaza = sources.iter().find(|s| s.id() == "simplaza");
    let mut catalog_lookups = 0usize;
    const CATALOG_LOOKUP_CAP: usize = 30; // cap por scan (amable con Simplaza)

    let mut downloaded = 0usize;
    for pkg in candidates {
        let root = Path::new(&pkg.install_path);
        if !needs_thumbnail(root) {
            continue;
        }
        let aircraft_q = extract_aircraft_query(&pkg.title);
        let result = if is_scenery(pkg)
            && pkg.icao.is_some()
            && pkg.airport_name.is_some()
        {
            let icao = pkg.icao.as_deref().unwrap_or_default();
            let name = pkg.airport_name.as_deref().unwrap_or_default();
            fetch_one(http, icao, name, "airport").await
        } else if let Some(model) = aircraft_q {
            fetch_one(http, &model, &pkg.title, "aircraft").await
        } else if let Some(src) = simplaza {
            // Misc/utility/sound: scrape de Simplaza.
            if catalog_lookups >= CATALOG_LOOKUP_CAP {
                continue;
            }
            catalog_lookups += 1;
            fetch_from_catalog(http, src.as_ref(), &pkg.title).await
        } else {
            Ok(None)
        };
        match result {
            Ok(Some(bytes)) => {
                let out = root.join("simfleet-thumbnail.jpg");
                if let Err(e) = std::fs::write(&out, &bytes) {
                    tracing::debug!(target: "scan", "thumbs: no pude escribir {}: {e}", out.display());
                } else {
                    tracing::info!(
                        target: "scan",
                        "thumbs: {} ← Wikipedia ({} KB)",
                        pkg.folder_name,
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

/// (v4.28.0) Para utilities/misc/sound packs sin foto local ni modelo
/// extraíble: busca el título en Simplaza y descarga la `image_url`
/// del primer resultado. Heurística amable — `query_title()` recorta
/// el título a algo buscable; sin embeddings ni stemming.
async fn fetch_from_catalog(
    http: &reqwest::Client,
    source: &dyn Source,
    title: &str,
) -> anyhow::Result<Option<Vec<u8>>> {
    let q = query_title(title);
    if q.is_empty() {
        return Ok(None);
    }
    let results = match source.search(&q).await {
        Ok(r) => r,
        Err(e) => {
            tracing::debug!(target: "scan", "thumbs: simplaza search '{q}' falló: {e:?}");
            return Ok(None);
        }
    };
    let Some(image_url) = results.iter().find_map(|a| a.image_url.clone()) else {
        return Ok(None);
    };
    let img = http
        .get(&image_url)
        .timeout(std::time::Duration::from_secs(20))
        .send()
        .await?;
    if !img.status().is_success() {
        return Ok(None);
    }
    let ct = img
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    if !ct.starts_with("image/") {
        return Ok(None);
    }
    let bytes = img.bytes().await?;
    if bytes.len() < 5 * 1024 || bytes.len() > 4 * 1024 * 1024 {
        return Ok(None);
    }
    Ok(Some(bytes.to_vec()))
}

/// Recorta el título a 2-3 palabras útiles para buscar en catálogo.
fn query_title(title: &str) -> String {
    let cleaned: String = title
        .chars()
        .map(|c| if c.is_alphanumeric() || c == ' ' { c } else { ' ' })
        .collect();
    cleaned
        .split_whitespace()
        .filter(|w| w.len() >= 3)
        .take(4)
        .collect::<Vec<_>>()
        .join(" ")
}

/// Busca el artículo en Wikipedia y descarga la imagen principal.
/// `kind` = "airport" → exige keyword de aeropuerto en title/desc;
/// "aircraft" → exige keyword de avión. Sin guard, "Boeing 777"
/// podría aterrizar en la página de Boeing the company.
async fn fetch_one(
    http: &reqwest::Client,
    primary: &str,
    secondary: &str,
    kind: &str,
) -> anyhow::Result<Option<Vec<u8>>> {
    for term in [primary, secondary] {
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
        // Guard: el redirect debe aterrizar en el tipo de artículo
        // esperado (aeropuerto / avión), no en una sigla cualquiera.
        let hay = format!(
            "{} {}",
            body.get("title").and_then(|v| v.as_str()).unwrap_or(""),
            body.get("description").and_then(|v| v.as_str()).unwrap_or("")
        )
        .to_lowercase();
        let looks_right = match kind {
            "airport" => ["airport", "aerodrome", "airfield", "air base", "airpark"]
                .iter()
                .any(|k| hay.contains(k)),
            "aircraft" => [
                "aircraft",
                "airliner",
                "airplane",
                "narrow-body",
                "wide-body",
                "twinjet",
                "trijet",
                "quadjet",
                "regional jet",
                "turboprop",
                "freighter",
                "family of",
            ]
            .iter()
            .any(|k| hay.contains(k)),
            _ => true,
        };
        if !looks_right {
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
