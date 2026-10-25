//! Detector de actualizaciones para paquetes instalados en
//! Community.
//!
//! Combina dos pasos:
//!   1. **Refresh activo** (`refresh_for_installed`) — para cada
//!      ICAO con versión instalada, lanza una búsqueda en cada
//!      fuente con ese ICAO como query. Concurrencia con
//!      `tokio::sync::Semaphore` para no machacar el origen.
//!      Skip por (ICAO, source) cuya entrada en
//!      `update_check_cache` esté dentro del TTL.
//!   2. **Detección** (`compute_available`) — JOIN entre
//!      `community_packages` y la versión más reciente del catálogo
//!      por ICAO; comparamos semver y emitimos update sólo si
//!      catálogo > local.
//!
//! ¿Por qué cache + concurrencia?
//!
//! Sin cache: cada apertura de la app gatilla N×M HTTP queries,
//! con N ≈ 100 ICAOs y M = 2 fuentes → 200 GETs. Eso se sentía
//! "minutos en el backend buscando y buscando" como reportó el
//! usuario.
//!
//! Con cache (TTL 6h): la primera apertura sigue siendo costosa
//! pero las siguientes son **instantáneas** porque saltamos el
//! 100% de las queries vigentes. Sólo se recheckea lo que venció.
//!
//! Con concurrencia (semáforo de 6): la primera vez baja de
//! ~50s a ~10s en una conexión decente. El semáforo es global
//! (no por fuente) para no abrir 12 conexiones simultáneas.

use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use sqlx::SqlitePool;
use tokio::sync::Semaphore;

use crate::db::repo;
use crate::sources::{Addon, Source};

/// TTL del cache: 6 horas. Lo bastante alto para que repetir una
/// sesión no toque la red, lo bastante bajo para que las updates
/// reales no tarden más de un día en aparecer.
const CACHE_TTL: Duration = Duration::from_secs(6 * 60 * 60);

/// Concurrencia global del refresh. Probado en SceneryAddons +
/// Simplaza: 12 simultáneas no dispara rate-limits y reduce el
/// tiempo total ≈8× respecto a queries en serie. Más arriba
/// empezamos a ver `429 Too Many Requests` en sceneryaddons.
const MAX_CONCURRENT: usize = 12;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AvailableUpdate {
    pub folder_name: String,
    pub title: String,
    pub icao: String,
    pub installed_version: String,
    pub latest_version: String,
    pub source: String,
    pub addon_id: String,
    pub page_url: String,
    /// (25/10) Para la tarjeta del modal: imagen y desarrollador del catálogo.
    pub image_url: Option<String>,
    pub developer: String,
}

