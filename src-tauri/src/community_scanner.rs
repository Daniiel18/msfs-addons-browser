//! Escáner de la carpeta Community de MSFS.
//!
//! Recorre cada subcarpeta inmediata, lee su `manifest.json`, y
//! produce un `ScannedPackage` por cada paquete válido. La salida
//! se persiste en `community_packages` (migración 005) — esa tabla
//! es la fuente de verdad para la vista de mapa y el detector de
//! actualizaciones, en vez de `installed_addons` (que sólo registra
//! lo que se descargó a través de esta app).
//!
//! El escaneo es **sincrónico y bloqueante** porque:
//!   · La carpeta Community vive en SSD local — leer 200 manifests
//!     son ~200ms.
//!   · Lo invocamos desde un `spawn_blocking` o desde una tarea
//!     async normal — `std::fs` no requiere ceremonia adicional.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScannedPackage {
    /// (05/11) Carpeta real, si en Community es un enlace.
    pub link_target: Option<String>,
    /// (05/11) Esa carpeta real es de un instalador.
    pub link_managed: bool,
    /// (05/11) Dónde más está este paquete (`Community2024`).
    pub also_in: Option<String>,
    pub folder_name: String,
    pub install_path: String,
    pub title: String,
    pub creator: Option<String>,
    pub content_type: Option<String>,
    pub package_version: Option<String>,
    pub minimum_game_version: Option<String>,
    pub icao: Option<String>,
    pub size_bytes: Option<u64>,
    pub folder_modified_at: Option<String>,
    /// Cantidad de entries en `manifest.dependencies`. Usamos esto
    /// para detectar liveries: en MSFS las liveries se declaran como
    /// `content_type=AIRCRAFT` con dependencia al aircraft base, así
    /// que `AIRCRAFT + deps>0` distingue livery de aircraft completo.
    pub dependencies_count: usize,
    /// (v4.25.0) Nombres de paquete declarados en
    /// `manifest.dependencies[].name`. Siembran las aristas 'auto' del
    /// Link Map (livery → su aircraft base) sin que el usuario tenga
    /// que enlazarlos a mano.
    pub dependency_names: Vec<String>,
    /// (v4.25.0) Estado físico del paquete en el simulador. MSFS solo
    /// carga paquetes con `layout.json` presente — nuestro toggle lo
    /// renombra a `layout.json.disabled` para apagarlos sin mover la
    /// carpeta. El scanner refleja ese estado acá.
    pub enabled: bool,
    /// (v4.26.0) Nombres de las carpetas bajo `SimObjects/Airplanes`
    /// (y Rotorcraft) del paquete. Un aircraft base "posee" su
    /// contenedor (p.ej. `FNX_320`); las liveries lo referencian.
    pub simobject_dirs: Vec<String>,
    /// (v4.26.0) Valores de `base_container` extraídos de los
    /// aircraft.cfg del paquete (último componente del path, p.ej.
    /// `FNX_320`). Es la referencia REAL livery → aircraft base que
    /// usa MSFS — mucho más fiable que adivinar por folder name.
    pub base_containers: Vec<String>,
    /// (03/11) Tipos ICAO que declara el `aircraft.cfg` (`icao_type_designator`).
    /// Los aviones cifrados (PMDG/Fenix/iniBuilds) no lo traen.
    pub icao_types: Vec<String>,
    /// (04/11) Última actualización real, ver `last_update`.
    pub last_update: Option<String>,
    /// (v4.29.0) Ruta absoluta del thumbnail "nativo" de MSFS,
    /// pre-resuelto durante el scan. Cuatro estrategias en orden:
    ///   1. `<pkg>/thumbnail.jpg|png` (raíz del paquete).
    ///   2. `<pkg>/SimObjects/Airplanes/*/thumbnail.jpg|png`.
    ///   3. `<pkg>/simfleet-thumbnail.jpg` (descargado por nosotros).
    ///   4. `<pkg>/layout.json` → primer entry cuyo `path` contiene
    ///      "thumbnail" — fallback ultra-seguro de la spec MSFS.
    /// None ⇒ el frontend pinta la silueta premium.
    pub thumbnail_path: Option<String>,
    /// (v4.31.0) true si algún container bajo SimObjects/Airplanes
    /// tiene carpeta `model*` con geometría 3D propia → es un AVIÓN
    /// real. Si tiene containers pero NINGUNO con modelo propio, es
    /// una modificación (texturas de cabina, etc.) aunque el manifest
    /// diga content_type=AIRCRAFT.
    pub has_own_model: bool,
    /// (v7.2.2) true si algún container trae su MODELO DE VUELO
    /// (flight_model.cfg o un .air). Señal DEFINITIVA de "esto es un avión":
    /// vale para los aviones encriptados (PMDG/Fenix/iniBuilds) donde
    /// has_own_model es false, y excluye liveries y mods (cabina/luces/
    /// texturas) que declaran content_type=AIRCRAFT pero heredan el vuelo del
    /// avión base.
    pub has_flight_model: bool,
    /// (v4.31.0) false si el paquete NO trae manifest.json — 3rd-party
    /// no estándar (badge en la UI).
    pub has_manifest: bool,
}

/// Estructura literal del `manifest.json` de MSFS. Sólo
/// deserializamos los campos que conocemos; el resto se ignora
/// vía `#[serde(default)]` y comportamiento por defecto de serde.
///
/// El manifest puede traer `manufacturer` y/o `creator` — mostramos
/// `creator` cuando exista (es lo que ven los usuarios en el sim);
/// si no, caemos a `manufacturer`.
#[derive(Debug, Deserialize)]
struct RawManifest {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    manufacturer: Option<String>,
    #[serde(default)]
    creator: Option<String>,
    #[serde(default)]
    content_type: Option<String>,
    #[serde(default)]
    package_version: Option<String>,
    #[serde(default)]
    minimum_game_version: Option<String>,
    /// Array genérico — no nos importa la forma exacta, sólo cuántos
    /// hay. Usar `serde_json::Value` evita modelar la estructura
    /// (que varía entre paquetes).
    #[serde(default)]
    dependencies: Vec<serde_json::Value>,
}

/// Resultado del escaneo: paquetes encontrados + recuento de
/// errores no fatales (manifests rotos, carpetas sin manifest…).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanReport {
    pub packages: Vec<ScannedPackage>,
    pub skipped_no_manifest: usize,
    pub skipped_invalid_manifest: usize,
    pub community_path: String,
    /// (05/11) Enlaces cuya carpeta real ya no existe.
    pub broken_links: Vec<BrokenLink>,
}

/// (05/11) No es un paquete, pero se ofrece quitarlo.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrokenLink {
    pub folder_name: String,
    pub target: String,
}

/// (05/11) Une el escaneo de una segunda carpeta. Si un paquete está en las dos,
/// manda el de la principal y se apunta dónde más está.
pub fn merge(mut main: ScanReport, extra: ScanReport) -> ScanReport {
    for pkg in extra.packages {
        let same = main
            .packages
            .iter_mut()
            .find(|p| p.folder_name.eq_ignore_ascii_case(&pkg.folder_name));
        match same {
            Some(same) => same.also_in = Some(pkg.install_path),
            None => main.packages.push(pkg),
        }
    }
    main.broken_links.extend(extra.broken_links);
    main.skipped_no_manifest += extra.skipped_no_manifest;
    main.skipped_invalid_manifest += extra.skipped_invalid_manifest;
    main
}

/// Escanea Community recursivamente al primer nivel. No descendemos
/// porque los paquetes de MSFS son siempre `Community/<paquete>/...`
/// — `<paquete>/<sub>/manifest.json` no es válido.
/// (v6.2.25) Cache de tamaños del scan anterior: folder_name →
/// (firma_mtime, size_bytes). La firma es el mismo string que
/// `folder_modified_at`; si no cambió desde el último scan, reusamos
/// el `size_bytes` guardado y nos ahorramos el walk recursivo del
/// árbol (lo caro cuando hay cientos de paquetes). El resto de datos
/// derivados (manifest, SimObjects, thumbnail) son baratos y se
/// recalculan siempre.
pub type SizeCache = std::collections::HashMap<String, (String, u64)>;

pub fn scan(community_path: &Path, size_cache: &SizeCache) -> anyhow::Result<ScanReport> {
    let mut packages = Vec::new();
    let mut broken_links = Vec::new();
    let mut skipped_no_manifest = 0usize;
    let mut skipped_invalid_manifest = 0usize;

    let entries = std::fs::read_dir(community_path)?;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            if let Some(link) = crate::folder_link::read(&path).filter(|link| link.broken) {
                broken_links.push(BrokenLink {
                    folder_name: entry.file_name().to_string_lossy().into_owned(),
                    target: link.target.to_string_lossy().into_owned(),
                });
            }
            continue;
        }
        let folder_name = match path.file_name().and_then(|s| s.to_str()) {
            Some(s) => s.to_string(),
            None => continue,
        };

        // (v4.25.0) Estado enabled/disabled del paquete. Disabled =
        // nuestro toggle renombró `layout.json` → `layout.json.disabled`
        // (MSFS ignora paquetes sin layout). Si no hay NINGUNO de los
        // dos lo tratamos como enabled — paquetes raros sin layout no
        // deben aparecer como "apagados por SimFleet".
        let enabled = path.join("layout.json").is_file()
            || !path.join("layout.json.disabled").is_file();

        // (v4.26.0) Contenedores SimObjects propios + base_container
        // referenciados — alimentan el auto-link del Link Map.
        // (v4.31.0) + has_own_model: ¿hay geometría 3D propia?
        let (simobject_dirs, base_containers, has_own_model, has_flight_model, icao_types) =
            scan_simobjects(&path);
        // (v4.29.0) Thumbnail resuelto UNA vez (no en runtime por la
        // UI). Persistido en DB para que `package_thumbnail` sea O(1).
        let thumbnail_path = resolve_thumbnail(&path);

        let manifest_path = path.join("manifest.json");
        let has_manifest = manifest_path.is_file();

        if !has_manifest {
            // (v4.31.0) Sin manifest.json. MSFS no carga estos paquetes
            // por sí mismo, PERO el usuario quiere verlos marcados como
            // 3rd-party SI tienen estructura de addon real (layout.json
            // o SimObjects). Carpetas sueltas sin nada de eso (backups,
            // basura) las seguimos saltando.
            let looks_like_addon = path.join("layout.json").is_file()
                // (fix) También `.disabled`: si el paquete SOLO se detectaba por
                // layout.json y el usuario lo desactivó (layout.json →
                // layout.json.disabled), sin esto desaparecía del scan y de la
                // DB, y ya no se podía re-activar desde la app.
                || path.join("layout.json.disabled").is_file()
                || !simobject_dirs.is_empty()
                || path.join("SimObjects").is_dir()
                || path.join("scenery").is_dir();
            if !looks_like_addon {
                skipped_no_manifest += 1;
                continue;
            }
            packages.push(ScannedPackage {
                icao_types,
                ..materialize_fallback(
                    &path,
                    &folder_name,
                    enabled,
                    simobject_dirs,
                    base_containers,
                    thumbnail_path,
                    has_own_model,
                    has_flight_model,
                    false, // has_manifest
                    size_cache,
                )
            });
            continue;
        }

        match parse_manifest(&manifest_path) {
            Ok(raw) => {
                packages.push(ScannedPackage {
                    icao_types,
                    ..materialize(
                        &path,
                        &folder_name,
                        raw,
                        enabled,
                        simobject_dirs,
                        base_containers,
                        thumbnail_path.clone(),
                        has_own_model,
                        has_flight_model,
                        size_cache,
                    )
                });
            }
            Err(e) => {
                tracing::debug!(
                    "scanner: manifest inválido en {} ({:#})",
                    manifest_path.display(),
                    e
                );
                // Fallback: aún sin manifest válido, registramos el
                // paquete con info derivada del folder name. Eso
                // permite que el usuario lo vea en el mapa con un
                // título plausible y nuestra heurística de ICAO.
                skipped_invalid_manifest += 1;
                packages.push(ScannedPackage {
                    icao_types,
                    ..materialize_fallback(
                        &path,
                        &folder_name,
                        enabled,
                        simobject_dirs,
                        base_containers,
                        thumbnail_path,
                        has_own_model,
                        has_flight_model,
                        true, // tiene manifest pero está corrupto
                        size_cache,
                    )
                });
            }
        }
    }

    Ok(ScanReport {
        packages,
        skipped_no_manifest,
        skipped_invalid_manifest,
        community_path: community_path.to_string_lossy().into_owned(),
        broken_links,
    })
}

