//! Dataset de aeropuertos para la vista de mapa.
//!
//! Origen: OurAirports (CC0). El mirror estable de GitHub Pages es
//! `https://davidmegginson.github.io/ourairports-data/airports.csv`,
//! pesa ~7MB y trae ~80k filas. Filtramos por `type` (descartamos
//! heliports/closed/seaplane bases) y por ICAO de 4 letras ASCII.
//!
//! Estrategia de actualización:
//!   · La descarga ocurre en el primer arranque, en background, sin
//!     bloquear la UI.
//!   · TTL = 30 días. Si la fila `airports_dataset_fetched_at` en
//!     `settings` indica un fetch reciente, no hacemos nada.
//!   · Si la red falla, dejamos lo que haya (o nada en el primer
//!     arranque). El frontend muestra "cargando aeropuertos…" hasta
//!     que la tabla tenga >0 filas.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

const SOURCE_URL: &str =
    "https://davidmegginson.github.io/ourairports-data/airports.csv";

const SETTING_KEY: &str = "airports_dataset_fetched_at";

const TTL: Duration = Duration::from_secs(30 * 24 * 60 * 60);

/// Tipos de aeropuerto que nos interesa pintar en el mapa. El dataset
/// también contiene `heliport`, `seaplane_base`, `balloonport`,
/// `closed`; los descartamos para no llenar el mapa de chatarra.
const ACCEPTED_TYPES: &[&str] = &["large_airport", "medium_airport", "small_airport"];

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct AirportPoint {
    pub icao: String,
    pub name: String,
    pub latitude: f64,
    pub longitude: f64,
    pub iso_country: Option<String>,
    #[sqlx(rename = "type")]
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub municipality: Option<String>,
}

/// Una fila tal como llega del CSV. Sólo deserializamos lo que vamos
/// a guardar — cualquier columna nueva en upstream se ignora.
#[derive(Debug, Deserialize)]
struct RawAirport {
    #[serde(rename = "ident")]
    ident: String,
    #[serde(rename = "type")]
    kind: String,
    #[serde(rename = "name")]
    name: String,
    #[serde(rename = "latitude_deg")]
    latitude_deg: String,
    #[serde(rename = "longitude_deg")]
    longitude_deg: String,
    #[serde(default, rename = "iso_country")]
    iso_country: String,
    #[serde(default, rename = "municipality")]
    municipality: String,
}

/// Lanza la descarga del dataset si:
///   · La tabla está vacía, o
///   · El timestamp guardado supera el TTL.
///
/// Es seguro llamarla varias veces: si está al día, sale rápido.
/// Esta función sólo se debe invocar desde una tarea async normal —
/// **no** quemamos el thread principal con la descarga.
pub async fn ensure_dataset(pool: &SqlitePool, http: &reqwest::Client) -> anyhow::Result<()> {
    ensure_dataset_with_app(pool, http, None).await
}

/// Misma lógica que `ensure_dataset` pero acepta el `AppHandle`
/// para resolver el recurso bundleado `resources/airports.csv`.
/// Cuando `app` viene `None` o el recurso no existe, cae a la
/// descarga HTTP — sirve para builds de desarrollo donde el CSV
/// no está incluido.
///
/// Estrategia para que la app funcione "out of the box" en una
/// instalación nueva sin internet:
///   1. Si la tabla no necesita refresh, salir.
///   2. Si app+resource → parse CSV bundleado → persistir.
///   3. Si fallo → reintentar download HTTP.
///
/// Esto cumple el requisito del usuario: distribuir el setup.exe
/// con la base de datos integrada.
pub async fn ensure_dataset_with_app(
    pool: &SqlitePool,
    http: &reqwest::Client,
    app: Option<&tauri::AppHandle>,
) -> anyhow::Result<()> {
    if !needs_refresh(pool).await? {
        return Ok(());
    }

    // Intento 1: recurso bundleado (`resources/airports.csv`).
    if let Some(handle) = app {
        match try_load_bundled(pool, handle).await {
            Ok(true) => {
                tracing::info!(target: "airports", "dataset cargado desde recurso bundleado (offline-first OK)");
                return Ok(());
            }
            Ok(false) => {
                tracing::info!(target: "airports", "recurso bundleado no encontrado — fallback a download");
            }
            Err(e) => {
                tracing::warn!(target: "airports", "recurso bundleado falló: {e:#} — fallback a download");
            }
        }
    }

    // Intento 2: download HTTP.
    tracing::info!(target: "airports", "downloading dataset from {}", SOURCE_URL);
    let body = http
        .get(SOURCE_URL)
        .timeout(Duration::from_secs(60))
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;

    let parsed = parse_csv(body.as_ref())?;
    tracing::info!(target: "airports", "parsed {} usable rows from download", parsed.len());

    persist(pool, &parsed).await?;
    set_fetched_at_now(pool).await?;
    Ok(())
}