/// (25/10) Cuántas palabras del título instalado lleva el nombre del
/// catálogo: "PMDG 777-200LR" casa mejor con "PMDG Boeing 777-200LR" (3) que
/// con "PMDG Boeing 777-200ER" (2), que también entra por "777-200".
fn closeness(installed: &str, catalog: &str) -> usize {
    let words = |s: &str| -> Vec<String> {
        s.to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
            .map(str::to_string)
            .collect()
    };
    let cat = words(catalog);
    words(installed).iter().filter(|w| cat.contains(w)).count()
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RefreshSummary {
    pub icaos_checked: usize,
    pub queries_run: usize,
    pub queries_skipped_cached: usize,
    pub queries_failed: usize,
    pub addons_seen: usize,
    pub elapsed_ms: u128,
}

/// Lee la cache local (catálogo + community packages) y devuelve
/// las actualizaciones detectadas. Cero red — apta para llamarse
/// en cada apertura del panel de notificaciones.
///
/// Agrupa por `folder_name` y, dentro de cada grupo, elige el
/// candidato con la **versión más alta** (no el más reciente que
/// vimos). Eso evita el bug en que abrir Simplaza después de
/// SceneryAddons hacía aparecer una "update" a una versión menor.
/// (v4.16.3) Normaliza un nombre de creator/developer a SOLO caracteres
/// alfanuméricos ASCII en minúsculas. Colapsa "MK-STUDIOS", "MK–STUDIOS"
/// (en-dash), "MK Studios", "mk.studios" → todos a "mkstudios". Hace el
/// match creator↔developer robusto ante puntuación/espacios/Unicode.
fn norm_alnum(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// (v5.4.1) Colapsa sub-marcas de desarrollador a un token canónico. Algunas
/// casas publican la escenografía bajo otra marca: el manifest trae
/// "iniScene" pero el catálogo (SceneryAddons) lista "iniBuilds" — son la
/// misma casa. Sin esto, el match creator↔developer fallaba y el addon no se
/// reconocía como instalado ni se comparaba la versión (falso update / no
/// marcado como instalado, reportado en KLAX de iniScene/iniBuilds).
pub fn canon_dev(s: &str) -> String {
    let n = norm_alnum(s);
    match n.as_str() {
        "iniscene" => "inibuilds".to_string(),
        _ => n,
    }
}

pub async fn compute_available(pool: &SqlitePool) -> anyhow::Result<Vec<AvailableUpdate>> {
    use std::collections::HashMap;

    let candidates = repo::catalog_versions_for_community(pool).await?;
    let dismissed = repo::list_dismissed_versions(pool).await?;
    tracing::info!(
        "compute_available: {} filas del JOIN community↔addons↔airports, {} dismissed",
        candidates.len(),
        dismissed.len()
    );

    // (v6.2.21) Nº de paquetes INSTALADOS distintos por ICAO — para el
    // fallback "Camino D" de abajo (igual que la tarjeta de Search).
    let mut folders_per_icao: HashMap<String, std::collections::HashSet<String>> =
        HashMap::new();
    for c in &candidates {
        folders_per_icao
            .entry(c.icao.to_ascii_uppercase())
            .or_default()
            .insert(c.folder_name.clone());
    }

    // Por folder, quedarse con el match cuyo catalog_version sea
    // mayor (semver lenient). En empate, preserva el primero visto.
    // Usamos `Entry` para no chocar con el borrow checker al
    // necesitar lectura + escritura del mismo bucket.
    let mut by_folder: HashMap<String, repo::UpdateCandidate> = HashMap::new();
    for c in candidates {
        // (v4.16.3) Match creator↔developer NORMALIZADO (solo alfanuméricos,
        // lowercase) — robusto ante guiones Unicode: el catálogo trae
        // "MK–STUDIOS" (en-dash) y el manifest "MK-STUDIOS" (guion ASCII);
        // el INSTR de SQL no lo cazaba → el update no se detectaba en
        // Dashboard/Map. Aquí ambos colapsan a "mkstudios" → match.
        // Sigue evitando falsos positivos cross-developer (mismo ICAO, dev
        // distinto), que era el motivo original del filtro.
        //
        // (v6.2.21) PARIDAD con la tarjeta de Search ("Camino D"): si el
        // catálogo o el manifest NO traen developer/creator parseable, la
        // tarjeta igual marcaba "Update" cuando ese ICAO tiene UN ÚNICO
        // paquete instalado — pero aquí se saltaba y la campanita no avisaba
        // (reportado con KJFK iniBuilds v1.0.2→v1.0.3). Regla espejo:
        //   · ambos con developer y DISTINTO → skip (conflicto real);
        //   · alguno vacío → solo pasa si es el único instalado de ese ICAO.
        let cn = canon_dev(&c.creator);
        let dn = canon_dev(&c.developer);
        let dev_match =
            !cn.is_empty() && !dn.is_empty() && (cn.contains(&dn) || dn.contains(&cn));
        let dev_conflict = !cn.is_empty() && !dn.is_empty() && !dev_match;
        if dev_conflict {
            tracing::debug!(
                "compute_available: skip {} — creator '{}' ≠ developer '{}'",
                c.folder_name, c.creator, c.developer
            );
            continue;
        }
        if !dev_match {
            let unique = folders_per_icao
                .get(&c.icao.to_ascii_uppercase())
                .map_or(false, |s| s.len() == 1);
            if !unique {
                tracing::debug!(
                    "compute_available: skip {} — sin developer fiable y {} instalados con ICAO {}",
                    c.folder_name,
                    folders_per_icao
                        .get(&c.icao.to_ascii_uppercase())
                        .map_or(0, |s| s.len()),
                    c.icao
                );
                continue;
            }
        }
        use std::collections::hash_map::Entry;
        match by_folder.entry(c.folder_name.clone()) {
            Entry::Vacant(v) => {
                v.insert(c);
            }
            Entry::Occupied(mut o) => {
                let old = o.get();
                // (25/10) Misma versión: la ficha que más se parece a lo
                // instalado (el 777-200LR no abre la del 777-200ER).
                let tie = !is_newer(&old.catalog_version, &c.catalog_version)
                    && closeness(&c.title, &c.catalog_name) > closeness(&old.title, &old.catalog_name);
                if is_newer(&c.catalog_version, &old.catalog_version) || tie {
                    o.insert(c);
                }
            }
        }
    }
    tracing::info!(
        "compute_available: {} folders únicos tras agrupar por max version",
        by_folder.len()
    );

    let mut out = Vec::new();
    for (_, c) in by_folder {
        // (v6.2.22) El descarte es POR VERSIÓN: solo se oculta mientras el
        // catálogo siga en la versión que el usuario descartó. Si salió una
        // más nueva, el aviso REAPARECE (bug reportado: descartó KJFK en
        // v1.0.2 y el aviso de v1.0.3 nunca llegó a campanita/dashboard/mapa).
        if dismissed
            .get(&c.folder_name)
            .is_some_and(|v| v == &c.catalog_version)
        {
            tracing::debug!(
                "compute_available: skip {} (dismissed por usuario en v{})",
                c.folder_name,
                c.catalog_version
            );
            continue;
        }
        if is_newer(&c.catalog_version, &c.installed_version) {
            tracing::info!(
                "compute_available: UPDATE {} ({}): v{} → v{} (source={})",
                c.folder_name,
                c.icao,
                c.installed_version,
                c.catalog_version,
                c.catalog_source
            );
            out.push(AvailableUpdate {
                folder_name: c.folder_name,
                title: c.title,
                icao: c.icao,
                installed_version: c.installed_version,
                latest_version: c.catalog_version,
                source: c.catalog_source,
                addon_id: c.catalog_addon_id,
                page_url: c.catalog_page_url,
                image_url: c.catalog_image_url.filter(|u| !u.is_empty()),
                developer: c.developer,
            });
        } else {
            tracing::debug!(
                "compute_available: skip {} ({}): instalado v{} >= catálogo v{}",
                c.folder_name,
                c.icao,
                c.installed_version,
                c.catalog_version
            );
        }
    }
    // Orden determinista para que la UI no salte fila a fila al
    // refrescar — alfabético por título.
    out.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()));
    tracing::info!("compute_available: {} updates a emitir al frontend", out.len());
    Ok(out)
}

