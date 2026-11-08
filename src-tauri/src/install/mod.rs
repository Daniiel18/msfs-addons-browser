//! Archive extraction + install into the MSFS 2020 Community folder.
//!
//! Key differences vs. the legacy .NET `InstallerService`:
//!
//! * **Temp extraction dir is auto-cleaned.** The .NET version created
//!   a per-run folder under `%TEMP%` and never deleted it,
//!   slowly filling the user's disk. We wrap the extract dir in
//!   [`tempfile::TempDir`] which removes it on drop — success or
//!   failure — so cleanup is a structural guarantee, not a TODO.
//! * **Deeper package search.** Legacy only looked at immediate children
//!   for `manifest.json` + `layout.json`. Many archives wrap their
//!   package inside an extra folder (e.g. `release/pkgname/manifest.json`).
//!   We BFS up to depth 3.
//! * **Formatos soportados: .zip, .rar, .7z.** El código heredado
//!   abortaba ante RAR ("opened Explorer as a side effect") — aquí
//!   extraemos en proceso con `unrar` + `sevenz-rust2`. Si aparece un
//!   formato desconocido, devolvemos un error limpio.
//!
//! **`.ptp` no soportado** (v0.1.13): el formato `.ptp` (PMDG liveries)
//! es propietario encriptado AES — sin la key de PMDG es imposible
//! instalarlo. La app rechaza explícitamente archivos `.ptp` y los
//! `.ptp` dentro de archivos comprimidos se ignoran.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context};
use serde::Serialize;

/// Maximum depth we will descend when hunting for MSFS packages inside
/// the extracted tree. 3 comfortably handles single- and double-wrapper
/// archives without wandering into the layout.json-listed asset tree.
const MAX_PACKAGE_DEPTH: usize = 3;

