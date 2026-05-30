//! Estadísticas agregadas para la vista «Dashboard».
//!
//! Toda la data sale de `community_packages` (poblada por el scanner)
//! y de `compute_available` (cache de updates). No tocamos disco — el
//! `size_bytes` ya viene del scan, así que el comando es barato y se
//! puede invocar en cada apertura del tab sin penalty.
//!
//! La categorización de tipos la hacemos en Rust (no SQL) porque
//! distinguir AIRCRAFT-base de AIRCRAFT-livery requiere mirar
//! `dependencies_count`, y combinarlo con `content_type` resulta más
//! claro como código procedural que como CASE anidados.

use serde::Serialize;
use sqlx::SqlitePool;

use crate::updates;
use crate::AppState;

/// Estadística por categoría: cuántos paquetes y cuánto pesan.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TypeStat {
    pub label: String,
    pub count: i64,
    pub size_bytes: i64,
}

/// Top creator por número de paquetes (y su peso total).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatorStat {
    pub creator: String,
    pub count: i64,
    pub size_bytes: i64,
}

/// Paquete individual — usado para «top N más grandes» y «más
/// recientes». La info es la mínima para pintar una fila en la UI.
#[derive(Debug, Clone, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct PackageRef {
    pub folder_name: String,
    pub title: String,
    pub creator: Option<String>,
    pub size_bytes: Option<i64>,
    pub content_type: Option<String>,
}

/// Bundle que devuelve el comando — todo lo que pinta el dashboard.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardStats {
    pub total_packages: i64,
    pub total_size_bytes: i64,
    pub updates_available: i64,
    pub by_type: Vec<TypeStat>,
    pub top_creators: Vec<CreatorStat>,
    pub largest_packages: Vec<PackageRef>,
    pub recently_added: Vec<PackageRef>,
    pub airports_count: i64,
    pub liveries_count: i64,
    pub aircraft_count: i64,
}

#[tauri::command]
pub async fn get_dashboard_stats(
    state: tauri::State<'_, AppState>,
) -> Result<DashboardStats, String> {
    compute_stats(&state.db).await.map_err(|e| e.to_string())
}

