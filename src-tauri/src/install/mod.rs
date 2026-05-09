//! Archive extraction + install into the MSFS 2020 Community folder.
//!
//! Key differences vs. the legacy .NET `InstallerService`:
//!
//! * **Temp extraction dir is auto-cleaned.** The .NET version created
//!   `%TEMP%\SceneryAddonsBrowser\extract\{guid}` and never deleted it,
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
    /// Si el archivo contenía liveries `.ptp` (formato PMDG Operations
    /// Center), las copiamos a un "Inbox" visible y retornamos los
    /// paths para que la UI le diga al usuario "abre PMDG OC y haz
    /// click en Install Livery". No tocamos el folder del aircraft
    /// directamente porque PMDG OC valida firma y tipo de variante
    /// (737-700/800/900, 777-200/300ER), y un drop manual mal
    /// orientado deja el avión en un estado raro.
    pub ptp_payload: Option<PtpPayload>,
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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PtpPayload {
    /// Carpeta inbox donde quedan las liveries (`Documents/MSFS Addons
    /// Browser/PMDG Liveries Inbox/`). Se mantiene como fallback
    /// manual cuando el auto-install a PMDG OC no es posible.
    pub inbox_dir: String,
    /// Liveries copiadas — paths absolutos finales en `inbox_dir`.
    pub ptp_files: Vec<String>,
    /// Aircraft detectado por archivo (en mismo orden que `ptp_files`).
    /// Heurístico: lee el nombre + intenta abrir el .ptp como ZIP y
    /// extraer su manifest. `None` cuando no se pudo determinar.
    pub detected_aircraft: Vec<Option<String>>,
    /// Path donde se copió la livery dentro de PMDG Operations
    /// Center (auto-install). En el mismo orden que `ptp_files`.
    /// `None` cuando el aircraft no se detectó o cuando OC no está
    /// instalado para ese aircraft. La UI usa esto para mostrar
    /// "✓ Auto-instalado en PMDG OC" vs "Importa manualmente".
    pub auto_installed_at: Vec<Option<String>>,
}

