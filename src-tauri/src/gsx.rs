//! Lookup de perfiles GSX Pro en flightsim.to.
//!
//! El sitio es una SPA detrás de Cloudflare; el HTML de
//! `https://flightsim.to/miscellaneous/gsx-pro` es un shell vacío que
//! hidrata su contenido vía JS. Examinando el bundle (`/assets/index-*.js`)
//! encontramos el endpoint que usa el propio frontend:
//!
//! ```text
//! GET https://flightsim.to/backend/files/filter?cat=miscellaneous&sub_cat=gsx-pro&limit=N
//! ```
//!
//! El endpoint **no admite filtrar por ICAO**, así que descargamos un
//! batch grande (≈300 perfiles) una sola vez al día y hacemos el match
//! por ICAO en cliente buscando la cadena en el título del perfil. Las
//! consultas resultantes se cachean en SQLite (tabla `gsx_lookups`,
//! migración 003) con un TTL de 24h — los perfiles no se actualizan tan
//! seguido como para justificar machacar el origen en cada búsqueda.
//!
//! El scraper completo del catálogo (la lista de N=300) se mantiene
//! aparte de la caché por ICAO: lo refrescamos en una `OnceCell` en
//! memoria. La razón: si el usuario busca 30 ICAOs distintos en un
//! minuto, queremos hacer **un** GET a flightsim.to, no 30. La
//! propagación a la fila por ICAO en SQLite es para sobrevivir a
//! reinicios de la app.

use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tokio::time::Instant;

use crate::db::repo;

/// URL base de la API interna de flightsim.to. El endpoint completo
/// se compone añadiendo `/filter?cat=…&sub_cat=…&limit=…`.
const BACKEND: &str = "https://flightsim.to/backend/files";

/// User-Agent realista. El servidor responde 403 a UAs por defecto de
/// `reqwest` (`reqwest/x.y`). Cualquier UA de navegador moderno vale.
const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0 Safari/537.36";

/// Cuántos perfiles pedir por barrido. flightsim.to tiene del orden de
/// ~250 perfiles GSX activos al momento de escribir esto (abril 2026);
/// 400 da margen sin pedirle gigantes a su backend.
const CATALOG_LIMIT: usize = 400;

/// TTL de las filas en `gsx_lookups`. Después de esto re-consultamos.
/// Es deliberadamente generoso: el catálogo apenas cambia día a día.
const CACHE_TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// TTL del catálogo en memoria — más corto que el de SQLite porque el
/// proceso vive menos tiempo y queremos que un reinicio de dev no
/// cargue datos rancios.
const MEMORY_CATALOG_TTL: Duration = Duration::from_secs(60 * 60);

/// Forma serializable del perfil que consume el frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GsxProfile {
    pub id: i64,
    pub title: String,
    pub link: String,
    pub thumbnail: Option<String>,
    pub author_name: Option<String>,
    pub simulator: Option<String>,
}

// ---- Tipos de la respuesta cruda de flightsim.to ----------------------------
//
// Sólo deserializamos los campos que necesitamos. El payload real tiene
// muchos más (downloads, rating, tags…); ignorarlos con `serde(default)`
// nos protege contra cambios futuros del schema.