/// (v4.26.0) Inspecciona `SimObjects/{Airplanes,Rotorcraft}` del
/// paquete: nombres de contenedores propios + los `base_container`
/// que referencian sus aircraft.cfg. Barato: lista 1-2 dirs y lee
/// los primeros KB de cada aircraft.cfg.
fn scan_simobjects(root: &Path) -> (Vec<String>, Vec<String>, bool, bool, Vec<String>) {
    let mut dirs: Vec<String> = Vec::new();
    let mut bases: Vec<String> = Vec::new();
    let mut types: Vec<String> = Vec::new();
    // (v4.31.0) ¿Algún container tiene geometría 3D propia? Un AVIÓN
    // real trae `model/` o `model.<variant>/` con .gltf/.bin; una
    // modificación (texturas de cabina, sonido) sólo trae `texture.*`
    // o `panel/` y reusa el modelo del avión base.
    let mut has_own_model = false;
    // (v7.2.2) Señal DEFINITIVA de avión: ¿algún container trae su MODELO DE
    // VUELO (flight_model.cfg o un archivo .air)? Los aviones de verdad —
    // incluidos los ENCRIPTADOS (PMDG/Fenix/iniBuilds), que no exponen
    // geometría .gltf y por eso tienen has_own_model=false— SIEMPRE lo traen.
    // Las liveries y los mods (texturas de cabina, luces, cabina) NUNCA lo
    // traen: heredan el del avión base. Es mucho más fiable que has_own_model,
    // que da falsos NEGATIVOS (encriptados) y falsos POSITIVOS (un mod de
    // cabina puede traer geometría sin ser un avión).
    let mut has_flight_model = false;
    for kind in ["Airplanes", "Rotorcraft"] {
        let so = root.join("SimObjects").join(kind);
        let Ok(entries) = std::fs::read_dir(&so) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
                dirs.push(name.to_string());
            }
            // Escaneo del contenido del container UNA sola vez: (a) geometría
            // 3D propia (model*/ con .gltf/.bin) y (b) señal de avión de verdad.
            // Señal de avión — cubre AMBOS sims:
            //   · MSFS 2020 y aviones de modelo abierto (iFly/FBW/FSS): traen su
            //     modelo de vuelo en claro → `flight_model.cfg` o un `.air`.
            //   · MSFS 2024 aviones ENCRIPTADOS (PMDG/Fenix/iniBuilds/TFDi): NO
            //     exponen flight_model.cfg (va cifrado), pero su container trae
            //     una carpeta `presets` (loadouts del avión) que las liveries y
            //     los mods NO tienen. (`attachments` sola NO vale: los módulos
            //     AICopilot la traen; por eso usamos `presets`.)
            if !has_own_model || !has_flight_model {
                if let Ok(subs) = std::fs::read_dir(&path) {
                    for sub in subs.flatten() {
                        let sp = sub.path();
                        let sname = sp
                            .file_name()
                            .and_then(|s| s.to_str())
                            .unwrap_or("")
                            .to_lowercase();
                        if sp.is_dir() {
                            // `model`, `model.300ER`, `model.WL`, etc.
                            if !has_own_model
                                && (sname == "model" || sname.starts_with("model."))
                                && dir_has_geometry(&sp)
                            {
                                has_own_model = true;
                            }
                            // `presets` → avión encriptado de MSFS 2024.
                            if !has_flight_model && sname == "presets" {
                                has_flight_model = true;
                            }
                        } else if !has_flight_model
                            && (sname == "flight_model.cfg"
                                || sp
                                    .extension()
                                    .and_then(|s| s.to_str())
                                    .map(|e| e.eq_ignore_ascii_case("air"))
                                    .unwrap_or(false))
                        {
                            has_flight_model = true;
                        }
                    }
                }
            }
            let cfg = path.join("aircraft.cfg");
            // Con pérdida: muchos cfg no son UTF-8 y sin esto se saltaban enteros.
            if let Ok(bytes) = std::fs::read(&cfg) {
                let raw = String::from_utf8_lossy(&bytes);
                icao_types_in(&raw, &mut types);
                // `base_container = "..\Asobo_A320_NEO"` (o con más ..\).
                // Tomamos el último componente del path entre comillas.
                for line in raw.lines().take(200) {
                    let lower = line.trim_start();
                    if !lower.to_ascii_lowercase().starts_with("base_container") {
                        continue;
                    }
                    if let Some(value) = line.split('=').nth(1) {
                        let value = value.trim().trim_matches('"');
                        let last = value
                            .rsplit(['\\', '/'])
                            .next()
                            .unwrap_or("")
                            .trim()
                            .to_string();
                        if !last.is_empty() && !last.starts_with('.') {
                            bases.push(last);
                        }
                    }
                    break;
                }
            }
        }
    }
    bases.sort();
    bases.dedup();
    types.sort();
    types.dedup();
    (dirs, bases, has_own_model, has_flight_model, types)
}

/// (03/11) Tipos ICAO de un `aircraft.cfg`: `icao_type_designator = "A20N"`,
/// uno por cada `[FLTSIM.N]`/`[GENERAL]`. Las líneas comentadas no cuentan.
fn icao_types_in(raw: &str, out: &mut Vec<String>) {
    const KEY: &str = "icao_type_designator";
    for line in raw.lines() {
        let t = line.trim_start();
        if !t.get(..KEY.len()).is_some_and(|k| k.eq_ignore_ascii_case(KEY)) {
            continue;
        }
        let Some(value) = t.split('=').nth(1) else { continue };
        let value = value.split(';').next().unwrap_or("").trim().trim_matches('"').trim();
        if (2..=4).contains(&value.len()) && value.chars().all(|c| c.is_ascii_alphanumeric()) {
            out.push(value.to_ascii_uppercase());
        }
    }
}

/// ¿La carpeta `model*` contiene geometría 3D real? Un avión trae
/// `.gltf`/`.bin`; las modificaciones que crean un `model.cfg` vacío
/// apuntando al base no traen geometría. Comprobamos los primeros
/// archivos sin recursión profunda.
fn dir_has_geometry(model_dir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(model_dir) else {
        return false;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        let ext = p
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| s.to_ascii_lowercase())
            .unwrap_or_default();
        if matches!(ext.as_str(), "gltf" | "glb" | "bin") {
            return true;
        }
    }
    false
}

/// (v4.29.0) Resuelve el thumbnail del paquete UNA vez por scan
/// siguiendo la jerarquía de la spec MSFS. Cada paso es un único
/// `metadata()` o `read()` corto, no recursión profunda — es el cambio
/// clave para que con cientos de GB de liveries el escaneo no se
/// vuelva I/O-bound.
fn resolve_thumbnail(root: &Path) -> Option<String> {
    // (0) (v4.31.0) `ContentInfo/<pkg>/Thumbnail.jpg` — EL thumbnail
    //     oficial del marketplace de MSFS. Es la imagen exacta que el
    //     sim muestra en su menú (Fenix, PMDG, DominicDesignTeam… lo
    //     ponen aquí). Gana sobre todo: es la imagen REAL del addon.
    //     Antes no lo mirábamos, así que Fenix/aeropuertos quedaban sin
    //     foto nativa y la descarga de internet ponía una incorrecta.
    if let Some(p) = find_in_content_info(root) {
        return Some(p);
    }
    // (1) `<pkg>/thumbnail.{jpg,png,webp,jpeg}` — la convención MSFS.
    //     (Windows es case-insensitive: matchea Thumbnail.JPG.)
    for ext in ["jpg", "png", "jpeg", "webp"] {
        let p = root.join(format!("thumbnail.{ext}"));
        if p.is_file() && is_real_image(&p) {
            return Some(p.to_string_lossy().into_owned());
        }
    }
    // (2) `<pkg>/simfleet-thumbnail.jpg` — el que descargamos nosotros
    //     (Wikipedia). Va DESPUÉS del nativo/ContentInfo: si el addon
    //     trae su imagen oficial, esa manda.
    let custom = root.join("simfleet-thumbnail.jpg");
    if custom.is_file() && is_real_image(&custom) {
        return Some(custom.to_string_lossy().into_owned());
    }
    // (3) `<pkg>/SimObjects/Airplanes/*/thumbnail.{jpg,png}`.
    for kind in ["Airplanes", "Rotorcraft"] {
        let dir = root.join("SimObjects").join(kind);
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let folder = entry.path();
            if !folder.is_dir() {
                continue;
            }
            for ext in ["jpg", "png", "jpeg", "webp"] {
                let p = folder.join(format!("thumbnail.{ext}"));
                if p.is_file() && is_real_image(&p) {
                    return Some(p.to_string_lossy().into_owned());
                }
            }
            // texture.X subfolders (PMDG, Fenix, FBW…)
            if let Ok(subs) = std::fs::read_dir(&folder) {
                for sub in subs.flatten() {
                    let sub_path = sub.path();
                    let Some(name) = sub_path.file_name().and_then(|s| s.to_str())
                    else {
                        continue;
                    };
                    if !name.to_lowercase().starts_with("texture") {
                        continue;
                    }
                    for ext in ["jpg", "png", "jpeg", "webp"] {
                        let p = sub_path.join(format!("thumbnail.{ext}"));
                        if p.is_file() && is_real_image(&p) {
                            return Some(p.to_string_lossy().into_owned());
                        }
                    }
                }
            }
        }
    }
    // (4) `layout.json` — fallback ultra-seguro de la spec MSFS.
    //     Cualquier entry cuyo `path` contiene "thumbnail" se usa tal
    //     cual. Streaming-read: bytes() en lugar de Value completo
    //     para no construir el árbol JSON entero (algunos layout.json
    //     pesan MB).
    if let Some(p) = thumbnail_from_layout(root) {
        return Some(p);
    }
    None
}

