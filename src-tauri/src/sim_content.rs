//! (05/11) Aeropuertos que MSFS 2024 trae por su cuenta: los de fábrica y los
//! de su tienda. Solo se leen nombres de carpeta. Muchas están casi vacías hasta
//! que el simulador las baja, así que una carpeta dice que el paquete existe,
//! y que falte no prueba que no exista. SimFleet nunca escribe en ellas.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::Serialize;
use sqlx::SqlitePool;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SimAirport {
    pub icao: String,
    /// `asobo`, `microsoft` o quien lo publique en la tienda.
    pub publisher: String,
    pub name: Option<String>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
}

/// `fs24-asobo-airport-cytz-toronto` → (`asobo`, `CYTZ`).
fn parse_folder(folder: &str) -> Option<(String, String)> {
    let lower = folder.to_ascii_lowercase();
    let mut parts = lower.split('-').peekable();
    let has_sim = parts
        .peek()
        .is_some_and(|p| p.len() == 4 && p.starts_with("fs") && p[2..].chars().all(|c| c.is_ascii_digit()));
    if has_sim {
        parts.next();
    }
    let publisher = parts.next()?;
    if parts.next()? != "airport" {
        return None;
    }
    let code = parts.next()?;
    let valid = (3..=4).contains(&code.len())
        && code.chars().all(|c| c.is_ascii_alphanumeric())
        && code.chars().any(|c| c.is_ascii_alphabetic());
    valid.then(|| (publisher.to_string(), code.to_ascii_uppercase()))
}

/// Carpetas de contenido junto a Community.
fn roots_beside(community: &Path) -> Vec<PathBuf> {
    let Some(base) = community.parent() else {
        return Vec::new();
    };
    vec![
        base.join("StreamedPackages"),
        base.join("Official2024").join("Steam"),
        base.join("Official2024").join("OneStore"),
    ]
}

/// ICAO → editor. Si un aeropuerto sale dos veces, se queda el primero.
fn scan(roots: &[PathBuf]) -> BTreeMap<String, String> {
    let mut found = BTreeMap::new();
    for root in roots {
        let Ok(entries) = std::fs::read_dir(root) else {
            continue;
        };
        for entry in entries.flatten() {
            if let Some((publisher, icao)) = entry.file_name().to_str().and_then(parse_folder) {
                found.entry(icao).or_insert(publisher);
            }
        }
    }
    found
}

async fn with_airport_data(pool: &SqlitePool, found: BTreeMap<String, String>) -> Result<Vec<SimAirport>> {
    let mut query = sqlx::QueryBuilder::new("SELECT icao, name, latitude, longitude FROM airports WHERE icao IN (");
    let mut codes = query.separated(", ");
    for icao in found.keys() {
        codes.push_bind(icao.clone());
    }
    codes.push_unseparated(")");
    let known: Vec<(String, String, f64, f64)> = query.build_query_as().fetch_all(pool).await?;
    let known: BTreeMap<String, (String, f64, f64)> =
        known.into_iter().map(|(icao, name, lat, lon)| (icao, (name, lat, lon))).collect();
    Ok(found
        .into_iter()
        .map(|(icao, publisher)| {
            let data = known.get(&icao);
            SimAirport {
                name: data.map(|d| d.0.clone()),
                latitude: data.map(|d| d.1),
                longitude: data.map(|d| d.2),
                icao,
                publisher,
            }
        })
        .collect())
}

/// Aeropuertos del simulador elegido. Solo MSFS 2024 guarda así su contenido.
pub async fn list(pool: &SqlitePool) -> Result<Vec<SimAirport>> {
    if !crate::sim::is_2024() {
        return Ok(Vec::new());
    }
    let Some(community) = crate::community::detect_community_folder()? else {
        return Ok(Vec::new());
    };
    let roots = roots_beside(Path::new(&community.path));
    let found = tokio::task::spawn_blocking(move || scan(&roots)).await?;
    with_airport_data(pool, found).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn airports_are_read_from_folder_names() {
        assert_eq!(
            parse_folder("fs24-asobo-airport-cytz-toronto"),
            Some(("asobo".into(), "CYTZ".into()))
        );
        assert_eq!(
            parse_folder("fs20-microsoft-airport-scvh"),
            Some(("microsoft".into(), "SCVH".into())),
            "sin nombre detrás"
        );
        assert_eq!(parse_folder("fs24-asobo-airport-c53-x"), Some(("asobo".into(), "C53".into())), "código corto");
        assert_eq!(
            parse_folder("acme-airport-klax-los-angeles"),
            Some(("acme".into(), "KLAX".into())),
            "de la tienda, sin prefijo de simulador"
        );
    }

    #[test]
    fn what_is_not_an_airport_is_skipped() {
        for folder in [
            "fs20-microsoft-airport-voloport",
            "fs24-asobo-modellib-airport-generic",
            "fs24-asobo-travelbook-airportspotting-europe",
            "fs24-asobo-passiveaircraft-a320",
            "fs-base-coverage-map-canada-tin",
            "fs24-asobo-airport-123",
            ".fscontent",
        ] {
            assert_eq!(parse_folder(folder), None, "{folder}");
        }
    }

    #[test]
    fn folders_beside_community_are_merged_and_missing_ones_ignored() {
        let tmp = tempfile::tempdir().unwrap();
        let community = tmp.path().join("Community");
        let streamed = tmp.path().join("StreamedPackages");
        let official = tmp.path().join("Official2024").join("Steam");
        std::fs::create_dir_all(streamed.join("fs24-asobo-airport-cytz-toronto")).unwrap();
        std::fs::create_dir_all(streamed.join("fs24-asobo-aircraft-a320")).unwrap();
        std::fs::create_dir_all(official.join("acme-airport-klax-los-angeles")).unwrap();
        std::fs::create_dir_all(official.join("fs24-microsoft-airport-cytz-otro")).unwrap();

        let found = scan(&roots_beside(&community));

        assert_eq!(found.len(), 2);
        assert_eq!(found["CYTZ"], "asobo", "StreamedPackages va primero");
        assert_eq!(found["KLAX"], "acme");
    }

    #[tokio::test]
    async fn unknown_codes_stay_but_without_a_place_on_the_map() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        sqlx::query("INSERT INTO airports (icao, name, latitude, longitude) VALUES ('CYTZ', 'Billy Bishop', 43.6, -79.4)")
            .execute(&pool)
            .await
            .unwrap();
        let found = BTreeMap::from([("CYTZ".to_string(), "asobo".to_string()), ("ZZZZ".to_string(), "asobo".to_string())]);

        let list = with_airport_data(&pool, found).await.unwrap();

        assert_eq!(list[0].name.as_deref(), Some("Billy Bishop"));
        assert_eq!(list[0].latitude, Some(43.6));
        assert_eq!(list[1].icao, "ZZZZ");
        assert_eq!(list[1].latitude, None);
    }
}
