use crate::updater::{self, UpdateInfo};
use crate::AppState;

/// Llama al chequeo de GitHub Releases y devuelve `None` si no hay
/// nada nuevo (o si la consulta falló en silencio). Diseñado para
/// invocarse al arrancar la app — la UI muestra el banner sólo si
/// devolvemos `Some`.
#[tauri::command]
pub async fn check_for_update(
    state: tauri::State<'_, AppState>,
) -> Result<Option<UpdateInfo>, String> {
    match updater::check_latest(&state.http).await {
        Ok(info) => Ok(info),
        Err(e) => {
            // Errores de red no deben pintar rojo en la UI — ya logeamos.
            tracing::warn!("updater: check failed: {e:#}");
            Ok(None)
        }
    }
}