async fn compute_stats(pool: &SqlitePool) -> anyhow::Result<DashboardStats> {
    // Totales globales — un solo round-trip.
    let (total_packages, total_size_bytes): (i64, Option<i64>) = sqlx::query_as(
        "SELECT COUNT(*), SUM(COALESCE(size_bytes, 0)) FROM community_packages",
    )
    .fetch_one(pool)
    .await?;
    let total_size_bytes = total_size_bytes.unwrap_or(0);

    // Filas crudas para categorización: necesitamos content_type,
    // dependencies_count, icao y size para clasificar.
    // (v3.1.0) JOIN con airports para que `icao_resolvable` sólo sea
    // true cuando el ICAO realmente existe en el dataset de OurAirports.
    // Eso descarta librerías y "packs de aeropuerto" cuyo ICAO no es
    // un aeropuerto real (típicos: librerías de objetos, packs de
    // texturas con título sugestivo, etc.).
    #[derive(sqlx::FromRow)]
    struct RawRow {
        content_type: Option<String>,
        dependencies_count: i64,
        icao: Option<String>,
        size_bytes: Option<i64>,
        icao_resolvable: bool,
    }
    let raws: Vec<RawRow> = sqlx::query_as(
        r#"
        SELECT cp.content_type,
               cp.dependencies_count,
               cp.icao,
               cp.size_bytes,
               (a.icao IS NOT NULL) AS icao_resolvable
        FROM community_packages cp
        LEFT JOIN airports a ON a.icao = UPPER(cp.icao)
        "#,
    )
    .fetch_all(pool)
    .await?;

    // Buckets — usamos `Vec<(label, count, bytes)>` para preservar
    // orden de aparición. Pasamos a Vec<TypeStat> al final.
    let mut buckets: Vec<(&'static str, i64, i64)> = vec![
        ("Aeropuertos", 0, 0),
        ("Aviones", 0, 0),
        ("Liveries", 0, 0),
        ("Instrumentos", 0, 0),
        ("Sonido / Misc", 0, 0),
        ("Otros", 0, 0),
    ];

    let mut airports_count: i64 = 0;
    let mut aircraft_count: i64 = 0;
    let mut liveries_count: i64 = 0;

    for r in &raws {
        let ct = r
            .content_type
            .as_deref()
            .map(|s| s.to_ascii_uppercase())
            .unwrap_or_default();
        let bytes = r.size_bytes.unwrap_or(0);
        let _icao_present = r
            .icao
            .as_deref()
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false);

        let bucket_idx: usize = match ct.as_str() {
            "SCENERY" => {
                // (v3.1.0) airports_count = SCENERY + ICAO que match
                // un aeropuerto REAL en OurAirports. Antes contábamos
                // cualquier SCENERY con icao_present (172) — incluyendo
                // librerías con título sugestivo. Ahora el counter (121)
                // coincide con el mapa.
                if r.icao_resolvable {
                    airports_count += 1;
                }
                0
            }
            "AIRCRAFT" => {
                if r.dependencies_count > 0 {
                    liveries_count += 1;
                    2
                } else {
                    aircraft_count += 1;
                    1
                }
            }
            "INSTRUMENT" => 3,
            "MISC" => 4,
            _ => 5,
        };
        buckets[bucket_idx].1 += 1;
        buckets[bucket_idx].2 += bytes;
    }

    let by_type: Vec<TypeStat> = buckets
        .into_iter()
        .filter(|(_, c, _)| *c > 0)
        .map(|(label, count, size_bytes)| TypeStat {
            label: label.to_string(),
            count,
            size_bytes,
        })
        .collect();

    // Top creators por número de paquetes — limitamos a 10 para
    // que la UI no se infinite-scroll. Si quieres ver todos hay
    // que ir a la pestaña «Addons» con el filtro correspondiente.
    let top_creators: Vec<CreatorStat> = sqlx::query_as::<_, (String, i64, Option<i64>)>(
        r#"
        SELECT
            creator                              AS creator,
            COUNT(*)                             AS cnt,
            SUM(COALESCE(size_bytes, 0))         AS bytes
        FROM community_packages
        WHERE creator IS NOT NULL AND TRIM(creator) <> ''
        GROUP BY creator
        ORDER BY cnt DESC, bytes DESC
        LIMIT 10
        "#,
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|(creator, count, bytes)| CreatorStat {
        creator,
        count,
        size_bytes: bytes.unwrap_or(0),
    })
    .collect();

    // Top 10 más grandes — orientativo para limpieza de disco.
    let largest_packages: Vec<PackageRef> = sqlx::query_as::<_, PackageRef>(
        r#"
        SELECT folder_name, title, creator, size_bytes, content_type
        FROM community_packages
        WHERE size_bytes IS NOT NULL AND size_bytes > 0
        ORDER BY size_bytes DESC
        LIMIT 10
        "#,
    )
    .fetch_all(pool)
    .await?;

    // Más recientes por `folder_modified_at`. Cuando el manifest
    // no expone fecha, `folder_modified_at` viene de `metadata` del
    // FS — siempre debería haber algo.
    let recently_added: Vec<PackageRef> = sqlx::query_as::<_, PackageRef>(
        r#"
        SELECT folder_name, title, creator, size_bytes, content_type
        FROM community_packages
        WHERE folder_modified_at IS NOT NULL
        ORDER BY folder_modified_at DESC
        LIMIT 8
        "#,
    )
    .fetch_all(pool)
    .await?;

    // Updates pendientes — reusa el mismo pipeline que el panel
    // de notificaciones. El número incluye dismissed (es global).
    let updates_available = match updates::compute_available(pool).await {
        Ok(list) => list.len() as i64,
        Err(e) => {
            tracing::warn!("dashboard: compute_available falló: {e:#}");
            0
        }
    };

    Ok(DashboardStats {
        total_packages,
        total_size_bytes,
        updates_available,
        by_type,
        top_creators,
        largest_packages,
        recently_added,
        airports_count,
        liveries_count,
        aircraft_count,
    })
}