/// Refresh con cache + concurrencia. Devuelve resumen detallado.
///
/// Flujo:
///   1. Lista ICAOs instalados con versión.
///   2. Para cada (icao, source): si cache vigente → skip; si no,
///      añade tarea.
///   3. Lanza tareas en paralelo limitado por `MAX_CONCURRENT`.
///   4. Cada tarea: search → upsert addons → set cache.
///
/// Errores aislados: una fuente caída no rompe a las otras; sólo
/// se cuenta en `queries_failed` y la cache no se actualiza para
/// ese (icao, source) — se reintentará en la próxima apertura.
pub async fn refresh_for_installed(
    pool: &SqlitePool,
    sources: &[Arc<dyn Source>],
) -> anyhow::Result<RefreshSummary> {
    let started = std::time::Instant::now();
    // ICAOs vienen sólo de SCENERY instalada — útiles para
    // SceneryAddons (que ES un catálogo de aeropuertos).
    let icaos = repo::community_icaos_with_version(pool).await?;
    // Keywords vienen de AIRCRAFT/INSTRUMENT/MISC — útiles para
    // ambos catálogos (Simplaza tiene aviones; SceneryAddons a
    // veces también).
    let keywords = repo::community_addon_keywords_with_version(pool).await?;

    let mut summary = RefreshSummary {
        icaos_checked: icaos.len() + keywords.len(),
        queries_run: 0,
        queries_skipped_cached: 0,
        queries_failed: 0,
        addons_seen: 0,
        elapsed_ms: 0,
    };
    if (icaos.is_empty() && keywords.is_empty()) || sources.is_empty() {
        summary.elapsed_ms = started.elapsed().as_millis();
        return Ok(summary);
    }

    // **Bug fix v0.1.16**: en v0.1.15 mandábamos TODOS los términos
    // a TODAS las sources. Eso era ~50% queries desperdiciadas:
    // Simplaza no tiene aeropuertos → buscar `KLAS` en Simplaza
    // siempre devolvía 0 resultados útiles, pero igual ocupaba un
    // request del semáforo y ensuciaba el log.
    //
    // Ahora seleccionamos qué query va a qué source según pertinencia:
    //   · ICAOs → sólo `sceneryaddons` (catálogo de aeropuertos).
    //   · Keywords aircraft → todas las sources (los aviones se
    //     publican en ambos catálogos).
    // (v8.3) A aerohub no se le busca addon por addon. (25/10) Decisión del
    // usuario: sus updates salen sin entrar a su catálogo → se lee su
    // listado entero en segundo plano (como mucho cada 12 h), a la vez que
    // las búsquedas del resto de fuentes.
    let listing = sources.iter().find(|s| s.id() == "aerohub").cloned().map(|src| {
        let pool = pool.clone();
        tokio::spawn(async move { refresh_listing(&pool, src).await })
    });
    let sources: Vec<Arc<dyn Source>> =
        sources.iter().filter(|s| s.id() != "aerohub").cloned().collect();
    let sources = &sources[..];
    let mut tasks: Vec<(String, Arc<dyn Source>)> = Vec::new();
    for icao in &icaos {
        for source in sources {
            // Skip Simplaza para ICAOs — no tiene aeropuertos.
            if source.id() == "simplaza" {
                continue;
            }
            if cache_is_fresh(pool, icao, source.id()).await {
                summary.queries_skipped_cached += 1;
                continue;
            }
            tasks.push((icao.clone(), source.clone()));
        }
    }
    for keyword in &keywords {
        for source in sources {
            if cache_is_fresh(pool, keyword, source.id()).await {
                summary.queries_skipped_cached += 1;
                continue;
            }
            tasks.push((keyword.clone(), source.clone()));
        }
    }

    if tasks.is_empty() {
        summary.elapsed_ms = started.elapsed().as_millis();
        tracing::info!(
            "updates: 100% en cache ({} skips), nada que pedir",
            summary.queries_skipped_cached
        );
        return Ok(summary);
    }

    let total_tasks = tasks.len();
    tracing::info!(
        target: "updates",
        "[Addon Update Scan] starting · {} queries por correr ({} ya en cache); concurrencia={}",
        total_tasks,
        summary.queries_skipped_cached,
        MAX_CONCURRENT
    );

    let sem = Arc::new(Semaphore::new(MAX_CONCURRENT));
    let down: DownSources = Arc::default();
    let mut handles = Vec::with_capacity(tasks.len());
    // (v3.1.3) Contador atómico para enumerar tareas N/total al
    // procesar — el usuario pidió audibility para saber si el
    // scanner se queda colgado en un addon específico.
    let progress_counter = Arc::new(std::sync::atomic::AtomicUsize::new(0));

    for (icao, source) in tasks {
        let sem = sem.clone();
        let pool = pool.clone();
        let counter = progress_counter.clone();
        let down = down.clone();
        handles.push(tokio::spawn(async move {
            // Permit liberado al salir del scope — limita la
            // ventana de in-flight requests al backend.
            let _permit = sem.acquire_owned().await.ok()?;
            if down.lock().map(|d| d.contains(source.id())).unwrap_or(false) {
                return Some(QueryOutcome::Failed);
            }
            let n = counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
            tracing::info!(
                target: "updates",
                "[Addon Update Scan] [{}/{}] {} en {} | Processing…",
                n, total_tasks, icao, source.id()
            );
            let res = source.search(&icao).await;
            let outcome = match res {
                Ok(addons) => {
                    let count = addons.len();
                    let icao_matches = addons
                        .iter()
                        .filter(|a| {
                            a.icao
                                .as_deref()
                                .map(|s| s.eq_ignore_ascii_case(&icao))
                                .unwrap_or(false)
                        })
                        .count();
                    persist_addons(&pool, &addons).await;
                    let best = pick_best_for_icao(&addons, &icao);
                    let (version, addon_id) = best
                        .map(|a| (a.version.clone(), Some(a.id.clone())))
                        .unwrap_or((None, None));
                    tracing::info!(
                        "updates: search({} en {}) → {} resultados, {} con ICAO match, mejor versión = {:?}",
                        icao,
                        source.id(),
                        count,
                        icao_matches,
                        version
                    );
                    if let Err(e) = repo::set_update_check_cache(
                        &pool,
                        &icao,
                        source.id(),
                        version.as_deref(),
                        addon_id.as_deref(),
                    )
                    .await
                    {
                        tracing::warn!("updates: cache set falló ({}, {}): {e:#}", icao, source.id());
                    }
                    QueryOutcome::Ok { addons_seen: count }
                }
                Err(e) => {
                    let first_down = e.is_unavailable()
                        && down.lock().map(|mut d| d.insert(source.id())).unwrap_or(false);
                    if first_down {
                        tracing::warn!(
                            "updates: {} no responde ({}) — se omiten sus demás búsquedas en este escaneo",
                            source.id(),
                            e
                        );
                    } else if !e.is_unavailable() {
                        tracing::warn!(
                            "updates: search('{}') en {} falló: {}",
                            icao,
                            source.id(),
                            e
                        );
                    }
                    QueryOutcome::Failed
                }
            };
            Some(outcome)
        }));
    }

    // Recoger resultados — `join_all` para esperar a todas.
    for h in handles {
        match h.await {
            Ok(Some(QueryOutcome::Ok { addons_seen })) => {
                summary.queries_run += 1;
                summary.addons_seen += addons_seen;
            }
            Ok(Some(QueryOutcome::Failed)) | Ok(None) => {
                summary.queries_run += 1;
                summary.queries_failed += 1;
            }
            Err(e) => {
                summary.queries_run += 1;
                summary.queries_failed += 1;
                tracing::warn!("updates: tarea panicó: {e}");
            }
        }
    }

    if let Some(h) = listing {
        if let Ok(seen) = h.await {
            summary.addons_seen += seen;
        }
    }

    summary.elapsed_ms = started.elapsed().as_millis();
    tracing::info!(
        "updates: refresh terminado en {}ms — {} OK, {} fail, {} skipped, {} addons vistos",
        summary.elapsed_ms,
        summary.queries_run - summary.queries_failed,
        summary.queries_failed,
        summary.queries_skipped_cached,
        summary.addons_seen
    );
    Ok(summary)
}

