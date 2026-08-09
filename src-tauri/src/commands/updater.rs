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
/// (v3.25.0) Este dir además aloja el **orquestador de update** (.ps1)
/// y su log: tiene que vivir en un disco/dir que el instalador NO toque,
/// porque el usuario instala SimFleet en `D:\...\AppData\Local\SimFleet`
/// (¡disco D:!) y el TEMP heredado por el helper apuntaba dentro de esa
/// carpeta — NSIS la borra/reemplaza mid-install y el log/script
/// desaparecían (por eso los relaunch fallaban silenciosamente).
///
/// Estrategia (en orden de preferencia):
///   1. `%LOCALAPPDATA%\SimFleet\updater-cache\` — user-scoped, fuera
///      del install dir, escritible sin admin.
///   2. `%USERPROFILE%\AppData\Local\SimFleet\updater-cache\` — fallback
///      cuando `LOCALAPPDATA` no está expuesta (raro en Windows
///      moderno pero defensivo).
///   3. `%TEMP%` (la sobrescrita) — último recurso.
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
/// ¿Estamos en un build LOCAL de desarrollo (no una instalación real)?
/// En ese caso NO ofrecemos updates: el instalador NSIS reemplaza el exe
/// en la ruta de instalación REGISTRADA, nunca un binario que corre desde
/// `target\debug` o `target\release`. Así que "Instalar" jamás actualiza
/// ese binario y el banner reaparece en cada arranque — el loop infinito
/// que reportó el usuario ("por más que le doy a install vuelve a salir").
/// Sólo las instalaciones reales pueden auto-actualizarse.
///
/// Para probar el flujo de update desde un build local: exportá
/// `SIMFLEET_FORCE_UPDATE_CHECK=1`.
fn is_local_dev_build() -> bool {
    if std::env::var_os("SIMFLEET_FORCE_UPDATE_CHECK").is_some() {
        return false;
    }
    if cfg!(debug_assertions) {
        return true;
    }
    if let Ok(exe) = std::env::current_exe() {
        let p = exe.to_string_lossy().to_lowercase().replace('\\', "/");
        if p.contains("/target/debug/") || p.contains("/target/release/") {
            return true;
        }
    }
    false
}

