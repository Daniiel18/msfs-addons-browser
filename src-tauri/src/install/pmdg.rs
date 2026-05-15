//! Instalación de liveries PMDG (`.ptp`) **directamente en la carpeta
//! Community**, sin pasar por PMDG Operations Center.
//!
//! El usuario pidió: «no quiero tener que abrir el PMDG Center, quiero
//! que esta app lo instale como lo hace OC». Eso significa replicar
//! aquí el flujo que OC ejecuta cuando aceptás una livery:
//!
//!   1. Detectar a qué variante PMDG corresponde la livery (`737-700`,
//!      `737-800`, `777-300ER`, etc.).
//!   2. Encontrar en Community el paquete `pmdg-aircraft-*` que
//!      contiene la carpeta `SimObjects/Airplanes/PMDG <tipo>/`.
//!   3. Extraer el `.ptp` (es un ZIP renombrado) — adentro hay una o
//!      más carpetas `texture.<key>/` y un snippet de `aircraft.cfg`
//!      con el bloque `[fltsim.N]`.
//!   4. Copiar la(s) carpeta(s) de textura al directorio del avión
//!      en Community, renombrando si el nombre ya existe.
//!   5. Mergear el bloque `[fltsim.N]` en el `aircraft.cfg` base:
//!      buscar el siguiente índice libre, renombrar `texture = ` si
//!      hubo colisión, y apendear al final del archivo.
//!   6. Regenerar `layout.json` del paquete — MSFS no carga archivos
//!      que no estén listados ahí.
//!
//! ## Riesgos y mitigaciones
//!
//!   * **Romper `aircraft.cfg` base** — siempre guardamos backup
//!     `aircraft.cfg.bak` antes de modificar y se restaura ante
//!     cualquier error de parseo.
//!   * **Colisión de nombres** — si `texture.LOT` ya existe en el
//!     destino, generamos `texture.LOT_<n>` y actualizamos la línea
//!     `texture = ` del bloque para que apunte al folder real.
//!   * **`layout.json` corrupto** — lo reescribimos por walk completo;
//!     mantiene `content[]` ordenado y consistente. El campo `date`
//!     se calcula como FILETIME desde el `mtime` real del archivo.

use std::collections::HashMap;
use std::fs;
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context};
use serde::Serialize;

/// Resultado de un auto-install exitoso. Lo devolvemos para que la UI
/// pueda mostrar "✓ Auto-instalado en <path>" con info precisa.
#[derive(Debug, Clone, Serialize)]
pub struct PmdgInstallReport {
    /// Path al paquete `pmdg-aircraft-*` que se modificó.
    pub package_root: PathBuf,
    /// Path al directorio del avión (`SimObjects/Airplanes/PMDG …/`).
    pub aircraft_dir: PathBuf,
    /// Nombre final de la carpeta de textura instalada (puede haber
    /// sido renombrada si hubo colisión, ej. `texture.LOT_2`).
    pub texture_folder: String,
    /// Índice `[fltsim.N]` que se le asignó a esta livery en el
    /// `aircraft.cfg` base.
    pub fltsim_index: u32,
}