/// (04/11) Carpeta de trabajo para cambiar un paquete por otro. Vive dentro de
/// Community para estar en el mismo disco (renombrar es instantáneo, también
/// si Community es un enlace a otro disco) y MSFS no la lee: no tiene manifest
/// y el simulador no mira dentro de las subcarpetas de un paquete.
const WORK_DIR: &str = ".simfleet-install";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledPackage {
    pub name: String,
    pub install_path: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallResult {
    pub packages: Vec<InstalledPackage>,
    pub total_bytes: u64,
    /// Si el archivo contenía un instalador (.exe / .msi / .msixbundle)
    /// pero **ningún** paquete MSFS válido, dejamos los archivos
    /// extraídos en una ruta persistente y reportamos el path absoluto
    /// del primer instalador para que el frontend pueda ofrecérselo
    /// al usuario. `None` cuando todo se instaló como paquete normal
    /// o cuando no había nada útil dentro.
    pub installer_payload: Option<InstallerPayload>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallerPayload {
    /// Carpeta donde se persistió el contenido extraído (sobrevive
    /// al borrado del temp dir). Para que el usuario pueda navegar.
    pub extracted_dir: String,
    /// Path absoluto al instalador principal recomendado para ejecutar.
    pub primary_installer: String,
    /// Resto de instaladores detectados (puede haber múltiples
    /// en archivos de pago con .exe + uninstall + helpers).
    pub other_installers: Vec<String>,
}

/// Extract `archive_path` into a temp dir, find every MSFS package it
/// contains, and install each into `community_path`.
///
/// The temp dir is deleted when this function returns, regardless of
/// outcome — that's what fixes the legacy "dangling `%TEMP%` dirs" bug.
pub fn install_archive(
    archive_path: &Path,
    community_path: &Path,
) -> anyhow::Result<InstallResult> {
    if !archive_path.is_file() {
        return Err(anyhow!(
            "{}",
            crate::tr!("install.fileNotFound", path = archive_path.display())
        ));
    }
    if !community_path.is_dir() {
        return Err(anyhow!(
            "{}",
            crate::tr!("install.communityMissing", path = community_path.display())
        ));
    }

    // `.ptp` rechazado explícitamente — formato encriptado AES
    // de PMDG que sólo PMDG Operations Center puede instalar.
    // Nunca tuvimos la clave; la app no soporta este formato.
    let ext = archive_path
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();
    if ext == "ptp" {
        return Err(anyhow!("{}", crate::tr!("install.ptpUnsupported")));
    }

    let temp = tempfile::Builder::new()
        .prefix("msfs-addon-extract-")
        .tempdir()
        .context(crate::tr!("install.tempDirFailed"))?;
    let extract_root = temp.path();
    tracing::info!(
        "install: extracting {} → {}",
        archive_path.display(),
        extract_root.display()
    );

    extract_to(archive_path, extract_root)?;

    let packages = find_msfs_packages(extract_root);

    // Sin paquetes MSFS dentro: probar instaladores ejecutables.
    //
    //   1. `.exe` / `.msi` / `.msixbundle` — instalador hecho a
    //      mano (Aerosoft, FlyByWire, etc.). Persistimos el extracto
    //      en un sidedir junto al archivo original.
    //   2. Nada útil — error explícito.
    if packages.is_empty() {
        let installers = find_installers(extract_root);
        if installers.is_empty() {
            return Err(anyhow!("{}", crate::tr!("install.noPackagesOrInstallers")));
        }
        let payload = persist_installer_payload(archive_path, extract_root, &installers)?;
        tracing::info!(
            "install: archive es un instalador — primary={}",
            payload.primary_installer
        );
        return Ok(InstallResult {
            packages: vec![],
            total_bytes: 0,
            installer_payload: Some(payload),
        });
    }
    tracing::info!("install: found {} package(s) to install", packages.len());

    let mut installed = Vec::with_capacity(packages.len());
    let mut total_bytes = 0u64;

    for pkg in &packages {
        let name = pkg
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or_else(|| {
                anyhow!("{}", crate::tr!("install.nonUtf8PackageName", path = pkg.display()))
            })?
            .to_string();

        let target = community_path.join(&name);
        tracing::info!("install: {} → {}", name, target.display());

        let size = place_package(pkg, community_path, &name)?;

        total_bytes += size;
        installed.push(InstalledPackage {
            name,
            install_path: target.to_string_lossy().into_owned(),
            size_bytes: size,
        });
    }

    // `temp` drops here → extraction dir is removed.
    tracing::info!("install: complete; temp dir cleaned");
    Ok(InstallResult {
        packages: installed,
        total_bytes,
        installer_payload: None,
    })
}

/// (04/11) Deja `src` en `Community/<name>` sin quedarse nunca sin addon. Lo
/// nuevo se copia aparte; solo con la copia entera se aparta lo viejo y se pone
/// lo nuevo, con dos renombrados. Si MSFS tiene algo abierto, el renombrado
/// falla entero y no se ha tocado nada. Lo viejo se borra al final.
///
/// Si en Community hay un enlace, lo nuevo va a su carpeta real y el enlace se
/// queda. Lo que es de un instalador no se toca: se actualiza con ese programa.
pub fn place_package(src: &Path, community: &Path, name: &str) -> anyhow::Result<u64> {
    let link = crate::folder_link::read(&community.join(name)).filter(|link| !link.broken);
    if let Some(link) = link.as_ref().filter(|link| link.managed) {
        return Err(anyhow!(
            "{}",
            crate::tr!("install.linkManaged", name = name, path = link.target.display())
        ));
    }
    let real = link.and_then(|link| {
        let name = link.target.file_name()?.to_str()?.to_string();
        Some((link.target.parent()?.to_path_buf(), name))
    });
    let (root, name) = match &real {
        Some((root, name)) => (root.as_path(), name.as_str()),
        None => (community, name),
    };
    let work = root.join(WORK_DIR);
    let result = swap_in(src, root, &work, name);
    // Solo se van si quedaron vacías: puede haber otra instalación en curso.
    let _ = fs::remove_dir(work.join("new"));
    let _ = fs::remove_dir(work.join("old"));
    let _ = fs::remove_dir(&work);
    result
}

fn swap_in(src: &Path, community: &Path, work: &Path, name: &str) -> anyhow::Result<u64> {
    let target = community.join(name);
    let fresh = work.join("new").join(name);
    let old = work.join("old").join(name);
    let copy_failed = || crate::tr!("install.copyFailed", from = src.display(), to = target.display());

    remove_path(&fresh).with_context(copy_failed)?;
    fs::create_dir_all(work.join("new")).with_context(copy_failed)?;
    let size = match copy_dir_recursive(src, &fresh) {
        Ok(size) => size,
        Err(e) => {
            let _ = remove_path(&fresh);
            // Con la causa en el mensaje: «no hay espacio en el disco».
            return Err(anyhow!("{}: {e:#}", copy_failed()));
        }
    };

    let had_previous = fs::symlink_metadata(&target).is_ok();
    if had_previous {
        let moved = remove_path(&old)
            .and_then(|_| fs::create_dir_all(work.join("old")))
            .and_then(|_| rename_patiently(&target, &old));
        if let Err(e) = moved {
            let _ = remove_path(&fresh);
            return Err(anyhow::Error::new(e)
                .context(crate::tr!("install.removePrevFailed", path = name)));
        }
    }
    if let Err(e) = rename_patiently(&fresh, &target) {
        if had_previous {
            if let Err(back) = rename_patiently(&old, &target) {
                tracing::error!("install: no pude devolver {}: {back}", target.display());
            }
        }
        let _ = remove_path(&fresh);
        return Err(anyhow::Error::new(e).context(copy_failed()));
    }
    if had_previous {
        // Ya está lo nuevo: si esto falla, el próximo arranque lo limpia.
        if let Err(e) = remove_path(&old) {
            tracing::warn!("install: queda {} por borrar: {e}", old.display());
        }
    }
    Ok(size)
}

/// (04/11) Renombra con paciencia: el antivirus o el indexador revisan lo recién
/// copiado y un archivo abierto dentro de una carpeta impide renombrarla. Si
/// sigue fallando a los 3 segundos, es que alguien la tiene abierta de verdad.
fn rename_patiently(from: &Path, to: &Path) -> io::Result<()> {
    let mut tries = 0;
    loop {
        match fs::rename(from, to) {
            Err(_) if tries < 6 => {
                tries += 1;
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
            other => return other,
        }
    }
}

/// Borra lo que haya en `path`, si hay algo. Un enlace se quita él solo, sin
/// tocar aquello a lo que apunta.
fn remove_path(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
        Ok(m) if m.file_type().is_symlink() => {
            fs::remove_dir(path).or_else(|_| fs::remove_file(path))
        }
        Ok(m) if m.is_dir() => fs::remove_dir_all(path),
        Ok(_) => fs::remove_file(path),
    }
}

/// (04/11) Si SimFleet se cerró a mitad de un cambio de carpetas, lo viejo
/// vuelve a su sitio cuando falta y lo que sobra se limpia. Solo al arrancar:
/// con una instalación en curso borraría su copia.
pub fn recover_interrupted(community: &Path) {
    let work = community.join(WORK_DIR);
    if !work.is_dir() {
        return;
    }
    tracing::warn!("install: quedó un cambio a medias en {}", work.display());
    if let Ok(entries) = fs::read_dir(work.join("old")) {
        for entry in entries.flatten() {
            let target = community.join(entry.file_name());
            let done = if fs::symlink_metadata(&target).is_err() {
                fs::rename(entry.path(), &target)
            } else {
                remove_path(&entry.path())
            };
            if let Err(e) = done {
                tracing::warn!("install: no pude recuperar {}: {e}", target.display());
            }
        }
    }
    let _ = remove_path(&work.join("new"));
    let _ = fs::remove_dir(work.join("old"));
    let _ = fs::remove_dir(&work);
}

/// (05/11) [`recover_interrupted`] en Community y junto a las carpetas reales de
/// sus addons enlazados, que es donde se cambian.
pub fn recover_everywhere(community: &Path) {
    recover_interrupted(community);
    let Ok(entries) = fs::read_dir(community) else {
        return;
    };
    let mut seen = std::collections::HashSet::new();
    for entry in entries.flatten() {
        let parent = crate::folder_link::read(&entry.path())
            .and_then(|link| link.target.parent().map(Path::to_path_buf));
        if let Some(parent) = parent {
            if seen.insert(parent.clone()) {
                recover_interrupted(&parent);
            }
        }
    }
}

/// Busca ejecutables sueltos en el árbol extraído. Sólo extensiones
/// que sean realmente instalables en Windows. No descendemos a
/// profundidad infinita: 4 niveles cubren la mayoría de archivos
/// de pago (release/<addon>/install.exe).
fn find_installers(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    walk_installers(root, 0, &mut out);
    out
}

fn walk_installers(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth > 4 || !dir.is_dir() {
        return;
    }
    let Ok(iter) = fs::read_dir(dir) else { return };
    for entry in iter.flatten() {
        let path = entry.path();
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_dir() {
            walk_installers(&path, depth + 1, out);
        } else if ft.is_file() {
            let ext = path
                .extension()
                .and_then(|s| s.to_str())
                .map(|s| s.to_ascii_lowercase())
                .unwrap_or_default();
            if matches!(ext.as_str(), "exe" | "msi" | "msixbundle") {
                out.push(path);
            }
        }
    }
}

/// Persiste el contenido extraído en `<archive_dir>/<archive_stem>_extracted/`
/// para que el usuario pueda ejecutar el instalador después de que
/// el temp dir desaparezca. Si el destino existe, lo reemplazamos.
fn persist_installer_payload(
    archive: &Path,
    extract_root: &Path,
    installers: &[PathBuf],
) -> anyhow::Result<InstallerPayload> {
    let parent = archive
        .parent()
        .ok_or_else(|| anyhow!("{}", crate::tr!("install.noParentDir")))?;
    let stem = archive
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("archive");
    let mut target = parent.join(format!("{}_extracted", stem));

    if target.exists() {
        let unique = format!("{}_{}_extracted", stem, chrono::Utc::now().format("%H%M%S"));
        target = parent.join(unique);
    }
    copy_dir_recursive(extract_root, &target)
        .with_context(|| crate::tr!("install.persistFailed", path = target.display()))?;

    // Recalcular paths de los instaladores en el nuevo destino.
    let primary_in_temp = pick_primary_installer(installers);
    let primary_rel = primary_in_temp
        .strip_prefix(extract_root)
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|_| primary_in_temp.clone());
    let primary = target.join(primary_rel);

    let mut others: Vec<String> = installers
        .iter()
        .filter(|p| p != &&primary_in_temp)
        .map(|p| {
            let rel = p
                .strip_prefix(extract_root)
                .map(|x| x.to_path_buf())
                .unwrap_or_else(|_| p.clone());
            target.join(rel).to_string_lossy().into_owned()
        })
        .collect();
    others.sort();

    Ok(InstallerPayload {
        extracted_dir: target.to_string_lossy().into_owned(),
        primary_installer: primary.to_string_lossy().into_owned(),
        other_installers: others,
    })
}

/// Heurística para elegir el instalador "principal":
///   1. Prefiere `setup.exe` / `install.exe` (nombres canónicos).
///   2. Si no, el .exe con el path más corto (raíz vs subcarpetas).
///   3. Como último recurso, el primero ordenado alfabéticamente.
fn pick_primary_installer(installers: &[PathBuf]) -> PathBuf {
    let canonical_names = ["setup.exe", "install.exe", "installer.exe"];
    for name in &canonical_names {
        if let Some(p) = installers.iter().find(|p| {
            p.file_name()
                .and_then(|s| s.to_str())
                .map(|s| s.eq_ignore_ascii_case(name))
                .unwrap_or(false)
        }) {
            return p.clone();
        }
    }
    let mut sorted: Vec<&PathBuf> = installers.iter().collect();
    sorted.sort_by_key(|p| (p.components().count(), p.as_os_str().to_owned()));
    sorted
        .first()
        .cloned()
        .cloned()
        .unwrap_or_else(|| installers[0].clone())
}

/// (v6.2.42) Contraseñas de archivo conocidas que probamos AUTOMÁTICAMENTE
/// cuando un archivo viene cifrado. Skybound protege TODOS sus RAR con la
/// misma clave ("https://skybound.cx"), indicada en la ficha del addon, así
/// que la instalación no falla por pedir la contraseña. Si algún día
/// aparecen otras, se añaden aquí.
pub fn known_archive_passwords() -> &'static [&'static str] {
    &["https://skybound.cx"]
}

