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
        #[allow(dead_code)]
        dependencies_count: i64,
        icao: Option<String>,
        size_bytes: Option<i64>,
        icao_resolvable: bool,
        title: String,
        folder_name: String,
        base_containers_json: Option<String>,
        simobject_dirs_json: Option<String>,
        has_flight_model: bool,
    }
    let raws: Vec<RawRow> = sqlx::query_as(
        r#"
        SELECT cp.content_type,
               cp.dependencies_count,
               cp.icao,
               cp.size_bytes,
               (a.icao IS NOT NULL) AS icao_resolvable,
               cp.title,
               cp.folder_name,
               cp.base_containers_json,
               cp.simobject_dirs_json,
               cp.has_flight_model
        FROM community_packages cp
        LEFT JOIN airports a ON a.icao = UPPER(cp.icao)
        "#,
    )
    .fetch_all(pool)
    .await?;

    // Helper: parsea un JSON array de strings a Vec<String> minúsculas.
    fn parse_str_array(raw: Option<&str>) -> Vec<String> {
        raw.and_then(|s| serde_json::from_str::<Vec<String>>(s).ok())
            .unwrap_or_default()
            .into_iter()
            .map(|s| s.trim().to_lowercase())
            .collect()
    }

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

    // (v4.24.1) AIRPORTS = ICAOs DISTINTOS de paquetes SCENERY que
    // resuelven a un aeropuerto real Y que NO son librerías/complementos
    // (AIRAC, night lights, enhancements, excludes, merges…). El MISMO
    // criterio que el mapa de scenery (`is_library_pack` compartido) y
    // deduplicado por ICAO — dashboard y mapa siempre coinciden.
    let mut airport_icaos: std::collections::HashSet<String> =
        std::collections::HashSet::new();
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
                // (v3.1.0 → v4.24.1) SCENERY + ICAO real + NO librería,
                // deduplicado por ICAO — igual que el mapa.
                if r.icao_resolvable
                    && !crate::community_scanner::is_library_pack(
                        &r.title,
                        &r.folder_name,
                    )
                {
                    if let Some(ic) = r.icao.as_deref() {
                        airport_icaos.insert(ic.trim().to_ascii_uppercase());
                    }
                }
                0
            }
            "AIRCRAFT" => {
                // (v7.2.2) content_type=AIRCRAFT lo declaran TANTO los aviones
                // como sus liveries y sus mods (cabina/luces/texturas). La señal
                // DEFINITIVA de avión es `has_flight_model` (trae flight_model.cfg
                // / .air) — igual que derivedType en el front. Sirve para los
                // ENCRIPTADOS (PMDG/Fenix, has_own_model=false) y excluye mods
                // que traen geometría sin ser aviones.
                if r.has_flight_model {
                    aircraft_count += 1;
                    1 // Aviones
                } else {
                    // Sin modelo de vuelo → livery o mod. Señal de livery
                    // (título "livery/liveries/repaint" o base_container FUERA
                    // de los containers propios) → Liveries; si no, es un mod →
                    // Sonido/Misc. Aproximación: no replica el match por
                    // aerolínea del front.
                    let hay = format!("{} {}", r.title, r.folder_name).to_lowercase();
                    let own: std::collections::HashSet<String> =
                        parse_str_array(r.simobject_dirs_json.as_deref())
                            .into_iter()
                            .collect();
                    let base_outside = parse_str_array(r.base_containers_json.as_deref())
                        .iter()
                        .any(|b| !own.contains(b));
                    let is_livery = hay.contains("livery")
                        || hay.contains("liveries")
                        || hay.contains("repaint")
                        || base_outside;
                    if is_livery {
                        liveries_count += 1;
                        2 // Liveries
                    } else {
                        4 // Sonido / Misc (mod de cabina/luces/texturas)
                    }
                }
            }
            "LIVERY" | "PAINT" | "REPAINT" | "PAINTKIT" | "TEXTURE" => {
                liveries_count += 1;
                2
            }
            "INSTRUMENT" | "INSTRUMENTS" => 3,
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
        airports_count: airport_icaos.len() as i64,
        liveries_count,
        aircraft_count,
    })
}

// ===========================================================================
// (v7.3.0) Centro de salud de addons — reporte read-only para el Dashboard.
// ===========================================================================

/// Addon nativo de un sim instalado en la Community del OTRO (no cargará ahí).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SimCompatMismatch {
    pub folder_name: String,
    pub title: String,
    pub community_sim: String, // "2020" | "2024" — la Community donde está
    pub builder_sim: String,   // "2020" | "2024" — para el que fue compilado
    pub install_path: String,
}

/// Resto/artefacto que se puede limpiar sin romper nada.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthArtifact {
    pub kind: &'static str, // "cvt" | "orphan_tracked"
    pub folder_name: String,
    pub title: Option<String>,
    pub size_bytes: i64,
    pub detail: String,
}