/// Instala una livery `.ptp` directamente en el paquete PMDG dentro
/// de Community. Devuelve `Ok(Some(report))` si todo salió bien,
/// `Ok(None)` si no encontramos un paquete PMDG compatible (la
/// livery queda sólo en el inbox manual), `Err` ante fallos
/// catastróficos (disco lleno, permisos, archivos rotos, etc.).
pub fn install_livery(
    ptp_path: &Path,
    aircraft: &str,
    community: &Path,
) -> anyhow::Result<Option<PmdgInstallReport>> {
    let Some(target) = find_pmdg_package(community, aircraft) else {
        tracing::info!(
            target: "install",
            "PMDG: no se encontró paquete en Community para {} — se deja en inbox manual",
            aircraft
        );
        return Ok(None);
    };
    tracing::info!(
        target: "install",
        "PMDG: instalando livery en {}",
        target.aircraft_dir.display()
    );

    // Descomprimir el .ptp en un tempdir — se borra al salir.
    let temp = tempfile::Builder::new()
        .prefix("msfs-ptp-")
        .tempdir()
        .context("no se pudo crear tempdir para extraer .ptp")?;
    extract_ptp_zip(ptp_path, temp.path())?;

    let textures = find_texture_folders(temp.path());
    if textures.is_empty() {
        return Err(anyhow!(
            "el .ptp '{}' no contiene ninguna carpeta `texture.*` — no es una livery PMDG válida",
            ptp_path.display()
        ));
    }
    if textures.len() > 1 {
        tracing::info!(
            target: "install",
            "PMDG: .ptp contiene {} folders de textura — instalando todas",
            textures.len()
        );
    }

    // El aircraft.cfg snippet del .ptp puede traer 1 o más
    // bloques `[fltsim.N]`. Los parseamos todos y los re-emitimos
    // con índices únicos en el aircraft.cfg destino.
    let snippet = read_aircraft_cfg_snippet(temp.path())?;
    let blocks = parse_fltsim_blocks(&snippet);
    if blocks.is_empty() {
        return Err(anyhow!(
            "el .ptp '{}' no contiene ningún bloque [fltsim.N] en aircraft.cfg",
            ptp_path.display()
        ));
    }
    tracing::info!(
        target: "install",
        "PMDG: .ptp aporta {} bloque(s) [fltsim.N]",
        blocks.len()
    );

    // Backup del aircraft.cfg base antes de tocar nada. Si algo
    // sale mal a media hora, restauramos.
    let cfg_path = target.aircraft_dir.join("aircraft.cfg");
    let backup_path = target.aircraft_dir.join("aircraft.cfg.bak");
    if cfg_path.is_file() {
        fs::copy(&cfg_path, &backup_path).with_context(|| {
            format!(
                "no se pudo crear backup de aircraft.cfg en {}",
                backup_path.display()
            )
        })?;
    } else {
        return Err(anyhow!(
            "el paquete PMDG en {} no tiene aircraft.cfg — instalación abortada",
            target.aircraft_dir.display()
        ));
    }

    // Aplicar todo en transacción "best effort": cualquier fallo
    // restaura el backup.
    let result: anyhow::Result<PmdgInstallReport> = (|| {
        // Para cada folder de textura: copiar, posiblemente
        // renombrando si ya existe. Registramos el mapping
        // <nombre original en .ptp → nombre final en destino>
        // para reescribir la línea `texture = ` del bloque.
        let mut texture_remap: HashMap<String, String> = HashMap::new();
        let mut installed_textures: Vec<PathBuf> = Vec::new();
        for tx in &textures {
            let original_name = tx
                .file_name()
                .and_then(|s| s.to_str())
                .ok_or_else(|| anyhow!("nombre de textura inválido: {:?}", tx))?
                .to_string();
            let final_name = pick_unique_texture_name(&target.aircraft_dir, &original_name);
            let dst = target.aircraft_dir.join(&final_name);
            copy_dir_recursive(tx, &dst).with_context(|| {
                format!(
                    "no se pudo copiar textura {} → {}",
                    tx.display(),
                    dst.display()
                )
            })?;
            // Mapping para la(s) líneas `texture = ` del cfg.
            // Quitamos el prefijo `texture.` para comparar/reemplazar
            // tal como aparece en el cfg (la línea suele ser
            // `texture = "LOT"`, sin el prefijo).
            let original_short = strip_texture_prefix(&original_name);
            let final_short = strip_texture_prefix(&final_name);
            texture_remap.insert(original_short.to_string(), final_short.to_string());
            installed_textures.push(dst);
        }

        // Mergear bloques `[fltsim.N]` en aircraft.cfg base.
        let next_index = next_fltsim_index(&cfg_path)?;
        let mut written_blocks = Vec::with_capacity(blocks.len());
        for (i, block) in blocks.into_iter().enumerate() {
            let idx = next_index + i as u32;
            let rewritten = rewrite_fltsim_block(&block, idx, &texture_remap);
            written_blocks.push(rewritten);
        }
        append_to_cfg(&cfg_path, &written_blocks)?;

        // Regenerar layout.json del paquete completo.
        rebuild_layout_json(&target.package_root)?;

        // Reportar el primer índice + nombre de textura — sirve para
        // mostrar al usuario "instalado como [fltsim.N]". Si hubo
        // varios bloques, el resto siguen consecutivos.
        let first_texture = installed_textures
            .into_iter()
            .next()
            .and_then(|p| p.file_name().map(|s| s.to_string_lossy().into_owned()))
            .unwrap_or_default();

        Ok(PmdgInstallReport {
            package_root: target.package_root.clone(),
            aircraft_dir: target.aircraft_dir.clone(),
            texture_folder: first_texture,
            fltsim_index: next_index,
        })
    })();

    match result {
        Ok(report) => {
            // Backup ya no es necesario.
            let _ = fs::remove_file(&backup_path);
            Ok(Some(report))
        }
        Err(e) => {
            tracing::warn!(
                target: "install",
                "PMDG install falló — restaurando backup: {e:#}"
            );
            if backup_path.is_file() {
                if let Err(restore_err) = fs::rename(&backup_path, &cfg_path) {
                    tracing::error!(
                        target: "install",
                        "ADEMÁS no se pudo restaurar el backup de aircraft.cfg: {restore_err}"
                    );
                }
            }
            Err(e)
        }
    }
}