/// Borra el contenido de `dir` (dejándolo vacío) — para reintentar la
/// extracción con otra contraseña sin arrastrar archivos a medias.
fn clear_dir(dir: &Path) -> anyhow::Result<()> {
    if dir.exists() {
        for entry in fs::read_dir(dir)?.flatten() {
            let p = entry.path();
            if p.is_dir() {
                let _ = fs::remove_dir_all(&p);
            } else {
                let _ = fs::remove_file(&p);
            }
        }
    } else {
        fs::create_dir_all(dir)?;
    }
    Ok(())
}

fn extract_to(archive: &Path, dest: &Path) -> anyhow::Result<()> {
    let ext = archive
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();

    // (v6.2.42) Probamos SIN contraseña primero (el caso normal); si falla
    // (archivo cifrado), reintentamos con cada contraseña conocida. La
    // mayoría de archivos no están cifrados, así que el primer intento gana
    // sin coste. Sólo hay reintentos cuando de verdad falla.
    let mut candidates: Vec<Option<&str>> = vec![None];
    for p in known_archive_passwords() {
        candidates.push(Some(p));
    }

    let mut last_err: Option<anyhow::Error> = None;
    for (i, pw) in candidates.iter().enumerate() {
        if i > 0 {
            // Limpia lo extraído a medias del intento anterior.
            let _ = clear_dir(dest);
        }
        let r = match ext.as_str() {
            "zip" => extract_zip(archive, dest, *pw),
            "rar" => extract_rar(archive, dest, *pw),
            "7z" => extract_7z(archive, dest, *pw),
            other => {
                return Err(anyhow!(
                    "{}",
                    crate::tr!("install.unsupportedFormat", ext = other)
                ))
            }
        };
        match r {
            Ok(()) => return Ok(()),
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow!("{}", crate::tr!("install.extractionFailed"))))
}

