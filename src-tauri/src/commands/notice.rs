//! (23/10) Aviso de escritorio con el diseño de SimFleet (ver `desktop_notice`).

/// Muestra un aviso en la esquina de la pantalla. `kind`: "info" | "success" | "error".
/// `anchor: "app"` lo pone en la esquina de la ventana de SimFleet (p. ej. para
/// que se vea encima de flightsim.to); si no, en la del monitor.
#[tauri::command]
pub async fn desktop_notice(
    app: tauri::AppHandle,
    title: String,
    body: Option<String>,
    kind: Option<String>,
    anchor: Option<String>,
) -> Result<(), String> {
    crate::desktop_notice::show(&app, title, body, kind, anchor.as_deref() == Some("app"));
    Ok(())
}