#[tauri::command]
pub async fn check_for_update(
    state: tauri::State<'_, AppState>,
) -> Result<Option<UpdateInfo>, String> {
    if is_local_dev_build() {
        tracing::info!(
            target: "updater",
            "build local de desarrollo (target/ o debug) — se omite el chequeo de updates para evitar el loop de 'actualización pendiente'; usá SIMFLEET_FORCE_UPDATE_CHECK=1 para forzarlo"
        );
        return Ok(None);
    }
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

/// Descarga el instalador del release y delega TODO el resto a un
/// **único proceso orquestador** (PowerShell detached). Después cierra
/// la app para que el instalador pueda reemplazar el `.exe` en disco.
///
/// (v3.25.0) **Reescritura del relaunch** — antes lanzábamos el
/// instalador NSIS y, en paralelo, un helper que adivinaba con
/// timestamps cuándo había terminado. Eso fallaba por 3 causas raíz
/// confirmadas con los logs del usuario:
///   1. La app está instalada en `D:\...\AppData\Local\SimFleet`, pero
///      el helper buscaba el exe en rutas hardcodeadas de `C:`
///      (`%LOCALAPPDATA%\Programs\...`) que no existen.
///   2. El helper heredaba el `TEMP` redirigido al data dir portable
///      (DENTRO del install dir en D:) y escribía su log/temporales ahí
///      — NSIS borra esa carpeta mid-install, así que el log nunca
///      llegaba a disco (aparecía "no existe") y no había forma de
///      diagnosticar.
///   3. Lanzar instalador + helper en paralelo = race: el helper podía
///      abrir el exe VIEJO justo cuando NSIS lo mataba.
///
/// El orquestador nuevo resuelve las 3:
///   · Vive y loguea en `updater-cache` (C:, fuera del install dir).
///   · Recibe el `current_exe()` real (autoridad, sea C: o D:) + el PID.
///   · Es SECUENCIAL con esperas reales: espera a que la app cierre →
///     corre el instalador con `-Wait` → confirma que el exe quedó en la
///     versión nueva → lo relanza y lo trae al frente.
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

    // (v7 fix, security) Defensa en profundidad: sólo descargamos/ejecutamos
    // assets alojados en GitHub. `asset_url` llega desde el frontend; sin esta
    // validación de host, una URL a otro dominio (release/asset comprometido o
    // redirigido) terminaría en ejecución SILENCIOSA (/S) de un binario
    // arbitrario tras cerrar la app. (El ideal — verificar SHA-256/firma —
    // requiere publicar el hash en el release; esto es el mínimo self-contained.)
    let host_ok = tauri::Url::parse(&asset_url)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_ascii_lowercase()))
        .map(|h| h == "github.com" || h.ends_with(".githubusercontent.com"))
        .unwrap_or(false);
    if !host_ok {
        tracing::error!(target: "updater", "asset_url con host NO confiable: {asset_url}");
        return Err(crate::tr!("updater.urlNotGithub"));
    }

    let resp = state
        .http
        .get(&asset_url)
        .header("User-Agent", "SimFleet/updater")
        .send()
        .await
        .map_err(|e| {
            tracing::error!(target: "updater", "GET asset falló: {e:#}");
            crate::tr!("updater.downloadFailed", e = e)
        })?;
    if !resp.status().is_success() {
        return Err(crate::tr!("updater.downloadFailedHttp", status = resp.status()));
    }
    let total_bytes = resp.content_length();

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
        crate::tr!("updater.tempCreateFailed", e = e)
    })?;

    use futures_util::StreamExt;
    use tokio::io::AsyncWriteExt;
    let mut stream = resp.bytes_stream();
    let mut downloaded: u64 = 0;
    while let Some(chunk_res) = stream.next().await {
        let chunk = chunk_res.map_err(|e| {
            tracing::error!(target: "updater", "stream chunk falló: {e}");
            crate::tr!("updater.downloadInterrupted", e = e)
        })?;
        file.write_all(&chunk).await.map_err(|e| {
            tracing::error!(target: "updater", "write chunk falló: {e}");
            crate::tr!("updater.chunkWriteFailed", e = e)
        })?;
        downloaded += chunk.len() as u64;

        // Emitir progreso. Si falla, no es fatal — sólo perdemos un tick.
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

    // Path AUTORITATIVO del exe a relanzar: el que está corriendo AHORA.
    // Resuelve correctamente sin importar el disco (C: o D:) porque viene
    // de `std::env::current_exe()`, no de rutas adivinadas.
    let current_exe = std::env::current_exe().map_err(|e| {
        tracing::error!(target: "updater", "current_exe falló: {e}");
        crate::tr!("updater.currentExeFailed", e = e)
    })?;
    let exe_str = current_exe.to_string_lossy().into_owned();
    let exe_clean = exe_str
        .strip_prefix(r"\\?\")
        .unwrap_or(&exe_str)
        .to_string();

    // Lanzar el orquestador (instalar + esperar + relanzar) en un único
    // proceso detached. Loguea en `cache_dir\relaunch.log`.
    launch_update_orchestrator(
        &cache_dir,
        &installer_path,
        ext,
        &exe_clean,
        std::process::id(),
        env!("CARGO_PKG_VERSION"),
        auto_restart,
    )
    .map_err(|e| {
        tracing::error!(target: "updater", "no se pudo lanzar el orquestador: {e}");
        crate::tr!("updater.launchInstallerFailed", e = e)
    })?;
    tracing::info!(
        target: "updater",
        "orquestador de update lanzado (exe={}, pid={}, auto_restart={})",
        exe_clean,
        std::process::id(),
        auto_restart
    );

    // Breve margen para que el orquestador arranque y registre el PID
    // antes de que cerremos. Luego cerramos para liberar el lock del .exe.
    tokio::time::sleep(std::time::Duration::from_millis(600)).await;
    tracing::info!(target: "updater", "exit(0) — el orquestador instala y relanza");
    app.exit(0);
    Ok(())
}

/// Escribe un script `.ps1` orquestador en `cache_dir` y lo lanza
/// detached. El script:
///   1. Espera a que el proceso de la app (`$AppPid`) termine.
///   2. Corre el instalador y **espera** a que finalice (`-Wait`).
///   3. Verifica (poll) que el exe quedó en la versión nueva / reescrito.
///   4. Lanza el exe nuevo y lo trae al frente.
///
/// Loguea cada paso en `cache_dir\relaunch.log` (C:, estable). Se lanza con
/// `-EncodedCommand` (inmune a ExecutionPolicy restrictiva por GPO) y con
/// stdin/stdout/stderr a **null** — sin esto PowerShell hereda los handles
/// inválidos de la app GUI (sin consola) y MUERE al inicializar sus streams
/// (el proceso se creaba pero moría antes de ejecutar; era la causa de que
/// el relaunch fallara en cualquier equipo). Sobrevive al `exit(0)` con
/// `CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW`. `TEMP`/`TMP` → `cache_dir`
/// (el heredado apuntaba al install dir en D:, que NSIS borra).
#[cfg(target_os = "windows")]
fn launch_update_orchestrator(
    cache_dir: &PathBuf,
    installer: &PathBuf,
    ext: &str,
    exe_path: &str,
    app_pid: u32,
    old_version: &str,
    auto_restart: bool,
) -> std::io::Result<()> {
    use std::os::windows::process::CommandExt;
    // (v3.27.0) SIN DETACHED_PROCESS: combinado con CREATE_NO_WINDOW rompía
    // el arranque de powershell.exe (el proceso se creaba pero moría antes
    // de ejecutar — ni dejaba log). CREATE_NO_WINDOW ya lo oculta; el hijo
    // sobrevive al exit(0) porque Windows no mata hijos al cerrar el padre.
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let log_path = cache_dir.join("relaunch.log");
    let script_path = cache_dir.join("simfleet-update.ps1");

    // Escapado para strings PowerShell single-quoted: ' → ''.
    let esc = |s: &str| s.replace('\'', "''");
    let installer_s = esc(&installer.to_string_lossy());
    let exe_s = esc(exe_path);
    let log_s = esc(&log_path.to_string_lossy());
    let old_s = esc(old_version);
    let relaunch_lit = if auto_restart { "$true" } else { "$false" };

    // Valores horneados como variables PowerShell al inicio del script —
    // evita el quoting frágil de pasar args a `-File`.
    let script = format!(
        r#"$ErrorActionPreference = 'Continue'
$AppPid = {pid}
$Installer = '{installer}'
$Ext = '{ext}'
$ExePath = '{exe}'
$OldVer = '{old}'
$Relaunch = {relaunch}
$LogPath = '{log}'

function Log($m) {{
  try {{ "$(Get-Date -Format o) [updater] $m" | Add-Content -LiteralPath $LogPath -ErrorAction SilentlyContinue }} catch {{ }}
}}
Log ("=== orchestrator start pid={{0}} ext={{1}} exe='{{2}}' oldVer={{3}} relaunch={{4}}" -f $AppPid, $Ext, $ExePath, $OldVer, $Relaunch)

# 1) Esperar a que la app cierre (libera el lock del .exe).
$deadline = (Get-Date).AddSeconds(30)
while ((Get-Date) -lt $deadline) {{
  if (-not (Get-Process -Id $AppPid -ErrorAction SilentlyContinue)) {{ break }}
  Start-Sleep -Milliseconds 300
}}
if (Get-Process -Id $AppPid -ErrorAction SilentlyContinue) {{
  Log 'WARN: la app sigue viva tras 30s; continuo igual'
}} else {{
  Log 'app cerrada'
}}

# 2) Correr el instalador y ESPERAR a que termine.
$startInstall = Get-Date
try {{
  if ($Ext -eq 'msi') {{
    $proc = Start-Process 'msiexec.exe' -ArgumentList @('/i', ('"' + $Installer + '"'), '/quiet', '/norestart') -Wait -PassThru -ErrorAction Stop
  }} else {{
    $proc = Start-Process -FilePath $Installer -ArgumentList '/S' -Wait -PassThru -ErrorAction Stop
  }}
  Log ("instalador terminó exitCode={{0}}" -f $proc.ExitCode)
}} catch {{
  Log ("ERROR lanzando instalador: {{0}}" -f $_)
}}

if (-not $Relaunch) {{
  Log 'relaunch deshabilitado por el usuario; fin'
  return
}}

# 3) Confirmar que el exe quedó en la versión NUEVA (o reescrito). NSIS a
#    veces retorna del -Wait antes de soltar el archivo, o se relanza a sí
#    mismo; este poll cubre ambos casos.
function NormVer($v) {{
  if (-not $v) {{ return '' }}
  $p = ($v.ToString().Trim() -split '\.')
  if ($p.Count -ge 3) {{ return ($p[0..2] -join '.') }}
  return $v.ToString().Trim()
}}
$oldNorm = NormVer $OldVer
$waitExe = (Get-Date).AddSeconds(90)
$ready = $false
while ((Get-Date) -lt $waitExe) {{
  if (Test-Path -LiteralPath $ExePath) {{
    try {{
      $item = Get-Item -LiteralPath $ExePath -ErrorAction Stop
      $verNorm = NormVer $item.VersionInfo.ProductVersion
      $isNew = ($verNorm -ne '' -and $oldNorm -ne '' -and $verNorm -ne $oldNorm)
      $isFresh = ($item.LastWriteTime -gt $startInstall)
      if ($isNew -or $isFresh) {{
        $ready = $true
        Log ("exe listo ver='{{0}}' old='{{1}}' fresh={{2}}" -f $verNorm, $oldNorm, $isFresh)
        break
      }}
    }} catch {{ Log ("error get-item: {{0}}" -f $_) }}
  }}
  Start-Sleep -Milliseconds 500
}}
if (-not $ready) {{ Log 'WARN: no detecté el exe actualizado en 90s; intento lanzar igual' }}

# settle: dar 1.5s a NSIS para soltar el lock del archivo recién escrito.
Start-Sleep -Milliseconds 1500

# 4) Lanzar el exe nuevo. Candidatos: el path original + el
#    InstallLocation del registro (por si el dir cambió).
$targets = New-Object System.Collections.Generic.List[string]
$targets.Add($ExePath)
try {{
  $unKeys = @(
    'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall',
    'HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall',
    'HKLM:\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall'
  )
  foreach ($base in $unKeys) {{
    if (Test-Path $base) {{
      Get-ChildItem $base -ErrorAction SilentlyContinue | ForEach-Object {{
        $pp = Get-ItemProperty $_.PSPath -ErrorAction SilentlyContinue
        if ($pp -and $pp.DisplayName -eq 'SimFleet' -and $pp.InstallLocation) {{
          $loc = $pp.InstallLocation.Trim().Trim('"')
          $cand = Join-Path $loc 'simfleet.exe'
          if (-not $targets.Contains($cand)) {{ $targets.Add($cand) }}
        }}
      }}
    }}
  }}
}} catch {{ Log ("registro fallback falló: {{0}}" -f $_) }}

$launched = $null
foreach ($t in $targets) {{
  if (Test-Path -LiteralPath $t) {{
    try {{
      $np = Start-Process -FilePath $t -PassThru -ErrorAction Stop
      $launched = $np
      Log ("relanzado '{{0}}' pid={{1}}" -f $t, $np.Id)
      break
    }} catch {{ Log ("fallo lanzar '{{0}}': {{1}}" -f $t, $_) }}
  }} else {{
    Log ("candidato no existe: '{{0}}'" -f $t)
  }}
}}

if ($launched -ne $null) {{
  # Traer la ventana al frente (sin esto puede arrancar detrás de otras).
  Start-Sleep -Seconds 2
  try {{
    Add-Type -AssemblyName Microsoft.VisualBasic -ErrorAction SilentlyContinue
    [Microsoft.VisualBasic.Interaction]::AppActivate($launched.Id) | Out-Null
    Log ("foreground OK pid={{0}}" -f $launched.Id)
  }} catch {{
    Log ("foreground falló pid={{0}}: {{1}}" -f $launched.Id, $_)
  }}
}} else {{
  Log 'ERROR: ningún candidato se pudo relanzar'
}}
Log '=== orchestrator fin'
"#,
        pid = app_pid,
        installer = installer_s,
        ext = ext,
        exe = exe_s,
        old = old_s,
        relaunch = relaunch_lit,
        log = log_s,
    );

    // Escribir el script a disco (respaldo + diagnóstico manual).
    std::fs::write(&script_path, &script)?;

    // (v3.27.0) Lanzar vía -EncodedCommand (base64 de UTF-16LE) en vez de
    // -File: inmune a ExecutionPolicy restrictiva por GPO (equipos de otros
    // usuarios) y a cualquier problema de encoding/BOM del .ps1.
    let encoded = {
        use base64::Engine;
        let utf16: Vec<u8> = script
            .encode_utf16()
            .flat_map(|u| u.to_le_bytes())
            .collect();
        base64::engine::general_purpose::STANDARD.encode(utf16)
    };

    let mut cmd = std::process::Command::new("powershell");
    cmd.args([
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-EncodedCommand",
    ])
    .arg(&encoded)
    // Override del TEMP heredado (apuntaba al install dir en D:, que NSIS
    // borra). El cache_dir vive en C:\…\SimFleet\updater-cache, estable.
    .env("TEMP", cache_dir)
    .env("TMP", cache_dir)
    // CRÍTICO: null en los 3 streams. SimFleet es una app GUI sin consola;
    // sin esto PowerShell hereda handles inválidos y muere al inicializar.
    .stdin(std::process::Stdio::null())
    .stdout(std::process::Stdio::null())
    .stderr(std::process::Stdio::null())
    .creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
    cmd.spawn().map(|_| ())
}

#[cfg(not(target_os = "windows"))]
fn launch_update_orchestrator(
    _cache_dir: &PathBuf,
    _installer: &PathBuf,
    _ext: &str,
    _exe_path: &str,
    _app_pid: u32,
    _old_version: &str,
    _auto_restart: bool,
) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        crate::tr!("updater.autoInstallWindowsOnly"),
    ))
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