/// Extract `archive_path` into a temp dir, find every MSFS package it
/// contains, and install each into `community_path`.
///
/// The temp dir is deleted when this function returns, regardless of
/// outcome — that's what fixes the legacy "dangling `%TEMP%` dirs" bug.
pub fn install_archive(archive_path: &Path, community_path: &Path) -> anyhow::Result<InstallResult> {
    if !archive_path.is_file() {
        return Err(anyhow!(
            "No se encontró el archivo: {}",
            archive_path.display()
        ));
    }
    if !community_path.is_dir() {
        return Err(anyhow!(
            "La carpeta Community no existe: {}",
            community_path.display()
        ));
    }

    // **Caso especial — `.ptp` arrastrado directamente**.
    //
    // Las liveries PMDG vienen como un único archivo `.ptp` (formato
    // propio del PMDG Operations Center) o dentro de un .rar/.zip.
    // Si el usuario arrastra el .ptp directo, no hay nada que
    // extraer: lo copiamos al inbox de inmediato y reportamos el
    // payload — la rama de descompresión se salta entera.
    let ext = archive_path
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();
    if ext == "ptp" {
        let payload = persist_ptp_payload(&[archive_path.to_path_buf()])?;
        tracing::info!(
            "install: archivo .ptp directo → {}",
            payload.inbox_dir
        );
        return Ok(InstallResult {
            packages: vec![],
            total_bytes: 0,
            installer_payload: None,
            ptp_payload: Some(payload),
        });
    }

    let temp = tempfile::Builder::new()
        .prefix("msfs-addon-extract-")
        .tempdir()
        .context("no se pudo crear el directorio temporal de extracción")?;
    let extract_root = temp.path();
    tracing::info!(
        "install: extracting {} → {}",
        archive_path.display(),
        extract_root.display()
    );

    extract_to(archive_path, extract_root)?;

    let packages = find_msfs_packages(extract_root);

    // Sin paquetes MSFS dentro: probar otras pistas.
    //
    //   1. `.ptp` (liveries PMDG Operations Center) — las copiamos
    //      al inbox público y avisamos al usuario.
    //   2. `.exe` / `.msi` / `.msixbundle` — instalador hecho a
    //      mano (Aerosoft, FlyByWire, etc.). Persistimos el extracto
    //      en un sidedir junto al archivo original.
    //   3. Nada útil — error explícito.
    //
    // Probamos PTP primero porque es muy específico (extensión
    // exclusiva de PMDG) y no se confunde con instaladores genéricos.
    if packages.is_empty() {
        let ptps = find_ptp_files(extract_root);
        if !ptps.is_empty() {
            let payload = persist_ptp_payload(&ptps)?;
            tracing::info!(
                "install: archive contiene {} livery(es) PMDG (.ptp) → {}",
                payload.ptp_files.len(),
                payload.inbox_dir
            );
            return Ok(InstallResult {
                packages: vec![],
                total_bytes: 0,
                installer_payload: None,
                ptp_payload: Some(payload),
            });
        }

        let installers = find_installers(extract_root);
        if installers.is_empty() {
            return Err(anyhow!(
                "No se encontraron paquetes MSFS (manifest.json + layout.json), liveries PMDG (.ptp) ni instaladores ejecutables dentro del archivo."
            ));
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
            ptp_payload: None,
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
                anyhow!("el paquete tiene un nombre de carpeta no-UTF-8: {:?}", pkg)
            })?
            .to_string();

        let target = community_path.join(&name);
        tracing::info!("install: {} → {}", name, target.display());

        if target.exists() {
            fs::remove_dir_all(&target).with_context(|| {
                format!(
                    "no se pudo eliminar la instalación previa en {} (¿archivo en uso?)",
                    target.display()
                )
            })?;
        }

        let size = copy_dir_recursive(pkg, &target).with_context(|| {
            format!(
                "falló la copia: {} → {}",
                pkg.display(),
                target.display()
            )
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
        ptp_payload: None,
    })
}

/// Busca archivos `.ptp` (liveries PMDG Operations Center) en el
/// árbol extraído. Profundidad limitada a 4 — los .rar/.zip que
/// distribuyen liveries pueden envolver con uno o dos folders
/// extras pero rara vez más.
fn find_ptp_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    walk_ptp(root, 0, &mut out);
    out
}

fn walk_ptp(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth > 4 || !dir.is_dir() {
        return;
    }
    let Ok(iter) = fs::read_dir(dir) else { return };
    for entry in iter.flatten() {
        let path = entry.path();
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_dir() {
            walk_ptp(&path, depth + 1, out);
        } else if ft.is_file() {
            let ext = path
                .extension()
                .and_then(|s| s.to_str())
                .map(|s| s.to_ascii_lowercase())
                .unwrap_or_default();
            if ext == "ptp" {
                out.push(path);
            }
        }
    }
}

