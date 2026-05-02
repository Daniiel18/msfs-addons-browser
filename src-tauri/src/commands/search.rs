use crate::db::repo;
use crate::sources::{Addon, BrowsePage, SourceDescriptor};
use crate::AppState;

#[tauri::command]
pub async fn list_sources(state: tauri::State<'_, AppState>) -> Result<Vec<SourceDescriptor>, String> {
    Ok(state.sources.iter().map(|s| s.descriptor()).collect())
}

#[tauri::command]
pub async fn search(
    query: String,
    source_id: String,
    state: tauri::State<'_, AppState>,
) -> Result<Vec<Addon>, String> {
    tracing::info!("cmd:search query={:?} source={:?}", query, source_id);
    let source = state
        .source(&source_id)
        .ok_or_else(|| format!("unknown source: {}", source_id))?;

    let addons = source.search(&query).await.map_err(|e| e.to_string())?;

    for a in &addons {
        if let Err(e) = repo::upsert_addon(&state.db, a).await {
            tracing::warn!("failed to cache addon {}: {}", a.id, e);
        }
    }
    Ok(addons)
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
    tracing::info!("cmd:browse_source source={:?} page={}", source_id, page);
    let source = state
        .source(&source_id)
        .ok_or_else(|| format!("unknown source: {}", source_id))?;

    let result = source
        .browse(page.max(1))
        .await
        .map_err(|e| e.to_string())?;

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