#[derive(Debug, Deserialize)]
struct RawListResponse {
    #[serde(default)]
    data: Vec<RawProfile>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawProfile {
    id: i64,
    title: String,
    link: Option<String>,
    thumbnail: Option<String>,
    simulator: Option<String>,
    #[serde(default)]
    author: Option<RawAuthor>,
}

#[derive(Debug, Deserialize)]
struct RawAuthor {
    #[serde(default)]
    name: Option<String>,
    #[serde(default, rename = "displayName")]
    display_name: Option<String>,
}

impl From<RawProfile> for GsxProfile {
    fn from(r: RawProfile) -> Self {
        let author_name = r
            .author
            .and_then(|a| a.display_name.or(a.name))
            .filter(|s| !s.trim().is_empty());
        Self {
            id: r.id,
            title: r.title,
            link: r.link.unwrap_or_default(),
            thumbnail: r.thumbnail,
            author_name,
            simulator: r.simulator,
        }
    }
}

// ---- Cliente con catálogo cacheado en memoria -------------------------------

/// Una entrada del cache en memoria con su timestamp. La idea es no
/// pegarle a flightsim.to N veces si el usuario busca varios ICAOs
/// seguidos: cargamos el catálogo entero una vez y hacemos el match
/// localmente.
struct MemoryCatalog {
    profiles: Vec<GsxProfile>,
    fetched_at: Instant,
}

#[derive(Clone)]
pub struct GsxClient {
    inner: Arc<Inner>,
}

struct Inner {
    http: reqwest::Client,
    catalog: Mutex<Option<MemoryCatalog>>,
}

impl GsxClient {
    pub fn new(http: reqwest::Client) -> Self {
        Self {
            inner: Arc::new(Inner {
                http,
                catalog: Mutex::new(None),
            }),
        }
    }

    /// Devuelve el catálogo entero, recargándolo si está vencido. Esto
    /// es lo único que llega a la red — el filtrado por ICAO se hace
    /// sobre la lista en memoria.
    async fn catalog(&self) -> anyhow::Result<Vec<GsxProfile>> {
        let mut guard = self.inner.catalog.lock().await;
        if let Some(cat) = guard.as_ref() {
            if cat.fetched_at.elapsed() < MEMORY_CATALOG_TTL {
                return Ok(cat.profiles.clone());
            }
        }
        let fresh = self.fetch_catalog().await?;
        *guard = Some(MemoryCatalog {
            profiles: fresh.clone(),
            fetched_at: Instant::now(),
        });
        Ok(fresh)
    }

    async fn fetch_catalog(&self) -> anyhow::Result<Vec<GsxProfile>> {
        let url = format!(
            "{}/filter?cat=miscellaneous&sub_cat=gsx-pro&limit={}",
            BACKEND, CATALOG_LIMIT
        );
        tracing::info!("gsx: fetching catalog from {}", url);

        let resp = self
            .inner
            .http
            .get(&url)
            // Cabeceras *exactamente* como las pone el frontend de
            // flightsim.to. Sin Referer el backend devuelve 403; sin
            // Accept JSON se cae a HTML.
            .header("Accept", "application/json")
            .header("Referer", "https://flightsim.to/miscellaneous/gsx-pro")
            .header("User-Agent", UA)
            .send()
            .await?
            .error_for_status()?;

        let raw: RawListResponse = resp.json().await?;
        let profiles: Vec<GsxProfile> = raw.data.into_iter().map(GsxProfile::from).collect();
        tracing::info!("gsx: catalog fetched ({} profiles)", profiles.len());
        Ok(profiles)
    }

    /// Filtra el catálogo por ICAO. Empareja con frontera de palabra
    /// (no es regex de Unicode pero alcanza para ICAO 4-letras
    /// ASCII): el título tiene que contener el ICAO rodeado de
    /// caracteres no-alfanuméricos o estar al principio/fin. Eso
    /// evita el clásico falso positivo: ICAO `LFPG` no debe
    /// machear `LFPGX`.
    fn match_by_icao<'a>(
        profiles: &'a [GsxProfile],
        icao: &str,
    ) -> Vec<&'a GsxProfile> {
        let needle = icao.trim().to_ascii_uppercase();
        if needle.is_empty() {
            return Vec::new();
        }
        profiles
            .iter()
            .filter(|p| title_has_icao(&p.title, &needle))
            .collect()
    }
}

