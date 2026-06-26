use std::path::{Path, PathBuf};

use once_cell::sync::Lazy;
use regex::Regex;
use serde::Serialize;

/// Variantes de instalación de MSFS soportadas. Cada combo
/// (sim, store) tiene su propio `UserCfg.opt` en una ruta distinta,
/// y por tanto su propia carpeta `Community`.
///
/// Lo usamos para dos cosas:
///   1. Detección automática del Community folder (la variante
///      por defecto sigue siendo la primera encontrada — comportamiento
///      v0.1.17 — para no romper a usuarios con una sola instalación).
///   2. **Desinstalación total** (v0.1.19): cuando el usuario quita un
///      paquete queremos borrarlo de todas las Community que existan,
///      por si tiene Steam y MS Store en paralelo o convivencia
///      MSFS 2020 + MSFS 2024.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MsfsVariant {
    /// MSFS 2020 versión de MS Store (`Microsoft.FlightSimulator_8wekyb3d8bbwe`).
    MsStore2020,
    /// MSFS 2020 versión de Steam (perfil en `%APPDATA%\Microsoft Flight Simulator\`).
    Steam2020,
    /// MSFS 2024 versión de MS Store (`Microsoft.Limitless_8wekyb3d8bbwe`).
    MsStore2024,
    /// MSFS 2024 versión de Steam (perfil en `%APPDATA%\Microsoft Flight Simulator 2024\`).
    Steam2024,
}

impl MsfsVariant {
    /// Etiqueta corta para logs y para el campo `variant` que se
    /// serializa al frontend.
    pub const fn label(self) -> &'static str {
        match self {
            MsfsVariant::MsStore2020 => "msstore-2020",
            MsfsVariant::Steam2020 => "steam-2020",
            MsfsVariant::MsStore2024 => "msstore-2024",
            MsfsVariant::Steam2024 => "steam-2024",
        }
    }
}

/// Description of a detected MSFS Community folder, ready for the UI.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommunityInfo {
    /// Absolute path to the Community directory.
    pub path: String,
    /// Etiqueta de la variante (`steam-2020`, `msstore-2020`,
    /// `steam-2024`, `msstore-2024`). Antes era sólo `"steam"` /
    /// `"msstore"`; en v0.1.19 se amplió para distinguir entre 2020 y
    /// 2024 — el frontend lo trata como string opaco así que añadir
    /// nuevos valores es backward-compatible.
    pub variant: &'static str,
    /// True if the directory actually exists and is writable.
    pub exists: bool,
}

/// Attempt to auto-detect **a** MSFS Community folder by parsing
/// `UserCfg.opt`. Devuelve la primera variante encontrada — orden de
/// preferencia: MSFS 2020 MS Store → Steam → MSFS 2024 MS Store → Steam.
///
/// Este es el endpoint que usa el scanner / instalador para saber
/// "dónde instalo nuevos paquetes". Por eso preferimos 2020 (MSFS 2024
/// todavía no está soportado para descargas de SceneryAddons/Simplaza
/// en la mayoría de paquetes — el universo ahí es muy distinto).
///
/// Returns `Ok(None)` si no encontramos ninguna instalación.
///
/// (v5.1.0) La preferencia ya NO es fija a 2020: elige la variante de la
/// versión de MSFS que el usuario seleccionó (`crate::sim`). Prioriza una
/// carpeta de esa versión que EXISTA; si no, cualquiera de esa versión; y
/// como último recurso, la primera disponible.
pub fn detect_community_folder() -> anyhow::Result<Option<CommunityInfo>> {
    let all = detect_all_community_folders()?;
    let token = if crate::sim::is_2024() { "2024" } else { "2020" };
    let chosen = all
        .iter()
        .find(|c| c.variant.contains(token) && c.exists)
        .or_else(|| all.iter().find(|c| c.variant.contains(token)))
        .or_else(|| all.iter().find(|c| c.exists))
        .or_else(|| all.first())
        .cloned();
    Ok(chosen)
}