enum QueryOutcome {
    Ok { addons_seen: usize },
    Failed,
}

/// (25/10) Cada cuánto se relee entero el listado de una fuente (aerohub).
const LISTING_EVERY_HOURS: i64 = 12;
/// Tope de páginas por lectura (aerohub: 24 por página, ~7 hoy).
const LISTING_MAX_PAGES: usize = 30;

/// (25/10) Lee el listado entero de `source` (sólo tarjetas) y guarda sus
/// versiones en el catálogo local, de donde las saca `compute_available`.
/// Como mucho cada 12 h; si una página falla se reintenta en el siguiente
/// arranque. Devuelve los addons vistos.
async fn refresh_listing(pool: &SqlitePool, source: Arc<dyn Source>) -> usize {
    let key = format!("{}_listing_scan_at", source.id());
    let last: Option<String> = sqlx::query_scalar("SELECT value FROM settings WHERE key = ?1")
        .bind(&key)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();
    let fresh = last
        .and_then(|v| chrono::DateTime::parse_from_rfc3339(&v).ok())
        .is_some_and(|at| chrono::Utc::now() - at.with_timezone(&chrono::Utc) < chrono::Duration::hours(LISTING_EVERY_HOURS));
    if fresh {
        tracing::debug!("updates: listado de {} leído hace < {LISTING_EVERY_HOURS} h", source.id());
        return 0;
    }
    let mut seen = 0;
    for page in 1..=LISTING_MAX_PAGES {
        match source.listing(page).await {
            Ok(res) => {
                persist_addons(pool, &res.addons).await;
                seen += res.addons.len();
                if !res.has_more {
                    let _ = sqlx::query("INSERT OR REPLACE INTO settings (key, value, updated_at) VALUES (?1, ?2, datetime('now'))")
                        .bind(&key)
                        .bind(chrono::Utc::now().to_rfc3339())
                        .execute(pool)
                        .await;
                    tracing::info!("updates: listado de {} leído ({page} páginas, {seen} addons)", source.id());
                    return seen;
                }
                // Sin ráfagas: el nginx de aerohub responde 503 si van juntas.
                tokio::time::sleep(Duration::from_millis(400)).await;
            }
            Err(e) => {
                tracing::warn!("updates: listado de {} falló en la página {page}: {e}", source.id());
                return seen;
            }
        }
    }
    seen
}