/// Copia los `.ptp` detectados al inbox visible al usuario y, si
/// PMDG Operations Center está instalado, también a su carpeta de
/// "Liveries" — así el usuario sólo tiene que abrir OC y la
/// livery aparece importada sin tener que apuntar manualmente al
/// inbox de la app. Si el aircraft no se detectó, se queda sólo
/// en el inbox visible y el usuario importa manualmente.
fn persist_ptp_payload(ptps: &[PathBuf]) -> anyhow::Result<PtpPayload> {
    let documents = directories_documents()
        .ok_or_else(|| anyhow!("no se pudo localizar la carpeta `Documents` del usuario"))?;
    let inbox = documents
        .join("MSFS Addons Browser")
        .join("PMDG Liveries Inbox");
    fs::create_dir_all(&inbox).with_context(|| {
        format!("no se pudo crear la carpeta inbox {}", inbox.display())
    })?;

    let mut copied: Vec<String> = Vec::with_capacity(ptps.len());
    let mut detected: Vec<Option<String>> = Vec::with_capacity(ptps.len());
    let mut auto_installed: Vec<Option<String>> = Vec::with_capacity(ptps.len());
    for src in ptps {
        let name = src
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or_else(|| anyhow!("nombre de livery inválido: {:?}", src))?;
        let dst = inbox.join(name);
        fs::copy(src, &dst).with_context(|| {
            format!("no se pudo copiar {} → {}", src.display(), dst.display())
        })?;
        let aircraft = detect_ptp_aircraft(&dst);
        let mut auto_target: Option<String> = None;
        if let Some(ac) = &aircraft {
            tracing::info!(target: "install", ".ptp '{}' → aircraft detectado: {}", name, ac);
            match try_install_to_pmdg_oc(&dst, ac, &documents) {
                Ok(Some(target)) => {
                    tracing::info!(
                        target: "install",
                        "PMDG OC auto-install: {} → {}",
                        name,
                        target.display()
                    );
                    auto_target = Some(target.to_string_lossy().into_owned());
                }
                Ok(None) => tracing::info!(
                    target: "install",
                    "PMDG OC no detectado para {}, queda en inbox manual",
                    ac
                ),
                Err(e) => tracing::warn!(
                    target: "install",
                    "PMDG OC auto-install falló para {}: {e:#}",
                    name
                ),
            }
        } else {
            tracing::info!(
                target: "install",
                ".ptp '{}' sin aircraft detectado — queda sólo en inbox manual",
                name
            );
        }
        detected.push(aircraft);
        auto_installed.push(auto_target);
        copied.push(dst.to_string_lossy().into_owned());
    }

    Ok(PtpPayload {
        inbox_dir: inbox.to_string_lossy().into_owned(),
        ptp_files: copied,
        detected_aircraft: detected,
        auto_installed_at: auto_installed,
    })
}

/// Intenta copiar `ptp` al folder donde PMDG Operations Center
/// espera encontrar la livery del aircraft indicado.
///
/// Las versiones recientes de PMDG OC para MSFS escanean dos
/// posibles lugares:
///   1. `Documents/PMDG/PMDG_737-800/Liveries/`  (legacy + 2020)
///   2. `Documents/PMDG/Operations Center/Liveries/<aircraft>/` (OC v3)
///
/// Mapeamos `aircraft` ("PMDG 737-800") al nombre de carpeta que
/// PMDG usa internamente. Devolvemos el path al que se copió, o
/// `None` si no encontramos ninguna ruta candidata existente.
fn try_install_to_pmdg_oc(
    ptp: &Path,
    aircraft: &str,
    documents: &Path,
) -> anyhow::Result<Option<PathBuf>> {
    let folder_name = ptp
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| anyhow!("nombre inválido"))?;

    // Mapping de nombre legible → token usado por PMDG en sus
    // paths. PMDG mantiene la convención `PMDG_<tipo>` en folders
    // bajo Documents/PMDG, por compatibilidad legado.
    let pmdg_token = match aircraft {
        "PMDG 737-600" => "PMDG_737-600",
        "PMDG 737-700" => "PMDG_737-700",
        "PMDG 737-800" => "PMDG_737-800",
        "PMDG 737-900" => "PMDG_737-900",
        "PMDG 737"     => "PMDG_737",
        "PMDG 747-400" => "PMDG_747-400",
        "PMDG 747-8"   => "PMDG_747-8",
        "PMDG 747F"    => "PMDG_747F",
        "PMDG 747"     => "PMDG_747",
        "PMDG 777-200LR" => "PMDG_777-200LR",
        "PMDG 777-300ER" => "PMDG_777-300ER",
        "PMDG 777F"    => "PMDG_777F",
        "PMDG 777"     => "PMDG_777",
        "PMDG DC-6"    => "PMDG_DC-6",
        _ => return Ok(None),
    };

    // Candidatos en orden de preferencia. Si OC v3 está instalado
    // (carpeta "Operations Center" existe), preferimos ese path
    // porque es el que la versión actual del OC scanea.
    let oc_v3_root = documents
        .join("PMDG")
        .join("Operations Center")
        .join("Liveries")
        .join(pmdg_token);
    let legacy_root = documents.join("PMDG").join(pmdg_token).join("Liveries");

    let candidates = [oc_v3_root, legacy_root];

    for target_dir in candidates {
        // Sólo escribimos si el directorio del PMDG OC existe —
        // si no existe, OC no está instalado para ese aircraft o
        // el usuario lo tiene en otra ubicación. No creamos
        // jerarquías que el OC no espera.
        let parent = target_dir.parent();
        let parent_exists = parent.map(|p| p.exists()).unwrap_or(false);
        if !parent_exists {
            continue;
        }
        if !target_dir.exists() {
            // Crear la subcarpeta del aircraft sí es seguro — OC
            // tolera carpetas vacías de Liveries.
            if fs::create_dir_all(&target_dir).is_err() {
                continue;
            }
        }
        let dst = target_dir.join(folder_name);
        if let Err(e) = fs::copy(ptp, &dst) {
            tracing::warn!(
                target: "install",
                "PMDG OC: copia {} → {} falló: {e}",
                ptp.display(),
                dst.display()
            );
            continue;
        }
        return Ok(Some(dst));
    }
    Ok(None)
}