/// Localiza el paquete PMDG en Community que provee `aircraft` (ej.
/// `PMDG 737-800`). Recorremos los hijos directos buscando carpetas
/// `pmdg-aircraft-*`, y dentro de cada una `SimObjects/Airplanes/`
/// hasta encontrar el `aircraft.cfg` correcto.
struct PmdgTarget {
    package_root: PathBuf,
    aircraft_dir: PathBuf,
}

fn find_pmdg_package(community: &Path, aircraft: &str) -> Option<PmdgTarget> {
    // Folder hints — `pmdg-aircraft-XXX` donde XXX suele ser:
    //   737 / 738 / 739 (para 737-700 / -800 / -900)
    //   77w / 77l (para 777-300ER / 777-200LR)
    //   744 / 748 (747-400, 747-8)
    // Pero hay distribuciones donde la carpeta no incluye el sufijo
    // — basta entonces con que adentro haya `SimObjects/Airplanes/PMDG <tipo>`.
    let expected_inner = pmdg_inner_folder(aircraft)?;
    let iter = fs::read_dir(community).ok()?;
    for entry in iter.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name_lower = entry.file_name().to_string_lossy().to_lowercase();
        if !name_lower.contains("pmdg") {
            continue;
        }
        let aircraft_dir = path
            .join("SimObjects")
            .join("Airplanes")
            .join(&expected_inner);
        if aircraft_dir.join("aircraft.cfg").is_file() {
            return Some(PmdgTarget {
                package_root: path,
                aircraft_dir,
            });
        }
        // Algunos paquetes tienen varios aviones dentro
        // (`PMDG 737-700 BBJ`, `PMDG 737-800` en `pmdg-aircraft-737`).
        // Hacer un fallback escaneando los hijos de Airplanes/.
        let airplanes_dir = path.join("SimObjects").join("Airplanes");
        if let Ok(sub_iter) = fs::read_dir(&airplanes_dir) {
            for sub in sub_iter.flatten() {
                let sub_path = sub.path();
                let sub_name = sub.file_name().to_string_lossy().to_string();
                if sub_name.eq_ignore_ascii_case(&expected_inner)
                    && sub_path.join("aircraft.cfg").is_file()
                {
                    return Some(PmdgTarget {
                        package_root: path,
                        aircraft_dir: sub_path,
                    });
                }
            }
        }
    }
    None
}

