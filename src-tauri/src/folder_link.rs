//! (05/11) Addons que en Community son un enlace a otra carpeta. Los crean
//! instaladores (FS2Crew, SayIntentions, GSX), gestores de addons y SimFleet al
//! compartir un paquete entre MSFS 2020 y 2024.

use std::path::{Path, PathBuf};

/// Carpetas de programas que instalan sus propios paquetes: lo que hay dentro
/// se actualiza y se borra con ese programa, no desde SimFleet.
const INSTALLER_FOLDERS: [&str; 6] = [
    "\\program files",
    "\\programdata",
    "\\windowsapps",
    "\\aerosoft one library",
    "\\addon manager\\",
    "\\orbx\\",
];

#[derive(Debug, Clone, PartialEq)]
pub struct FolderLink {
    /// Carpeta real.
    pub target: PathBuf,
    /// La carpeta real es de un instalador.
    pub managed: bool,
    /// La carpeta real ya no existe.
    pub broken: bool,
}

/// El enlace que hay en `path`, si lo es. Junctions y symlinks, sin seguirlos.
pub fn read(path: &Path) -> Option<FolderLink> {
    if !std::fs::symlink_metadata(path).ok()?.file_type().is_symlink() {
        return None;
    }
    let raw = strip_verbatim(std::fs::read_link(path).ok()?);
    let target = if raw.is_absolute() { raw } else { path.parent()?.join(raw) };
    let name = path.file_name()?.to_str()?;
    Some(FolderLink {
        managed: is_managed(name, &target),
        broken: !target.is_dir(),
        target,
    })
}

/// Windows devuelve los destinos como `\\?\B:\…`.
fn strip_verbatim(path: PathBuf) -> PathBuf {
    match path.to_str().and_then(|s| s.strip_prefix(r"\\?\")) {
        Some(rest) if !rest.starts_with("UNC\\") => PathBuf::from(rest),
        _ => path,
    }
}

/// Un paquete enlazado desde otra Community o desde una biblioteca conserva su
/// nombre; el de un instalador suele llamarse distinto o vive en sus carpetas.
fn is_managed(link_name: &str, target: &Path) -> bool {
    let parent = target
        .parent()
        .and_then(Path::file_name)
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    if parent.eq_ignore_ascii_case("Community") || parent.eq_ignore_ascii_case("Community2024") {
        return false;
    }
    let same_name = target
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.eq_ignore_ascii_case(link_name));
    let path = target.to_string_lossy().to_lowercase();
    !same_name || INSTALLER_FOLDERS.iter().any(|folder| path.contains(folder))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Crea una junction real.
    #[cfg(windows)]
    pub(crate) fn junction(link: &Path, target: &Path) {
        let made = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .output()
            .unwrap();
        assert!(made.status.success(), "no se creó el enlace");
    }

    #[test]
    fn a_library_copy_is_ours_to_manage() {
        let lib = Path::new(r"B:\MSFS Addons\Airport\aerosoft-airport-eddb");
        assert!(!is_managed("aerosoft-airport-eddb", lib));
        assert!(!is_managed("AEROSOFT-airport-eddb", lib), "mayúsculas");
        let orbx = Path::new(r"B:\MSFS Addons\Airport\orbx-airport-essa-stockholm-arlanda");
        assert!(!is_managed("orbx-airport-essa-stockholm-arlanda", orbx), "«orbx» en el nombre");
        let other_sim = Path::new(r"C:\Program Files\Steam\MSFS\Community\acme-airport");
        assert!(!is_managed("acme-airport", other_sim), "enlace entre simuladores");
        let second = Path::new(r"B:\MSFS 2024\Community2024\acme-airport");
        assert!(!is_managed("acme-airport", second));
    }

    #[test]
    fn what_an_installer_put_there_is_left_alone() {
        let renamed = Path::new(r"C:\Program Files\SayIntentions.AI\SayIntentionsAI\efb");
        assert!(is_managed("sayintentions-efb", renamed));
        let same_name = Path::new(r"B:\Program Files (x86)\Addon Manager\MSFS\fsdreamteam-gsx-pro");
        assert!(is_managed("fsdreamteam-gsx-pro", same_name));
        let aerosoft = Path::new(r"B:\Aerosoft One Library\Add-ons\as1\gameDirectory~Community\aerosoft-x");
        assert!(is_managed("aerosoft-x", aerosoft));
        assert!(is_managed("orbx-airport-essa", Path::new(r"C:\Orbx\Library\orbx-airport-essa")));
        assert!(is_managed("acme-airport", Path::new(r"B:\Otra\carpeta")), "otro nombre");
    }

    #[test]
    fn the_verbatim_prefix_is_removed_from_targets() {
        assert_eq!(strip_verbatim(PathBuf::from(r"\\?\B:\MSFS Addons\a")), PathBuf::from(r"B:\MSFS Addons\a"));
        assert_eq!(strip_verbatim(PathBuf::from(r"B:\a")), PathBuf::from(r"B:\a"));
        let unc = PathBuf::from(r"\\?\UNC\nas\addons");
        assert_eq!(strip_verbatim(unc.clone()), unc);
    }

    #[cfg(windows)]
    #[test]
    fn a_junction_is_read_without_following_it() {
        let tmp = tempfile::tempdir().unwrap();
        let real = tmp.path().join("lib").join("acme-airport");
        std::fs::create_dir_all(&real).unwrap();
        let link = tmp.path().join("acme-airport");
        junction(&link, &real);

        let found = read(&link).expect("es un enlace");
        assert_eq!(found.target.file_name(), real.file_name());
        assert!(!found.broken);
        assert!(!found.managed);
        assert_eq!(read(&real), None, "una carpeta normal no es un enlace");

        std::fs::remove_dir_all(&real).unwrap();
        assert!(read(&link).expect("sigue ahí").broken);
    }
}
