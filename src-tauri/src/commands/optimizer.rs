use crate::cmd_log;
use crate::optimizer::{self, CleanedCache, OptimizerReport};

/// (06/11) Tarjeta, ajustes de MSFS, cachés y consejos de Windows.
#[tauri::command]
pub async fn optimizer_report() -> Result<OptimizerReport, String> {
    optimizer::report().await.map_err(|e| e.to_string())
}

/// (06/11) Vacía las cachés marcadas. Llegan por id: el front no manda rutas.
#[tauri::command]
pub async fn optimizer_clean(ids: Vec<String>) -> Result<Vec<CleanedCache>, String> {
    cmd_log!("optimizer_clean", "{:?}", ids);
    optimizer::clean(ids).await.map_err(|e| e.to_string())
}