fn extract_zip(archive: &Path, dest: &Path, password: Option<&str>) -> anyhow::Result<()> {
    if let Some(pw) = password {
        // Descifrado por entrada (ZipCrypto/AES).
        let file = fs::File::open(archive)
            .with_context(|| crate::tr!("install.openArchiveFailed", path = archive.display()))?;
        let mut zip = zip::ZipArchive::new(file).context(crate::tr!("install.zipReadFailed"))?;
        for i in 0..zip.len() {
            let mut entry = zip
                .by_index_decrypt(i, pw.as_bytes())
                .map_err(|e| anyhow!("zip decrypt: {e}"))?;
            let Some(rel) = entry.enclosed_name() else { continue };
            let out = dest.join(rel);
            if entry.is_dir() {
                fs::create_dir_all(&out)?;
            } else {
                if let Some(parent) = out.parent() {
                    fs::create_dir_all(parent)?;
                }
                let mut sink = fs::File::create(&out)?;
                io::copy(&mut entry, &mut sink)?;
            }
        }
        return Ok(());
    }
    extract_zip_plain(archive, dest)
}

fn extract_zip_plain(archive: &Path, dest: &Path) -> anyhow::Result<()> {
    let file = fs::File::open(archive)
        .with_context(|| crate::tr!("install.openArchiveFailed", path = archive.display()))?;
    let mut zip = zip::ZipArchive::new(file).context(crate::tr!("install.zipReadFailed"))?;

    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;
        // `enclosed_name` rechaza rutas absolutas y componentes `..` —
        // la guardia anti-"zip-slip" que el código heredado no tenía.
        let rel = match entry.enclosed_name() {
            Some(p) => p.to_path_buf(),
            None => {
                tracing::warn!("install: entrada zip insegura ignorada: {}", entry.name());
                continue;
            }
        };
        let out = dest.join(rel);

        if entry.is_dir() {
            fs::create_dir_all(&out)?;
        } else {
            if let Some(parent) = out.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut sink = fs::File::create(&out)?;
            io::copy(&mut entry, &mut sink)?;
        }
    }
    Ok(())
}

