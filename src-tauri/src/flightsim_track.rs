//! (v6.2.60) Tracking de updates de addons descargados de flightsim.to.
//!
//! Forward-only y FIABLE: al instalar por el navegador EMBEBIDO registramos el
//! `file_id` de flightsim.to del que salió cada carpeta de Community + su
//! `updatedAt` de ese momento (baseline). El chequeo compara el `updatedAt`
//! ACTUAL del catálogo contra el baseline — si avanzó, hay una versión nueva de
//! verdad. Nada de adivinar por nombre (eso daba falsos positivos como en GSX).
//!
//! API de flightsim.to usada (pública, sólo requiere User-Agent de navegador):
//!   `GET /backend/files/detail?id=<id>` → objeto plano con `title`, `version`,
//!   `updatedAt` (ISO-8601) y `link` (`https://flightsim.to/addon/<id>/<slug>`).

use serde::Deserialize;
use sqlx::SqlitePool;

/// User-Agent de navegador — la API responde 403 a UAs por defecto de reqwest.
const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
    (KHTML, like Gecko) Chrome/120.0 Safari/537.36";
const DETAIL: &str = "https://flightsim.to/backend/files/detail";

#[derive(Debug, Clone, Deserialize)]
struct RawDetail {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    version: Option<String>,
    #[serde(default, rename = "updatedAt")]
    updated_at: Option<String>,
    #[serde(default)]
    link: Option<String>,
    /// (v6.2.74) Compatibilidad de sim reportada por flightsim.to.
    #[serde(default)]
    msfs2020: Option<bool>,
    #[serde(default)]
    msfs2024: Option<bool>,
    /// (v6.2.75) Galería de imágenes del addon.
    #[serde(default)]
    images: Vec<RawImage>,
}

#[derive(Debug, Clone, Deserialize)]
struct RawImage {
    #[serde(default)]
    url: Option<String>,
    #[serde(default, rename = "isMain")]
    is_main: Option<bool>,
}

#[derive(Debug, Clone)]
struct FileMeta {
    title: Option<String>,
    version: Option<String>,
    updated_at: Option<String>,
    link: Option<String>,
    msfs2020: Option<bool>,
    msfs2024: Option<bool>,
    /// (v6.2.75) URL de la imagen principal (con `?width` para tamaño razonable).
    thumbnail: Option<String>,
}

/// (v6.2.74) Metadata de compatibilidad de un archivo de flightsim.to, para
/// decidir en qué Community (2020/2024) instalar lo que se descargó.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FlightsimMeta {
    pub file_id: String,
    pub title: Option<String>,
    /// true/false si flightsim.to lo declara; None si no reporta el flag.
    pub msfs2020: Option<bool>,
    pub msfs2024: Option<bool>,
    pub link: String,
}

/// Update disponible de un addon rastreado (uno por carpeta de Community).
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FlightsimUpdate {
    pub folder_name: String,
    pub file_id: String,
    pub title: Option<String>,
    pub has_update: bool,
    /// Versión instalada (baseline).
    pub current_version: Option<String>,
    /// Versión actual publicada en flightsim.to.
    pub latest_version: Option<String>,
    /// URL de la página del addon (para abrir en el embebido y re-descargar).
    pub link: String,
    /// (v6.2.75) Imagen del addon en flightsim.to (para la card/modal en Addons).
    pub thumbnail: Option<String>,
}

/// Extrae el `file_id` numérico de una URL de flightsim.to
/// (`…/file/<id>/<slug>` o `…/addon/<id>/<slug>`). None si no es una página de
/// archivo de flightsim.to.
pub fn file_id_from_url(url: &str) -> Option<String> {
    let parsed = tauri::Url::parse(url).ok()?;
    if !parsed.host_str().unwrap_or("").contains("flightsim.to") {
        return None;
    }
    let mut segs = parsed.path_segments()?;
    while let Some(s) = segs.next() {
        if s == "file" || s == "addon" {
            if let Some(id) = segs.next() {
                if !id.is_empty() && id.chars().all(|c| c.is_ascii_digit()) {
                    return Some(id.to_string());
                }
            }
        }
    }
    None
}