/// (v4.31.0) Busca `ContentInfo/<algo>/thumbnail.{jpg,png}` — el
/// thumbnail oficial del marketplace. ContentInfo tiene una subcarpeta
/// por paquete; buscamos un nivel adentro.
fn find_in_content_info(root: &Path) -> Option<String> {
    let ci = root.join("ContentInfo");
    let Ok(subs) = std::fs::read_dir(&ci) else {
        return None;
    };
    for sub in subs.flatten() {
        let dir = sub.path();
        if !dir.is_dir() {
            continue;
        }
        for ext in ["jpg", "png", "jpeg", "webp"] {
            let p = dir.join(format!("thumbnail.{ext}"));
            if p.is_file() && is_real_image(&p) {
                return Some(p.to_string_lossy().into_owned());
            }
        }
    }
    None
}

/// Veta el arte "PLACEHOLDER" del SDK (mismo bicho que en `command::
/// find_thumbnail`): tamaño <8 KB o nombre con "placeholder" => no.
fn is_real_image(path: &Path) -> bool {
    let lower = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_lowercase();
    if lower.contains("placeholder") {
        return false;
    }
    std::fs::metadata(path)
        .map(|m| m.len() >= 8 * 1024 && m.len() < 8 * 1024 * 1024)
        .unwrap_or(false)
}

/// Lee `layout.json` buscando la PRIMERA entry cuyo `"path"` contiene
/// la palabra "thumbnail". No deserializamos el JSON entero porque
/// layout.json puede pesar varios MB con miles de archivos; un parse
/// streaming-style basta para encontrar la primera línea válida.
fn thumbnail_from_layout(root: &Path) -> Option<String> {
    use std::io::{BufRead, BufReader};
    let layout = root.join("layout.json");
    let file = std::fs::File::open(&layout).ok()?;
    let reader = BufReader::new(file);
    for line in reader.lines() {
        let Ok(line) = line else { continue };
        let trimmed = line.trim();
        // Solo nos interesan líneas con "path" + "thumbnail". El
        // formato típico es `{ "path": "model.0/thumbnail.jpg", ... }`.
        if !trimmed.contains("\"path\"") {
            continue;
        }
        let lower = trimmed.to_lowercase();
        if !lower.contains("thumbnail") {
            continue;
        }
        // Extraer el valor entre las DOS primeras comillas que sigan
        // a `"path"`. Frágil pero suficiente para el formato canónico.
        let Some(after) = trimmed.split("\"path\"").nth(1) else {
            continue;
        };
        let mut chars = after.chars();
        // saltar al primer "
        while let Some(c) = chars.next() {
            if c == '"' {
                break;
            }
        }
        let mut buf = String::new();
        for c in chars {
            if c == '"' {
                break;
            }
            buf.push(c);
        }
        if buf.is_empty() {
            continue;
        }
        // El path es relativo al paquete y usa '/' como separador.
        let candidate = root.join(buf.replace('/', std::path::MAIN_SEPARATOR_STR));
        if candidate.is_file() && is_real_image(&candidate) {
            return Some(candidate.to_string_lossy().into_owned());
        }
    }
    None
}

fn parse_manifest(path: &Path) -> anyhow::Result<RawManifest> {
    let raw = std::fs::read_to_string(path)?;
    // M11: algunos manifests traen BOM UTF-8 al inicio.
    let raw = raw.strip_prefix('\u{feff}').unwrap_or(&raw);
    let parsed: RawManifest = serde_json::from_str(raw)?;
    Ok(parsed)
}

fn materialize(
    path: &Path,
    folder_name: &str,
    raw: RawManifest,
    enabled: bool,
    simobject_dirs: Vec<String>,
    base_containers: Vec<String>,
    thumbnail_path: Option<String>,
    has_own_model: bool,
    has_flight_model: bool,
    size_cache: &SizeCache,
) -> ScannedPackage {
    let title = raw
        .title
        .as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| pretty_folder_name(folder_name));

    let creator = raw
        .creator
        .or(raw.manufacturer)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let content_type = raw
        .content_type
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    // ICAO sólo tiene sentido para SCENERY. Aplicar la heurística
    // a AIRCRAFT/INSTRUMENT/MISC produce falsos positivos como
    // "PMDG" → matching contra el aeropuerto Palmar Sur — y
    // dispara notificaciones de update que el usuario reporta como
    // "no tienen sentido". Para todo lo que no sea SCENERY el
    // ICAO se queda en None de forma estricta.
    //
    // (v3.1.3) ORDEN INVERTIDO: probamos folder name PRIMERO, title
    // como fallback. Los folder names siguen convenciones rígidas
    // (`{vendor}-airport[s]-{ICAO}-{nombre}`), los titles son
    // free-form y dan falsos positivos:
    //   · "Paris Orly LFPO" → `ORLY` se pillaba antes que `LFPO`
    //   · "Saint Martin & Grand Case" → `CASE` se pillaba (y el
    //     título no contiene TNCM en absoluto). El folder
    //     `awdesigns-airports-tncm-tffg-saint-martin` SÍ lo tiene.
    let icao = if matches_scenery(&content_type) {
        extract_icao(folder_name).or_else(|| extract_icao(&title))
    } else {
        None
    };

    let (size_bytes, folder_modified_at) = folder_metadata(path, folder_name, size_cache);

    // (v4.25.0) Nombres de las dependencias del manifest. Cada entry
    // suele ser `{ "name": "fnx-aircraft-320", "package_version": … }`
    // — el name ES el folder name del paquete base, lo que nos permite
    // auto-enlazar livery → aircraft en el Link Map.
    let dependency_names: Vec<String> = raw
        .dependencies
        .iter()
        .filter_map(|v| v.get("name").and_then(|n| n.as_str()))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    let link = crate::folder_link::read(path);
    ScannedPackage {
        link_target: link.as_ref().map(|l| l.target.to_string_lossy().into_owned()),
        link_managed: link.is_some_and(|l| l.managed),
        also_in: None,
        folder_name: folder_name.to_string(),
        install_path: path.to_string_lossy().into_owned(),
        title,
        creator,
        content_type,
        package_version: raw.package_version.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()),
        minimum_game_version: raw
            .minimum_game_version
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
        icao,
        size_bytes,
        folder_modified_at,
        dependencies_count: raw.dependencies.len(),
        dependency_names,
        enabled,
        simobject_dirs,
        base_containers,
        icao_types: Vec::new(),
        last_update: last_update(path),
        thumbnail_path,
        has_own_model,
        has_flight_model,
        has_manifest: true,
    }
}

fn materialize_fallback(
    path: &Path,
    folder_name: &str,
    enabled: bool,
    simobject_dirs: Vec<String>,
    base_containers: Vec<String>,
    thumbnail_path: Option<String>,
    has_own_model: bool,
    has_flight_model: bool,
    has_manifest: bool,
    size_cache: &SizeCache,
) -> ScannedPackage {
    let (size_bytes, folder_modified_at) = folder_metadata(path, folder_name, size_cache);
    let title = pretty_folder_name(folder_name);
    // Sin manifest válido no podemos saber si es SCENERY. Conservador:
    // no extraemos ICAO para evitar falsos positivos cuando alguien
    // tiene un livery o sound pack con folder name que parezca ICAO.
    let link = crate::folder_link::read(path);
    ScannedPackage {
        link_target: link.as_ref().map(|l| l.target.to_string_lossy().into_owned()),
        link_managed: link.is_some_and(|l| l.managed),
        also_in: None,
        folder_name: folder_name.to_string(),
        install_path: path.to_string_lossy().into_owned(),
        title,
        creator: None,
        content_type: None,
        package_version: None,
        minimum_game_version: None,
        icao: None,
        size_bytes,
        folder_modified_at,
        dependencies_count: 0,
        dependency_names: Vec::new(),
        enabled,
        simobject_dirs,
        base_containers,
        icao_types: Vec::new(),
        last_update: last_update(path),
        thumbnail_path,
        has_own_model,
        has_flight_model,
        has_manifest,
    }
}

#[inline]
fn matches_scenery(content_type: &Option<String>) -> bool {
    content_type
        .as_deref()
        .map(|s| s.trim().eq_ignore_ascii_case("SCENERY"))
        .unwrap_or(false)
}

fn folder_metadata(
    path: &Path,
    folder_name: &str,
    size_cache: &SizeCache,
) -> (Option<u64>, Option<String>) {
    let meta = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(_) => return (None, None),
    };
    // (v3.1.0) "Recientemente añadido" — antes usábamos sólo
    // `modified()` que en Windows refleja la última escritura DENTRO
    // de la carpeta. Si el usuario instala manualmente un addon viejo
    // (descomprimiendo un .zip que preservaba mtimes originales) el
    // valor reflejaba la fecha del .zip, no la del install — y por
    // eso el dashboard sólo veía los drag-drop nuestros (que SÍ
    // generan mtime fresco al copiar).
    //
    // Fix: tomamos el MÁXIMO entre `created()` (birth time, en Windows
    // siempre presente y refleja la creación del folder en este disco
    // = momento del install) y `modified()` (último update). El que
    // sea más reciente gana, lo que cubre ambos casos: install nuevo
    // (created reciente) y edit reciente (modified más nuevo que
    // created).
    let to_secs = |t: std::time::SystemTime| -> Option<i64> {
        t.duration_since(std::time::UNIX_EPOCH)
            .ok()
            .map(|d| d.as_secs() as i64)
    };
    let created_secs = meta.created().ok().and_then(to_secs);
    let modified_secs = meta.modified().ok().and_then(to_secs);
    let chosen = match (created_secs, modified_secs) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    };
    let modified = chosen
        .and_then(|secs| chrono::DateTime::from_timestamp(secs, 0))
        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string());

    // (v6.2.25) Arranque incremental: si la firma de mtime del folder
    // no cambió desde el último scan, reutilizamos el tamaño cacheado
    // y evitamos el walk recursivo del árbol (lo caro a gran escala).
    // El mtime de un directorio en Windows/NTFS cambia al añadir,
    // quitar o renombrar sus hijos directos — justo lo que hace un
    // update/instalación —, así que un tamaño obsoleto sólo podría
    // venir de ediciones profundas sin tocar ningún índice de carpeta,
    // un caso marginal y meramente cosmético (el tamaño es un dato
    // informativo, no afecta correctitud).
    let size = match (&modified, size_cache.get(folder_name)) {
        (Some(m), Some((prev_mtime, prev_size))) if m == prev_mtime => Some(*prev_size),
        _ => directory_size(path).ok(),
    };
    (size, modified)
}