async fn try_load_bundled(
    pool: &SqlitePool,
    app: &tauri::AppHandle,
) -> anyhow::Result<bool> {
    use tauri::path::BaseDirectory;
    use tauri::Manager;

    let resource = match app
        .path()
        .resolve("resources/airports.csv", BaseDirectory::Resource)
    {
        Ok(p) => p,
        Err(e) => {
            tracing::debug!(target: "airports", "no se pudo resolver resource path: {e}");
            return Ok(false);
        }
    };
    if !resource.exists() {
        tracing::debug!(target: "airports", "resource airports.csv no existe en {}", resource.display());
        return Ok(false);
    }
    tracing::info!(target: "airports", "leyendo airports.csv bundleado: {}", resource.display());
    let bytes = tokio::task::spawn_blocking({
        let p = resource.clone();
        move || std::fs::read(p)
    })
    .await??;
    let parsed = parse_csv(&bytes)?;
    if parsed.is_empty() {
        // Placeholder de dev (sólo headers) — devolvemos `false` para
        // que el caller intente download HTTP. Sin esto, marcaríamos
        // como "ya cargado" un dataset vacío y la app quedaría sin
        // aeropuertos hasta el próximo TTL (30d).
        tracing::info!(
            target: "airports",
            "bundle vacío (placeholder dev) — fallback a download"
        );
        return Ok(false);
    }
    tracing::info!(target: "airports", "bundle parseado: {} filas usables", parsed.len());
    persist(pool, &parsed).await?;
    set_fetched_at_now(pool).await?;
    Ok(true)
}

async fn needs_refresh(pool: &SqlitePool) -> anyhow::Result<bool> {
    // Caso 1: tabla vacía → siempre refrescar.
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM airports")
        .fetch_one(pool)
        .await?;
    if count == 0 {
        return Ok(true);
    }

    // Caso 2: tenemos datos → mirar timestamp.
    let value: Option<String> =
        sqlx::query_scalar("SELECT value FROM settings WHERE key = ?1")
            .bind(SETTING_KEY)
            .fetch_optional(pool)
            .await?;
    let Some(ts) = value else {
        return Ok(true);
    };

    let parsed = chrono::NaiveDateTime::parse_from_str(ts.trim(), "%Y-%m-%d %H:%M:%S")
        .ok()
        .map(|dt| dt.and_utc());
    let Some(then) = parsed else {
        return Ok(true);
    };

    let now = chrono::Utc::now();
    let age = now.signed_duration_since(then).to_std().unwrap_or_default();
    Ok(age >= TTL)
}

async fn set_fetched_at_now(pool: &SqlitePool) -> anyhow::Result<()> {
    sqlx::query(
        r#"
        INSERT INTO settings (key, value, updated_at)
        VALUES (?1, datetime('now'), datetime('now'))
        ON CONFLICT(key) DO UPDATE SET
            value = excluded.value,
            updated_at = excluded.updated_at
        "#,
    )
    .bind(SETTING_KEY)
    .execute(pool)
    .await?;
    Ok(())
}