/// Livery cuyo avión base ya no está instalado.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrphanLivery {
    pub folder_name: String,
    pub title: String,
    pub size_bytes: i64,
    /// Container del avión base que referencia (para mostrar cuál falta).
    pub base: Option<String>,
}

/// Reporte de salud completo.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthReport {
    pub total_packages: i64,
    pub issue_count: i64,
    pub reclaimable_bytes: i64,
    pub orphaned_liveries: Vec<OrphanLivery>,
    pub sim_compat_mismatches: Vec<SimCompatMismatch>,
    pub artifacts: Vec<HealthArtifact>,
}

#[tauri::command]
pub async fn get_addon_health(
    state: tauri::State<'_, AppState>,
) -> Result<HealthReport, String> {
    compute_health(&state.db).await.map_err(|e| e.to_string())
}

/// (v7.3.0) Mueve un addon de su Community actual a la del sim indicado —
/// para arreglar un nativo de 2024 metido en la Community de 2020. Devuelve la
/// ruta destino. (El borrado de liveries/artefactos reusa `uninstall_by_folder`.)
#[tauri::command]
pub async fn health_move_to_sim(
    install_path: String,
    target_sim: String,
) -> Result<String, String> {
    let src = std::path::PathBuf::from(&install_path);
    if !src.is_dir() {
        return Err("La carpeta ya no existe.".into());
    }
    let folder = src
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or("Ruta inválida")?
        .to_string();
    let comms =
        crate::community::detect_all_community_folders().map_err(|e| e.to_string())?;
    let dst_comm = comms
        .iter()
        .find(|c| c.variant.contains(&target_sim) && c.exists)
        .or_else(|| comms.iter().find(|c| c.variant.contains(&target_sim)))
        .ok_or_else(|| format!("No encontré la Community de MSFS {target_sim}."))?;
    let dst = std::path::PathBuf::from(&dst_comm.path).join(&folder);
    if dst.exists() {
        return Err(format!(
            "Ya existe '{folder}' en la Community de MSFS {target_sim}."
        ));
    }
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    // Rename directo (mismo disco); si falla (cross-drive), copiar + borrar.
    if std::fs::rename(&src, &dst).is_err() {
        copy_dir_all(&src, &dst).map_err(|e| e.to_string())?;
        std::fs::remove_dir_all(&src).map_err(|e| e.to_string())?;
    }
    tracing::info!(target: "health", "moved {} -> {}", install_path, dst.display());
    Ok(dst.to_string_lossy().into_owned())
}