/// (04/11) Cuándo cambió el addon de verdad: lo último entre que su carpeta se
/// creó aquí (instalación) y las fechas de `manifest.json` y `layout.json`
/// (nueva versión). La fecha de la carpeta no vale: encender y apagar renombran
/// `layout.json`, y eso la mueve sin que el addon haya cambiado.
fn last_update(path: &Path) -> Option<String> {
    let secs = |t: std::io::Result<std::time::SystemTime>| {
        t.ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
    };
    let mut latest = secs(std::fs::metadata(path).ok()?.created());
    for file in ["manifest.json", "layout.json", "layout.json.disabled"] {
        if let Ok(meta) = std::fs::metadata(path.join(file)) {
            latest = latest.max(secs(meta.modified()));
        }
    }
    latest
        .and_then(|s| chrono::DateTime::from_timestamp(s, 0))
        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
}

/// Suma el tamaño de los archivos del folder. Se consulta on-demand
/// (no cacheada) — a 200 paquetes y miles de ficheros, sigue siendo
/// inferior a 1s en SSD. El cache caería en `community_packages`
/// y se invalidaría en cada scan.
fn directory_size(path: &Path) -> std::io::Result<u64> {
    let mut total: u64 = 0;
    let mut stack: Vec<PathBuf> = vec![path.to_path_buf()];
    while let Some(p) = stack.pop() {
        for entry in std::fs::read_dir(&p)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            if kind.is_dir() {
                stack.push(entry.path());
            } else if kind.is_file() {
                if let Ok(m) = entry.metadata() {
                    total = total.saturating_add(m.len());
                }
            }
        }
    }
    Ok(total)
}