/// Detecta el aircraft de una livery PMDG `.ptp` con dos pasadas:
///
///   1. **Nombre de archivo** — patrones canónicos `PMDG_737-800_…`,
///      `_777-300ER_`, `_747-8_`, etc. Cubre 90% de las liveries que
///      la comunidad distribuye.
///
///   2. **Contenido como ZIP** — la mayoría de `.ptp` son archivos
///      ZIP renombrados. Abrimos y buscamos `aircraft.cfg`,
///      `manifest.json` o un fichero similar; el campo de aircraft
///      suele aparecer ahí. Sirve como fallback cuando el filename
///      no es informativo.
///
/// Devuelve la cadena del aircraft (ej. "PMDG 737-800") o `None`
/// cuando ambas pasadas fallaron.
fn detect_ptp_aircraft(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_string_lossy().to_string();
    if let Some(ac) = detect_aircraft_from_name(&name) {
        return Some(ac);
    }
    // Fallback: abrir como ZIP y buscar aircraft.cfg / manifest dentro.
    if let Ok(file) = fs::File::open(path) {
        if let Ok(mut zip) = zip::ZipArchive::new(file) {
            for i in 0..zip.len() {
                let Ok(entry) = zip.by_index(i) else {
                    continue;
                };
                let entry_name = entry.name().to_lowercase();
                if entry_name.ends_with("aircraft.cfg")
                    || entry_name.ends_with("manifest.json")
                    || entry_name.contains("livery")
                {
                    // Detección por nombre de la entrada — los ZIPs
                    // de PMDG OC suelen llamar a sus carpetas con el
                    // nombre del avión.
                    if let Some(ac) = detect_aircraft_from_name(&entry_name) {
                        return Some(ac);
                    }
                }
            }
        }
    }
    None
}