/// (v7.4.2) Fuentes que dieron "caída" (`SourceError::is_unavailable`)
/// durante ESTE escaneo. Antes, con Skybound devolviendo HTTP 530 desde el
/// 10/10, cada escaneo lanzaba ~50 búsquedas condenadas a fallar (y 50 WARN
/// en el log). Ahora la primera caída "abre el circuito" y el resto de
/// búsquedas a esa fuente se omiten hasta el próximo escaneo.
type DownSources = Arc<std::sync::Mutex<std::collections::HashSet<&'static str>>>;

async fn cache_is_fresh(pool: &SqlitePool, icao: &str, source: &str) -> bool {
    let row = match repo::get_update_check_cache(pool, icao, source).await {
        Ok(Some(r)) => r,
        _ => return false,
    };
    // Si la última vez no extrajimos versión (parser fallido, fuente
    // caída, regex roto…) tratamos la entrada como vencida e
    // intentamos de nuevo. Sin esto, una búsqueda fallida
    // queda "cacheada" 6h y el usuario ve "0 updates" eternamente
    // hasta el próximo TTL — exactamente lo que reportó el usuario
    // tras arreglar el regex de versión.
    if row
        .last_known_version
        .as_deref()
        .map(|v| v.trim().is_empty())
        .unwrap_or(true)
    {
        tracing::debug!(
            "updates: cache invalidada para ({}, {}) por last_known_version vacía",
            icao,
            source
        );
        return false;
    }
    let Some(dt) = chrono::NaiveDateTime::parse_from_str(row.checked_at.trim(), "%Y-%m-%d %H:%M:%S").ok()
    else {
        return false;
    };
    let then = dt.and_utc();
    let now = chrono::Utc::now();
    let age = match now.signed_duration_since(then).to_std() {
        Ok(d) => d,
        Err(_) => return false,
    };
    age < CACHE_TTL
}

async fn persist_addons(pool: &SqlitePool, addons: &[Addon]) {
    for a in addons {
        if let Err(e) = repo::upsert_addon(pool, a).await {
            tracing::warn!("updates: upsert_addon falló para {}: {e:#}", a.id);
        }
    }
}

