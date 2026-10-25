use crate::db::repo;
use crate::logger::CmdTimer;
use crate::sources::{Addon, BrowsePage, SourceDescriptor};
use crate::{cmd_log, AppState};

#[tauri::command]
pub async fn list_sources(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<SourceDescriptor>, String> {
    cmd_log!("list_sources");
    Ok(state.sources.iter().map(|s| s.descriptor()).collect())
}

#[tauri::command]
pub async fn search(
    query: String,
    source_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<Addon>, String> {
    cmd_log!("search", "query={:?} source={}", query, source_id);
    let _t = CmdTimer::start("search");
    let source = state
        .source(&source_id)
        .ok_or_else(|| format!("unknown source: {}", source_id))?;

    let addons = source.search(&query).await.map_err(|e| {
        tracing::error!(
            "search('{}', source={}) error: {e}",
            query,
            source_id
        );
        e.to_string()
    })?;
    tracing::info!(
        "search('{}', source={}) → {} resultados",
        query,
        source_id,
        addons.len()
    );

    for a in &addons {
        if let Err(e) = repo::upsert_addon(&state.db, a).await {
            tracing::warn!("failed to cache addon {}: {}", a.id, e);
        }
    }
    Ok(addons)
}

/// (25/10) Abre un addon concreto del catálogo (modal de updates y
/// campanita): lo busca en su fuente por su nombre (y por su ICAO si el
/// nombre no lo trae) y devuelve sólo ese, con sus enlaces de descarga. Si
/// la fuente ya no lo lista, devuelve lo que encontró.
#[tauri::command]
pub async fn catalog_addon(
    addon_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<Addon>, String> {
    cmd_log!("catalog_addon", "id={}", addon_id);
    let (source_id, name, icao) = repo::addon_lookup(&state.db, &addon_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("addon no encontrado en el catálogo: {addon_id}"))?;
    let source = state
        .source(&source_id)
        .ok_or_else(|| format!("unknown source: {}", source_id))?;
    let mut fallback = Vec::new();
    for q in std::iter::once(name).chain(icao) {
        let mut found = source.search(&q).await.map_err(|e| e.to_string())?;
        for a in &found {
            if let Err(e) = repo::upsert_addon(&state.db, a).await {
                tracing::warn!("failed to cache addon {}: {}", a.id, e);
            }
        }
        if let Some(i) = found.iter().position(|a| a.id == addon_id) {
            return Ok(vec![found.swap_remove(i)]);
        }
        if fallback.is_empty() {
            fallback = found;
        }
    }
    tracing::info!("catalog_addon({addon_id}): {source_id} ya no lo lista; {} resultados", fallback.len());
    Ok(fallback)
}

/// Recorre el catálogo paginado de un `source` (sin query). Lo usa
/// la pantalla de inicio para mostrar al usuario los addons más
/// recientes en lugar de un placeholder vacío. La fuente decide la
/// cantidad por página — el frontend sólo pinta `addons` y respeta
/// `hasMore` para decidir si dibuja un botón "Siguiente".
#[tauri::command]
pub async fn browse_source(
    source_id: String,
    page: usize,
    state: tauri::State<'_, AppState>,
) -> Result<BrowsePage, String> {
    cmd_log!("browse_source", "source={} page={}", source_id, page);
    let _t = CmdTimer::start("browse_source");
    let source = state
        .source(&source_id)
        .ok_or_else(|| format!("unknown source: {}", source_id))?;

    let result = source.browse(page.max(1)).await.map_err(|e| {
        tracing::error!("browse_source({}, page={}) error: {e}", source_id, page);
        e.to_string()
    })?;
    tracing::info!(
        "browse_source({}, page={}) → {} addons, hasMore={}",
        source_id,
        page,
        result.addons.len(),
        result.has_more
    );

    // Persistimos los addons también en cache — así, una update
    // detector que comparta ICAO con un paquete instalado puede
    // resolverse incluso sin que el usuario haya tecleado búsqueda.
    for a in &result.addons {
        if let Err(e) = repo::upsert_addon(&state.db, a).await {
            tracing::warn!("failed to cache addon {}: {}", a.id, e);
        }
    }
    Ok(result)
}