/// Convierte `bravoairspace-airport-mdsd-las-americas` →
/// `Bravoairspace Airport Mdsd Las Americas`. Heurística para
/// fallback cuando no hay título en el manifest.
fn pretty_folder_name(folder: &str) -> String {
    folder
        .split(|c: char| c == '-' || c == '_')
        .filter(|s| !s.is_empty())
        .map(|s| {
            let mut chars = s.chars();
            match chars.next() {
                Some(c) => c.to_ascii_uppercase().to_string() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Extrae un ICAO (4 letras ASCII consecutivas) con frontera de
/// palabra. La validación contra airports reales se hace en el caller.
///
/// **(v1.1.4) Heurística "airport- wins"**: si hay múltiples 4-letras
/// candidates en el texto y al menos uno está inmediatamente precedido
/// por "airport-" / "airport_" / "airport " (típico de los packs de
/// scenery: `dev-airport-ICAO-name`), preferimos ése sobre los demás.
///
/// Bug que esto arregla: para `kaze-airport-mhtg-toncontin`, la
/// versión anterior elegía `KAZE` (developer name, casualmente ICAO
/// real de Hazlehurst GA) en vez de `MHTG` (Toncontin Honduras). El
/// resultado: Toncontin aparecía en EE.UU. en el mapa.
fn extract_icao(text: &str) -> Option<String> {
    let upper = text.to_ascii_uppercase();
    let bytes = upper.as_bytes();
    if bytes.len() < 4 {
        return None;
    }

    // Recolecta TODOS los candidatos con word boundary.
    let mut candidates: Vec<(usize, String)> = Vec::new();
    for i in 0..=bytes.len() - 4 {
        let candidate = &bytes[i..i + 4];
        if !candidate.iter().all(|b| b.is_ascii_alphabetic()) {
            continue;
        }
        let before_ok = i == 0 || !bytes[i - 1].is_ascii_alphanumeric();
        let after_ok = i + 4 == bytes.len() || !bytes[i + 4].is_ascii_alphanumeric();
        if before_ok && after_ok {
            if let Ok(s) = String::from_utf8(candidate.to_vec()) {
                candidates.push((i, s));
            }
        }
    }
    if candidates.is_empty() {
        return None;
    }
    if candidates.len() == 1 {
        return Some(candidates.into_iter().next().unwrap().1);
    }

    // Prioridad 1: candidato precedido inmediatamente por "AIRPORT"
    // o "AIRPORTS" + un separador (`-`, `_`, ` `). Los packs de
    // scenery siguen la convención `developer-airport-ICAO-nombre`,
    // y algunos vendors usan plural `developer-airports-ICAO-…`
    // (ej. `awdesigns-airports-tncm-tffg-saint-martin`).
    for (i, cand) in &candidates {
        // Match "AIRPORT-" / "AIRPORT_" / "AIRPORT " — 8 chars antes.
        if *i >= 8 {
            let prefix = &bytes[*i - 8..*i];
            if prefix.starts_with(b"AIRPORT")
                && matches!(prefix[7], b'-' | b'_' | b' ')
            {
                return Some(cand.clone());
            }
        }
        // (v3.1.3) Match "AIRPORTS-" plural — 9 chars antes.
        if *i >= 9 {
            let prefix = &bytes[*i - 9..*i];
            if prefix.starts_with(b"AIRPORTS")
                && matches!(prefix[8], b'-' | b'_' | b' ')
            {
                return Some(cand.clone());
            }
        }
    }

    // Prioridad 2: el primer match (comportamiento original). Usado
    // cuando ningún candidato tiene "airport-" delante — la mayoría
    // de folders fuera de la convención usan dev como prefijo opcional.
    Some(candidates.into_iter().next().unwrap().1)
}

/// (v7.2.6) Extrae el ICAO REAL de un escenario leyendo su ESTRUCTURA DE
/// ARCHIVOS — la fuente definitiva cuando el `manifest.json` no trae el ICAO
/// (muchos packs, sobre todo de MSFS 2024, sólo ponen el nombre "Taiyuan Wusu
/// International Airport" y el regex de nombre pesca una palabra de 4 letras
/// como "WUSU", que NO es el ICAO — el real es ZBYN). MSFS SIEMPRE nombra la
/// carpeta y el BGL del aeropuerto por su ICAO:
///     Scenery/airport-<icao>-<hash>/scenery/<ICAO>.bgl
/// Busca primero una carpeta `airport-<ICAO>-…` (señal fuerte) y, si no, un
/// archivo `<ICAO>.bgl`.
fn icao_from_scenery_files(root: &Path) -> Option<String> {
    let scenery = root.join("Scenery");
    if !scenery.is_dir() {
        return None;
    }
    // Recolectamos los ICAO candidatos de BGL: sólo confiamos en ellos si hay
    // UNO ÚNICO (un scenery suele traer varios BGL de 4 letras — objetos,
    // exclusiones, librerías — y elegir "el primero" da falsos positivos como
    // BALI/MYNN). La carpeta `airport-<icao>` sí es señal fuerte por sí sola.
    let mut bgl_icaos: std::collections::HashSet<String> = std::collections::HashSet::new();
    for entry in walkdir::WalkDir::new(&scenery)
        .max_depth(4)
        .into_iter()
        .flatten()
    {
        let name = entry.file_name().to_string_lossy();
        if entry.file_type().is_dir() {
            // `airport-ZBYN-…` → la carpeta lleva el ICAO.
            if let Some(icao) = icao_from_airport_folder(&name) {
                return Some(icao);
            }
        } else if let Some(icao) = icao_from_bgl_filename(&name) {
            bgl_icaos.insert(icao);
        }
    }
    if bgl_icaos.len() == 1 {
        bgl_icaos.into_iter().next()
    } else {
        None
    }
}

/// Igual que `extract_icao` pero devuelve SÓLO el candidato precedido por
/// `airport-`/`airports-` (la convención `dev-airport-<ICAO>-nombre`), sin
/// caer al primer token de 4 letras. Es la señal MÁS fiable del ICAO y tiene
/// prioridad sobre la estructura de archivos (que puede traer un template con
/// otro ICAO, p. ej. un `airport-MYNN` bundleado).
fn extract_icao_airport_prefixed(text: &str) -> Option<String> {
    let upper = text.to_ascii_uppercase();
    let bytes = upper.as_bytes();
    if bytes.len() < 4 {
        return None;
    }
    for i in 0..=bytes.len() - 4 {
        let cand = &bytes[i..i + 4];
        if !cand.iter().all(|b| b.is_ascii_alphabetic()) {
            continue;
        }
        let after_ok = i + 4 == bytes.len() || !bytes[i + 4].is_ascii_alphanumeric();
        if !after_ok {
            continue;
        }
        if i >= 8 {
            let p = &bytes[i - 8..i];
            if p.starts_with(b"AIRPORT") && matches!(p[7], b'-' | b'_' | b' ') {
                return String::from_utf8(cand.to_vec()).ok();
            }
        }
        if i >= 9 {
            let p = &bytes[i - 9..i];
            if p.starts_with(b"AIRPORTS") && matches!(p[8], b'-' | b'_' | b' ') {
                return String::from_utf8(cand.to_vec()).ok();
            }
        }
    }
    None
}

/// (07/11) Etapa F: código corto de aeropuerto (`S50`, `0S9`, `EG78`) en un
/// patrón claro: justo detrás de `airport-`. Lleva algún dígito (el de 4 letras
/// ya lo resuelve `extract_icao_airport_prefixed`) y alguna letra.
fn short_airport_code(folder: &str) -> Option<String> {
    static RE: once_cell::sync::Lazy<regex::Regex> = once_cell::sync::Lazy::new(|| {
        regex::Regex::new(r"(?i)(?:^|[-_ ])airports?[-_ ]([a-z0-9]{3,4})(?:[-_ ]|$)").expect("regex")
    });
    let code = RE.captures(folder)?.get(1)?.as_str().to_ascii_uppercase();
    let mixed = code.chars().any(|c| c.is_ascii_digit()) && code.chars().any(|c| c.is_ascii_alphabetic());
    mixed.then_some(code)
}

/// El código como está en la base de aeropuertos, directo o con la K de
/// EE. UU. delante (`S50` → `KS50`). Si no está, no se acepta: sin él no habría
/// sitio donde poner el pin y podría ser cualquier otra cosa.
fn known_short_code(code: &str, valid: &std::collections::HashSet<String>) -> Option<String> {
    [code.to_string(), format!("K{code}")].into_iter().find(|c| valid.contains(c))
}

/// `airport-zbyn---` → `Some("ZBYN")`. Requiere el prefijo `airport-` y 4
/// letras ASCII a continuación.
fn icao_from_airport_folder(name: &str) -> Option<String> {
    let rest = name.to_ascii_lowercase();
    let rest = rest.strip_prefix("airport-")?;
    let icao: String = rest.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
    if icao.len() == 4 {
        Some(icao.to_ascii_uppercase())
    } else {
        None
    }
}

/// `ZBYN.bgl` → `Some("ZBYN")`; `ZBYN_OBJ.bgl` → `Some("ZBYN")`. Sólo acepta un
/// stem de EXACTAMENTE 4 letras o 4 letras seguidas de separador (evita
/// `objects.bgl`, `vegetation.bgl`, etc.).
fn icao_from_bgl_filename(name: &str) -> Option<String> {
    let lower = name.to_ascii_lowercase();
    let stem = lower.strip_suffix(".bgl")?;
    let bytes = stem.as_bytes();
    if bytes.len() < 4 || !bytes[..4].iter().all(|b| b.is_ascii_alphabetic()) {
        return None;
    }
    if bytes.len() == 4 || matches!(bytes[4], b'-' | b'_' | b' ' | b'.') {
        Some(stem[..4].to_ascii_uppercase())
    } else {
        None
    }
}

/// (v6.2.25) Carga el cache de tamaños del scan anterior desde la DB
/// (`folder_name → (folder_modified_at, size_bytes)`). Alimenta el
/// arranque incremental: los folders cuya firma de mtime no cambió
/// reutilizan su tamaño y se saltan el walk recursivo. Best-effort:
/// si la query falla, se devuelve vacío y el scan recalcula todo.
pub async fn load_size_cache(pool: &SqlitePool) -> SizeCache {
    let rows: Vec<(String, Option<String>, Option<i64>)> = sqlx::query_as(
        "SELECT folder_name, folder_modified_at, size_bytes FROM community_packages",
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    rows.into_iter()
        .filter_map(|(folder, mtime, size)| {
            let mtime = mtime?;
            let size = size?;
            if size < 0 {
                return None;
            }
            Some((folder, (mtime, size as u64)))
        })
        .collect()
}

/// Sincroniza los resultados del scan con la base de datos. Borra
/// entradas que ya no existen físicamente (paquete desinstalado
/// fuera de la app) y hace upsert del resto.
pub async fn sync_to_db(pool: &SqlitePool, report: &ScanReport) -> anyhow::Result<usize> {
    let mut tx = pool.begin().await?;

    // Borra los que ya no están — por exclusión del set actual.
    // (audit) Si no se encontró NADA, la Community probablemente no
    // existe o el disco está desconectado — no purgar la DB entera.
    if report.packages.is_empty() {
        tracing::warn!(
            target: "scanner",
            "sync_to_db: 0 paquetes escaneados en {}; se conservan las filas existentes",
            report.community_path
        );
    } else {
        // SQLite no soporta `DELETE … WHERE x NOT IN (?,?,…)` con
        // `bind` plural genérico; armamos placeholders manualmente.
        let mut sql = String::from("DELETE FROM community_packages WHERE folder_name NOT IN (");
        let mut first = true;
        for _ in &report.packages {
            if !first {
                sql.push(',');
            }
            sql.push('?');
            first = false;
        }
        sql.push(')');
        let mut q = sqlx::query(&sql);
        for p in &report.packages {
            q = q.bind(&p.folder_name);
        }
        q.execute(&mut *tx).await?;
    }

    // (v7.2.6) Set de ICAOs REALES (OurAirports) para validar el candidato de
    // nombre y NO persistir una palabra de 4 letras que no es un aeropuerto
    // (p. ej. "WUSU" del título "Taiyuan Wusu International Airport"). Si el
    // dataset aún no cargó (set vacío), no validamos.
    let valid_icaos: std::collections::HashSet<String> =
        sqlx::query_scalar::<_, String>("SELECT icao FROM airports")
            .fetch_all(&mut *tx)
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|s| s.trim().to_ascii_uppercase())
            .collect();

    // Upsert por folder_name. Reemplazamos todos los campos en cada
    // scan — más simple que diff incremental y suficientemente rápido.
    for pkg in &report.packages {
        // (v4.28.0) Limpieza de icaos basura del scanner viejo: si el
        // pack es library_pack OR no es SCENERY → forzamos icao=NULL
        // para que no se cuele en el mapa. El extract_icao() de
        // materialize() ya no asigna a no-SCENERY, pero rows de scans
        // anteriores quedaron con icao=BASE/CITY/MISC/MSFS/HONG…
        let icao_to_persist = if matches_scenery(&pkg.content_type)
            && !is_library_pack(&pkg.title, &pkg.folder_name)
        {
            // (v7.2.6) Orden de confianza para el ICAO (cada paso validado
            // contra el dataset de aeropuertos reales salvo que aún no cargó):
            //  1. Nombre con convención `…-airport-<ICAO>-…` — la señal más
            //     fiable (KMSP, OEJN, KAUS…).
            //  2. ESTRUCTURA del escenario: carpeta `airport-<icao>` o un ÚNICO
            //     BGL `<ICAO>.bgl` — resuelve los packs cuyo nombre NO trae el
            //     ICAO (p. ej. "millet2732s-taiyuanwusu" → ZBYN del BGL, en vez
            //     de "WUSU" que es sólo una palabra del título).
            //  3. Cualquier token de 4 letras del nombre/título, pero SÓLO si es
            //     un aeropuerto real (evita persistir "WUSU").
            let validate = |ic: Option<String>| -> Option<String> {
                let up = ic?.trim().to_ascii_uppercase();
                if up.is_empty() {
                    None
                } else if valid_icaos.is_empty() || valid_icaos.contains(&up) {
                    Some(up)
                } else {
                    None
                }
            };
            let path = std::path::Path::new(&pkg.install_path);
            validate(extract_icao_airport_prefixed(&pkg.folder_name))
                .or_else(|| short_airport_code(&pkg.folder_name).and_then(|c| known_short_code(&c, &valid_icaos)))
                .or_else(|| validate(extract_icao_airport_prefixed(&pkg.title)))
                .or_else(|| validate(icao_from_scenery_files(path)))
                .or_else(|| validate(pkg.icao.clone()))
        } else {
            None
        };
        let simobject_dirs_json = serde_json::to_string(&pkg.simobject_dirs)
            .unwrap_or_else(|_| "[]".to_string());
        let base_containers_json = serde_json::to_string(&pkg.base_containers)
            .unwrap_or_else(|_| "[]".to_string());
        let icao_types_json = serde_json::to_string(&pkg.icao_types)
            .unwrap_or_else(|_| "[]".to_string());
        sqlx::query(
            r#"
            INSERT INTO community_packages (
                folder_name, install_path, title, creator, content_type,
                package_version, minimum_game_version, icao, size_bytes,
                folder_modified_at, dependencies_count, scanned_at, enabled,
                simobject_dirs_json, base_containers_json, thumbnail_path,
                has_own_model, has_flight_model, has_manifest, icao_types_json, last_update,
                link_target, link_managed, also_in
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, datetime('now'), ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23)
            ON CONFLICT(folder_name) DO UPDATE SET
                install_path = excluded.install_path,
                title = excluded.title,
                creator = excluded.creator,
                content_type = excluded.content_type,
                package_version = excluded.package_version,
                minimum_game_version = excluded.minimum_game_version,
                icao = excluded.icao,
                size_bytes = excluded.size_bytes,
                folder_modified_at = excluded.folder_modified_at,
                dependencies_count = excluded.dependencies_count,
                scanned_at = datetime('now'),
                enabled = excluded.enabled,
                simobject_dirs_json = excluded.simobject_dirs_json,
                base_containers_json = excluded.base_containers_json,
                thumbnail_path = excluded.thumbnail_path,
                has_own_model = excluded.has_own_model,
                has_flight_model = excluded.has_flight_model,
                has_manifest = excluded.has_manifest,
                icao_types_json = excluded.icao_types_json,
                last_update = excluded.last_update,
                link_target = excluded.link_target,
                link_managed = excluded.link_managed,
                also_in = excluded.also_in
            "#,
        )
        .bind(&pkg.folder_name)
        .bind(&pkg.install_path)
        .bind(&pkg.title)
        .bind(&pkg.creator)
        .bind(&pkg.content_type)
        .bind(&pkg.package_version)
        .bind(&pkg.minimum_game_version)
        .bind(&icao_to_persist)
        .bind(pkg.size_bytes.map(|n| n as i64))
        .bind(&pkg.folder_modified_at)
        .bind(pkg.dependencies_count as i64)
        .bind(pkg.enabled)
        .bind(&simobject_dirs_json)
        .bind(&base_containers_json)
        .bind(&pkg.thumbnail_path)
        .bind(pkg.has_own_model)
        .bind(pkg.has_flight_model)
        .bind(pkg.has_manifest)
        .bind(&icao_types_json)
        .bind(&pkg.last_update)
        .bind(&pkg.link_target)
        .bind(pkg.link_managed)
        .bind(&pkg.also_in)
        .execute(&mut *tx)
        .await?;
    }

    // (v4.25.0) Mantenimiento del grafo de enlaces (Link Map):
    //   1. Borra aristas y posiciones cuyos extremos ya no existen
    //      (paquete desinstalado fuera de la app).
    //   2. Re-siembra las aristas 'auto' desde manifest.dependencies —
    //      cada livery/soundpack declara su paquete base por folder
    //      name. Las aristas 'manual' del usuario nunca se tocan.
    sqlx::query(
        r#"
        DELETE FROM addon_links
        WHERE source_folder NOT IN (SELECT folder_name FROM community_packages)
           OR target_folder NOT IN (SELECT folder_name FROM community_packages)
        "#,
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        r#"
        DELETE FROM addon_link_positions
        WHERE folder_name NOT IN (SELECT folder_name FROM community_packages)
        "#,
    )
    .execute(&mut *tx)
    .await?;
    {
        // (v4.26.0) Auto-link v2 — tres niveles de evidencia, en orden
        // de fiabilidad. Solo entre ADDONS (no SCENERY): el Link Map
        // es de aviones/liveries/sonidos, y los aeropuertos comparten
        // prefijos de vendor que generarían falsos enlaces.
        //   1. manifest.dependencies[].name == folder instalado.
        //   2. base_container del aircraft.cfg de la livery apunta al
        //      contenedor SimObjects de OTRO paquete (la referencia
        //      REAL que usa MSFS — cubre liveries cuyo folder no se
        //      parece en nada al avión, p.ej. "Etihad A6-API").
        //   3. Prefijo de folder: `fnx-aircraft-320-DLHDAIQSAL-CFM`
        //      empieza con `fnx-aircraft-320` (≥6 chars, el más largo
        //      gana).
        let is_addon =
            |p: &ScannedPackage| !matches_scenery(&p.content_type);
        let by_lower: std::collections::HashMap<String, &str> = report
            .packages
            .iter()
            .filter(|p| is_addon(p))
            .map(|p| (p.folder_name.to_lowercase(), p.folder_name.as_str()))
            .collect();
        // Dueño de cada contenedor SimObjects (en minúsculas).
        let mut owner_of_container: std::collections::HashMap<String, &str> =
            std::collections::HashMap::new();
        for p in report.packages.iter().filter(|p| is_addon(p)) {
            for d in &p.simobject_dirs {
                owner_of_container
                    .entry(d.to_lowercase())
                    .or_insert(p.folder_name.as_str());
            }
        }
        // (v4.33.0) Nivel 4: AVIONES por FAMILIA. Un addon (sound/util/
        // livery genérica) que menciona un modelo se conecta al/los
        // avión(es) instalado(s) de esa familia — la "correlación" que
        // pidió el usuario, sin internet. Ej: "BAW_CRJ" → Aerosoft CRJ;
        // "AICopilot PMDG B777" → PMDG 777-300ER. Solo los paquetes con
        // modelo 3D propio cuentan como destino-avión.
        let mut aircraft_by_family: std::collections::HashMap<String, Vec<&str>> =
            std::collections::HashMap::new();
        for p in report.packages.iter().filter(|p| is_addon(p) && p.has_own_model) {
            if let Some(fam) = aircraft_family(&format!("{} {}", p.title, p.folder_name)) {
                aircraft_by_family
                    .entry(fam)
                    .or_default()
                    .push(p.folder_name.as_str());
            }
        }
        let mut seeded = 0usize;
        for pkg in report.packages.iter().filter(|p| is_addon(p)) {
            let mut sources: Vec<&str> = Vec::new();
            // Nivel 1 — dependencias del manifest.
            for dep in &pkg.dependency_names {
                if let Some(s) = by_lower.get(&dep.to_lowercase()) {
                    sources.push(s);
                }
            }
            // Nivel 2 — base_container del aircraft.cfg.
            for bc in &pkg.base_containers {
                if let Some(s) = owner_of_container.get(&bc.to_lowercase()) {
                    sources.push(s);
                }
            }
            // Nivel 3 — prefijo de folder (solo si 1 y 2 no dieron nada).
            if sources
                .iter()
                .all(|s| s.eq_ignore_ascii_case(&pkg.folder_name))
            {
                let pkg_lower = pkg.folder_name.to_lowercase();
                let mut best: Option<&str> = None;
                for (cand_lower, cand) in &by_lower {
                    if cand_lower.len() >= 6
                        && *cand_lower != pkg_lower
                        && pkg_lower.starts_with(cand_lower.as_str())
                        && best.map_or(true, |b| cand.len() > b.len())
                    {
                        best = Some(cand);
                    }
                }
                if let Some(b) = best {
                    sources.push(b);
                }
            }
            // Nivel 4 — correlación por familia de avión. Solo si los
            // niveles 1-3 no dieron source Y el propio paquete NO es un
            // avión (un avión no se conecta a otro avión por familia).
            let no_source_yet = sources
                .iter()
                .all(|s| s.eq_ignore_ascii_case(&pkg.folder_name));
            if no_source_yet && !pkg.has_own_model {
                if let Some(fam) =
                    aircraft_family(&format!("{} {}", pkg.title, pkg.folder_name))
                {
                    if let Some(planes) = aircraft_by_family.get(&fam) {
                        for plane in planes {
                            if !plane.eq_ignore_ascii_case(&pkg.folder_name) {
                                sources.push(plane);
                            }
                        }
                    }
                }
            }
            for source in sources {
                if source.eq_ignore_ascii_case(&pkg.folder_name) {
                    continue; // self-loop (el avión referencia su propio contenedor)
                }
                let r = sqlx::query(
                    r#"
                    INSERT OR IGNORE INTO addon_links
                        (source_folder, target_folder, origin)
                    VALUES (?1, ?2, 'auto')
                    "#,
                )
                .bind(source)
                .bind(&pkg.folder_name)
                .execute(&mut *tx)
                .await?;
                seeded += r.rows_affected() as usize;
            }
        }
        if seeded > 0 {
            tracing::info!(target: "scan", "auto-link: {} enlaces nuevos sembrados", seeded);
        }
    }

    tx.commit().await?;

    // (v4.24.0) RESCATE de ICAOs embebidos SIN separador. `extract_icao`
    // exige word boundaries, así que folders tipo `montevideosumu` (el
    // ICAO pegado al final) quedaban con icao NULL y el aeropuerto no
    // aparecía en el mapa (bug reportado con SUMU). Acá probamos TODAS
    // las ventanas de 4 letras del folder/título VALIDÁNDOLAS contra la
    // tabla `airports` (OurAirports) — solo se acepta un match real, con
    // preferencia por sufijo/prefijo para evitar falsos positivos.
    let unresolved: Vec<(String, Option<String>)> = sqlx::query_as(
        r#"
        SELECT folder_name, title FROM community_packages
        WHERE (icao IS NULL OR icao = '')
          AND UPPER(COALESCE(content_type, '')) = 'SCENERY'
        "#,
    )
    .fetch_all(pool)
    .await?;
    for (folder, title) in unresolved {
        // (v4.24.1) Los packs librería/complemento NO se rescatan: el
        // rescue v4.24.0 promovía pseudo-ICAOs reales escondidos en
        // palabras comunes (ENHA en "enhanced", SING en "singapore",
        // VIGR en "navigraph") y llenó el mapa de night-lights/AIRAC
        // como si fueran aeropuertos (verificado en la DB del usuario:
        // 58 packs "Night Enhanced" rescatados por error).
        if is_library_pack(title.as_deref().unwrap_or(""), &folder) {
            continue;
        }
        // (v4.31.0) Rescue inteligente que resuelve AMBOS casos
        // patológicos del usuario:
        //   · `montevideosumu` → SUMU (sufijo ICAO deliberado: el
        //     prefijo "montevideo" es EXACTAMENTE el municipio de un
        //     aeropuerto, así que "sumu" es el código que el dev pegó).
        //   · `gakki-taiyuanwushu` → ZBYN (el sufijo "ushu"/USHU es
        //     coincidencia con el nombre "Wushu"; el prefijo "taiyuanw"
        //     NO es un municipio exacto, así que ganamos por nombre).
        let rescued = smart_rescue_icao(pool, &folder, title.as_deref()).await;
        if let Some(icao) = rescued {
            sqlx::query("UPDATE community_packages SET icao = ?1 WHERE folder_name = ?2")
                .bind(&icao)
                .bind(&folder)
                .execute(pool)
                .await?;
            tracing::info!(
                target: "scan",
                "icao rescatado para '{}': {} (substring validado contra airports)",
                folder, icao
            );
        }
    }

    Ok(report.packages.len())
}

/// (v4.33.0) Familia de avión mencionada en el texto, normalizada a
/// una clave estable (a320, b777, crj, md11…). Une variantes a una
/// sola familia para que un addon (sound/tool/livery) se correlacione
/// con el avión base instalado de esa familia. None si no hay modelo
/// reconocible. El ORDEN importa: probamos las claves más
/// específicas/largas primero para no confundir 737 con 73x, etc.
pub fn aircraft_family(text: &str) -> Option<String> {
    let t = text.to_lowercase();
    use once_cell::sync::Lazy;
    use regex::Regex;
    // (patrón regex con word-boundary, familia canónica). Primer match
    // gana — el orden coloca las claves más específicas primero.
    static RES: Lazy<Vec<(Regex, &'static str)>> = Lazy::new(|| {
        [
            (r"\b(?:a3(?:1[89]|2[01])|a32nx|a20n)\b", "a320"),
            (r"\b(?:a330|a338|a339|a33n)\b", "a330"),
            (r"\ba340\b", "a340"),
            (r"\b(?:a350|a359|a35k)\b", "a350"),
            (r"\b(?:a380|a388|a380x)\b", "a380"),
            (r"\b(?:747|74[478]|jumbo)\b", "b747"),
            (r"\b(?:777|77w|77l|77f|772|773|77er)\b", "b777"),
            (r"\b(?:787|78[789]|dreamliner)\b", "b787"),
            (r"\b(?:767|76[0-9])\b", "b767"),
            (r"\b(?:757)\b", "b757"),
            (r"\b(?:737|738|739|73m|73x|b38m)\b", "b737"),
            (r"\bcrj\b", "crj"),
            (r"\bmd[\s-]?11\b", "md11"),
            (r"\b(?:md[\s-]?8[0-9]|md[\s-]?90)\b", "md80"),
            (r"\batr\b", "atr"),
            (r"\b(?:e1?[79][05]|embraer)\b", "ejet"),
            (r"\b(?:dh[c]?[\s-]?8|q400)\b", "dash8"),
            (r"\btbm\b", "tbm"),
            (r"\b(?:c172|cessna)\b", "c172"),
        ]
        .iter()
        .map(|(p, f)| (Regex::new(p).unwrap(), *f))
        .collect()
    });
    for (re, fam) in RES.iter() {
        if re.is_match(&t) {
            return Some((*fam).to_string());
        }
    }
    None
}

/// (v4.24.1) ¿El paquete es una LIBRERÍA / complemento y no un
/// aeropuerto? Única fuente de verdad para el mapa Y el dashboard: el
/// flag viaja en CommunityPackageRow (`isLibraryPack`) y el conteo de
/// AIRPORTS del dashboard lo usa igual — así ambos números coinciden
/// SIEMPRE y los packs de AIRAC/night lights/enhancements/excludes no
/// aparecen como aeropuertos (pedido del usuario).
/// (v6.1) Una livery dentro de un pack: la entrada `[FLTSIM.N]` del aircraft.cfg.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackLivery {
    /// `title` de la entrada (nombre que muestra MSFS).
    pub title: String,
    /// `atc_id` — la matrícula (p.ej. "N399DA").
    pub registration: Option<String>,
    /// `atc_airline` — aerolínea visible.
    pub airline: Option<String>,
    /// `texture` — sufijo de la variante de textura.
    pub texture: Option<String>,
    /// Contenedor (carpeta bajo SimObjects/Airplanes) al que pertenece.
    pub container: String,
}

/// (v6.1) Enumera las liveries de un pack: lee cada
/// `SimObjects/{Airplanes,Rotorcraft}/<dir>/aircraft.cfg` y devuelve una entrada
/// por bloque `[FLTSIM.N]` con su título + matrícula + aerolínea. Para que al
/// clicar un livery pack (en Addons o Link Map) se vea qué liveries trae.
pub fn list_pack_liveries(install_path: &str) -> Vec<PackLivery> {
    let root = Path::new(install_path);
    let mut out: Vec<PackLivery> = Vec::new();
    for kind in ["Airplanes", "Rotorcraft"] {
        let so = root.join("SimObjects").join(kind);
        let Ok(entries) = std::fs::read_dir(&so) else {
            continue;
        };
        for entry in entries.flatten() {
            let dir = entry.path();
            if !dir.is_dir() {
                continue;
            }
            let container = dir
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            let cfg = dir.join("aircraft.cfg");
            if let Ok(raw) = std::fs::read_to_string(&cfg).or_else(|_| {
                std::fs::read(&cfg).map(|b| String::from_utf8_lossy(&b).into_owned())
            }) {
                parse_fltsim_blocks(&raw, &container, &mut out);
            }
            // (v6.2.21) Formato FENIX: sus packs de liveries no traen
            // aircraft.cfg con [FLTSIM.N] — cada livery vive en
            // `<container>/liveries/**/livery.cfg` con `[GENERAL] name` +
            // `[FLTSIM] atc_id/atc_airline`. Sin esto el modal salía VACÍO
            // para packs Fenix (reportado con fnx-aircraft-321-liveries).
            let liveries_dir = dir.join("liveries");
            if liveries_dir.is_dir() {
                for e in walkdir::WalkDir::new(&liveries_dir)
                    .max_depth(4)
                    .into_iter()
                    .flatten()
                {
                    if e.file_type().is_file()
                        && e.file_name().eq_ignore_ascii_case("livery.cfg")
                    {
                        if let Ok(raw) = std::fs::read_to_string(e.path()).or_else(|_| {
                            std::fs::read(e.path())
                                .map(|b| String::from_utf8_lossy(&b).into_owned())
                        }) {
                            parse_fenix_livery_cfg(&raw, &container, &mut out);
                        }
                    }
                }
            }
        }
    }
    // Ordena por aerolínea y luego matrícula para una lista legible.
    out.sort_by(|a, b| {
        a.airline
            .cmp(&b.airline)
            .then(a.registration.cmp(&b.registration))
            .then(a.title.cmp(&b.title))
    });
    out
}

/// (v6.2.21) Parsea un `livery.cfg` de FENIX: `[GENERAL] name = "..."` +
/// `[FLTSIM] atc_id = / atc_airline =`. Un archivo = una livery.
fn parse_fenix_livery_cfg(raw: &str, container: &str, out: &mut Vec<PackLivery>) {
    let unquote = |v: &str| -> Option<String> {
        let s = v.trim().trim_matches('"').trim().to_string();
        if s.is_empty() {
            None
        } else {
            Some(s)
        }
    };
    let mut title = String::new();
    let mut registration: Option<String> = None;
    let mut airline: Option<String> = None;
    let mut section = String::new();
    for line in raw.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            section = t.to_ascii_lowercase();
            continue;
        }
        let Some((k, v)) = t.split_once('=') else {
            continue;
        };
        let key = k.trim().to_ascii_lowercase();
        match (section.as_str(), key.as_str()) {
            ("[general]", "name") => {
                if let Some(s) = unquote(v) {
                    title = s;
                }
            }
            ("[fltsim]", "atc_id") => registration = unquote(v),
            ("[fltsim]", "atc_airline") => airline = unquote(v),
            // (04/11) Algunas pinturas de PMDG lo traen en [GENERAL].
            (_, "atc_id") if registration.is_none() => registration = unquote(v),
            (_, "atc_airline") if airline.is_none() => airline = unquote(v),
            _ => {}
        }
    }
    if !title.is_empty() {
        out.push(PackLivery {
            title,
            registration,
            airline,
            texture: None,
            container: container.to_string(),
        });
    }
}