/// Devuelve **todas** las carpetas Community detectadas — una por
/// cada variante MSFS instalada en el sistema. Útil para la
/// desinstalación total (v0.1.19): un paquete puede vivir
/// simultáneamente en la Community de Steam y en la de MS Store si
/// el usuario tiene ambas versiones, o duplicado entre 2020 y 2024.
///
/// El orden de la lista es estable: 2020 MS Store, 2020 Steam,
/// 2024 MS Store, 2024 Steam. Sólo se incluyen variantes con
/// `UserCfg.opt` localizable; las que no, no aparecen.
///
/// **Nota sobre `exists`:** el flag refleja sólo si la carpeta
/// Community existe físicamente — puede que el UserCfg apunte a una
/// ruta que el usuario movió/borró. El caller (`uninstall_by_folder`)
/// filtra por este flag para evitar tocar paths inválidos.
pub fn detect_all_community_folders() -> anyhow::Result<Vec<CommunityInfo>> {
    let mut out = Vec::new();
    for (cfg, variant) in find_all_user_cfgs() {
        match read_packages_path(&cfg) {
            Ok(Some(path)) => {
                let community = path.join("Community");
                let exists = community.is_dir();
                out.push(CommunityInfo {
                    path: community.to_string_lossy().into_owned(),
                    variant: variant.label(),
                    exists,
                });
            }
            Ok(None) => {
                tracing::warn!(
                    target: "community",
                    "{}: UserCfg.opt sin InstalledPackagesPath en {}",
                    variant.label(),
                    cfg.display()
                );
            }
            Err(e) => {
                tracing::warn!(
                    target: "community",
                    "{}: no se pudo leer {} ({:#})",
                    variant.label(),
                    cfg.display(),
                    e
                );
            }
        }
    }
    Ok(out)
}

/// (v5.3.4) ¿Qué versiones de MSFS están instaladas? Devuelve
/// `(tiene_2020, tiene_2024)` según qué variantes tienen `UserCfg.opt`.
/// El splash lo usa para preguntar la versión SOLO cuando hay ambas.
pub fn detect_installed_sims() -> (bool, bool) {
    let mut has_2020 = false;
    let mut has_2024 = false;
    for (_, variant) in find_all_user_cfgs() {
        match variant {
            MsfsVariant::MsStore2020 | MsfsVariant::Steam2020 => has_2020 = true,
            MsfsVariant::MsStore2024 | MsfsVariant::Steam2024 => has_2024 = true,
        }
    }
    (has_2020, has_2024)
}

/// Lista todas las rutas a `UserCfg.opt` de cada variante MSFS
/// presente en el sistema, en orden de preferencia.
fn find_all_user_cfgs() -> Vec<(PathBuf, MsfsVariant)> {
    let mut out = Vec::new();

    // MSFS 2020 — MS Store (WinUI/UWP package).
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        let p = Path::new(&local)
            .join("Packages")
            .join("Microsoft.FlightSimulator_8wekyb3d8bbwe")
            .join("LocalCache")
            .join("UserCfg.opt");
        if p.is_file() {
            out.push((p, MsfsVariant::MsStore2020));
        }
    }

    // MSFS 2020 — Steam (perfil clásico en %APPDATA%).
    if let Some(roaming) = std::env::var_os("APPDATA") {
        let p = Path::new(&roaming)
            .join("Microsoft Flight Simulator")
            .join("UserCfg.opt");
        if p.is_file() {
            out.push((p, MsfsVariant::Steam2020));
        }
    }

    // MSFS 2024 — MS Store. El package family name oficial es
    // `Microsoft.Limitless_8wekyb3d8bbwe` (nombre interno del proyecto
    // antes del rebrand público a "Microsoft Flight Simulator 2024").
    // La estructura de carpetas es idéntica a la de 2020.
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        let p = Path::new(&local)
            .join("Packages")
            .join("Microsoft.Limitless_8wekyb3d8bbwe")
            .join("LocalCache")
            .join("UserCfg.opt");
        if p.is_file() {
            out.push((p, MsfsVariant::MsStore2024));
        }
    }

    // MSFS 2024 — Steam. La carpeta `%APPDATA%\Microsoft Flight
    // Simulator 2024\` es el equivalente directo de la de 2020 pero
    // con sufijo "2024".
    if let Some(roaming) = std::env::var_os("APPDATA") {
        let p = Path::new(&roaming)
            .join("Microsoft Flight Simulator 2024")
            .join("UserCfg.opt");
        if p.is_file() {
            out.push((p, MsfsVariant::Steam2024));
        }
    }

    out
}