fn parse_csv(bytes: &[u8]) -> anyhow::Result<Vec<AirportPoint>> {
    let mut rdr = csv::ReaderBuilder::new()
        .has_headers(true)
        .flexible(true)
        .from_reader(bytes);
    let mut out = Vec::with_capacity(40_000);
    for record in rdr.deserialize::<RawAirport>() {
        let raw = match record {
            Ok(r) => r,
            Err(e) => {
                tracing::debug!("airports: skipping malformed row: {}", e);
                continue;
            }
        };
        if !ACCEPTED_TYPES.contains(&raw.kind.as_str()) {
            continue;
        }
        let icao = raw.ident.trim().to_ascii_uppercase();
        if icao.len() != 4 || !icao.bytes().all(|b| b.is_ascii_alphanumeric()) {
            continue;
        }
        let lat = raw.latitude_deg.trim().parse::<f64>().ok();
        let lon = raw.longitude_deg.trim().parse::<f64>().ok();
        let (Some(latitude), Some(longitude)) = (lat, lon) else {
            continue;
        };
        if !(latitude.is_finite() && longitude.is_finite()) {
            continue;
        }
        out.push(AirportPoint {
            icao,
            name: raw.name.trim().to_string(),
            latitude,
            longitude,
            iso_country: opt_string(raw.iso_country),
            kind: Some(raw.kind),
            municipality: opt_string(raw.municipality),
        });
    }
    Ok(out)
}

fn opt_string(s: String) -> Option<String> {
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

/// Reemplaza el dataset entero. Más simple que diff incremental y
/// suficiente para una tabla de ~30k filas — la transacción tarda
/// menos de un segundo en SSD modesta.
async fn persist(pool: &SqlitePool, rows: &[AirportPoint]) -> anyhow::Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM airports").execute(&mut *tx).await?;

    // Bulk insert manual: chunks de 500 filas usando placeholders
    // expandidos. Probamos `execute_many` previamente pero genera un
    // volumen de roundtrips peor que un único INSERT por chunk.
    const CHUNK: usize = 500;
    for chunk in rows.chunks(CHUNK) {
        let mut sql = String::from(
            "INSERT INTO airports (icao, name, latitude, longitude, iso_country, type, municipality) VALUES ",
        );
        let mut first = true;
        for _ in chunk {
            if !first {
                sql.push(',');
            }
            sql.push_str("(?, ?, ?, ?, ?, ?, ?)");
            first = false;
        }
        let mut q = sqlx::query(&sql);
        for r in chunk {
            q = q
                .bind(&r.icao)
                .bind(&r.name)
                .bind(r.latitude)
                .bind(r.longitude)
                .bind(&r.iso_country)
                .bind(&r.kind)
                .bind(&r.municipality);
        }
        q.execute(&mut *tx).await?;
    }

    tx.commit().await?;
    Ok(())
}

/// Punto en el mapa para un addon **instalado** en el Community
/// folder. La fuente de verdad es la tabla `installed_addons` —
/// no el catálogo de búsquedas (`addons`), que se llena con cada
/// query del usuario y mostraba en el mapa cosas que sólo había
/// hojeado, no instalado.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AddonOnMap {
    pub addon_id: String,
    pub source: String,
    pub title: String,
    pub icao: String,
    pub airport_name: String,
    pub latitude: f64,
    pub longitude: f64,
}

#[derive(Debug, sqlx::FromRow)]
struct InstalledRow {
    id: String,
    title: String,
    source: Option<String>,
    addon_icao: Option<String>,
}

/// Devuelve un punto por instalación con ICAO resolvible. Reglas:
///
///   1. Si la instalación nació de un torrent del catálogo, usamos
///      el ICAO ya extraído por el parser (`addons.icao`).
///   2. Si es manual (sin `addon_id`), hacemos un mejor-esfuerzo:
///      buscamos un patrón `\b[A-Z]{4}\b` en el título y lo
///      validamos contra la tabla `airports`. Sin coincidencia →
///      la instalación no aparece en el mapa.
///
/// El INNER-JOIN implícito con `airports` es la red de seguridad
/// final: cualquier ICAO que no exista en el dataset de OurAirports
/// (typos, falsos positivos como "MSFS" o "PACK") se descarta.
pub async fn list_addons_on_map(pool: &SqlitePool) -> anyhow::Result<Vec<AddonOnMap>> {
    // Paso 1 — cargar el dataset de aeropuertos en memoria. ~30k
    // entradas con (icao → name/lat/lon) ocupan <2MB y nos ahorran
    // un round-trip por instalación.
    let airport_rows: Vec<(String, String, f64, f64)> = sqlx::query_as::<_, (String, String, f64, f64)>(
        "SELECT icao, name, latitude, longitude FROM airports",
    )
    .fetch_all(pool)
    .await?;
    let airports: std::collections::HashMap<String, (String, f64, f64)> = airport_rows
        .into_iter()
        .map(|(icao, name, lat, lon)| (icao, (name, lat, lon)))
        .collect();

    if airports.is_empty() {
        // Aún no se ha sincronizado el dataset → devolver vacío.
        return Ok(Vec::new());
    }

    // Paso 2 — instalaciones con ICAO opcional desde el catálogo.
    let installed = sqlx::query_as::<_, InstalledRow>(
        r#"
        SELECT ia.id           AS id,
               ia.title        AS title,
               ia.source       AS source,
               a.icao          AS addon_icao
        FROM installed_addons ia
        LEFT JOIN addons a ON a.id = ia.addon_id
        ORDER BY ia.installed_at DESC
        "#,
    )
    .fetch_all(pool)
    .await?;

    // Paso 3 — resolver ICAO + airport para cada uno.
    let mut out = Vec::with_capacity(installed.len());
    for row in installed {
        let icao_candidate = row
            .addon_icao
            .as_deref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_ascii_uppercase())
            .or_else(|| extract_icao_from_title(&row.title));

        let Some(icao) = icao_candidate else { continue };
        let Some((airport_name, lat, lon)) = airports.get(&icao).cloned() else {
            continue;
        };
        out.push(AddonOnMap {
            addon_id: row.id,
            source: row.source.unwrap_or_default(),
            title: row.title,
            icao,
            airport_name,
            latitude: lat,
            longitude: lon,
        });
    }
    Ok(out)
}

