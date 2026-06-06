//! Lector del gate que GSX muestra en su **menú in-sim**, leyendo el
//! archivo `menu` que el panel de GSX escribe en disco.
//!
//! ## Por qué existe
//!
//! Cuando un aeropuerto **no** tiene perfil de GSX (sin `.ini` en
//! `%APPDATA%\Virtuali\GSX\MSFS`), el parser de [`crate::gsx_parking`] no
//! encuentra parking. La API de Facility Data del simulador
//! (`SimConnect_RequestFacilityData` sobre `TAXI_PARKING`) **no devuelve
//! nada** en MSFS 2020 — por eso en su día la abandonamos, y el usuario
//! confirmó que el fallback de v4.2.0 no funciona.
//!
//! Pero GSX **siempre** sabe en qué gate está el avión: su motor Couatl lo
//! calcula desde la escena del simulador y lo muestra como título de su
//! menú, p. ej.:
//!
//! ```text
//! Activate Services at MDSD/Las Americas International, Gate B 14
//! ```
//!
//! GSX escribe ese menú a un archivo de **texto plano**:
//!
//! ```text
//! <FSDT root>\MSFS\fsdreamteam-gsx-pro\html_ui\InGamePanels\FSDT_GSX_Panel\menu
//! ```
//!
//! donde `<FSDT root>` sale del registro `HKCU\Software\Fsdreamteam\root`
//! (el instalador de FSDT lo escribe ahí). **Línea 0 = título**; líneas
//! 1..N = opciones del menú. Mecanismo verificado en los proyectos
//! open-source AccessGSX, `msfs-blind-assist` y Fenix2GSX, que leen ese
//! mismo archivo para reflejar/automatizar el menú de GSX.
//!
//! ## Limitación
//!
//! El archivo sólo tiene contenido **mientras el menú de GSX está
//! abierto** (se vacía al cerrarlo). El watcher lo lee mientras el avión
//! está parado en tierra; en cuanto el usuario abre el menú de GSX para
//! pedir servicios (lo que hace en el gate, tanto a la salida como a la
//! llegada), capturamos el gate y lo cacheamos.
//!
//! Esto **no** usa SimConnect ni la API del simulador: es file IO + parse.

use std::path::PathBuf;
use std::sync::OnceLock;

/// Ruta relativa (desde el root de FSDT) del archivo de menú de GSX.
const MENU_REL: &str =
    r"MSFS\fsdreamteam-gsx-pro\html_ui\InGamePanels\FSDT_GSX_Panel\menu";

/// Prefijo (en minúsculas) del título del menú **principal** de GSX — el
/// único que muestra el gate al que el avión está asignado.
const TITLE_PREFIX: &str = "activate services at";

/// Palabras-tipo de posición de MSFS/GSX. El nombre de gate/parking del
/// título contiene una de éstas como token. Sirve para validar que el
/// candidato es realmente una posición y no parte del nombre del
/// aeropuerto (p. ej. "...International").
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

/// Lee el archivo del menú de GSX y, si el menú abierto es el principal
/// ("Activate Services at …"), devuelve el nombre del gate/parking que GSX
/// muestra (p. ej. `"Gate B 14"`).
///
/// Devuelve `None` cuando:
///   · GSX no está instalado / no se resolvió la ruta,
///   · el menú está cerrado (archivo vacío),
///   · el menú abierto no es el principal (pushback, operador, etc.),
///   · el título no trae una posición reconocible.
pub fn read_gate() -> Option<String> {
    let path = menu_path()?;
    // Lectura barata: el archivo son unos cientos de bytes.
    let content = std::fs::read_to_string(path).ok()?;
    let title = content.lines().next()?;
    parse_gate_from_title(title)
}

/// Extrae el nombre de posición del título del menú principal de GSX.
/// `pub(crate)` para poder testearlo.
pub(crate) fn parse_gate_from_title(title: &str) -> Option<String> {
    let t = title.trim();
    if t.is_empty() || !t.to_lowercase().starts_with(TITLE_PREFIX) {
        return None;
    }

    // El candidato a posición es el texto tras la ÚLTIMA coma — preserva
    // prefijos direccionales ("N Parking 6") y nombres con número
    // ("Gate B 14"). Si no hay coma (formato raro), tomamos desde la
    // última palabra-tipo de posición hasta el final.
    let candidate: String = if let Some((_, after)) = t.rsplit_once(',') {
        after.trim().to_string()
    } else {
        let toks: Vec<&str> = t.split_whitespace().collect();
        match toks.iter().rposition(|tok| is_position_keyword(tok)) {
            Some(i) => toks[i..].join(" "),
            None => return None,
        }
    };

    // Colapsar espacios múltiples.
    let cleaned = candidate.split_whitespace().collect::<Vec<_>>().join(" ");
    // Salvaguarda de longitud: un título inesperado no debería colarse.
    if cleaned.is_empty() || cleaned.len() > 40 {
        return None;
    }
    // Validar que contenga una palabra-tipo de posición (evita capturar
    // el nombre del aeropuerto cuando GSX no muestra posición).
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
    use super::parse_gate_from_title;

    #[test]
    fn parses_gate_after_comma() {
        assert_eq!(
            parse_gate_from_title(
                "Activate Services at MDSD/Las Americas International, Gate B 14"
            ),
            Some("Gate B 14".to_string())
        );
    }

    #[test]
    fn parses_parking() {
        assert_eq!(
            parse_gate_from_title("Activate Services at KATL/Atlanta, Parking 30"),
            Some("Parking 30".to_string())
        );
    }

    #[test]
    fn preserves_directional_prefix() {
        assert_eq!(
            parse_gate_from_title("Activate Services at EDDF/Frankfurt, N Parking 6"),
            Some("N Parking 6".to_string())
        );
    }

    #[test]
    fn handles_extra_commas_in_airport_name() {
        assert_eq!(
            parse_gate_from_title(
                "Activate Services at LFPG/Paris, Charles de Gaulle, Stand 102"
            ),
            Some("Stand 102".to_string())
        );
    }

    #[test]
    fn handles_no_comma_format() {
        assert_eq!(
            parse_gate_from_title("Activate Services at KSEA Seattle Gate A1"),
            Some("Gate A1".to_string())
        );
    }

    #[test]
    fn rejects_non_main_menu() {
        assert_eq!(parse_gate_from_title("Select pushback direction"), None);
        assert_eq!(parse_gate_from_title("Request FollowMe"), None);
        // "Select Position at" NO es el menú principal — no afirma el gate
        // al que el avión ya está asignado.
        assert_eq!(
            parse_gate_from_title("Select Position at KATL/Atlanta"),
            None
        );
    }

    #[test]
    fn rejects_title_without_position() {
        assert_eq!(
            parse_gate_from_title(
                "Activate Services at MDSD/Las Americas International"
            ),
            None
        );
    }

    #[test]
    fn rejects_empty() {
        assert_eq!(parse_gate_from_title(""), None);
        assert_eq!(parse_gate_from_title("   "), None);
    }

    #[test]
    fn collapses_whitespace() {
        assert_eq!(
            parse_gate_from_title("Activate Services at X,   Gate   A1 "),
            Some("Gate A1".to_string())
        );
    }
}
