//! Integración con **PMDG Operations Center** para instalar liveries
//! `.ptp`.
//!
//! ## Por qué este módulo no descifra los `.ptp` directamente
//!
//! Los `.ptp` no son ZIP — son archivos en el formato propietario
//! `CRYP` (los primeros 4 bytes son literalmente `CRYP`). PMDG los
//! descifra con `Rfc2898DeriveBytes` + AES usando una clave que
//! sólo está en su binario `PMDG Operations Center.exe`. No
//! tenemos esa clave (ni queremos copiarla — sería piratería).
//!
//! ## Estrategia
//!
//! 1. **Detectar PMDG OC** en los paths típicos del sistema y
//!    en `HKCR\.ptp\OpenWithProgids` (file association).
//! 2. Para cada `.ptp`, **lanzar `PMDG Operations Center.exe`
//!    pasando el path como argumento** — exactamente lo que pasa
//!    al hacer doble-click sobre un `.ptp` en Windows.
//! 3. PMDG OC abre su flujo de import (descifra, parsea, copia
//!    a `<community>/pmdg-aircraft-XXX-liveries/SimObjects/Airplanes/...`,
//!    actualiza `layout.json` y `manifest.json` del paquete
//!    `-liveries`, todo en su .NET interno con SmartAssembly).
//! 4. Si OC tiene `AutoinstallLiveries=Yes` (en
//!    `%APPDATA%\PMDG\PMDG Operations Center\v2settings.ini`),
//!    el import es totalmente silencioso. Si está en `Prompt`
//!    (default), el usuario ve el dialog y aprueba.
//!
//! ## Resultado para el usuario
//!
//! - "✓ 7 liveries enviadas a PMDG Operations Center — confirma
//!   la instalación en su ventana." (cuando se encontró OC)
//! - "PMDG Operations Center no se detectó. Liveries en el Inbox
//!   — abre OC manualmente y arrástralas." (cuando no)

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

/// Reporte de qué pasó al intentar enviar una livery a PMDG OC.
#[derive(Debug, Clone, Serialize)]
pub struct PmdgInstallReport {
    /// Path absoluto al `PMDG Operations Center.exe` que lanzamos.
    /// Sirve como "prueba" para el usuario de adónde se envió.
    pub oc_executable: PathBuf,
    /// Path al `.ptp` que pasamos como argumento.
    pub ptp_path: PathBuf,
    /// `true` si conseguimos hacer `Command::spawn` (no implica
    /// que la livery se haya instalado — depende de OC y de si
    /// el usuario aprobó el prompt).
    pub launched: bool,
}

/// Punto de entrada principal — para una `.ptp` ya copiada al
/// inbox, intenta lanzar PMDG Operations Center con ella.
///
/// Devuelve:
///   · `Ok(Some(report))` si encontramos OC y conseguimos spawn.
///     El usuario verá la ventana de PMDG OC con la livery cargada.
///   · `Ok(None)` si no encontramos PMDG OC en el sistema.
///     La livery queda en el inbox manual.
///   · `Err(_)` ante fallos catastróficos (permisos, etc.).
///
/// Los argumentos `aircraft` y `community` se mantienen por
/// compatibilidad con el flujo anterior (firma estable) pero ya
/// no los usamos — PMDG OC sabe internamente a qué carpeta de
/// Community va cada livery (vía su `v2settings.ini`).
#[allow(unused_variables)]
pub fn install_livery(
    ptp_path: &Path,
    aircraft: &str,
    community: &Path,
    pmdg_oc_override: Option<&str>,
) -> anyhow::Result<Option<PmdgInstallReport>> {
    let Some(oc_exe) = find_pmdg_operations_center(pmdg_oc_override) else {
        tracing::info!(
            target: "install",
            "PMDG OC no encontrado en paths típicos — livery '{}' queda en inbox",
            ptp_path.display()
        );
        return Ok(None);
    };
    tracing::info!(
        target: "install",
        "PMDG OC detectado en: {}",
        oc_exe.display()
    );

    let launched = launch_oc_with_ptp(&oc_exe, ptp_path)?;
    Ok(Some(PmdgInstallReport {
        oc_executable: oc_exe,
        ptp_path: ptp_path.to_path_buf(),
        launched,
    }))
}

