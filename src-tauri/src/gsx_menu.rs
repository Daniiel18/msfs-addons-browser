//! Lector del **menú in-sim de GSX** para rellenar el gate cuando un
//! aeropuerto no tiene perfil de GSX.
//!
//! ## Cómo expone GSX el gate (verificado en el JS del panel)
//!
//! GSX escribe su menú a un archivo de texto plano en disco:
//!
//! ```text
//! <FSDT root>\MSFS\fsdreamteam-gsx-pro\html_ui\InGamePanels\FSDT_GSX_Panel\menu
//! ```
//!
//! (ruta vía registro `HKCU\Software\Fsdreamteam\root`). **Línea 0 = el
//! título** del menú; líneas 1..N = las opciones.
//!
//! PERO desde GSX Pro ~4.0.x el **gate ya NO está en ese archivo**. El
//! título (línea 0) es solo "Activate Services at \<aeropuerto\>", y el
//! gate ("Gate B 14") viaja en un **slot de string aparte codificado en
//! LVars** (`L:FSDT_GSX_MENU_SUBTITLE_*`), que NUNCA toca el disco — GSX
//! lo hizo a propósito para que las herramientas que parsean `./menu`
//! sigan viendo un título de una sola línea.
//!
//! Por eso aquí combinamos dos fuentes:
//!   · **archivo `menu`** → el TÍTULO, que dice QUÉ menú está abierto
//!     (solo nos interesa el principal: "Activate Services at …").
//!   · **subtítulo (LVar)** → el GATE, leído/decodificado por
//!     [`crate::mobiflight_lvars`] vía el puente WASM de MobiFlight y
//!     pasado a [`gate_from_menu`].
//!
//! El archivo solo tiene contenido mientras el menú de GSX está abierto;
//! el watcher lo lee mientras el avión está parado en tierra.

use std::path::PathBuf;
use std::sync::OnceLock;

/// Ruta relativa (desde el root de FSDT) del archivo de menú de GSX.
const MENU_REL: &str =
    r"MSFS\fsdreamteam-gsx-pro\html_ui\InGamePanels\FSDT_GSX_Panel\menu";

/// Prefijo (en minúsculas) del título del menú **principal** de GSX — el
/// único cuyo subtítulo es el gate al que el avión está asignado. Otros
/// menús ("Select Position at…", pushback, operador) tienen otro
/// subtítulo y NO deben rellenar el gate.
const TITLE_PREFIX: &str = "activate services at";

/// Palabras-tipo de posición de MSFS/GSX. El nombre de gate/parking
/// contiene una de éstas como token. Valida que el subtítulo sea de
/// verdad una posición y no otra cosa.
const POSITION_KEYWORDS: &[&str] = &[
    "gate", "ramp", "parking", "stand", "apron", "dock", "cargo", "remote",
    "hangar", "mil", "military", "fuel", "pad", "tie", "vehicle",
];

/// Resuelve **una sola vez** la ruta al archivo `menu` de GSX vía el
/// registro de Windows. Cacheada en un `OnceLock`.
fn menu_path() -> Option<&'static PathBuf> {
    static PATH: OnceLock<Option<PathBuf>> = OnceLock::new();
    PATH.get_or_init(resolve_menu_path).as_ref()
}

#[cfg(target_os = "windows")]
fn resolve_menu_path() -> Option<PathBuf> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    // El instalador de FSDT guarda el root en HKCU\Software\Fsdreamteam\root.
    let key = match hkcu.open_subkey(r"Software\Fsdreamteam") {
        Ok(k) => k,
        Err(_) => {
            tracing::info!(
                target: "gsx_menu",
                "FSDT no encontrado en el registro (HKCU\\Software\\Fsdreamteam) — \
                 GSX no instalado o instalado por otro usuario"
            );
            return None;
        }
    };
    let root: String = match key.get_value("root") {
        Ok(v) => v,
        Err(_) => return None,
    };
    if root.trim().is_empty() {
        return None;
    }
    let path = PathBuf::from(root).join(MENU_REL);
    tracing::info!(
        target: "gsx_menu",
        "menu file de GSX resuelto: {}",
        path.display()
    );
    Some(path)
}

#[cfg(not(target_os = "windows"))]
fn resolve_menu_path() -> Option<PathBuf> {
    None
}

/// Lee la **primera línea** (título) del archivo de menú de GSX. `None`
/// si GSX no está / el menú está cerrado (archivo vacío).
pub fn read_title() -> Option<String> {
    let path = menu_path()?;
    // Lectura barata: el archivo son unos cientos de bytes.
    let content = std::fs::read_to_string(path).ok()?;
    let title = content.lines().next()?.trim().to_string();
    if title.is_empty() {
        None
    } else {
        Some(title)
    }
}

/// ¿El título corresponde al menú principal ("Activate Services at …")?
pub fn is_services_menu(title: &str) -> bool {
    title.trim().to_lowercase().starts_with(TITLE_PREFIX)
}

/// Combina título (archivo) + subtítulo (LVar) → nombre de gate.
///
/// Devuelve `Some("Gate B 14")` SOLO si el menú abierto es el principal
/// ("Activate Services at …") y el subtítulo parece una posición. `None`
/// en cualquier otro caso (menú cerrado, otro menú, subtítulo no-posición).
pub fn gate_from_menu(subtitle: Option<&str>) -> Option<String> {
    let subtitle = subtitle?;
    let title = read_title()?;
    if !is_services_menu(&title) {
        return None;
    }
    clean_position(subtitle)
}

/// Normaliza y valida un string como nombre de posición de GSX. Colapsa
/// espacios, descarta vacíos / demasiado largos, y exige que contenga una
/// palabra-tipo de posición conocida. `pub(crate)` para testearlo.
pub(crate) fn clean_position(raw: &str) -> Option<String> {
    let cleaned = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if cleaned.is_empty() || cleaned.len() > 40 {
        return None;
    }
    if !cleaned.split_whitespace().any(is_position_keyword) {
        return None;
    }
    Some(cleaned)
}

/// ¿El token (limpio de puntuación) es una palabra-tipo de posición?
fn is_position_keyword(tok: &str) -> bool {
    let tl = tok
        .trim_matches(|c: char| !c.is_alphanumeric())
        .to_lowercase();
    POSITION_KEYWORDS.contains(&tl.as_str())
}

#[cfg(test)]
mod tests {
    use super::{clean_position, is_services_menu};

    #[test]
    fn detects_services_menu() {
        assert!(is_services_menu(
            "Activate Services at MDSD/Las Americas International"
        ));
        assert!(is_services_menu("activate services at KATL/Atlanta"));
        assert!(!is_services_menu("Select Position at KATL/Atlanta"));
        assert!(!is_services_menu("Select pushback direction"));
        assert!(!is_services_menu(""));
    }

    #[test]
    fn cleans_gate_subtitle() {
        assert_eq!(
            clean_position("Gate B 14"),
            Some("Gate B 14".to_string())
        );
        assert_eq!(clean_position("Parking 30"), Some("Parking 30".to_string()));
        assert_eq!(
            clean_position("N Parking 6"),
            Some("N Parking 6".to_string())
        );
        assert_eq!(
            clean_position("  Gate   A1 "),
            Some("Gate A1".to_string())
        );
    }

    #[test]
    fn rejects_non_position_subtitle() {
        // Subtítulos que NO son una posición (otro menú) → None.
        assert_eq!(clean_position("Lufthansa"), None);
        assert_eq!(clean_position("Boeing 737"), None);
        assert_eq!(clean_position(""), None);
        assert_eq!(clean_position("   "), None);
    }
}