/// Elige el "mejor" addon para representar la versión actual del
/// query en el catálogo. Heurística en dos fases:
///
///   1. Si el query parece ser un ICAO (4 letras, todas alfa), exigimos
///      `a.icao == query`. Eso vale para SCENERY.
///   2. Si NO parece ICAO (ej. "A350", "Fenix", "iniBuilds"), buscamos
///      el addon cuyo nombre/título contenga el keyword. Esto cubre
///      AIRCRAFT donde el catálogo no tiene ICAO populado.
///
/// En ambos casos exigimos version no vacía. El v0.1.13 sólo hacía la
/// fase 1 → para AIRCRAFT queries (keyword) siempre devolvía None →
/// updates de Simplaza nunca aparecían (bug del log app.log.2026-05-16).
fn pick_best_for_icao<'a>(addons: &'a [Addon], query: &str) -> Option<&'a Addon> {
    let q_lower = query.to_lowercase();
    let q_upper = query.to_ascii_uppercase();
    let looks_like_icao = query.len() == 4
        && query.chars().all(|c| c.is_ascii_alphabetic());

    addons
        .iter()
        .filter(|a| {
            // Versión no vacía es requisito en ambas fases.
            let has_version = a
                .version
                .as_deref()
                .map(|s| !s.trim().is_empty())
                .unwrap_or(false);
            if !has_version {
                return false;
            }
            if looks_like_icao {
                // Fase 1: ICAO match estricto.
                return a
                    .icao
                    .as_deref()
                    .map(|s| s.to_ascii_uppercase() == q_upper)
                    .unwrap_or(false);
            }
            // Fase 2: keyword aircraft — el nombre o título debe
            // contener el query (case-insensitive). Esto es lo
            // que matchea "A350" con "iniBuilds A350-900 …".
            let in_name = a.name.to_lowercase().contains(&q_lower);
            let in_title = a.title.to_lowercase().contains(&q_lower);
            in_name || in_title
        })
        .max_by(|a, b| {
            let av = a.version.as_deref().and_then(parse_lenient);
            let bv = b.version.as_deref().and_then(parse_lenient);
            match (av, bv) {
                (Some(x), Some(y)) => x.cmp(&y),
                _ => a.version.cmp(&b.version),
            }
        })
}

/// `latest > installed` con comparación de versiones tolerante.
///
/// Estrategia (en orden):
///   1. Ambos parsean como semver (con leading-zero strip) → comparar.
///   2. Si NO parsean ambos pero tienen "shape" similar (mismo
///      número de dígitos en el major), comparar lexicográfico.
///   3. Si tienen shape DISTINTO (ej. `1.0.0` vs `2026.01.03`),
///      **NO** comparar — devolver false. Esto evita el bug
///      reportado: catalog "Boeing 737-800 Liveries v2026.01.03"
///      contra un pkg con version="1.0.0" se marcaba como update
///      porque "2026.01.03" > "1.0.0" lexicográficamente, pero
///      claramente son projects distintos (formatos versioning
///      incompatibles).
fn is_newer(latest: &str, installed: &str) -> bool {
    if latest.trim().is_empty() || installed.trim().is_empty() {
        return false;
    }
    let (l, i) = align_year(latest, installed);
    let (l, i) = (l.as_str(), i.as_str());
    // Shape check ANTES de parsear: si los formatos son incompatibles
    // (ej. 1 dígito en major vs 4 dígitos), NO consideramos que sean
    // la misma identidad de versión, incluso si ambos parsean como
    // semver. Esto evita el bug de "1.0.0 → 2026.01.03".
    if !versions_have_compatible_shape(l, i) {
        tracing::debug!(
            "is_newer: shapes incompatibles '{}' vs '{}' — no se considera update",
            l, i
        );
        return false;
    }
    let lp = parse_lenient(l);
    let ip = parse_lenient(i);
    if let (Some(a), Some(b)) = (&lp, &ip) {
        return a > b;
    }
    // (25/10) Más de tres números ("2026.39.1.16"): semver no los parsea;
    // se comparan número a número (en lexicográfico "2026.4" > "2026.39").
    if let (Some(a), Some(b)) = (numeric_parts(l), numeric_parts(i)) {
        let n = a.len().max(b.len());
        let at = |v: &[u64], k: usize| v.get(k).copied().unwrap_or(0);
        return (0..n).map(|k| (at(&a, k), at(&b, k))).find(|(x, y)| x != y).is_some_and(|(x, y)| x > y);
    }
    // Mismo shape pero alguno no parsea — comparar lexicográfico
    // (las shapes son comparables aunque no sean semver válido).
    l > i
}

/// Todas las partes numéricas ("2026.39.1.16" → [2026, 39, 1, 16]); `None`
/// si alguna no lo es.
fn numeric_parts(s: &str) -> Option<Vec<u64>> {
    s.split('.').map(|p| p.parse::<u64>().ok()).collect()
}

/// (25/10) ChasePlane: su manifest dice "26.33.5" (año con dos cifras,
/// semana, build) y aerohub "2026.39.1.16". Si una versión empieza por un
/// año 20AA y la otra por sus dos últimas cifras (±1 por el cambio de año
/// entre ambas), la corta se reescribe con el año entero para poder
/// compararlas. Un major corriente ("10.2.0" frente a "2026.01.03") queda
/// lejos del año y sigue siendo de otro formato.
fn align_year(a: &str, b: &str) -> (String, String) {
    let strip = |s: &str| s.trim().trim_start_matches('v').trim().to_string();
    let (a, b) = (strip(a), strip(b));
    let widen = |short: &str, long: &str| -> Option<String> {
        let ms = short.split('.').next()?;
        let ml = long.split('.').next()?;
        if ms.len() != 2 || ml.len() != 4 || !ml.starts_with("20") {
            return None;
        }
        let year: i64 = ml.parse().ok()?;
        let yy = 2000 + ms.parse::<i64>().ok()?;
        ((yy - year).abs() <= 1).then(|| format!("{yy}{}", &short[2..]))
    };
    if let Some(w) = widen(&a, &b) {
        return (w, b);
    }
    if let Some(w) = widen(&b, &a) {
        return (a, w);
    }
    (a, b)
}