/// Busca `PMDG Operations Center.exe`. Prioridad:
///
///   1. **Override del usuario** vía setting `pmdg_oc_path` —
///      el frontend lo expone en Settings. Si está poblado y el
///      archivo existe, lo usamos directamente.
///   2. **Variable de entorno** `MSFS_PMDG_OC` (escape hatch para
///      power users).
///   3. **Paths típicos de instalación** en ambos Program Files.
///   4. **Carpetas comunes del usuario** (Downloads, Desktop,
///      Documents, AppData) — pero NO sólo en `C:` — escaneamos
///      todos los drives fijos. Esto es CRÍTICO para el caso del
///      usuario que tiene OC portable en `D:\Downloads\...`
///      cuando su `%USERPROFILE%\Downloads` está en `C:`.
///   5. **Registry `HKCR\.ptp`** para file association registrada
///      por el instalador oficial.
pub fn find_pmdg_operations_center(setting_override: Option<&str>) -> Option<PathBuf> {
    // 1) Override del usuario.
    if let Some(override_path) = setting_override {
        let p = PathBuf::from(override_path);
        if p.is_file() {
            tracing::info!(
                target: "install",
                "PMDG OC desde setting de usuario: {}",
                p.display()
            );
            return Some(p);
        } else if !override_path.is_empty() {
            tracing::warn!(
                target: "install",
                "Setting pmdg_oc_path apunta a '{}' que NO existe",
                override_path
            );
        }
    }

    // 2) ENV var.
    if let Ok(custom) = std::env::var("MSFS_PMDG_OC") {
        let p = PathBuf::from(custom);
        if p.is_file() {
            return Some(p);
        }
    }

    // 3+4) Construir lista completa de candidatos.
    let candidates = build_oc_candidate_paths();
    for c in &candidates {
        if c.is_file() {
            tracing::info!(
                target: "install",
                "PMDG OC encontrado: {}",
                c.display()
            );
            return Some(c.clone());
        }
    }

    // 5) Registry.
    if let Some(p) = find_oc_via_registry() {
        return Some(p);
    }

    tracing::info!(
        target: "install",
        "PMDG OC no encontrado. Paths probados: {}",
        candidates.len()
    );
    None
}

/// Lista completa de paths donde podría estar OC.exe. Función
/// expuesta para diagnóstico — el frontend la usa para que el
/// usuario sepa qué se intentó si la detección falla.
pub fn build_oc_candidate_paths() -> Vec<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();

    // Program Files (ambos) + variantes de subpath.
    for env_var in &["ProgramFiles", "ProgramFiles(x86)"] {
        if let Some(base) = std::env::var_os(env_var) {
            for sub in &[
                r"PMDG\Operations Center\PMDG Operations Center.exe",
                r"PMDG\PMDG Operations Center\PMDG Operations Center.exe",
            ] {
                candidates.push(Path::new(&base).join(sub));
            }
        }
    }

    // Carpetas del USERPROFILE.
    if let Some(home) = std::env::var_os("USERPROFILE") {
        let home_path = PathBuf::from(&home);
        for sub in &[
            r"Downloads\PMDG Operations Center\PMDG Operations Center.exe",
            r"Documents\PMDG\Operations Center\PMDG Operations Center.exe",
            r"Documents\PMDG Operations Center\PMDG Operations Center.exe",
            r"Desktop\PMDG Operations Center\PMDG Operations Center.exe",
            r"AppData\Local\Programs\PMDG\Operations Center\PMDG Operations Center.exe",
        ] {
            candidates.push(home_path.join(sub));
        }
    }

    // **Multi-drive scan** — el bug que reportó el usuario.
    // Tenía OC en `D:\Downloads\PMDG Operations Center\` (drive
    // distinto al USERPROFILE). Por eso escaneamos TODOS los
    // drives fijos del sistema con los patrones más comunes.
    for drive in available_fixed_drives() {
        for sub in &[
            r"Downloads\PMDG Operations Center\PMDG Operations Center.exe",
            r"PMDG Operations Center\PMDG Operations Center.exe",
            r"PMDG\Operations Center\PMDG Operations Center.exe",
            r"PMDG\PMDG Operations Center\PMDG Operations Center.exe",
            r"Games\PMDG Operations Center\PMDG Operations Center.exe",
            r"MSFS\PMDG Operations Center\PMDG Operations Center.exe",
        ] {
            candidates.push(Path::new(&drive).join(sub));
        }
    }

    candidates
}