/// Parsea los bloques `[FLTSIM.N]` de un aircraft.cfg y empuja una `PackLivery`
/// por cada uno que tenga al menos `title`.
fn parse_fltsim_blocks(raw: &str, container: &str, out: &mut Vec<PackLivery>) {
    let unquote = |v: &str| -> Option<String> {
        let s = v.trim().trim_matches('"').trim().to_string();
        if s.is_empty() {
            None
        } else {
            Some(s)
        }
    };
    let mut in_block = false;
    let mut cur: Option<PackLivery> = None;
    let flush = |cur: &mut Option<PackLivery>, out: &mut Vec<PackLivery>| {
        if let Some(l) = cur.take() {
            if !l.title.is_empty() {
                out.push(l);
            }
        }
    };
    for line in raw.lines() {
        let t = line.trim();
        let low = t.to_ascii_lowercase();
        if low.starts_with("[fltsim.") {
            flush(&mut cur, out);
            in_block = true;
            cur = Some(PackLivery {
                title: String::new(),
                registration: None,
                airline: None,
                texture: None,
                container: container.to_string(),
            });
            continue;
        }
        // Otra sección [..] cierra el bloque fltsim.
        if t.starts_with('[') {
            flush(&mut cur, out);
            in_block = false;
            continue;
        }
        if !in_block {
            continue;
        }
        let Some((key, val)) = t.split_once('=') else {
            continue;
        };
        let Some(l) = cur.as_mut() else { continue };
        match key.trim().to_ascii_lowercase().as_str() {
            "title" => {
                if let Some(v) = unquote(val) {
                    l.title = v;
                }
            }
            "atc_id" => l.registration = unquote(val),
            "atc_airline" => l.airline = unquote(val),
            "texture" => l.texture = unquote(val),
            _ => {}
        }
    }
    flush(&mut cur, out);
}