/// Traduce un nombre de aircraft ("PMDG 737-800") al nombre de
/// carpeta que el paquete usa dentro de `SimObjects/Airplanes/`.
fn pmdg_inner_folder(aircraft: &str) -> Option<String> {
    match aircraft {
        "PMDG 737-600" => Some("PMDG 737-600".into()),
        "PMDG 737-700" => Some("PMDG 737-700".into()),
        "PMDG 737-800" => Some("PMDG 737-800".into()),
        "PMDG 737-900" => Some("PMDG 737-900".into()),
        "PMDG 747-400" => Some("PMDG 747-400".into()),
        "PMDG 747-8"   => Some("PMDG 747-8".into()),
        "PMDG 747F"    => Some("PMDG 747-8F".into()),
        "PMDG 777-200LR" => Some("PMDG 777-200LR".into()),
        "PMDG 777-300ER" => Some("PMDG 777-300ER".into()),
        "PMDG 777F"    => Some("PMDG 777F".into()),
        "PMDG DC-6"    => Some("PMDG DC-6".into()),
        _ => None,
    }
}

/// Descomprime un `.ptp` (es un ZIP) al destino. La extracción es
/// "flat" — todos los archivos van bajo `dest` manteniendo su
/// estructura relativa.
fn extract_ptp_zip(ptp: &Path, dest: &Path) -> anyhow::Result<()> {
    let file = fs::File::open(ptp)
        .with_context(|| format!("no se pudo abrir .ptp {}", ptp.display()))?;
    let mut zip = zip::ZipArchive::new(file)
        .with_context(|| format!(".ptp no es un ZIP válido: {}", ptp.display()))?;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;
        let Some(rel) = entry.enclosed_name().map(|p| p.to_path_buf()) else {
            tracing::warn!(target: "install", "entrada .ptp insegura ignorada: {}", entry.name());
            continue;
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

/// Busca todas las carpetas `texture.*` (no `texture.cfg`, sólo
/// directorios) dentro del árbol extraído del .ptp.
fn find_texture_folders(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    walk_textures(root, 0, &mut out);
    out
}

fn walk_textures(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth > 5 || !dir.is_dir() {
        return;
    }
    let Ok(iter) = fs::read_dir(dir) else { return };
    for entry in iter.flatten() {
        let path = entry.path();
        let Ok(ft) = entry.file_type() else { continue };
        if !ft.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        let lower = name.to_lowercase();
        // `texture.<x>` pero no `texture` solo (carpetas auxiliares),
        // ni `model.<x>` ni otros.
        if lower.starts_with("texture.") && lower.len() > "texture.".len() {
            out.push(path);
        } else {
            walk_textures(&path, depth + 1, out);
        }
    }
}

/// Lee el primer `aircraft.cfg` que encontremos en el árbol del .ptp
/// extraído. Algunos .ptp lo ponen al raíz, otros dentro de `aircraft/`,
/// otros dentro de la propia carpeta de textura.
fn read_aircraft_cfg_snippet(root: &Path) -> anyhow::Result<String> {
    let mut candidates = Vec::new();
    collect_cfg_candidates(root, 0, &mut candidates);
    if candidates.is_empty() {
        return Err(anyhow!(
            "el .ptp no contiene ningún aircraft.cfg dentro de {}",
            root.display()
        ));
    }
    // Si hay varios, el más superficial gana — suele ser el snippet
    // global; los aircraft.cfg de las texturas son secundarios.
    candidates.sort_by_key(|p| p.components().count());
    let pick = &candidates[0];
    fs::read_to_string(pick)
        .with_context(|| format!("no se pudo leer aircraft.cfg en {}", pick.display()))
}

fn collect_cfg_candidates(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if depth > 5 || !dir.is_dir() {
        return;
    }
    let Ok(iter) = fs::read_dir(dir) else { return };
    for entry in iter.flatten() {
        let path = entry.path();
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_dir() {
            collect_cfg_candidates(&path, depth + 1, out);
        } else if entry
            .file_name()
            .to_string_lossy()
            .eq_ignore_ascii_case("aircraft.cfg")
        {
            out.push(path);
        }
    }
}

/// Un bloque `[fltsim.N]` parseado: header + líneas crudas en orden.
/// Mantener líneas crudas (en vez de un `HashMap`) preserva
/// comentarios y formato original — los simmers a veces meten
/// tags personalizados que PMDG OC respeta.
#[derive(Debug, Clone)]
struct FltsimBlock {
    raw_lines: Vec<String>,
}

/// Extrae todos los bloques `[fltsim.N]` del cfg, ignorando otras
/// secciones (`[GENERAL]`, `[VARIATION]`, etc.).
fn parse_fltsim_blocks(content: &str) -> Vec<FltsimBlock> {
    let mut out = Vec::new();
    let mut current: Option<Vec<String>> = None;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            // Cerrar bloque previo si existía.
            if let Some(lines) = current.take() {
                out.push(FltsimBlock { raw_lines: lines });
            }
            // ¿Es otro [fltsim.N]?
            let inner = trimmed.trim_start_matches('[').trim_end_matches(']');
            if inner.to_lowercase().starts_with("fltsim.") {
                current = Some(Vec::new());
            }
        } else if let Some(buf) = current.as_mut() {
            buf.push(line.to_string());
        }
    }
    if let Some(lines) = current.take() {
        out.push(FltsimBlock { raw_lines: lines });
    }
    out
}