/// Extrae un archivo RAR (incluyendo multi-volumen `.partN.rar` o
/// `.rar + .r00 + .r01…`).
///
/// `unrar` descubre automáticamente los volúmenes vecinos en el mismo
/// directorio cuando se le apunta al primer volumen — el caller
/// (`find_primary_archive` en `torrent.rs`) es quien se encarga de
/// elegir ese primer volumen.
///
/// La API del crate es una máquina de estados: cada llamada a
/// `read_header` / `extract`/`skip` consume el header actual y devuelve
/// el archivo posicionado en el siguiente. Por eso hay que reasignar
/// `archive = …` en cada vuelta del bucle.
fn extract_rar(archive: &Path, dest: &Path, password: Option<&str>) -> anyhow::Result<()> {
    let mut rar = match password {
        Some(pw) => unrar::Archive::with_password(archive, pw)
            .open_for_processing()
            .with_context(|| {
                crate::tr!("install.openRarFailed", path = archive.display())
            })?,
        None => unrar::Archive::new(archive)
            .open_for_processing()
            .with_context(|| {
                crate::tr!("install.openRarFailed", path = archive.display())
            })?,
    };

    while let Some(header) = rar.read_header().context(crate::tr!("install.rarHeaderInvalid"))? {
        let entry = header.entry();
        // Defensa contra "rar-slip": rutas absolutas o con `..` se
        // ignoran. `unrar` aplica el base dir internamente, pero este
        // chequeo nos protege si una futura versión cambia ese default.
        let filename = entry.filename.clone();
        let unsafe_path = filename.is_absolute()
            || filename
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir));

        if unsafe_path {
            tracing::warn!(
                "install: entrada rar insegura ignorada: {}",
                filename.display()
            );
            rar = header.skip().context(crate::tr!("install.rarSkipFailed"))?;
            continue;
        }

        rar = if entry.is_file() {
            header
                .extract_with_base(dest)
                .context(crate::tr!("install.rarExtractFailed"))?
        } else {
            // Crear el directorio explícitamente porque `unrar` a veces
            // no emite headers separados para directorios intermedios.
            let dir = dest.join(&filename);
            if let Err(e) = fs::create_dir_all(&dir) {
                tracing::warn!(
                    "install: no se pudo crear subdirectorio {}: {}",
                    dir.display(),
                    e
                );
            }
            header.skip().context(crate::tr!("install.rarSkipFailed"))?
        };
    }
    Ok(())
}