/// `title.contains(needle)` no nos sirve porque queremos frontera de
/// palabra. Usamos un escaneo manual que es más rápido que armar un
/// `Regex` por cada candidato del catálogo (~400 elementos).
fn title_has_icao(title: &str, icao: &str) -> bool {
    let upper = title.to_ascii_uppercase();
    let bytes = upper.as_bytes();
    let needle = icao.as_bytes();
    if needle.is_empty() || bytes.len() < needle.len() {
        return false;
    }
    let max = bytes.len() - needle.len();
    for i in 0..=max {
        if &bytes[i..i + needle.len()] != needle {
            continue;
        }
        let before_ok = i == 0 || !is_word_char(bytes[i - 1]);
        let after_idx = i + needle.len();
        let after_ok = after_idx == bytes.len() || !is_word_char(bytes[after_idx]);
        if before_ok && after_ok {
            return true;
        }
    }
    false
}

#[inline]
fn is_word_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Resuelve la lista de perfiles para un ICAO, usando primero la
/// caché de SQLite y refrescando contra flightsim.to si está vencida.
///
/// La caché por ICAO evita repetir el filtrado cuando una misma
/// búsqueda toca los mismos ICAOs varias veces — pero la fuente de
/// verdad real es el catálogo en memoria de `GsxClient`.
pub async fn lookup_with_cache(
    client: &GsxClient,
    pool: &sqlx::SqlitePool,
    icao: &str,
) -> anyhow::Result<Vec<GsxProfile>> {
    let key = icao.trim().to_ascii_uppercase();
    if key.is_empty() {
        return Ok(Vec::new());
    }

    // Hit de caché válido → devolver tal cual.
    if let Some(row) = repo::get_gsx_lookup(pool, &key).await? {
        if let Some(age) = parse_sqlite_datetime(&row.fetched_at) {
            if age < CACHE_TTL {
                if let Ok(parsed) = serde_json::from_str::<Vec<GsxProfile>>(&row.profiles_json) {
                    tracing::debug!("gsx: cache hit for {} ({} profiles)", key, parsed.len());
                    return Ok(parsed);
                }
            }
        }
    }

    // Miss o vencida → recargar.
    let catalog = client.catalog().await?;
    let matches: Vec<GsxProfile> = GsxClient::match_by_icao(&catalog, &key)
        .into_iter()
        .cloned()
        .collect();
    tracing::info!(
        "gsx: lookup({}) → {} match(es) en {} perfiles del catálogo",
        key,
        matches.len(),
        catalog.len()
    );

    let payload = serde_json::to_string(&matches)?;
    if let Err(e) = repo::set_gsx_lookup(pool, &key, &payload).await {
        // Si la caché falla seguimos devolviendo el resultado en caliente.
        tracing::warn!("gsx: no se pudo escribir caché para {}: {e:#}", key);
    }
    Ok(matches)
}

/// SQLite formato `YYYY-MM-DD HH:MM:SS` (UTC). Devolvemos cuánto tiempo
/// pasó desde entonces hasta ahora; `None` si no parsea.
fn parse_sqlite_datetime(s: &str) -> Option<Duration> {
    let dt = chrono::NaiveDateTime::parse_from_str(s.trim(), "%Y-%m-%d %H:%M:%S").ok()?;
    let dt = dt.and_utc();
    let now = chrono::Utc::now();
    let elapsed = now.signed_duration_since(dt).to_std().ok()?;
    Some(elapsed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_match_word_boundary() {
        // Caso típico: "GSX Profile - Asobo LFPG Paris-Charles de Gaulle"
        assert!(title_has_icao("GSX Profile - Asobo LFPG Paris", "LFPG"));
        // Falso positivo a evitar: LFPGX no debe machear LFPG
        assert!(!title_has_icao("Profile LFPGX something", "LFPG"));
        // Al principio y al final del título
        assert!(title_has_icao("LFPG everything", "LFPG"));
        assert!(title_has_icao("airport LFPG", "LFPG"));
        // Case-insensitive
        assert!(title_has_icao("airport lfpg", "LFPG"));
        // No-match
        assert!(!title_has_icao("EGLL Heathrow", "LFPG"));
        // Vacío
        assert!(!title_has_icao("anything", ""));
    }
}