/// True si dos strings de versión podrían razonablemente ser la
/// misma identidad de versión del mismo addon. Distinguimos
/// "semver clásico" (major ≤ 2 dígitos, ej. 1.2.3) de "version-
/// fecha" (major = 4 dígitos, ej. 2026.01.03).
///
/// Aceptamos `1.6` vs `1.5.5` (ambos clásicos, distinto nº de
/// partes está OK) y `2026.05.16` vs `2026.01.03` (ambos fecha).
/// Rechazamos `1.0.0` vs `2026.01.03` — son shapes distintos
/// y compararlos genera false positives.
fn versions_have_compatible_shape(a: &str, b: &str) -> bool {
    let strip = |s: &str| s.trim_start_matches('v').trim().to_string();
    let aa = strip(a);
    let bb = strip(b);
    let major_a = aa.split('.').next().unwrap_or("");
    let major_b = bb.split('.').next().unwrap_or("");
    let a_digits = major_a.chars().filter(|c| c.is_ascii_digit()).count();
    let b_digits = major_b.chars().filter(|c| c.is_ascii_digit()).count();
    // Bucket: ≤3 dígitos = "clásico", ≥4 = "fecha/build-number".
    // Distintos buckets → incompatibles.
    let a_is_date = a_digits >= 4;
    let b_is_date = b_digits >= 4;
    a_is_date == b_is_date
}