/// Busca un código ICAO (4 letras ASCII consecutivas) en `title`
/// con frontera de palabra. No filtra por aeropuerto válido — eso
/// lo hace el caller via la tabla `airports`. Devuelve la primera
/// coincidencia, en mayúsculas.
///
/// Ejemplos:
///   · `"BravoAirspace MDSD v1.6"` → `Some("MDSD")`
///   · `"FlyByWire Sound Pack"`    → `Some("PACK")` (descartado por airports)
///   · `"A320 Livery Premium"`     → `None` (no hay 4-letras consecutivas)
fn extract_icao_from_title(title: &str) -> Option<String> {
    let upper = title.to_ascii_uppercase();
    let bytes = upper.as_bytes();
    if bytes.len() < 4 {
        return None;
    }
    // (v1.1.4) Misma heurística que `community_scanner::extract_icao`:
    // prioriza candidatos precedidos por "AIRPORT-" para que folders
    // tipo `kaze-airport-mhtg-toncontin` elijan MHTG en vez de KAZE.
    let mut candidates: Vec<(usize, String)> = Vec::new();
    for i in 0..=bytes.len() - 4 {
        let candidate = &bytes[i..i + 4];
        if !candidate.iter().all(|b| b.is_ascii_alphabetic()) {
            continue;
        }
        let before_ok = i == 0 || !bytes[i - 1].is_ascii_alphanumeric();
        let after_ok = i + 4 == bytes.len() || !bytes[i + 4].is_ascii_alphanumeric();
        if before_ok && after_ok {
            if let Ok(s) = String::from_utf8(candidate.to_vec()) {
                candidates.push((i, s));
            }
        }
    }
    if candidates.is_empty() {
        return None;
    }
    if candidates.len() == 1 {
        return Some(candidates.into_iter().next().unwrap().1);
    }
    for (i, cand) in &candidates {
        if *i < 8 {
            continue;
        }
        let prefix = &bytes[*i - 8..*i];
        if prefix.starts_with(b"AIRPORT")
            && matches!(prefix[7], b'-' | b'_' | b' ')
        {
            return Some(cand.clone());
        }
    }
    Some(candidates.into_iter().next().unwrap().1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icao_extraction_matches_expected() {
        assert_eq!(
            extract_icao_from_title("BravoAirspace MDSD v1.6"),
            Some("MDSD".to_string())
        );
        assert_eq!(
            extract_icao_from_title("KJFK New York John F. Kennedy"),
            Some("KJFK".to_string())
        );
        // 5+ letras consecutivas → no es ICAO de 4
        assert_eq!(extract_icao_from_title("MDSDX Airport"), None);
        // Sin 4-letras consecutivas
        assert_eq!(extract_icao_from_title("A320 Livery Premium"), None);
        // Case-insensitive
        assert_eq!(extract_icao_from_title("mdsd lower"), Some("MDSD".to_string()));
    }
}
