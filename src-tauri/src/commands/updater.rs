use std::path::PathBuf;

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

/// Reporte de progreso emitido por el comando `install_update`. El
/// frontend lo escucha en el evento `updater://progress` para mostrar
/// una barra mientras baja el setup.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateProgress {
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
}

/// Descarga el instalador del release y lo lanza en modo **silent**
/// (`/S` para NSIS / `/quiet` para MSI). Inmediatamente después
/// cierra la app actual para que el installer pueda reemplazar los
/// archivos en disco — Windows no deja sobreescribir un .exe que
/// está corriendo.
///
/// La cadena:
///   1. `GET <asset_url>` con streaming → escribe a
///      `%TEMP%\msfs-addons-browser-update.<ext>`.
///   2. Emite `updater://progress` cada chunk para que el banner
///      muestre porcentaje real.
///   3. `Command::new(installer).arg("/S").spawn()` con flags
///      `DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP` (Windows) —
///      eso hace que el proceso del installer sobreviva al `exit(0)`
///      que ejecutamos a renglón seguido.
///   4. `app.exit(0)` — la app desaparece; NSIS termina la
///      instalación; el "Run after install" del setup la abre con
///      la versión nueva.
///
/// Errores: cualquier fallo de descarga, escritura o spawn aborta y
/// devuelve un string al frontend; **no** cerramos la app si algo
/// salió mal, así el usuario puede ver el mensaje y reintentar.
#[tauri::command]
pub async fn install_update(
    asset_url: String,
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    tracing::info!(target: "updater", "install_update solicitado: {}", asset_url);

    let resp = state
        .http
        .get(&asset_url)
        .header("User-Agent", "MSFSAddonsBrowser/updater")
        .send()
        .await
        .map_err(|e| {
            tracing::error!(target: "updater", "GET asset falló: {e:#}");
            format!("descarga falló: {}", e)
        })?;
    if !resp.status().is_success() {
        return Err(format!("descarga falló con HTTP {}", resp.status()));
    }
    let total_bytes = resp.content_length();

    // Path destino — preferimos %TEMP% por dos motivos:
    //   1. El installer lo borra Windows tras un reboot si quedó
    //      huérfano.
    //   2. No necesitamos permisos extra de escritura.
    let ext = pick_installer_extension(&asset_url);
    let temp_dir = std::env::temp_dir();
    let installer_path = temp_dir.join(format!("msfs-addons-browser-update.{ext}"));
    // Borrar versión previa si quedó de un intento anterior.
    let _ = std::fs::remove_file(&installer_path);

    let mut file = tokio::fs::File::create(&installer_path).await.map_err(|e| {
        tracing::error!(target: "updater", "create temp falló: {e}");
        format!("no se pudo crear el archivo temporal: {}", e)
    })?;

    use futures_util::StreamExt;
    use tokio::io::AsyncWriteExt;
    let mut stream = resp.bytes_stream();
    let mut downloaded: u64 = 0;
    while let Some(chunk_res) = stream.next().await {
        let chunk = chunk_res.map_err(|e| {
            tracing::error!(target: "updater", "stream chunk falló: {e}");
            format!("descarga interrumpida: {}", e)
        })?;
        file.write_all(&chunk).await.map_err(|e| {
            tracing::error!(target: "updater", "write chunk falló: {e}");
            format!("no se pudo escribir el chunk: {}", e)
        })?;
        downloaded += chunk.len() as u64;

        // Emitir progreso. Si falla, no es fatal — sólo perdemos
        // un tick de la barra. Lanzar `emit` panic-free: discard error.
        let _ = tauri::Emitter::emit(
            &app,
            "updater://progress",
            UpdateProgress {
                downloaded_bytes: downloaded,
                total_bytes,
            },
        );
    }
    file.flush().await.map_err(|e| e.to_string())?;
    drop(file);
    tracing::info!(
        target: "updater",
        "descarga completa ({} bytes) → {}",
        downloaded,
        installer_path.display()
    );

    // Lanzar instalador silencioso y desadjuntado.
    launch_installer_detached(&installer_path, ext)
        .map_err(|e| format!("no se pudo lanzar el instalador: {}", e))?;

    // Un breve sleep para que el proceso del installer tenga tiempo
    // de iniciarse antes de que cerremos. 800ms es generoso pero
    // no perceptible para el usuario (ya vio la barra al 100%).
    tokio::time::sleep(std::time::Duration::from_millis(800)).await;
    tracing::info!(target: "updater", "lanzando exit(0) para que el installer reemplace archivos");
    app.exit(0);
    Ok(())
}

fn pick_installer_extension(url: &str) -> &'static str {
    let lower = url.to_lowercase();
    if lower.ends_with(".msi") {
        "msi"
    } else if lower.ends_with(".msix") || lower.ends_with(".msixbundle") {
        "msix"
    } else {
        "exe"
    }
}

#[cfg(target_os = "windows")]
fn launch_installer_detached(installer: &PathBuf, ext: &str) -> std::io::Result<()> {
    use std::os::windows::process::CommandExt;

    // CREATE_NEW_PROCESS_GROUP + DETACHED_PROCESS: el hijo sobrevive
    // a nuestro exit(0). Si no usáramos esto, cerrar la app mataría
    // también al installer.
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    let flags = CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS;

    match ext {
        // NSIS (Tauri default) usa `/S` (uppercase) para silent.
        "exe" => std::process::Command::new(installer)
            .arg("/S")
            .creation_flags(flags)
            .spawn()
            .map(|_| ()),
        // MSI se ejecuta vía msiexec con /quiet y /norestart.
        "msi" => std::process::Command::new("msiexec")
            .args(["/i"])
            .arg(installer)
            .args(["/quiet", "/norestart"])
            .creation_flags(flags)
            .spawn()
            .map(|_| ()),
        // MSIX/MSIXBUNDLE — Add-AppxPackage via PowerShell. No silent
        // de facto pero al menos lo arranca sin abrir la GUI.
        "msix" | "msixbundle" => std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command"])
            .arg(format!(
                "Add-AppxPackage -Path '{}' -ForceUpdateFromAnyVersion",
                installer.display()
            ))
            .creation_flags(flags)
            .spawn()
            .map(|_| ()),
        _ => Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("extensión no soportada: {}", ext),
        )),
    }
}

#[cfg(not(target_os = "windows"))]
fn launch_installer_detached(_installer: &PathBuf, _ext: &str) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "auto-install sólo soportado en Windows",
    ))
}