/// Re-emite un bloque `[fltsim.N]` con un índice nuevo y, si hubo
/// remapeo de carpeta de textura (por colisión), actualiza la línea
/// `texture = "…"` apropiadamente.
fn rewrite_fltsim_block(
    block: &FltsimBlock,
    new_index: u32,
    texture_remap: &HashMap<String, String>,
) -> String {
    let mut out = String::new();
    out.push_str(&format!("[fltsim.{}]\n", new_index));
    for line in &block.raw_lines {
        // Detectar línea `texture = "FOO"` o `texture=FOO` y
        // posiblemente reemplazar el valor.
        if let Some(eq) = line.find('=') {
            let (key, rest) = line.split_at(eq);
            let key_trim = key.trim().to_ascii_lowercase();
            if key_trim == "texture" {
                // Extraer valor actual (puede venir con o sin
                // comillas, con espacios).
                let value_raw = rest.trim_start_matches('=').trim();
                let value_unquoted = value_raw
                    .trim_start_matches('"')
                    .trim_end_matches('"')
                    .to_string();
                if let Some(new_val) = texture_remap.get(&value_unquoted) {
                    out.push_str(&format!("texture = {}\n", new_val));
                    continue;
                }
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// Devuelve el primer índice `[fltsim.N]` que no está usado en el
/// cfg base. Escanea las secciones existentes y elige `max + 1`.
fn next_fltsim_index(cfg: &Path) -> anyhow::Result<u32> {
    let file = fs::File::open(cfg)
        .with_context(|| format!("no se pudo abrir {}", cfg.display()))?;
    let reader = BufReader::new(file);
    let mut max_seen: i64 = -1;
    for line in reader.lines() {
        let line = line?;
        let trimmed = line.trim();
        if let Some(inner) = trimmed
            .strip_prefix('[')
            .and_then(|s| s.strip_suffix(']'))
        {
            let lower = inner.to_ascii_lowercase();
            if let Some(num) = lower.strip_prefix("fltsim.") {
                if let Ok(n) = num.parse::<i64>() {
                    if n > max_seen {
                        max_seen = n;
                    }
                }
            }
        }
    }
    Ok((max_seen + 1).max(0) as u32)
}

/// Apendea los bloques formateados al final del cfg. Asegura un
/// salto de línea antes de cada bloque.
fn append_to_cfg(cfg: &Path, blocks: &[String]) -> anyhow::Result<()> {
    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(cfg)
        .with_context(|| format!("no se pudo abrir {} para append", cfg.display()))?;
    for block in blocks {
        file.write_all(b"\n")?;
        file.write_all(block.as_bytes())?;
    }
    Ok(())
}

/// Si el nombre `original` ya existe como subcarpeta dentro de
/// `parent`, devuelve `<original>_2`, `<original>_3`, … hasta
/// encontrar uno libre. Si no existe, devuelve `original` tal cual.
fn pick_unique_texture_name(parent: &Path, original: &str) -> String {
    if !parent.join(original).exists() {
        return original.to_string();
    }
    for n in 2..1000 {
        let candidate = format!("{}_{}", original, n);
        if !parent.join(&candidate).exists() {
            return candidate;
        }
    }
    // Fallback extremadamente improbable: usar timestamp.
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{}_{}", original, stamp)
}

/// Convierte `texture.LOT` → `LOT`. Los cfg de MSFS suelen referir
/// a la carpeta sólo por el sufijo sin el prefijo.
fn strip_texture_prefix(name: &str) -> &str {
    name.strip_prefix("texture.")
        .or_else(|| name.strip_prefix("TEXTURE."))
        .unwrap_or(name)
}

/// Copia recursiva — clon de la existente en `install/mod.rs`,
/// duplicada acá para no exponerla pub crate-wide.
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

// ---------------------------------------------------------------------------
// layout.json regeneration
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct LayoutFile {
    path: String,
    size: u64,
    date: u64,
}

#[derive(Debug, Serialize)]
struct LayoutDoc {
    content: Vec<LayoutFile>,
}

/// Regenera el `layout.json` del paquete listando *todos* los
/// archivos por debajo del root, salvo el propio `layout.json` (que
/// MSFS ignora cuando lo lista a sí mismo).
fn rebuild_layout_json(package_root: &Path) -> anyhow::Result<()> {
    let mut files = Vec::new();
    walk_layout(package_root, package_root, &mut files);
    files.sort_by(|a, b| a.path.cmp(&b.path));

    let doc = LayoutDoc { content: files };
    // MSFS espera el archivo con saltos `\n` y formato pretty con
    // sangría de 2 espacios — replicamos el estilo de PMDG OC.
    let json = serde_json::to_string_pretty(&doc)
        .context("no se pudo serializar layout.json")?;
    let path = package_root.join("layout.json");
    fs::write(&path, json)
        .with_context(|| format!("no se pudo escribir layout.json en {}", path.display()))?;
    Ok(())
}

fn walk_layout(root: &Path, dir: &Path, out: &mut Vec<LayoutFile>) {
    let Ok(iter) = fs::read_dir(dir) else { return };
    for entry in iter.flatten() {
        let path = entry.path();
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_dir() {
            walk_layout(root, &path, out);
            continue;
        }
        let Some(rel) = path.strip_prefix(root).ok() else {
            continue;
        };
        // Ignorar el propio layout.json — se reescribe al final
        // y no debe listarse a sí mismo.
        let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
        if rel.components().count() == 1 && name == "layout.json" {
            continue;
        }
        // Tampoco listamos manifest.json — MSFS lo lee aparte
        // y no debe estar en `content[]`.
        if rel.components().count() == 1 && name == "manifest.json" {
            continue;
        }
        // Backup que dejamos durante el install (limpiado al éxito,
        // pero por si quedó un huérfano de una corrida anterior).
        if name == "aircraft.cfg.bak" {
            continue;
        }
        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        let size = meta.len();
        let date = filetime_from_metadata(&meta);
        // MSFS usa `/` en `layout.json` aunque corra en Windows.
        let path_str = rel
            .to_string_lossy()
            .replace('\\', "/");
        out.push(LayoutFile {
            path: path_str,
            size,
            date,
        });
    }
}

/// Convierte el `mtime` de un archivo al formato Windows FILETIME
/// (100-ns intervals desde 1601-01-01) que MSFS usa en `layout.json`.
/// Si el sistema no expone mtime, devolvemos 0 — MSFS lo tolera.
fn filetime_from_metadata(meta: &fs::Metadata) -> u64 {
    let Ok(mtime) = meta.modified() else { return 0 };
    let Ok(dur) = mtime.duration_since(UNIX_EPOCH) else {
        return 0;
    };
    // 11644473600 = segundos entre 1601-01-01 y 1970-01-01.
    let secs_since_1601 = dur.as_secs().saturating_add(11_644_473_600);
    secs_since_1601.saturating_mul(10_000_000)
}