/// Drives fijos del sistema. En Windows enumera `C:`, `D:`, etc.
/// que existan. Limita a un puñado para no ser absurdamente lento.
#[cfg(target_os = "windows")]
fn available_fixed_drives() -> Vec<String> {
    let mut out = Vec::new();
    // Las letras A..Z mayúsculas — 26 candidatos, baratos de
    // chequear (un metadata syscall por cada).
    for c in b'C'..=b'Z' {
        let letter = c as char;
        let drive = format!("{}:\\", letter);
        if Path::new(&drive).is_dir() {
            out.push(drive);
        }
    }
    out
}

#[cfg(not(target_os = "windows"))]
fn available_fixed_drives() -> Vec<String> {
    Vec::new()
}

/// Resultado del comando de diagnóstico. La UI lo muestra en
/// Settings para que el usuario sepa qué se probó si la detección
/// automática falló.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OcDetectionReport {
    /// Path detectado o None si no se encontró.
    pub detected_path: Option<String>,
    /// Si vino de un setting de usuario.
    pub from_setting: bool,
    /// Paths candidatos que se exploraron (para diagnóstico).
    pub tried_paths: Vec<String>,
}

/// Diagnóstico expuesto al frontend. Devuelve el path detectado +
/// los paths que se chequearon (con flag de existencia) para que
/// el usuario pueda ver por qué falló y dónde apuntar el setting.
pub fn diagnose_oc_detection(setting_override: Option<&str>) -> OcDetectionReport {
    let from_setting = setting_override
        .map(|p| {
            let path = PathBuf::from(p);
            !p.is_empty() && path.is_file()
        })
        .unwrap_or(false);

    let detected = find_pmdg_operations_center(setting_override);
    let candidates = build_oc_candidate_paths();
    let tried_paths: Vec<String> = candidates
        .iter()
        .map(|p| {
            let exists = if p.is_file() { "✓" } else { "✗" };
            format!("{} {}", exists, p.display())
        })
        .collect();

    OcDetectionReport {
        detected_path: detected.map(|p| p.to_string_lossy().into_owned()),
        from_setting,
        tried_paths,
    }
}