pub fn is_library_pack(title: &str, folder_name: &str) -> bool {
    use once_cell::sync::Lazy;
    use regex::Regex;
    static RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(
            r"(?ix)\b(
              librar(?:y|ies) | developers?\s*pack | object\s*pack | asset\s*pack |
              jetways? | vehicles? | vehicle\s*pack | vegetation | trees? | grass |
              autogen | landmark\s*packs? | asobo\s*objects? | simobjects? | sdk | placeholder |
              airac | navdata | navigraph |
              night\s*-?\s*lights? | nightlights? | lights?\s*pack |
              enhancements? | enhanced |
              excludes? | excluder | merge | aerials? | ortho | mesh | photogrammetry |
              city\s*pack | citypack | static\s*aircraft |
              # v4.26.0 - GSX y sus packs de reemplazo NO son aeropuertos.
              # El rescue de prefijos validaba FSDR -fsdreamteam- contra
              # Desroches Airport y GSX Pro aparecia en el mapa; los
              # z-fsdreamteam-gsx-x de HAECO/CASL/catering igual.
              fsdreamteam-gsx | gsx[\s-]*pro | gsx\s*world
            )\b",
        )
        .unwrap()
    });
    let hay = format!("{} {}", title, folder_name);
    RE.is_match(&hay)
}

/// (v4.27.0) Resuelve ICAO por NOMBRE del aeropuerto. Tokeniza el
/// folder/title (incluyendo prefijos progresivos para "taiyuanwushu"
/// sin separador), busca cada token >=5 chars contra `airports.name`
/// (`LIKE '%token%'`), y devuelve el ICAO **sólo si todas las
/// coincidencias apuntan al mismo aeropuerto** (resultado único).
/// Caso real verificado: gakki-taiyuanwushu → "taiyuan" matchea
/// "Taiyuan Wusu International Airport" → ZBYN.
/// (v4.31.0) Orquestador del rescue. Combina tres señales en orden
/// de fiabilidad para distinguir un ICAO deliberado de una
/// coincidencia ortográfica:
///   1. **Sufijo ICAO ciudad-pegada**: las últimas 4 letras del
///      folder son un ICAO válido Y el prefijo restante es EXACTAMENTE
///      el municipio de un aeropuerto → deliberado (montevideo+sumu).
///   2. **Nombre del aeropuerto**: un token del folder matchea único
///      en airports.name/municipality (taiyuan → ZBYN).
///   3. **Sufijo/prefijo embebido** como último recurso.
async fn smart_rescue_icao(
    pool: &SqlitePool,
    folder: &str,
    title: Option<&str>,
) -> Option<String> {
    // (1) Sufijo ICAO con ciudad pegada delante.
    let lower = folder.to_lowercase();
    let alpha: String = lower.chars().filter(|c| c.is_ascii_alphabetic()).collect();
    if alpha.len() >= 6 {
        let suffix = &alpha[alpha.len() - 4..];
        let prefix = &alpha[..alpha.len() - 4];
        let suffix_upper = suffix.to_uppercase();
        // ¿El sufijo es un ICAO real?
        let suffix_is_icao = sqlx::query_scalar::<_, String>(
            "SELECT icao FROM airports WHERE icao = ?1 LIMIT 1",
        )
        .bind(&suffix_upper)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
        .is_some();
        if suffix_is_icao && prefix.len() >= 4 {
            // ¿El prefijo es EXACTAMENTE el municipio (o nombre sin
            // espacios) de algún aeropuerto? Entonces el dev escribió
            // ciudad+ICAO a propósito.
            let prefix_is_city = sqlx::query_scalar::<_, i64>(
                r#"
                SELECT 1 FROM airports
                WHERE LOWER(REPLACE(municipality,' ','')) = ?1
                   OR LOWER(REPLACE(name,' ','')) = ?1
                LIMIT 1
                "#,
            )
            .bind(prefix)
            .fetch_optional(pool)
            .await
            .ok()
            .flatten()
            .is_some();
            if prefix_is_city {
                return Some(suffix_upper);
            }
        }
    }
    // (2) Por nombre del aeropuerto.
    for text in [Some(folder), title].into_iter().flatten() {
        if let Some(icao) = rescue_by_airport_name(pool, text).await {
            return Some(icao);
        }
    }
    // (3) Sufijo/prefijo embebido (último recurso).
    for text in [Some(folder), title].into_iter().flatten() {
        if let Some(icao) = rescue_embedded_icao(pool, text).await {
            return Some(icao);
        }
    }
    None
}

async fn rescue_by_airport_name(pool: &SqlitePool, text: &str) -> Option<String> {
    let lower = text.to_lowercase();
    let mut tokens: std::collections::HashSet<String> = std::collections::HashSet::new();
    for raw in lower.split(|c: char| !c.is_ascii_alphabetic()) {
        if raw.len() >= 5 {
            tokens.insert(raw.to_string());
        }
        // Sin separador (`taiyuanwushu`) → probamos prefijos progresivos
        // de 5..=11 caracteres. Si "taiyuan" matchea único y nada más
        // contradice, ganamos.
        if raw.len() > 11 {
            for n in 5..=11 {
                tokens.insert(raw[..n].to_string());
            }
        }
    }
    if tokens.is_empty() {
        return None;
    }
    let mut winners: std::collections::HashSet<String> =
        std::collections::HashSet::new();
    for tok in &tokens {
        let pattern = format!("%{}%", tok);
        let rows = sqlx::query_scalar::<_, String>(
            "SELECT icao FROM airports WHERE LOWER(name) LIKE ?1 LIMIT 2",
        )
        .bind(&pattern)
        .fetch_all(pool)
        .await
        .ok()?;
        // Tokens que matchean MUCHOS aeropuertos no son útiles
        // ("inter"/"airport" matchearían cientos) — los descartamos.
        if rows.len() == 1 {
            winners.insert(rows[0].clone());
        }
    }
    if winners.len() == 1 {
        return winners.into_iter().next();
    }
    None
}