/// `GET /backend/files/detail?id=<id>` → metadata actual (versión + updatedAt).
async fn fetch_detail(http: &reqwest::Client, file_id: &str) -> Option<FileMeta> {
    let resp = http
        .get(DETAIL)
        .query(&[("id", file_id)])
        .header("User-Agent", UA)
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let raw: RawDetail = resp.json().await.ok()?;
    let clean = |o: Option<String>| o.filter(|s| !s.trim().is_empty());
    // Imagen principal (isMain) o la primera; añadimos `?width=800` para no
    // bajar el PNG a resolución completa (pueden ser varios MB).
    let thumbnail = raw
        .images
        .iter()
        .find(|i| i.is_main == Some(true))
        .or_else(|| raw.images.first())
        .and_then(|i| i.url.clone())
        .filter(|u| !u.trim().is_empty())
        .map(|u| if u.contains('?') { u } else { format!("{u}?width=800") });
    Some(FileMeta {
        title: clean(raw.title),
        version: clean(raw.version),
        updated_at: clean(raw.updated_at),
        link: clean(raw.link),
        msfs2020: raw.msfs2020,
        msfs2024: raw.msfs2024,
        thumbnail,
    })
}

/// (v6.2.74) Metadata de compatibilidad de un `file_id` (para el routing 2020/
/// 2024 al instalar lo descargado del embebido).
pub async fn fetch_meta(http: &reqwest::Client, file_id: &str) -> Option<FlightsimMeta> {
    let m = fetch_detail(http, file_id).await?;
    Some(FlightsimMeta {
        file_id: file_id.to_string(),
        title: m.title,
        msfs2020: m.msfs2020,
        msfs2024: m.msfs2024,
        link: m
            .link
            .unwrap_or_else(|| format!("https://flightsim.to/addon/{file_id}")),
    })
}

/// Registra el baseline de un addon recién instalado desde el embebido: por
/// cada carpeta de Community creada, guarda `file_id` + versión + `updatedAt`
/// actuales. Best-effort: si no logramos leer el `detail`, no registramos (no
/// queremos filas sin baseline que luego den falsos positivos).
pub async fn record_tracked(
    db: &SqlitePool,
    http: &reqwest::Client,
    file_id: &str,
    folders: &[String],
) {
    if folders.is_empty() {
        return;
    }
    let Some(meta) = fetch_detail(http, file_id).await else {
        tracing::warn!(
            target: "flightsim",
            "detail no disponible para file_id={} — no registro tracking",
            file_id
        );
        return;
    };
    for folder in folders {
        let res = sqlx::query(
            r#"
            INSERT INTO flightsim_tracked_files
                (folder_name, file_id, title, known_version, known_updated, link, thumbnail, tracked_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, datetime('now'))
            ON CONFLICT(folder_name) DO UPDATE SET
                file_id       = excluded.file_id,
                title         = excluded.title,
                known_version = excluded.known_version,
                known_updated = excluded.known_updated,
                link          = excluded.link,
                thumbnail     = excluded.thumbnail,
                tracked_at    = excluded.tracked_at
            "#,
        )
        .bind(folder)
        .bind(file_id)
        .bind(&meta.title)
        .bind(&meta.version)
        .bind(&meta.updated_at)
        .bind(&meta.link)
        .bind(&meta.thumbnail)
        .execute(db)
        .await;
        match res {
            Ok(_) => tracing::info!(
                target: "flightsim",
                "tracking registrado: {} → file_id={} v={:?}",
                folder, file_id, meta.version
            ),
            Err(e) => tracing::warn!(
                target: "flightsim",
                "no pude registrar tracking de {}: {}",
                folder, e
            ),
        }
    }
}

