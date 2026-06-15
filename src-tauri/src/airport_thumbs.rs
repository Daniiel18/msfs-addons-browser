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
/// después de cada scan. Sólo descarga imágenes de AEROPUERTOS sin
/// thumbnail (de Wikipedia, por ICAO/nombre). Aviones y liveries usan
/// su thumbnail NATIVO (ContentInfo/Thumbnail.jpg), que el scanner
/// resuelve — una foto de internet para una livery concreta saldría
/// incorrecta.
pub async fn fetch_missing(
    pool: &sqlx::SqlitePool,
    http: &reqwest::Client,
    app: &tauri::AppHandle,
) -> anyhow::Result<usize> {
    let pkgs = repo::list_community_packages(pool).await?;
    let is_scenery = |p: &repo::CommunityPackageRow| {
        p.content_type
            .as_deref()
            .map(|c| c.trim().eq_ignore_ascii_case("SCENERY"))
            .unwrap_or(false)
    };
    // (v5.0.0 N6) TODOS los addons sin thumbnail nativo (no library):
    //   · Aeropuertos → foto de Wikipedia (por ICAO/nombre).
    //   · Resto (aviones, liveries, mods) → imagen de su ficha en
    //     flightsim.to (su `og:image`). Cubre freeware + liveries. El
    //     payware (Fenix/PMDG) no está en flightsim.to → cae a la silueta.
    let candidates: Vec<_> = pkgs.iter().filter(|p| !p.is_library_pack).collect();

    // Tope de descargas web por scan para no martillar a flightsim.to;
    // el resto se resuelve en scans siguientes (marcador `.none` evita
    // reintentos infinitos).
    let mut web_budget = 40usize;
    let mut downloaded = 0usize;
    for pkg in candidates {
        let root = Path::new(&pkg.install_path);
        if !needs_thumbnail(root) {
            continue;
        }
        let is_airport =
            is_scenery(pkg) && pkg.icao.is_some() && pkg.airport_name.is_some();
        let (result, source) = if is_airport {
            let icao = pkg.icao.as_deref().unwrap_or_default();
            let name = pkg.airport_name.as_deref().unwrap_or_default();
            (fetch_one(http, icao, name, "airport").await, "Wikipedia")
        } else {
            if web_budget == 0 {
                continue; // se procesa en el próximo scan
            }
            web_budget -= 1;
            let creator = pkg.creator.as_deref().unwrap_or_default();
            (
                fetch_from_flightsimto(http, &pkg.title, creator).await,
                "flightsim.to",
            )
        };
        match result {
            Ok(Some(bytes)) => {
                let out = root.join("simfleet-thumbnail.jpg");
                if let Err(e) = std::fs::write(&out, &bytes) {
                    tracing::debug!(target: "scan", "thumbs: no pude escribir {}: {e}", out.display());
                } else {
                    tracing::info!(
                        target: "scan",
                        "thumbs: {} ← {} ({} KB)",
                        pkg.folder_name,
                        source,
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

/// (v5.0.0 N6) Imagen de marketing desde flightsim.to: busca el addon,
/// abre la primera ficha (`/file/...`) y toma su `og:image` (la imagen
/// hero del addon). Cubre liveries, aviones freeware y mods. Best-effort:
/// si no hay ficha o imagen, devuelve `None` (se usa la silueta tipada).
/// El payware (Fenix/PMDG) no está en flightsim.to → cae a la silueta.
async fn fetch_from_flightsimto(
    http: &reqwest::Client,
    title: &str,
    creator: &str,
) -> anyhow::Result<Option<Vec<u8>>> {
    const UA: &str = "Mozilla/5.0 (SimFleet)";
    let query = clean_addon_query(title, creator);
    if query.len() < 3 {
        return Ok(None);
    }
    let search_url = format!(
        "https://flightsim.to/search/{}",
        urlencoding::encode(&query)
    );
    let resp = http
        .get(&search_url)
        .header("user-agent", UA)
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await?;
    if resp.status().is_server_error() {
        // 5xx: transitorio → Err para reintentar (no marcar `.none`).
        anyhow::bail!("flightsim.to search {} → {}", query, resp.status());
    }
    if !resp.status().is_success() {
        return Ok(None);
    }
    let search_html = resp.text().await?;
    // Extracción SYNC (scraper no es Send): sacamos el path como String
    // antes de cruzar cualquier await.
    let Some(detail_path) = first_file_link(&search_html) else {
        return Ok(None);
    };
    let detail_url = if detail_path.starts_with("http") {
        detail_path
    } else {
        format!("https://flightsim.to{detail_path}")
    };
    let dresp = http
        .get(&detail_url)
        .header("user-agent", UA)
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await?;
    if !dresp.status().is_success() {
        return Ok(None);
    }
    let detail_html = dresp.text().await?;
    let Some(img_url) = og_image(&detail_html) else {
        return Ok(None);
    };
    let img = http
        .get(&img_url)
        .header("user-agent", UA)
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
    if bytes.len() < 5 * 1024 || bytes.len() > 6 * 1024 * 1024 {
        return Ok(None);
    }
    Ok(Some(bytes.to_vec()))
}

/// Limpia el título para buscar en flightsim.to: quita "(branch …)",
/// la versión, tokens de sim y "&"; recorta a 5 palabras útiles.
fn clean_addon_query(title: &str, creator: &str) -> String {
    let mut t = title.to_string();
    if let Some(i) = t.find('(') {
        t.truncate(i);
    }
    let cleaned = t
        .split_whitespace()
        .filter(|w| {
            let wl = w.to_lowercase();
            let is_version = wl.starts_with('v')
                && wl.chars().nth(1).map(|c| c.is_ascii_digit()).unwrap_or(false);
            !is_version && !wl.contains("fs2020") && !wl.contains("msfs") && *w != "&"
        })
        .take(5)
        .collect::<Vec<_>>()
        .join(" ");
    let cleaned = cleaned.trim();
    if cleaned.len() >= 3 {
        cleaned.to_string()
    } else if creator.trim().len() >= 3 {
        creator.trim().to_string()
    } else {
        String::new()
    }
}

/// Primer enlace a una ficha de flightsim.to (`/file/...`).
fn first_file_link(html: &str) -> Option<String> {
    use scraper::{Html, Selector};
    let doc = Html::parse_document(html);
    let sel = Selector::parse("a[href*='/file/']").ok()?;
    doc.select(&sel)
        .filter_map(|a| a.value().attr("href"))
        .map(|h| h.to_string())
        .next()
}

/// `og:image` (o twitter:image) de una ficha.
fn og_image(html: &str) -> Option<String> {
    use scraper::{Html, Selector};
    let doc = Html::parse_document(html);
    for prop in [
        "meta[property='og:image']",
        "meta[name='og:image']",
        "meta[property='twitter:image']",
        "meta[name='twitter:image']",
    ] {
        if let Ok(sel) = Selector::parse(prop) {
            if let Some(c) = doc
                .select(&sel)
                .next()
                .and_then(|el| el.value().attr("content"))
            {
                if c.starts_with("http") {
                    return Some(c.to_string());
                }
            }
        }
    }
    None
}