/// (v7) Guard anti **"7z-slip"**: `sevenz-rust2` NO valida los nombres de
/// entrada (a diferencia de zip vía `enclosed_name` y rar vía chequeo de
/// componentes), así que un `.7z` malicioso con rutas `..`/absolutas/con drive
/// podría escribir FUERA del tempdir (p.ej. la carpeta Startup). Rechazamos esas
/// entradas y dejamos pasar el resto al extractor por defecto del crate.
/// Compartido con `drop_install::extract_7z`.
pub(crate) fn safe_7z_entry(
    entry: &sevenz_rust2::SevenZArchiveEntry,
    reader: &mut dyn std::io::Read,
    path: &std::path::PathBuf,
) -> Result<bool, sevenz_rust2::Error> {
    let name = entry.name();
    let rel = std::path::Path::new(name);
    // `is_absolute()` en Windows es FALSE para rutas con raíz pero sin drive
    // (`\foo`, `/foo`) — y `dest.join("\\foo")` salta a la raíz del volumen. Por
    // eso chequeamos también `has_root()`. Y `:` cubre drive-relative/ADS.
    let unsafe_path = rel.is_absolute()
        || rel.has_root()
        || name.contains(':')
        || rel
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir));
    if unsafe_path {
        tracing::warn!(target: "install", "7z: entrada insegura ignorada: {}", name);
        return Ok(true);
    }
    sevenz_rust2::default_entry_extract_fn(entry, reader, path)
}

/// Extrae un archivo .7z (incluidos volúmenes `.7z.001`, `.7z.002`…) con el
/// guard anti-traversal [`safe_7z_entry`].
fn extract_7z(archive: &Path, dest: &Path, password: Option<&str>) -> anyhow::Result<()> {
    match password {
        Some(pw) => {
            let file = std::fs::File::open(archive)
                .with_context(|| crate::tr!("install.openFailed7z", path = archive.display()))?;
            sevenz_rust2::decompress_with_extract_fn_and_password(
                file,
                dest,
                sevenz_rust2::Password::from(pw),
                safe_7z_entry,
            )
            .with_context(|| crate::tr!("install.extractFailed7z", path = archive.display()))?;
        }
        None => sevenz_rust2::decompress_file_with_extract_fn(archive, dest, safe_7z_entry)
            .with_context(|| crate::tr!("install.extractFailed7z", path = archive.display()))?,
    }
    Ok(())
}

