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

        if target.exists() {
            fs::remove_dir_all(&target).with_context(|| {
                crate::tr!("install.removePrevFailed", path = target.display())
            })?;
        }

        let size = copy_dir_recursive(pkg, &target).with_context(|| {
            crate::tr!("install.copyFailed", from = pkg.display(), to = target.display())
        })?;

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
    let target = parent.join(format!("{}_extracted", stem));

    if target.exists() {
        fs::remove_dir_all(&target).ok();
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