/// (v4.24.0) Busca un ICAO embebido en `text` sin word boundaries: prueba
/// cada ventana de 4 letras contra la tabla `airports`. Selección:
///   1. ventana al FINAL del string (`montevideosumu` → SUMU),
///   2. ventana al INICIO,
///   3. único match distinto en el interior.
/// Varios matches interiores distintos → ambiguo → None.
async fn rescue_embedded_icao(pool: &SqlitePool, text: &str) -> Option<String> {
    let upper = text.to_ascii_uppercase();
    let bytes = upper.as_bytes();
    if bytes.len() < 4 {
        return None;
    }
    let mut windows: Vec<(usize, String)> = Vec::new();
    for i in 0..=bytes.len() - 4 {
        let w = &bytes[i..i + 4];
        if w.iter().all(|b| b.is_ascii_alphabetic()) {
            if let Ok(s) = String::from_utf8(w.to_vec()) {
                windows.push((i, s));
            }
        }
    }
    if windows.is_empty() {
        return None;
    }
    // Una sola query con IN (...) para validar todas las ventanas.
    let mut sql = String::from("SELECT icao FROM airports WHERE icao IN (");
    for (k, _) in windows.iter().enumerate() {
        if k > 0 {
            sql.push(',');
        }
        sql.push('?');
    }
    sql.push(')');
    let mut q = sqlx::query_scalar::<_, String>(&sql);
    for (_, w) in &windows {
        q = q.bind(w.clone());
    }
    let valid: std::collections::HashSet<String> =
        q.fetch_all(pool).await.ok()?.into_iter().collect();
    if valid.is_empty() {
        return None;
    }
    let hits: Vec<&(usize, String)> = windows
        .iter()
        .filter(|(_, w)| valid.contains(w))
        .collect();
    // (v4.24.1) SOLO sufijo o prefijo exactos. La regla anterior de
    // "único match interior" promovía pseudo-ICAOs escondidos en
    // palabras comunes (ENHA en "enhanced", SING en "singapore",
    // VIGR en "navigraph") y llenaba el mapa de basura.
    // 1. Sufijo exacto (`montevideosumu` → SUMU).
    if let Some((_, w)) = hits.iter().find(|(i, _)| i + 4 == bytes.len()) {
        return Some(w.clone());
    }
    // 2. Prefijo exacto (`sumu-montevideo` → SUMU).
    if let Some((_, w)) = hits.iter().find(|(i, _)| *i == 0) {
        return Some(w.clone());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_livery_registration_is_read_from_any_section() {
        let mut out = Vec::new();
        let cfg = ["[GENERAL]", "name = \"Air Canada\"", "atc_id = \"C-FIVM\"", "[FLTSIM]", "atc_airline = \"ACA\""];
        parse_fenix_livery_cfg(&cfg.join("\n"), "X", &mut out);
        assert_eq!(out[0].registration.as_deref(), Some("C-FIVM"));
        assert_eq!(out[0].airline.as_deref(), Some("ACA"));

        let mut out = Vec::new();
        let cfg = ["[GENERAL]", "name = \"A\"", "atc_id = \"X-AAAA\"", "[FLTSIM]", "atc_id = \"Y-BBBB\""];
        parse_fenix_livery_cfg(&cfg.join("\n"), "X", &mut out);
        assert_eq!(out[0].registration.as_deref(), Some("Y-BBBB"), "manda [FLTSIM]");
    }

    /// Encender y apagar renombra `layout.json`: la fecha no debe moverse.
    #[test]
    fn toggling_does_not_change_the_update_date() {
        let tmp = tempfile::tempdir().unwrap();
        let pkg = tmp.path().join("addon");
        std::fs::create_dir_all(&pkg).unwrap();
        std::fs::write(pkg.join("layout.json"), "{}").unwrap();
        let on = last_update(&pkg).expect("fecha");
        std::thread::sleep(std::time::Duration::from_millis(1100));
        std::fs::rename(pkg.join("layout.json"), pkg.join("layout.json.disabled")).unwrap();
        assert_eq!(last_update(&pkg).as_deref(), Some(on.as_str()));
        std::thread::sleep(std::time::Duration::from_millis(1100));
        std::fs::write(pkg.join("manifest.json"), "{}").unwrap();
        assert!(last_update(&pkg).unwrap() > on, "una versión nueva sí la mueve");
    }

    fn package(folder: &str, install_path: &str) -> ScannedPackage {
        ScannedPackage {
            link_target: None,
            link_managed: false,
            also_in: None,
            folder_name: folder.to_string(),
            install_path: install_path.to_string(),
            title: folder.to_string(),
            creator: None,
            content_type: None,
            package_version: None,
            minimum_game_version: None,
            icao: None,
            size_bytes: None,
            folder_modified_at: None,
            dependencies_count: 0,
            dependency_names: Vec::new(),
            enabled: true,
            simobject_dirs: Vec::new(),
            base_containers: Vec::new(),
            icao_types: Vec::new(),
            last_update: None,
            thumbnail_path: None,
            has_own_model: false,
            has_flight_model: false,
            has_manifest: true,
        }
    }

    fn report(packages: Vec<ScannedPackage>) -> ScanReport {
        ScanReport {
            packages,
            skipped_no_manifest: 0,
            skipped_invalid_manifest: 0,
            community_path: String::new(),
            broken_links: Vec::new(),
        }
    }

    /// Community2024 suma lo suyo, y lo repetido manda desde Community.
    #[test]
    fn a_second_folder_adds_its_packages_and_flags_the_repeated_ones() {
        let main = report(vec![package("acme-airport", r"B:\Community\acme-airport")]);
        let extra = report(vec![
            package("ACME-airport", r"B:\Community2024\ACME-airport"),
            package("solo-2024", r"B:\Community2024\solo-2024"),
        ]);

        let merged = merge(main, extra);

        assert_eq!(merged.packages.len(), 2);
        assert_eq!(merged.packages[0].install_path, r"B:\Community\acme-airport");
        assert_eq!(merged.packages[0].also_in.as_deref(), Some(r"B:\Community2024\ACME-airport"));
        assert_eq!(merged.packages[1].folder_name, "solo-2024");
        assert_eq!(merged.packages[1].also_in, None);
    }

    /// Un addon enlazado se escanea entero y dice dónde está; un enlace roto se
    /// cuenta aparte, porque ya no hay paquete que leer.
    #[cfg(windows)]
    #[test]
    fn a_scan_sees_through_links_and_reports_the_broken_ones() {
        let tmp = tempfile::tempdir().unwrap();
        let community = tmp.path().join("Community");
        let real = tmp.path().join("Addons").join("acme-airport");
        let gone = tmp.path().join("Addons").join("acme-perdido");
        std::fs::create_dir_all(&community).unwrap();
        std::fs::create_dir_all(&real).unwrap();
        std::fs::create_dir_all(&gone).unwrap();
        std::fs::write(real.join("manifest.json"), r#"{"title":"Acme","content_type":"SCENERY"}"#).unwrap();
        std::fs::write(real.join("layout.json"), "{}").unwrap();
        crate::folder_link::tests::junction(&community.join("acme-airport"), &real);
        crate::folder_link::tests::junction(&community.join("acme-perdido"), &gone);
        std::fs::remove_dir_all(&gone).unwrap();

        let found = scan(&community, &SizeCache::new()).unwrap();

        assert_eq!(found.packages.len(), 1);
        let pkg = &found.packages[0];
        assert_eq!(pkg.title, "Acme");
        assert!(pkg.link_target.as_deref().is_some_and(|t| t.ends_with("acme-airport")));
        assert!(!pkg.link_managed);
        assert_eq!(found.broken_links.len(), 1);
        assert_eq!(found.broken_links[0].folder_name, "acme-perdido");
    }

    /// Escaneo y guardado de punta a punta: el INSERT cuadra con sus columnas.
    #[tokio::test]
    async fn a_scan_saves_the_aircraft_types() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();

        let tmp = tempfile::tempdir().unwrap();
        let pkg = tmp.path().join("acme-aircraft-a320");
        let container = pkg.join("SimObjects").join("Airplanes").join("Acme_A320");
        std::fs::create_dir_all(&container).unwrap();
        std::fs::write(pkg.join("manifest.json"), r#"{"title":"Acme A320","content_type":"AIRCRAFT"}"#).unwrap();
        std::fs::write(pkg.join("layout.json"), "{}").unwrap();
        std::fs::write(container.join("aircraft.cfg"), "[GENERAL]\nicao_type_designator = \"A20N\"\n").unwrap();

        let report = scan(tmp.path(), &SizeCache::new()).unwrap();
        assert_eq!(report.packages.len(), 1);
        sync_to_db(&pool, &report).await.unwrap();
        let (types,): (String,) = sqlx::query_as("SELECT icao_types_json FROM community_packages")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(types, r#"["A20N"]"#);
        let (updated,): (Option<String>,) = sqlx::query_as("SELECT last_update FROM community_packages")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert!(updated.is_some());
    }

    #[test]
    fn aircraft_cfg_declares_its_icao_types() {
        let cfg = [
            "[GENERAL]",
            "; icao_type_designator = \"B738\"",
            "ICAO_type_designator = \"a20n\" ; neo",
            "[FLTSIM.0]",
            "icao_type_designator=E170",
            "icao_type_designator = \"$$:\"",
            "icao_type_designator = \"ASOBO4J\"",
        ]
        .join("\r\n");
        let mut out = Vec::new();
        icao_types_in(&cfg, &mut out);
        assert_eq!(out, vec!["A20N", "E170"]);
    }

    #[test]
    fn pretty_folder_name_handles_dashes_and_underscores() {
        assert_eq!(
            pretty_folder_name("bravoairspace-airport-mdsd-las-americas"),
            "Bravoairspace Airport Mdsd Las Americas"
        );
        assert_eq!(
            pretty_folder_name("foo_bar-baz"),
            "Foo Bar Baz"
        );
        assert_eq!(pretty_folder_name(""), "");
    }

    #[test]
    fn short_airport_codes_need_a_clear_pattern_and_a_real_airport() {
        let code = |f: &str| short_airport_code(f);
        assert_eq!(code("acme-airport-s50-auburn").as_deref(), Some("S50"));
        assert_eq!(code("acme-airports-0s9-jefferson").as_deref(), Some("0S9"));
        assert_eq!(code("acme_airport_1n7").as_deref(), Some("1N7"));
        assert_eq!(code("fs24-asobo-airport-eg78-outskerries").as_deref(), Some("EG78"));
        assert_eq!(code("acme-airport-klax-los-angeles"), None, "cuatro letras: ya lo resuelve el otro camino");
        assert_eq!(code("acme-airport-123-x"), None, "sin letras");
        assert_eq!(code("pmdg-a32-livery"), None, "fuera del patrón");
        assert_eq!(code("acme-b73-airport"), None);
        assert_eq!(code("airportsim-a32-liveries"), None);

        let valid: std::collections::HashSet<String> = ["KS50", "EG78"].iter().map(|s| s.to_string()).collect();
        assert_eq!(known_short_code("S50", &valid).as_deref(), Some("KS50"));
        assert_eq!(known_short_code("EG78", &valid).as_deref(), Some("EG78"));
        assert_eq!(known_short_code("0S9", &valid), None);
    }

    #[test]
    fn icao_extraction_word_boundary() {
        assert_eq!(extract_icao("MDSD Las Americas"), Some("MDSD".to_string()));
        assert_eq!(extract_icao("bravoairspace-mdsd"), Some("MDSD".to_string()));
        // 5 letras consecutivas no machean ICAO de 4
        assert_eq!(extract_icao("MDSDX International"), None);
        assert_eq!(extract_icao("a320 livery"), None);
    }
}