/// Find every directory in `root` that contains both `manifest.json`
/// and `layout.json` — the MSFS package signature. BFS up to
/// [`MAX_PACKAGE_DEPTH`] to handle archives that wrap the package
/// inside one or two extra folders.
fn find_msfs_packages(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    walk_packages(root, 0, &mut out);
    out
}

fn walk_packages(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth > MAX_PACKAGE_DEPTH || !dir.is_dir() {
        return;
    }
    if dir.join("manifest.json").is_file() && dir.join("layout.json").is_file() {
        out.push(dir.to_path_buf());
        // Don't descend into a valid package — its inner dirs are just assets.
        return;
    }
    let Ok(iter) = fs::read_dir(dir) else { return };
    for entry in iter.flatten() {
        if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            walk_packages(&entry.path(), depth + 1, out);
        }
    }
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> anyhow::Result<u64> {
    fs::create_dir_all(dst)?;
    let mut total = 0u64;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            total += copy_dir_recursive(&from, &to)?;
        } else {
            total += fs::copy(&from, &to)?;
        }
    }
    Ok(total)
}

#[cfg(test)]
mod place_tests {
    use super::*;

    fn write_pkg(dir: &Path, text: &str) {
        fs::create_dir_all(dir.join("scenery")).unwrap();
        fs::write(dir.join("manifest.json"), text).unwrap();
        fs::write(dir.join("scenery").join("a.bgl"), text).unwrap();
    }

    fn read(dir: &Path) -> String {
        fs::read_to_string(dir.join("manifest.json")).unwrap()
    }

    #[test]
    fn a_new_package_lands_in_community_and_leaves_nothing_else() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        let community = tmp.path().join("Community");
        fs::create_dir_all(&community).unwrap();
        write_pkg(&src, "nuevo");

        let size = place_package(&src, &community, "acme-airport").unwrap();