fn parse_lenient(s: &str) -> Option<semver::Version> {
    let s = s.trim_start_matches('v').trim();
    if let Ok(v) = semver::Version::parse(s) {
        return Some(v);
    }
    // **Bug fix v0.1.17**: la spec semver prohíbe leading zeros
    // en los componentes numéricos. Strings como "2026.01.03"
    // (formato de fecha) fallaban en parse y caían a lexicográfico,
    // dando false positives (lex "2026.01.03" > "1.0.0").
    // Ahora stripeamos leading zeros de cada parte numérica antes
    // de re-intentar el parse.
    let parts: Vec<String> = s
        .split('.')
        .map(|p| {
            // Conservar las partes no-numéricas tal cual (sufijos
            // alfa como `1.0.0-beta` o `1.0.0+build`).
            if p.chars().all(|c| c.is_ascii_digit()) {
                let trimmed = p.trim_start_matches('0');
                if trimmed.is_empty() {
                    "0".to_string()
                } else {
                    trimmed.to_string()
                }
            } else {
                p.to_string()
            }
        })
        .collect();
    let padded = match parts.len() {
        1 => format!("{}.0.0", parts[0]),
        2 => format!("{}.{}.0", parts[0], parts[1]),
        _ => parts.join("."),
    };
    semver::Version::parse(&padded).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Con una COPIA de la base real (`UPDATES_DB`): qué updates ve el panel.
    #[tokio::test]
    #[ignore]
    async fn available_real_copy() {
        let Ok(path) = std::env::var("UPDATES_DB") else { return };
        // El catálogo se filtra por el sim activo (el del usuario: 2024).
        crate::sim::set_from_str(&std::env::var("UPDATES_SIM").unwrap_or_else(|_| "msfs2024".into()));
        let pool = SqlitePool::connect(&format!("sqlite:{path}")).await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        for u in compute_available(&pool).await.unwrap() {
            println!("{:14} {:30} {} → {}  {}  img={}", u.source, u.title, u.installed_version, u.latest_version, u.page_url, u.image_url.is_some());
        }
    }

    /// Fuente de mentira: dos páginas de listado, cuenta las peticiones.
    struct FakeListing(std::sync::atomic::AtomicUsize);

    #[async_trait::async_trait]
    impl Source for FakeListing {
        fn id(&self) -> &'static str { "aerohub" }
        fn name(&self) -> &'static str { "AeroHub" }
        fn home_url(&self) -> &'static str { "https://aerohub.st" }
        async fn search(&self, _q: &str) -> Result<Vec<Addon>, crate::sources::SourceError> {
            Ok(vec![])
        }
        async fn listing(&self, page: usize) -> Result<crate::sources::BrowsePage, crate::sources::SourceError> {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let addon = Addon {
                id: format!("ah-{page}"),
                source: "aerohub".into(),
                title: format!("Addon {page}"),
                developer: Some("Dev".into()),
                name: format!("Addon {page}"),
                version: Some("1.2.7".into()),
                icao: None,
                simulator: "MSFS 2024".into(),
                page_url: format!("https://aerohub.st/addon/{page}"),
                download_methods: vec![],
                image_url: None,
                released_at: None,
                notes: None,
            };
            Ok(crate::sources::BrowsePage { addons: vec![addon], page, has_more: page < 2 })
        }
    }

    /// (25/10) aerohub: su listado entero va al catálogo local y no se
    /// vuelve a leer hasta pasadas 12 h.
    #[tokio::test]
    async fn a_listing_source_is_read_whole_at_most_every_12_hours() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        let fake = Arc::new(FakeListing(Default::default()));
        let src: Arc<dyn Source> = fake.clone();
        assert_eq!(refresh_listing(&pool, src.clone()).await, 2);
        let stored: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM addons WHERE source = 'aerohub' AND version = '1.2.7'")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(stored, 2);
        assert_eq!(refresh_listing(&pool, src.clone()).await, 0, "leído hace nada");
        assert_eq!(fake.0.load(std::sync::atomic::Ordering::SeqCst), 2);
        sqlx::query("UPDATE settings SET value = ?1 WHERE key = 'aerohub_listing_scan_at'")
            .bind((chrono::Utc::now() - chrono::Duration::hours(13)).to_rfc3339())
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(refresh_listing(&pool, src).await, 2, "pasadas 12 h se relee");
    }

    /// (25/10) Variantes con la misma versión: gana la del mismo nombre.
    #[test]
    fn the_closest_catalog_variant_wins_a_tie() {
        assert!(closeness("PMDG 777-200LR", "PMDG Boeing 777-200LR") > closeness("PMDG 777-200LR", "PMDG Boeing 777-200ER"));
        assert_eq!(closeness("PMDG 777F", "PMDG Boeing 777-Freighter"), 1);
    }

    #[test]
    fn newer_semver_strict() {
        assert!(is_newer("1.6.5", "1.5.5"));
        assert!(is_newer("2.0.0", "1.99.0"));
        assert!(!is_newer("1.5.5", "1.5.5"));
        assert!(!is_newer("1.5.4", "1.5.5"));
    }

    #[test]
    fn newer_padded_versions() {
        assert!(is_newer("1.6", "1.5.5"));
        assert!(is_newer("2", "1.9.9"));
        assert!(!is_newer("1.5", "1.5.5"));
    }

    #[test]
    fn newer_with_v_prefix() {
        assert!(is_newer("v1.6.5", "v1.5.5"));
    }

    #[test]
    fn empty_inputs_return_false() {
        assert!(!is_newer("", "1.0.0"));
        assert!(!is_newer("1.0.0", ""));
    }

    #[test]
    fn date_versions_with_leading_zeros_parse() {
        // Bug reportado por usuario: catalog "v2026.01.03" debe
        // parsearse como Version{2026, 1, 3} y NO caer a lex.
        let v = parse_lenient("2026.01.03").expect("debe parsear");
        assert_eq!(v.major, 2026);
        assert_eq!(v.minor, 1);
        assert_eq!(v.patch, 3);
    }

    #[test]
    fn rejects_incompatible_shapes() {
        // Catalog formato fecha (2026.01.03 = 4 dígitos en major)
        // vs installed semver clásico (1.0.0 = 1 dígito en major)
        // = shape distinto = no es la misma identidad de versión.
        // El user reportó el false positive "1.0.0 → 2026.01.03"
        // para liveries que no debían matchear.
        assert!(!is_newer("2026.01.03", "1.0.0"));
        assert!(!is_newer("v2026.01.03", "v1.0.0"));
        // Inverso también: instalado fecha, catalog semver → no
        // claim como "older".
        assert!(!is_newer("1.0.0", "2026.01.03"));
    }

    /// (25/10) ChasePlane: "26.33.5" instalado, "2026.39.1.16" en aerohub.
    #[test]
    fn a_two_digit_year_meets_its_four_digit_version() {
        assert!(is_newer("2026.39.1.16", "26.33.5"));
        assert!(!is_newer("26.33.5", "2026.39.1.16"));
        assert!(!is_newer("2026.33.5", "26.33.5"), "la misma versión");
        assert!(is_newer("2026.02.1", "25.50.1"), "cambio de año");
        assert!(is_newer("v2026.39.1.16", "v26.33.5"));
        // Un major corriente no es un año: sigue siendo otro formato.
        assert!(!is_newer("2026.01.03", "10.2.0"));
        assert!(!is_newer("2026.01.03", "1.0.0"));
    }

    /// (25/10) Cuatro números: número a número, no lexicográfico.
    #[test]
    fn four_part_versions_compare_number_by_number() {
        assert!(!is_newer("2026.4.0.1", "2026.39.1"));
        assert!(is_newer("2026.39.1.16", "2026.39.1.9"));
        assert!(!is_newer("2026.39.1.16", "2026.39.1.16"));
    }

    #[test]
    fn accepts_compatible_date_versions() {
        // Dos versiones-fecha del mismo shape SÍ se comparan.
        assert!(is_newer("2026.05.16", "2026.01.03"));
        assert!(!is_newer("2026.01.03", "2026.05.16"));
    }

    #[test]
    fn accepts_pmdg_style_versions() {
        // PMDG usa esquemas como "1.0.7" — semver clásico.
        assert!(is_newer("1.0.8", "1.0.7"));
    }
}