/// (v6.1) Carpeta RAÍZ del PERFIL de estado de MSFS donde viven los `work/` de
/// los addons (iFly, PMDG…). OJO: NO es la carpeta Community
/// (`InstalledPackagesPath`); es el perfil de usuario de MSFS. La estructura
/// difiere por versión — verificado en disco:
///   · 2020 → `<perfil>\Packages\<pkg>\work`
///     (Steam: `%APPDATA%\Microsoft Flight Simulator\Packages`)
///   · 2024 → `<perfil>\WASM\MSFS2020\<pkg>\work`
///     (Steam: `%APPDATA%\Microsoft Flight Simulator 2024\WASM\MSFS2020`)
/// `prefer_2024` viene de `pref_sim_version`. Devuelve la primera variante de esa
/// versión cuyo directorio raíz exista, o `None` si no la encuentra.
pub fn addon_work_root(prefer_2024: bool) -> Option<PathBuf> {
    for (cfg, variant) in find_all_user_cfgs() {
        let is_2024 = matches!(
            variant,
            MsfsVariant::MsStore2024 | MsfsVariant::Steam2024
        );
        if is_2024 != prefer_2024 {
            continue;
        }
        // El padre de `UserCfg.opt` es la carpeta del perfil de esa variante.
        let profile = cfg.parent()?;
        let root = if is_2024 {
            profile.join("WASM").join("MSFS2020")
        } else {
            profile.join("Packages")
        };
        if root.is_dir() {
            return Some(root);
        }
    }
    None
}

static PACKAGES_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"(?i)InstalledPackagesPath\s+"(.+?)""#).unwrap());

fn read_packages_path(cfg: &Path) -> anyhow::Result<Option<PathBuf>> {
    let content = std::fs::read_to_string(cfg)?;
    Ok(PACKAGES_RE
        .captures(&content)
        .and_then(|c| c.get(1))
        .map(|m| PathBuf::from(m.as_str().trim())))
}

/// Carpetas auxiliares donde algunos instaladores (.exe) dejan
/// archivos fuera de Community — son los "extras" candidatos a
/// limpieza durante la desinstalación total. Devolvemos los paths
/// **base** (sin el nombre del paquete) para que el caller los
/// combine con el folder name del paquete a borrar.
///
/// Cada entrada es `(label, path_base)` donde:
///   · `label`: etiqueta corta para el log / UI.
///   · `path_base`: carpeta a la que se le añadiría el folder name
///                  del paquete para formar el candidato a borrar.
///
/// **¿Qué incluimos?**
///   · `Documents\MSFS Sceneries\` — convención histórica de
///     instaladores .exe (Aerosoft, Drzewiecki, FSDreamTeam). Algunos
///     ponen ahí los assets en lugar de Community y luego enlazan via
///     simbólico o registry. Si existe `Documents\MSFS Sceneries\<pkg>`
///     hay que borrarlo también.
///   · `Documents\MSFS 2024 Sceneries\` — variante 2024 con el mismo
///     patrón, observada en addons que ya distinguen versión.
///
/// **¿Qué NO incluimos?**
///   · Rolling cache de MSFS (`rolling.ccc` y similares) — son archivos
///     compartidos entre todos los paquetes; borrar entradas concretas
///     requiere parsear el binario, y MSFS los regenera al arranque
///     si están corruptos. Tocarlos sólo añade riesgo sin beneficio.
///   · `LocalState\packages\` — sólo contiene paquetes oficiales del
///     marketplace en la versión MS Store, nunca Community.
pub fn extra_install_locations() -> Vec<(&'static str, PathBuf)> {
    let mut out: Vec<(&'static str, PathBuf)> = Vec::new();

    if let Some(docs) = dirs_documents() {
        out.push(("documents-msfs-sceneries", docs.join("MSFS Sceneries")));
        out.push((
            "documents-msfs-2024-sceneries",
            docs.join("MSFS 2024 Sceneries"),
        ));
    }

    out
}

/// Devuelve `%USERPROFILE%\Documents` cuando se puede resolver.
/// Tauri no expone `documents_dir()` sin un AppHandle, pero la
/// convención de Windows es estable: `USERPROFILE` siempre apunta a
/// `C:\Users\<user>` y `Documents` es subcarpeta directa. Esto evita
/// arrastrar `dirs` como dependencia para una sola ruta.
fn dirs_documents() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE").map(|p| Path::new(&p).join("Documents"))
}