        assert_eq!(size, 10);
        assert_eq!(read(&community.join("acme-airport")), "nuevo");
        assert!(!community.join(WORK_DIR).exists());
    }

    #[test]
    fn the_new_copy_fully_replaces_the_old_one() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        let community = tmp.path().join("Community");
        write_pkg(&src, "nuevo");
        write_pkg(&community.join("acme-airport"), "viejo");
        fs::write(community.join("acme-airport").join("solo-viejo.txt"), "x").unwrap();

        place_package(&src, &community, "acme-airport").unwrap();

        let pkg = community.join("acme-airport");
        assert_eq!(read(&pkg), "nuevo");
        assert!(!pkg.join("solo-viejo.txt").exists(), "no se mezcla con lo viejo");
        assert!(!community.join(WORK_DIR).exists());
    }

    #[test]
    fn a_failed_copy_leaves_the_old_package_untouched() {
        let tmp = tempfile::tempdir().unwrap();
        let community = tmp.path().join("Community");
        write_pkg(&community.join("acme-airport"), "viejo");

        let missing = tmp.path().join("no-existe");
        assert!(place_package(&missing, &community, "acme-airport").is_err());

        assert_eq!(read(&community.join("acme-airport")), "viejo");
        assert!(!community.join(WORK_DIR).exists());
    }

    /// Lo mismo que le pasa con MSFS abierto: un archivo en uso dentro.
    #[cfg(windows)]
    #[test]
    fn with_a_file_in_use_nothing_changes() {
        use std::os::windows::fs::OpenOptionsExt;

        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        let community = tmp.path().join("Community");
        write_pkg(&src, "nuevo");
        write_pkg(&community.join("acme-airport"), "viejo");

        let bgl = community.join("acme-airport").join("scenery").join("a.bgl");
        let lock = fs::OpenOptions::new().read(true).share_mode(1).open(&bgl).unwrap();
        let result = place_package(&src, &community, "acme-airport");
        drop(lock);

        assert!(result.is_err());
        assert_eq!(read(&community.join("acme-airport")), "viejo");
        assert_eq!(fs::read_to_string(&bgl).unwrap(), "viejo");
        assert!(!community.join(WORK_DIR).exists());
    }

    #[cfg(windows)]
    #[test]
    fn a_linked_package_is_updated_in_its_real_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        let community = tmp.path().join("Community");
        let library = tmp.path().join("Addons").join("Airport");
        let real = library.join("acme-airport");
        fs::create_dir_all(&community).unwrap();
        write_pkg(&src, "nuevo");
        write_pkg(&real, "viejo");
        fs::write(real.join("solo-viejo.txt"), "x").unwrap();
        let link = community.join("acme-airport");
        crate::folder_link::tests::junction(&link, &real);

        place_package(&src, &community, "acme-airport").unwrap();

        assert!(fs::symlink_metadata(&link).unwrap().file_type().is_symlink(), "el enlace sigue");
        assert_eq!(read(&link), "nuevo");
        assert_eq!(read(&real), "nuevo");
        assert!(!real.join("solo-viejo.txt").exists(), "no queda la copia vieja");
        assert!(!library.join(WORK_DIR).exists());
        assert!(!community.join(WORK_DIR).exists());
    }

    #[cfg(windows)]
    #[test]
    fn what_an_installer_linked_is_not_replaced() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        let community = tmp.path().join("Community");
        let real = tmp.path().join("App").join("efb");
        fs::create_dir_all(&community).unwrap();
        write_pkg(&src, "nuevo");
        write_pkg(&real, "instalador");
        let link = community.join("acme-efb");
        crate::folder_link::tests::junction(&link, &real);

        assert!(place_package(&src, &community, "acme-efb").is_err());

        assert_eq!(read(&real), "instalador");
        assert!(fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
    }

    #[cfg(windows)]
    #[test]
    fn a_broken_link_gives_way_to_a_real_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        let community = tmp.path().join("Community");
        let gone = tmp.path().join("Addons").join("acme-airport");
        fs::create_dir_all(&community).unwrap();
        fs::create_dir_all(&gone).unwrap();
        write_pkg(&src, "nuevo");
        let link = community.join("acme-airport");
        crate::folder_link::tests::junction(&link, &gone);
        fs::remove_dir_all(&gone).unwrap();

        place_package(&src, &community, "acme-airport").unwrap();

        assert_eq!(read(&link), "nuevo");
        assert!(!fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
    }

    #[cfg(windows)]
    #[test]
    fn a_change_cut_short_next_to_a_linked_folder_is_recovered() {
        let tmp = tempfile::tempdir().unwrap();
        let community = tmp.path().join("Community");
        let library = tmp.path().join("Addons");
        let real = library.join("acme-airport");
        fs::create_dir_all(&community).unwrap();
        write_pkg(&real, "viejo");
        crate::folder_link::tests::junction(&community.join("acme-airport"), &real);
        write_pkg(&library.join(WORK_DIR).join("old").join("otro-airport"), "viejo");

        recover_everywhere(&community);

        assert_eq!(read(&library.join("otro-airport")), "viejo");
        assert!(!library.join(WORK_DIR).exists());
    }

    #[test]
    fn after_a_crash_the_old_package_comes_back_if_it_went_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let community = tmp.path().join("Community");
        let work = community.join(WORK_DIR);
        write_pkg(&work.join("old").join("sin-sustituir"), "viejo");
        write_pkg(&work.join("new").join("sin-sustituir"), "a medias");
        write_pkg(&work.join("old").join("ya-sustituido"), "viejo");
        write_pkg(&community.join("ya-sustituido"), "nuevo");

        recover_interrupted(&community);

        assert_eq!(read(&community.join("sin-sustituir")), "viejo");
        assert_eq!(read(&community.join("ya-sustituido")), "nuevo");
        assert!(!work.exists());
    }

    #[test]
    fn with_nothing_interrupted_recovery_does_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        write_pkg(&tmp.path().join("acme-airport"), "viejo");

        recover_interrupted(tmp.path());

        assert_eq!(read(&tmp.path().join("acme-airport")), "viejo");
    }
}