/// Busca patrones de aircraft en un nombre. Ordenados de más
/// específico a menos para ganar precisión.
fn detect_aircraft_from_name(name: &str) -> Option<String> {
    let lower = name.to_lowercase();
    // Boeing — variantes del 737, 747, 777
    if lower.contains("737-700") || lower.contains("_737_700") || lower.contains("737ng-700") {
        return Some("PMDG 737-700".into());
    }
    if lower.contains("737-800") || lower.contains("_738_") || lower.contains("738-") {
        return Some("PMDG 737-800".into());
    }
    if lower.contains("737-900") || lower.contains("_739_") {
        return Some("PMDG 737-900".into());
    }
    if lower.contains("737-600") {
        return Some("PMDG 737-600".into());
    }
    if lower.contains("777-200lr") || lower.contains("_77l_") {
        return Some("PMDG 777-200LR".into());
    }
    if lower.contains("777-300er") || lower.contains("_77w_") {
        return Some("PMDG 777-300ER".into());
    }
    if lower.contains("777f") || lower.contains("777-200f") {
        return Some("PMDG 777F".into());
    }
    if lower.contains("747-400") || lower.contains("_744_") {
        return Some("PMDG 747-400".into());
    }
    if lower.contains("747-8") || lower.contains("_748_") {
        return Some("PMDG 747-8".into());
    }
    if lower.contains("747f") {
        return Some("PMDG 747F".into());
    }
    if lower.contains("dc-6") || lower.contains("dc6") {
        return Some("PMDG DC-6".into());
    }
    // Heurística genérica para 737/747/777 sin variante específica.
    if lower.contains("_737") || lower.contains("-737") {
        return Some("PMDG 737".into());
    }
    if lower.contains("_777") || lower.contains("-777") {
        return Some("PMDG 777".into());
    }
    if lower.contains("_747") || lower.contains("-747") {
        return Some("PMDG 747".into());
    }
    None
}

/// Resuelve `~/Documents` en Windows sin pulling de `dirs`/`directories`
/// (no son deps actuales). Lee `USERPROFILE\Documents` que es donde
/// MSFS y PMDG OC esperan encontrar configs/inputs.
fn directories_documents() -> Option<PathBuf> {
    if let Ok(profile) = std::env::var("USERPROFILE") {
        let p = PathBuf::from(profile).join("Documents");
        if p.is_dir() {
            return Some(p);
        }
    }
    None
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
        .ok_or_else(|| anyhow!("el archivo no tiene un directorio padre"))?;
    let stem = archive
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("archive");
    let target = parent.join(format!("{}_extracted", stem));

    if target.exists() {
        fs::remove_dir_all(&target).ok();
    }
    copy_dir_recursive(extract_root, &target)
        .with_context(|| format!("no se pudo persistir el extracto en {}", target.display()))?;

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

fn extract_to(archive: &Path, dest: &Path) -> anyhow::Result<()> {
    let ext = archive
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();

    match ext.as_str() {
        "zip" => extract_zip(archive, dest),
        "rar" => extract_rar(archive, dest),
        "7z"  => extract_7z(archive, dest),
        other => Err(anyhow!(
            "Formato de archivo no soportado: .{} (se esperaba .zip, .rar o .7z)",
            other
        )),
    }
}

fn extract_zip(archive: &Path, dest: &Path) -> anyhow::Result<()> {
    let file = fs::File::open(archive)
        .with_context(|| format!("no se pudo abrir el archivo {}", archive.display()))?;
    let mut zip = zip::ZipArchive::new(file).context("no se pudo leer el archivo ZIP")?;

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
fn extract_rar(archive: &Path, dest: &Path) -> anyhow::Result<()> {
    let mut rar = unrar::Archive::new(archive)
        .open_for_processing()
        .with_context(|| {
            format!("no se pudo abrir el archivo RAR {}", archive.display())
        })?;

    while let Some(header) = rar.read_header().context("header RAR inválido")? {
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
            rar = header.skip().context("skip RAR falló")?;
            continue;
        }

        rar = if entry.is_file() {
            header
                .extract_with_base(dest)
                .context("extracción RAR falló")?
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
            header.skip().context("skip RAR falló")?
        };
    }
    Ok(())
}

/// Extrae un archivo .7z (incluidos volúmenes `.7z.001`, `.7z.002`…).
///
/// `sevenz-rust2` expone una API de nivel alto que hace todo en una
/// sola llamada y maneja solid blocks + multi-volumen internamente.
fn extract_7z(archive: &Path, dest: &Path) -> anyhow::Result<()> {
    sevenz_rust2::decompress_file(archive, dest)
        .with_context(|| format!("no se pudo extraer {}", archive.display()))?;
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
