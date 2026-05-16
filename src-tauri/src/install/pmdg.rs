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
) -> anyhow::Result<Option<PmdgInstallReport>> {
    let Some(oc_exe) = find_pmdg_operations_center() else {
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

/// Busca `PMDG Operations Center.exe` en los lugares conocidos:
///
///   1. **Registry**: `HKCR\.ptp\OpenWithProgids` → ProgID →
///      `HKCR\<ProgID>\shell\open\command` da el path exacto al
///      `.exe` registrado por el instalador oficial. Es la fuente
///      más fiable cuando OC se instaló normalmente.
///   2. **Paths estándar de instalación**: `Program Files (x86)\
///      PMDG\Operations Center\` y variantes.
///   3. **Downloads / portable**: muchos usuarios bajan el ZIP
///      de OC y lo dejan en `Downloads\PMDG Operations Center\`.
///   4. **Variable de entorno** `MSFS_PMDG_OC` — escape hatch
///      para usuarios con instalación custom.
fn find_pmdg_operations_center() -> Option<PathBuf> {
    // 1) Variable de entorno (escape hatch).
    if let Ok(custom) = std::env::var("MSFS_PMDG_OC") {
        let p = PathBuf::from(custom);
        if p.is_file() {
            return Some(p);
        }
    }

    // 2) Paths típicos.
    let mut candidates: Vec<PathBuf> = Vec::new();
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
    if let Some(home) = std::env::var_os("USERPROFILE") {
        let home_path = PathBuf::from(&home);
        for sub in &[
            r"Downloads\PMDG Operations Center\PMDG Operations Center.exe",
            r"Documents\PMDG\Operations Center\PMDG Operations Center.exe",
            r"Desktop\PMDG Operations Center\PMDG Operations Center.exe",
            r"AppData\Local\Programs\PMDG\Operations Center\PMDG Operations Center.exe",
        ] {
            candidates.push(home_path.join(sub));
        }
    }
    for c in &candidates {
        if c.is_file() {
            tracing::debug!(target: "install", "PMDG OC encontrado en path típico: {}", c.display());
            return Some(c.clone());
        }
    }

    // 3) Registry: `.ptp` file association.
    if let Some(p) = find_oc_via_registry() {
        return Some(p);
    }

    None
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