fn copy_dir_all(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    for entry in walkdir::WalkDir::new(src) {
        let entry = entry.map_err(std::io::Error::other)?;
        let rel = entry
            .path()
            .strip_prefix(src)
            .map_err(std::io::Error::other)?;
        let target = dst.join(rel);
        if entry.file_type().is_dir() {
            std::fs::create_dir_all(&target)?;
        } else {
            if let Some(p) = target.parent() {
                std::fs::create_dir_all(p)?;
            }
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

/// (v7.3.0) Quita una fila de tracking de flightsim.to huérfana (su addon ya no
/// está instalado). Inofensivo — solo borra un registro de la DB.
#[tauri::command]
pub async fn health_prune_tracked(
    folder_name: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    sqlx::query("DELETE FROM flightsim_tracked_files WHERE folder_name = ?1")
        .bind(&folder_name)
        .execute(&state.db)
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

async fn compute_health(pool: &SqlitePool) -> anyhow::Result<HealthReport> {
    // --- Check 1: compatibilidad de sim (por campo `builder` del manifest) ---
    // MSFS 2024 carga los addons de 2020 (retrocompatible), pero MSFS 2020 NO
    // carga los nativos de 2024. Marcamos SOLO ese caso: builder=2024 dentro de
    // la Community de 2020. Escaneamos las carpetas Community en DISCO (no la
    // DB, que solo tiene la del sim activo) para ver AMBOS simuladores.
    let communities = crate::community::detect_all_community_folders().unwrap_or_default();
    let sim_compat_mismatches = tokio::task::spawn_blocking(move || {
        let mut out: Vec<SimCompatMismatch> = Vec::new();
        for c in &communities {
            if !c.exists {
                continue;
            }
            // Solo la Community de 2020 puede tener el problema.
            if c.variant.contains("2024") {
                continue;
            }
            let Ok(entries) = std::fs::read_dir(&c.path) else {
                continue;
            };
            for e in entries.flatten() {
                let p = e.path();
                if !p.is_dir() {
                    continue;
                }
                let Ok(raw) = std::fs::read_to_string(p.join("manifest.json")) else {
                    continue;
                };
                let Ok(json) = serde_json::from_str::<serde_json::Value>(&raw) else {
                    continue;
                };
                let builder = json.get("builder").and_then(|b| b.as_str()).unwrap_or("");
                if builder.contains("2024") {
                    let title = json
                        .get("title")
                        .and_then(|t| t.as_str())
                        .unwrap_or("")
                        .to_string();
                    let folder = p
                        .file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or("")
                        .to_string();
                    out.push(SimCompatMismatch {
                        folder_name: folder,
                        title,
                        community_sim: "2020".into(),
                        builder_sim: "2024".into(),
                        install_path: p.to_string_lossy().into_owned(),
                    });
                }
            }
        }
        out
    })
    .await
    .unwrap_or_default();

    // --- Check 2: restos y artefactos (sobre la DB = Community activa) ---
    let mut artifacts: Vec<HealthArtifact> = Vec::new();
    let mut reclaimable_bytes: i64 = 0;

    // (a) Carpetas de conversión sobrantes `*_CVT_`.
    let cvt: Vec<(String, String, Option<i64>)> = sqlx::query_as(
        "SELECT folder_name, title, size_bytes FROM community_packages \
         WHERE LOWER(folder_name) LIKE '%_cvt_%'",
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    for (folder, title, size) in cvt {
        let sz = size.unwrap_or(0);
        reclaimable_bytes += sz;
        artifacts.push(HealthArtifact {
            kind: "cvt",
            folder_name: folder,
            title: Some(title),
            size_bytes: sz,
            detail: "Carpeta de conversión sobrante".into(),
        });
    }

    // (b) Filas de tracking de flightsim.to cuyo addon ya no está instalado.
    let orphan_tracked: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT t.folder_name, t.title FROM flightsim_tracked_files t \
         LEFT JOIN community_packages c ON c.folder_name = t.folder_name \
         WHERE c.folder_name IS NULL",
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    for (folder, title) in orphan_tracked {
        artifacts.push(HealthArtifact {
            kind: "orphan_tracked",
            folder_name: folder,
            title,
            size_bytes: 0,
            detail: "Seguimiento de flightsim.to sin addon instalado".into(),
        });
    }

    // --- Check 3: liveries huérfanas (el avión base ya no está instalado) ---
    #[derive(sqlx::FromRow)]
    struct LivRow {
        folder_name: String,
        title: String,
        content_type: Option<String>,
        size_bytes: Option<i64>,
        has_flight_model: bool,
        simobject_dirs_json: Option<String>,
        base_containers_json: Option<String>,
    }
    let liv_rows: Vec<LivRow> = sqlx::query_as(
        "SELECT folder_name, title, content_type, size_bytes, has_flight_model, \
         simobject_dirs_json, base_containers_json FROM community_packages",
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    let parse_lc = |s: &Option<String>| -> Vec<String> {
        s.as_deref()
            .and_then(|x| serde_json::from_str::<Vec<String>>(x).ok())
            .unwrap_or_default()
            .into_iter()
            .map(|x| x.trim().to_lowercase())
            .collect::<Vec<String>>()
    };
    // Containers que POSEE un avión de verdad instalado (has_flight_model).
    let installed_containers: std::collections::HashSet<String> = liv_rows
        .iter()
        .filter(|r| r.has_flight_model)
        .flat_map(|r| parse_lc(&r.simobject_dirs_json))
        .collect();
    // Un container base "por defecto" (Asobo / Microsoft) NO es de Community,
    // vive en el sim → esas liveries NO son huérfanas.
    let is_default = |n: &str| {
        let n = n.to_ascii_lowercase();
        n.starts_with("asobo") || n.starts_with("microsoft") || n.contains("asobo_")
    };
    let mut orphaned_liveries: Vec<OrphanLivery> = Vec::new();
    for r in &liv_rows {
        if r.has_flight_model {
            continue;
        }
        let ct = r.content_type.as_deref().unwrap_or("").to_ascii_uppercase();
        if ct != "AIRCRAFT" && ct != "LIVERY" {
            continue;
        }
        let mut refs = parse_lc(&r.simobject_dirs_json);
        refs.extend(parse_lc(&r.base_containers_json));
        if refs.is_empty() {
            continue;
        }
        // Alguna base instalada → NO huérfana.
        if refs.iter().any(|x| installed_containers.contains(x)) {
            continue;
        }
        // Todas las bases son aviones por defecto del sim → NO huérfana.
        if refs.iter().all(|x| is_default(x)) {
            continue;
        }
        let sz = r.size_bytes.unwrap_or(0);
        reclaimable_bytes += sz;
        orphaned_liveries.push(OrphanLivery {
            folder_name: r.folder_name.clone(),
            title: r.title.clone(),
            size_bytes: sz,
            base: refs.into_iter().next(),
        });
    }

    let total_packages = liv_rows.len() as i64;
    let issue_count =
        (orphaned_liveries.len() + sim_compat_mismatches.len() + artifacts.len()) as i64;

    Ok(HealthReport {
        total_packages,
        issue_count,
        reclaimable_bytes,
        orphaned_liveries,
        sim_compat_mismatches,
        artifacts,
    })
}