/// Lee el ProgID asociado a `.ptp` en `HKCR` y resuelve su
/// `shell\open\command` para sacar el `.exe`. Funciona cuando
/// PMDG OC fue instalado con su instalador oficial (que registra
/// la file association).
#[cfg(target_os = "windows")]
fn find_oc_via_registry() -> Option<PathBuf> {
    use std::process::Command;

    // `reg query` es la forma más portable de leer registry
    // sin pulling crates extras (`winreg` añadiría 50KB al
    // binario por una sola lookup).
    let outputs = [
        // ProgID directo en HKEY_CLASSES_ROOT
        ["HKCR\\.ptp", ""],
        // Per-user en HKCU\Software\Classes
        ["HKCU\\Software\\Classes\\.ptp", ""],
    ];
    for [key, _] in &outputs {
        let out = Command::new("reg")
            .args(["query", key, "/ve"])
            .output()
            .ok()?;
        let stdout = String::from_utf8_lossy(&out.stdout);
        // Buscamos "(Default) REG_SZ <progid>" en la salida.
        for line in stdout.lines() {
            let line_lower = line.to_lowercase();
            if !line_lower.contains("reg_sz") {
                continue;
            }
            let progid = line
                .split("REG_SZ")
                .nth(1)
                .map(str::trim)
                .unwrap_or("");
            if progid.is_empty() {
                continue;
            }
            // Resolver `<progid>\shell\open\command`.
            let cmd_key = format!("HKCR\\{}\\shell\\open\\command", progid);
            let cmd_out = Command::new("reg")
                .args(["query", &cmd_key, "/ve"])
                .output()
                .ok()?;
            let cmd_stdout = String::from_utf8_lossy(&cmd_out.stdout);
            for cmd_line in cmd_stdout.lines() {
                if !cmd_line.to_lowercase().contains("reg_sz") {
                    continue;
                }
                let raw = cmd_line.split("REG_SZ").nth(1)?.trim();
                // El valor es típicamente:
                //   "C:\Program Files\PMDG\PMDG Operations Center.exe" "%1"
                // Extraemos lo entre las primeras comillas.
                let mut chars = raw.chars();
                if chars.next() != Some('"') {
                    continue;
                }
                let exe_path: String = chars.take_while(|c| *c != '"').collect();
                let p = PathBuf::from(&exe_path);
                if p.is_file() {
                    tracing::debug!(
                        target: "install",
                        "PMDG OC encontrado vía registry .ptp file assoc: {}",
                        p.display()
                    );
                    return Some(p);
                }
            }
        }
    }
    None
}

#[cfg(not(target_os = "windows"))]
fn find_oc_via_registry() -> Option<PathBuf> {
    None
}

/// Lanza PMDG OC pasando el `.ptp` como argumento. Usa
/// `DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP` (Windows) para
/// que OC sobreviva al proceso de la app si esta cierra. No
/// awaiteamos el output — OC corre en su propia ventana hasta
/// que el usuario lo cierre.
#[cfg(target_os = "windows")]
fn launch_oc_with_ptp(oc_exe: &Path, ptp: &Path) -> anyhow::Result<bool> {
    use std::os::windows::process::CommandExt;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;

    let result = std::process::Command::new(oc_exe)
        .arg(ptp)
        .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP)
        .spawn();
    match result {
        Ok(_) => {
            tracing::info!(
                target: "install",
                "PMDG OC lanzado con livery: {}",
                ptp.display()
            );
            Ok(true)
        }
        Err(e) => {
            tracing::warn!(
                target: "install",
                "no se pudo lanzar PMDG OC para {}: {}",
                ptp.display(),
                e
            );
            anyhow::bail!("spawn de PMDG OC falló: {}", e)
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn launch_oc_with_ptp(_oc_exe: &Path, _ptp: &Path) -> anyhow::Result<bool> {
    anyhow::bail!("PMDG OC sólo está disponible en Windows")
}

/// Hint para el usuario sobre dónde poner OC si no se encontró.
/// Usado para construir mensajes de error útiles.
pub fn search_paths_hint() -> String {
    let mut paths = Vec::new();
    for env_var in &["ProgramFiles", "ProgramFiles(x86)"] {
        if let Some(base) = std::env::var_os(env_var) {
            paths.push(format!(
                r"{}\PMDG\Operations Center\PMDG Operations Center.exe",
                base.to_string_lossy()
            ));
        }
    }
    if let Some(home) = std::env::var_os("USERPROFILE") {
        paths.push(format!(
            r"{}\Downloads\PMDG Operations Center\PMDG Operations Center.exe",
            home.to_string_lossy()
        ));
    }
    paths.join(" o ")
}

// Función legacy helper que el módulo `install/mod.rs` no usaba pero
// dejamos por si alguien quiere reactivar la copia al inbox legacy.
#[allow(dead_code)]
pub(crate) fn ensure_dir(p: &Path) -> std::io::Result<()> {
    if !p.exists() {
        fs::create_dir_all(p)?;
    }
    Ok(())
}
