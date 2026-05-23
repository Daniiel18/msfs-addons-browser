use std::path::PathBuf;

use crate::updater::{self, UpdateInfo};
use crate::AppState;

/// (v3.4.0) Resuelve un directorio cache para el instalador, garantizado
/// FUERA de la carpeta de instalación.
///
/// **Por qué importa**: en `lib.rs:init_state` redirigimos `TEMP`/`TMP` al
/// data dir portable (`<exe_dir>/data/temp/`) para que los archivos
/// temporales de drag-drop / extracciones no contaminen `%TEMP%` del SO.
/// PERO eso significa que `std::env::temp_dir()` ahora apunta DENTRO del
/// install dir. NSIS al ejecutar el setup suele limpiar/sobrescribir
/// archivos en el install dir → puede clobear al propio instalador
/// mid-execution. Resultado observado por el usuario: "se queda
/// congelado, nunca finaliza la instalación real" (log v3.3.0).
///
/// Estrategia (en orden de preferencia):
///   1. `%LOCALAPPDATA%\SimFleet\updater-cache\` — user-scoped, fuera
///      del install dir, escritible sin admin.
///   2. `%USERPROFILE%\AppData\Local\SimFleet\updater-cache\` — fallback
///      cuando `LOCALAPPDATA` no está expuesta (raro en Windows
///      moderno pero defensivo).
///   3. `%TEMP%` (la sobrescrita) — último recurso. Sigue funcionando
///      en la mayoría de casos donde el install dir no se limpia
///      agresivamente (ej. NSIS sólo reemplaza archivos cambiados).
fn updater_cache_dir() -> PathBuf {
    let candidates = [
        std::env::var_os("LOCALAPPDATA")
            .map(|s| PathBuf::from(s).join("SimFleet").join("updater-cache")),
        std::env::var_os("USERPROFILE").map(|s| {
            PathBuf::from(s)
                .join("AppData")
                .join("Local")
                .join("SimFleet")
                .join("updater-cache")
        }),
    ];
    for opt in candidates.into_iter().flatten() {
        if std::fs::create_dir_all(&opt).is_ok() {
            // Test de escritura — si el dir existe pero no podemos
            // escribir (perms raros), probamos el siguiente.
            let probe = opt.join(".write-probe");
            if std::fs::write(&probe, b"").is_ok() {
                let _ = std::fs::remove_file(&probe);
                return opt;
            }
        }
    }
    // Último recurso — la TEMP overrideada. Aunque tenga el bug del
    // install dir, el sistema arranca el setup y muchas veces NSIS
    // sí completa la instalación. Mejor que crashear.
    std::env::temp_dir()
}

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
    auto_restart: Option<bool>,
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let auto_restart = auto_restart.unwrap_or(true);
    tracing::info!(
        target: "updater",
        "install_update solicitado: {} (auto_restart={})",
        asset_url, auto_restart
    );

    let resp = state
        .http
        .get(&asset_url)
        .header("User-Agent", "SimFleet/updater")
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

    // (v3.4.0) Path destino — **NO** usamos `std::env::temp_dir()`
    // porque la TEMP del proceso está sobrescrita al data dir portable
    // (lib.rs:init_state) que vive DENTRO del install dir. NSIS al
    // ejecutar el setup desde ahí puede clobear el propio binario.
    // En su lugar usamos `updater_cache_dir()` que devuelve
    // `%LOCALAPPDATA%\SimFleet\updater-cache\` garantizado FUERA del
    // install dir.
    let ext = pick_installer_extension(&asset_url);
    let cache_dir = updater_cache_dir();
    let installer_path = cache_dir.join(format!("SimFleet-update.{ext}"));
    // Borrar versión previa si quedó de un intento anterior.
    let _ = std::fs::remove_file(&installer_path);
    tracing::debug!(
        target: "updater",
        "installer cache dir = {} (fuera del install dir)",
        cache_dir.display()
    );

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

    // **Safety net** (v0.1.15): aunque NSIS Tauri-default debería
    // relanzar la app post-install, los reports del usuario dicen
    // que no siempre lo hace. Lanzamos también un `cmd.exe` helper
    // detached que:
    //   1. Espera 8 segundos (tiempo de install típico).
    //   2. Lanza el `.exe` actual (mismo path) si existe.
    // Si NSIS ya relanzó por su cuenta, Windows abre una segunda
    // instancia que el `single_instance` plugin de Tauri rechazaría
    // — pero en nuestra app no tenemos single_instance, así que
    // simplemente se abrirá UNA ventana extra (raro pero no roto).
    // Si NSIS NO relanzó, este helper sí. Net-neutral.
    if auto_restart {
        if let Ok(current_exe) = std::env::current_exe() {
            let exe_str = current_exe.to_string_lossy().into_owned();
            let exe_clean = exe_str
                .strip_prefix(r"\\?\")
                .unwrap_or(&exe_str)
                .to_string();
            if let Err(e) = launch_relaunch_helper(&exe_clean) {
                tracing::warn!(
                    target: "updater",
                    "no se pudo lanzar el helper de relaunch (no fatal): {}",
                    e
                );
            } else {
                tracing::info!(
                    target: "updater",
                    "helper de relaunch agendado para {}",
                    exe_clean
                );
            }
        }
    } else {
        tracing::info!(
            target: "updater",
            "auto_restart=false — no se lanza helper. El usuario debe abrir la app manualmente."
        );
    }

    // Un breve sleep para que el proceso del installer tenga tiempo
    // de iniciarse antes de que cerremos. 800ms es generoso pero
    // no perceptible para el usuario (ya vio la barra al 100%).
    tokio::time::sleep(std::time::Duration::from_millis(800)).await;
    tracing::info!(target: "updater", "lanzando exit(0) para que el installer reemplace archivos");
    app.exit(0);
    Ok(())
}