/// Revisa updates de todos los addons rastreados. Por cada `file_id` consulta
/// UNA vez el `detail` actual y marca update si su `updatedAt` avanzó respecto
/// al baseline. Devuelve una entrada por carpeta rastreada (el front filtra por
/// `has_update` y por carpetas realmente instaladas).
pub async fn check_updates(db: &SqlitePool, http: &reqwest::Client) -> Vec<FlightsimUpdate> {
    let rows: Vec<(
        String,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    )> = sqlx::query_as(
        "SELECT folder_name, file_id, title, known_version, known_updated, link, thumbnail \
             FROM flightsim_tracked_files",
    )
    .fetch_all(db)
    .await
    .unwrap_or_default();

    // Cache por file_id: varias carpetas pueden venir del mismo archivo.
    let mut cache: std::collections::HashMap<String, Option<FileMeta>> =
        std::collections::HashMap::new();
    let mut out = Vec::with_capacity(rows.len());
    for (folder, file_id, title, known_version, known_updated, link, stored_thumb) in rows {
        let meta = match cache.get(&file_id) {
            Some(m) => m.clone(),
            None => {
                let m = fetch_detail(http, &file_id).await;
                cache.insert(file_id.clone(), m.clone());
                m
            }
        };
        let latest_version = meta.as_ref().and_then(|m| m.version.clone());
        let latest_updated = meta.as_ref().and_then(|m| m.updated_at.clone());
        let latest_link = meta.as_ref().and_then(|m| m.link.clone());
        // Update genuino: el catálogo avanzó su `updatedAt` respecto al baseline.
        let has_update = match (
            parse_iso(known_updated.as_deref()),
            parse_iso(latest_updated.as_deref()),
        ) {
            (Some(base), Some(cur)) => cur > base,
            _ => false,
        };
        let link = latest_link
            .or(link)
            .unwrap_or_else(|| format!("https://flightsim.to/addon/{file_id}"));
        // (v6.2.75) Thumbnail: el guardado, o el del detail (backfill de filas
        // viejas grabadas antes de la columna).
        let thumbnail =
            stored_thumb.or_else(|| meta.as_ref().and_then(|m| m.thumbnail.clone()));
        let _ = sqlx::query(
            "UPDATE flightsim_tracked_files SET last_checked_at = datetime('now'), \
             thumbnail = COALESCE(thumbnail, ?2) WHERE folder_name = ?1",
        )
        .bind(&folder)
        .bind(&thumbnail)
        .execute(db)
        .await;
        out.push(FlightsimUpdate {
            folder_name: folder,
            file_id,
            title,
            has_update,
            current_version: known_version,
            latest_version,
            link,
            thumbnail,
        });
    }
    out
}

/// Marca un addon como manejado: avanza su baseline al `updatedAt` actual para
/// que el badge no reaparezca tras pulsarlo (el clic abre flightsim.to).
pub async fn ack_update(db: &SqlitePool, http: &reqwest::Client, folder_name: &str) {
    let file_id: Option<String> =
        sqlx::query_scalar("SELECT file_id FROM flightsim_tracked_files WHERE folder_name = ?1")
            .bind(folder_name)
            .fetch_optional(db)
            .await
            .ok()
            .flatten();
    let Some(file_id) = file_id else { return };
    if let Some(meta) = fetch_detail(http, &file_id).await {
        let _ = sqlx::query(
            "UPDATE flightsim_tracked_files \
             SET known_version = ?2, known_updated = ?3, last_checked_at = datetime('now') \
             WHERE folder_name = ?1",
        )
        .bind(folder_name)
        .bind(&meta.version)
        .bind(&meta.updated_at)
        .execute(db)
        .await;
    }
}

/// ISO-8601 (`2026-07-26T21:28:01Z`) → epoch secs.
fn parse_iso(s: Option<&str>) -> Option<i64> {
    let s = s?;
    chrono::DateTime::parse_from_rfc3339(s.trim())
        .ok()
        .map(|dt| dt.timestamp())
}
