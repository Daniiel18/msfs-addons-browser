//! (v6 #1) Cross-linking MSFS 2020 ↔ 2024 mediante junctions NTFS.
//!
//! Cuando el usuario tiene **ambas** versiones de MSFS instaladas y descarga
//! un escenario rotulado "MSFS 2020/2024", tras instalarlo en la Community de
//! la versión ACTIVA ofrecemos crear un *junction* (symlink de directorio) en
//! la Community de la OTRA versión. Así el mismo paquete aparece en ambos sims
//! sin duplicar GBs en disco — el junction sólo apunta a la carpeta original.
//!
//! Usamos `mklink /J` (junction) en lugar de
//! [`std::os::windows::fs::symlink_dir`] a propósito: el junction NO requiere
//! privilegios de administrador ni "Developer Mode" activado, mientras que el
//! symlink de directorio sí. Para nuestro caso (enlazar carpetas en el mismo
//! equipo) el junction es funcionalmente idéntico.

use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::community;
use crate::install::InstalledPackage;

/// Un paquete recién instalado, candidato a enlazarse en la otra Community.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CrossLinkPkg {
    /// Nombre de la carpeta del paquete dentro de Community (será el nombre
    /// del junction en el destino).
    pub name: String,
    /// Ruta absoluta del paquete instalado (origen del junction).
    pub source_path: String,
}

/// Oferta de cross-link que el backend emite al frontend tras una instalación.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CrossLinkOffer {
    /// `"msfs2020"` | `"msfs2024"` — la OTRA versión (destino del enlace).
    pub other_version: String,
    /// Etiqueta legible para el modal (`"MSFS 2020"` / `"MSFS 2024"`).
    pub other_label: String,
    /// Carpeta Community destino donde se crearán los junctions.
    pub other_community: String,
    /// Paquetes a enlazar.
    pub packages: Vec<CrossLinkPkg>,
}

/// ¿El título del addon indica compatibilidad doble 2020 + 2024?
/// SceneryAddons rotula los posts como "MSFS 2020", "MSFS 2024" o
/// "MSFS 2020/2024" — sólo los que mencionan AMBOS años son enlazables.
pub fn is_dual_compat(title: &str) -> bool {
    let t = title.to_lowercase();
    t.contains("2020") && t.contains("2024")
}

/// Construye una oferta de cross-link SI:
///   · hay 2020 **y** 2024 instalados, y
///   · `compat_label` marca doble compatibilidad, y
///   · hay al menos un paquete y se localiza la Community de la otra versión.
///
/// `compat_label` debe ser la etiqueta de compatibilidad de simulador (campo
/// `simulator`, ej. "MSFS 2020/2024"), NO el nombre limpio del addon — éste no
/// menciona los años y haría que la oferta nunca se dispare.
///
/// Devuelve `None` en cualquier otro caso (no se ofrece nada).
pub fn build_offer(compat_label: &str, packages: &[InstalledPackage]) -> Option<CrossLinkOffer> {
    if packages.is_empty() || !is_dual_compat(compat_label) {
        return None;
    }
    let (has2020, has2024) = community::detect_installed_sims();
    if !(has2020 && has2024) {
        return None;
    }

    // La OTRA versión = la que NO es la activa.
    let active_is_2024 = crate::sim::is_2024();
    let (other_token, other_version, other_label) = if active_is_2024 {
        ("2020", "msfs2020", "MSFS 2020")
    } else {
        ("2024", "msfs2024", "MSFS 2024")
    };

    let all = community::detect_all_community_folders().ok()?;
    // Preferimos una Community de la otra versión que EXISTA; si no, la primera
    // de esa versión (la crearemos al enlazar).
    let other = all
        .iter()
        .find(|c| c.variant.contains(other_token) && c.exists)
        .or_else(|| all.iter().find(|c| c.variant.contains(other_token)))?;

    Some(CrossLinkOffer {
        other_version: other_version.to_string(),
        other_label: other_label.to_string(),
        other_community: other.path.clone(),
        packages: packages
            .iter()
            .map(|p| CrossLinkPkg {
                name: p.name.clone(),
                source_path: p.install_path.clone(),
            })
            .collect(),
    })
}

/// Resultado de crear los junctions.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CrossLinkResult {
    /// Paquetes enlazados con éxito.
    pub linked: Vec<String>,
    /// Paquetes que ya existían en el destino (se respetan, no se tocan).
    pub skipped: Vec<String>,
    /// Fallos como `"nombre: motivo"`.
    pub failed: Vec<String>,
}

/// Crea junctions NTFS en `other_community` apuntando a cada `source_path`.
/// El nombre del link es el nombre del paquete. No sobrescribe nada: si el
/// destino ya existe, se salta (skipped).
pub fn create_junctions(other_community: &str, packages: &[CrossLinkPkg]) -> CrossLinkResult {
    let mut linked = Vec::new();
    let mut skipped = Vec::new();
    let mut failed = Vec::new();

    let base = Path::new(other_community);
    if let Err(e) = std::fs::create_dir_all(base) {
        for p in packages {
            failed.push(format!(
                "{}: no se pudo crear la Community destino ({e})",
                p.name
            ));
        }
        return CrossLinkResult {
            linked,
            skipped,
            failed,
        };
    }

    for p in packages {
        let link = base.join(&p.name);
        if link.exists() {
            // Ya hay algo con ese nombre (carpeta real o junction previo).
            skipped.push(p.name.clone());
            continue;
        }
        if !Path::new(&p.source_path).is_dir() {
            failed.push(format!("{}: la carpeta de origen no existe", p.name));
            continue;
        }
        match make_junction(&link, Path::new(&p.source_path)) {
            Ok(()) => linked.push(p.name.clone()),
            Err(e) => failed.push(format!("{}: {e}", p.name)),
        }
    }

    CrossLinkResult {
        linked,
        skipped,
        failed,
    }
}

#[cfg(windows)]
fn make_junction(link: &Path, target: &Path) -> anyhow::Result<()> {
    use std::os::windows::process::CommandExt;
    use std::process::Command;
    // CREATE_NO_WINDOW — evita el parpadeo de una consola al invocar cmd.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let out = Command::new("cmd")
        .args([
            "/C",
            "mklink",
            "/J",
            &link.to_string_lossy(),
            &target.to_string_lossy(),
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output()?;
    if out.status.success() {
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        let std_out = String::from_utf8_lossy(&out.stdout);
        anyhow::bail!("mklink /J falló: {} {}", err.trim(), std_out.trim());
    }
}

#[cfg(not(windows))]
fn make_junction(link: &Path, target: &Path) -> anyhow::Result<()> {
    std::os::unix::fs::symlink(target, link)?;
    Ok(())
}