/// Lanza un proceso PowerShell detached que espera 8 segundos y
/// luego abre el `.exe` indicado. Sirve como backup garantizado
/// del auto-relaunch del NSIS Tauri (que no siempre dispara en
/// silent mode).
///
/// Usamos PowerShell en lugar de cmd porque:
///   · `Start-Process` maneja paths con espacios sin parsing manual.
///   · No depende del comportamiento de `cmd /c start ""` que
///     fallaba con prefijo NT `\\?\` (bug reportado por usuario
///     en v0.1.16, screenshot "Windows cannot find '\\\\'").
///   · `Start-Sleep` es predecible; `timeout` de cmd tiene flags
///     que varían según locale de Windows.
///
/// El proceso sobrevive al `exit(0)` de la app gracias a
/// `DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW`.
#[cfg(target_os = "windows")]
fn launch_relaunch_helper(exe_path: &str) -> std::io::Result<()> {
    use std::os::windows::process::CommandExt;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let escaped = exe_path.replace('\'', "''");
    // (v3.4.0) **UN SOLO log path activo**. Antes (v3.3.0) el helper
    // escribía a 4 candidatos a la vez para cubrir migraciones — pero
    // si dos coincidían (ej. portable `<exe>\data\logs\` Y legacy
    // `%APPDATA%\org.n0xful...\logs\` ambos existían), la MISMA línea
    // entraba dos veces y el usuario reportó "cada línea se escribe
    // exactamente dos veces". Como desde v3.2.0 la arquitectura es
    // portable + estable, hardcodeamos a UN solo path: el resuelto
    // de `<exe_dir>\data\logs\simfleet.log`. Si por alguna razón el
    // dir no es escribible, caemos sólo al temp log (no a otros
    // candidatos legacy).
    let ps_command = format!(
        r#"$origExe = '{esc}'
$tempLog = Join-Path $env:TEMP 'simfleet-relaunch.log'
$origDir = Split-Path -Parent $origExe
$appLog = Join-Path $origDir 'data\logs\simfleet.log'

function WriteAll($msg) {{
  $line = "$(Get-Date -Format o) [updater] $msg"
  # Siempre al temp log (garantizado escribible).
  try {{ Add-Content -Path $tempLog -Value $line }} catch {{ }}
  # Sólo al simfleet.log activo si su parent dir existe. UN solo
  # archivo — evita los duplicados de líneas reportados en v3.3.0.
  $parent = Split-Path -Parent $appLog
  if (Test-Path -LiteralPath $parent) {{
    try {{ Add-Content -Path $appLog -Value $line }} catch {{ }}
  }}
}}
WriteAll ("=== relaunch helper start exe={0}" -f $origExe)

# (v3.0.0) Lista de candidatos: el path original + variantes con el
# nuevo productName "SimFleet" en per-user y per-machine.
$candidates = @($origExe)
$origDir = Split-Path -Parent $origExe
$origLeaf = Split-Path -Leaf $origExe
$candidates += Join-Path $env:LOCALAPPDATA 'Programs\SimFleet\SimFleet.exe'
$candidates += Join-Path $env:ProgramFiles 'SimFleet\SimFleet.exe'
$candidates += Join-Path ${{env:ProgramFiles(x86)}} 'SimFleet\SimFleet.exe'
$candidates += Join-Path $origDir 'SimFleet.exe'
$candidates += Join-Path $env:LOCALAPPDATA "Programs\$origLeaf"
$candidates = $candidates | Where-Object {{ $_ -ne $null -and $_ -ne '' }} | Select-Object -Unique

$deadline = (Get-Date).AddSeconds(90)
$launched = $null
while ((Get-Date) -lt $deadline -and $launched -eq $null) {{
  foreach ($exe in $candidates) {{
    if (Test-Path -LiteralPath $exe) {{
      try {{
        $item = Get-Item -LiteralPath $exe -ErrorAction Stop
        $age = ((Get-Date) - $item.LastWriteTime).TotalSeconds
        if ($age -gt 3) {{
          WriteAll ("ready exe='{{0}}' age={{1:N1}}s — launching" -f $exe, $age)
          $proc = Start-Process -FilePath $exe -PassThru -ErrorAction Stop
          $launched = $proc
          break
        }} else {{
          WriteAll ("waiting exe='{{0}}' (age={{1:N1}}s)" -f $exe, $age)
        }}
      }} catch {{
        WriteAll ("error getting item '{{0}}': {{1}}" -f $exe, $_)
      }}
    }}
  }}
  if ($launched -eq $null) {{ Start-Sleep -Seconds 2 }}
}}

if ($launched -ne $null) {{
  # Esperar a que el main window aparezca y traerlo al front.
  # Sin esto, en perfiles con muchas ventanas, la app puede arrancar
  # detrás de otras y el usuario no la ve.
  Start-Sleep -Seconds 3
  try {{
    Add-Type -AssemblyName Microsoft.VisualBasic -ErrorAction SilentlyContinue
    [Microsoft.VisualBasic.Interaction]::AppActivate($launched.Id) | Out-Null
    WriteAll ("foreground OK pid={{0}}" -f $launched.Id)
  }} catch {{
    WriteAll ("foreground falló pid={{0}}: {{1}}" -f $launched.Id, $_)
  }}
}} else {{
  WriteAll 'TIMEOUT — ningún candidato quedó disponible en 90s'
}}"#,
        esc = escaped,
    );

    std::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-WindowStyle",
            "Hidden",
            "-Command",
            &ps_command,
        ])
        .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW)
        .spawn()
        .map(|_| ())
}

#[cfg(not(target_os = "windows"))]
fn launch_relaunch_helper(_exe_path: &str) -> std::io::Result<()> {
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
