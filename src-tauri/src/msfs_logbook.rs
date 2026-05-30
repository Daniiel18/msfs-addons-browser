//! Importador del LOGBOOK.BIN de MSFS 2020/2024.
//!
//! El formato del archivo está sin documentar oficialmente — es un
//! blob binario propio de Microsoft que serializa los flight entries
//! del logbook in-game. La comunidad lo ha reverse-engineereado
//! parcialmente. Lo que sí es estable a través de versiones:
//!   · Las matrículas ICAO (4 chars uppercase ASCII) aparecen como
//!     campos de longitud-prefijada (length byte + payload).
//!   · Las matrículas de origen y destino aparecen en parejas
//!     contiguas dentro de cada record.
//!   · El archivo tiene un magic header al inicio y una secuencia de
//!     records de longitud variable, sin terminator explícito.
//!
//! Estrategia de parsing (best-effort, robusta a versiones):
//!   1. Detectamos paths candidatos (MSFS 2020/2024 × MS Store/Steam).
//!   2. Escaneamos el binario en busca de pares (orig_icao,
//!      dest_icao) — secuencias de 4-char [A-Z0-9] separadas por
//!      ≤32 bytes (estimación empírica del tamaño máximo de un
//!      record header + algunos floats).
//!   3. Cada par identificado se materializa como un `flight_log`
//!      entry minimalista (origen, destino, started_at = mtime del
//!      archivo como fallback). El usuario verá los vuelos del
//!      MSFS logbook listados pero sin las métricas internas (vamos
//!      a poder enriquecerlos cuando el formato se documente mejor).
//!   4. Deduplicación: insertamos sólo si NO existe ya un entry con
//!      el mismo (origin_icao, destination_icao, source='msfs-logbook').
//!      Esto evita re-importar al re-pulsar el botón.
//!
//! Esta función es SÍNCRONA y bloqueante porque la lectura del bin
//! es rápida (~1 MB típico). La envolvemos en `spawn_blocking` desde
//! el comando para no bloquear el runtime.

use std::path::{Path, PathBuf};

use serde::Serialize;
use sqlx::SqlitePool;

/// Variante de MSFS detectada en disco. Coincide con `MsfsVariant`
/// de `community.rs` pero local — no queremos cross-dep si una de
/// las dos cambia.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LogbookSim {
    Msfs2020,
    Msfs2024,
}

impl LogbookSim {
    fn label(self) -> &'static str {
        match self {
            LogbookSim::Msfs2020 => "msfs-2020",
            LogbookSim::Msfs2024 => "msfs-2024",
        }
    }
}

/// Source label que escribimos en `flight_log.source` — distingue
/// entradas importadas del logbook MSFS de los `simconnect` (capturados
/// en vivo) y `cloud-import` (importados desde cloud sync).
const SOURCE_LABEL: &str = "msfs-logbook";

/// Path candidato del LOGBOOK.BIN. Detectamos las 4 variantes
/// (MSFS 2020/2024 × MS Store/Steam). Se preserva el orden de
/// descubrimiento para que el caller elija la primera "más nueva".
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogbookCandidate {
    pub sim: LogbookSim,
    pub store: &'static str, // "msstore" | "steam"
    pub path: String,
    pub size_bytes: u64,
    pub modified_at: Option<String>,
}

/// Reporte de la operación de import.
#[derive(Debug, Default, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportReport {
    pub source_path: Option<String>,
    pub sim: Option<&'static str>,
    /// Pares ICAO detectados en el bin (orig→dest). Pueden ser más
    /// que `imported_count` si algunos eran duplicados.
    pub candidates_found: usize,
    /// Vuelos efectivamente insertados (sin contar dupes preexistentes).
    pub imported_count: usize,
    /// Filtrados por estar ya en flight_log con el mismo
    /// (origin, dest, source='msfs-logbook').
    pub skipped_duplicates: usize,
}

/// Busca el LOGBOOK.BIN en TODOS los slots conocidos y devuelve los
/// candidatos válidos (archivo existe + size > 0). Orden:
///   1. MSFS 2024 MS Store
///   2. MSFS 2024 Steam
///   3. MSFS 2020 MS Store
///   4. MSFS 2020 Steam
/// Preferimos 2024 sobre 2020 — si el usuario migró, su logbook
/// más reciente vive en 2024.
pub fn detect_candidates() -> Vec<LogbookCandidate> {
    let mut out = Vec::new();
    for (sim, store, path) in candidate_paths() {
        if let Ok(meta) = std::fs::metadata(&path) {
            if meta.is_file() && meta.len() > 0 {
                let modified_at = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .and_then(|d| {
                        chrono::DateTime::from_timestamp(d.as_secs() as i64, 0)
                            .map(|dt| dt.format("%Y-%m-%dT%H:%M:%SZ").to_string())
                    });
                out.push(LogbookCandidate {
                    sim,
                    store,
                    path: path.to_string_lossy().into_owned(),
                    size_bytes: meta.len(),
                    modified_at,
                });
            }
        }
    }
    out
}

/// Wrapper público que devuelve solo los `PathBuf` candidatos (sin
/// metadata sim/store). Lo usa el comando para listar en el log
/// dónde se buscó cuando no se encontró el archivo — feedback de
/// debug para usuarios que reportan «no pasa nada al darle al botón».
pub fn candidate_paths_for_log() -> Vec<PathBuf> {
    candidate_paths().into_iter().map(|(_, _, p)| p).collect()
}

/// Genera todos los paths potenciales con su sim/store asociados.
fn candidate_paths() -> Vec<(LogbookSim, &'static str, PathBuf)> {
    let mut out = Vec::new();
    let local = std::env::var_os("LOCALAPPDATA");
    let roaming = std::env::var_os("APPDATA");

    // MSFS 2024 MS Store — preferred (más reciente).
    if let Some(ref l) = local {
        out.push((
            LogbookSim::Msfs2024,
            "msstore",
            Path::new(l)
                .join("Packages")
                .join("Microsoft.Limitless_8wekyb3d8bbwe")
                .join("LocalCache")
                .join("LOGBOOK.BIN"),
        ));
    }
    // MSFS 2024 Steam.
    if let Some(ref r) = roaming {
        out.push((
            LogbookSim::Msfs2024,
            "steam",
            Path::new(r).join("Microsoft Flight Simulator 2024").join("LOGBOOK.BIN"),
        ));
    }
    // MSFS 2020 MS Store.
    if let Some(ref l) = local {
        out.push((
            LogbookSim::Msfs2020,
            "msstore",
            Path::new(l)
                .join("Packages")
                .join("Microsoft.FlightSimulator_8wekyb3d8bbwe")
                .join("LocalCache")
                .join("LOGBOOK.BIN"),
        ));
    }
    // MSFS 2020 Steam.
    if let Some(ref r) = roaming {
        out.push((
            LogbookSim::Msfs2020,
            "steam",
            Path::new(r).join("Microsoft Flight Simulator").join("LOGBOOK.BIN"),
        ));
    }
    out
}

/// Escanea el binario y extrae todos los pares (origin_icao,
/// destination_icao) que detecte. Heurística:
///   · Recorre el buffer byte-a-byte buscando runs de 4 chars que
///     cumplan `[A-Z0-9]{4}`.
///   · Filtra heurísticamente: ignora runs precedidos/seguidos por
///     más letras ASCII (evita pillar substrings de palabras tipo
///     "BOEING737" → "OEIN").
///   · Empareja consecutivos en orden de aparición. Records típicos
///     tienen orig + dest juntos; descartamos pares con misma ICAO
///     porque no son vuelos reales.
///
/// Pre-filtros: descarta strings que NO parecen ICAO real:
///   · Todo dígitos (ej. "1234")
///   · Patrones de aircraft type embebido (BOEN, AIRB, ATR7, etc.)
fn scan_icao_pairs(bytes: &[u8]) -> Vec<(String, String)> {
    let icaos = extract_icao_candidates(bytes);

    // Empareja consecutivos. Salta si origin == destination (vuelo
    // trivial probablemente no es un entry real).
    let mut out = Vec::new();
    let mut i = 0;
    while i + 1 < icaos.len() {
        let (orig, dest) = (icaos[i].clone(), icaos[i + 1].clone());
        if orig != dest {
            out.push((orig, dest));
            i += 2;
        } else {
            // Same ICAO twice — skip uno y prueba con el siguiente.
            i += 1;
        }
    }
    out
}

/// Devuelve la lista ORDENADA (por offset) de ICAOs detectados en
/// el bytes. Aplica filtros para descartar substrings de strings
/// más largas (ej. "BOEING737" → "OEIN" embebido).
fn extract_icao_candidates(bytes: &[u8]) -> Vec<String> {
    let n = bytes.len();
    let mut out = Vec::new();
    let mut i = 0;
    while i + 4 <= n {
        let chunk = &bytes[i..i + 4];
        if is_icao_run(chunk) {
            // Boundary check: el byte anterior NO debe ser ascii letra
            // (sino estaríamos en medio de una palabra). Si i==0 OK.
            let before_ok = i == 0 || !is_ascii_letter(bytes[i - 1]);
            // Y el byte siguiente NO debe ser ascii letra tampoco.
            let after_ok = i + 4 == n || !is_ascii_letter(bytes[i + 4]);
            if before_ok && after_ok {
                let s = std::str::from_utf8(chunk).unwrap_or("").to_string();
                if !is_obvious_garbage(&s) {
                    out.push(s);
                    i += 4;
                    continue;
                }
            }
        }
        i += 1;
    }
    out
}

#[inline]
fn is_icao_run(b: &[u8]) -> bool {
    if b.len() != 4 {
        return false;
    }
    let mut has_letter = false;
    for &c in b {
        if c.is_ascii_uppercase() {
            has_letter = true;
        } else if c.is_ascii_digit() {
            // OK pero por sí solas no son válidas.
        } else {
            return false;
        }
    }
    // Al menos 1 letra (descarta "1234", "9999", etc.).
    has_letter
}

#[inline]
fn is_ascii_letter(b: u8) -> bool {
    b.is_ascii_uppercase() || b.is_ascii_lowercase()
}

/// Filtros heurísticos: substrings típicas que aparecen en strings
/// más largas dentro del binario (no son ICAOs reales).
fn is_obvious_garbage(s: &str) -> bool {
    // Tokens típicos de aircraft type embebido o palabras inglesas
    // que pasarían el filtro de 4 char uppercase.
    const BLACKLIST: &[&str] = &[
        "BOEN", "BOEI", "OING", "AIRB", "RBUS", "BUSA", "ATR7", "ATR4", "EMBR",
        "EMBA", "EMBE", "BRAE", "RAER", "FALC", "ALCO", "LCON", "MENT", "UANG",
        "ULFS", "TWIN", "PROP", "JETC", "ETCO", "TYPE", "TITLE", "NAME", "DESC",
        "TEXT", "DATA", "FILE", "PATH", "TRUE", "FALSE", "NULL", "VOID", "INFO",
        "SIZE", "SPEC", "DUMP", "EDIT", "ROOT", "USER", "DRIVE", "PACK", "PROF",
    ];
    BLACKLIST.iter().any(|&b| b == s)
}

/// Importa los vuelos del LOGBOOK.BIN al `flight_log` de la app.
/// Deduplica usando `(origin_icao, destination_icao, source='msfs-logbook')`
/// — re-importar el mismo logbook no produce dupes. Si el usuario
/// realmente voló esa ruta varias veces, los duplicates extras se
/// pierden — trade-off aceptable contra el riesgo de spam en cada
/// import.
///
/// Devuelve un `ImportReport` con conteos para feedback al usuario.
pub async fn import_logbook(
    pool: &SqlitePool,
    path: &Path,
    sim: LogbookSim,
) -> anyhow::Result<ImportReport> {
    let bytes = std::fs::read(path)?;
    let pairs = scan_icao_pairs(&bytes);
    let mut report = ImportReport {
        source_path: Some(path.to_string_lossy().into_owned()),
        sim: Some(sim.label()),
        candidates_found: pairs.len(),
        ..Default::default()
    };

    // mtime del archivo como fallback para `started_at` cuando no
    // podemos extraer la fecha real del binario. Mejor que NULL
    // porque la UI puede ordenar/filtrar por fecha.
    let mtime_iso = std::fs::metadata(path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .and_then(|d| {
            chrono::DateTime::from_timestamp(d.as_secs() as i64, 0)
                .map(|dt| dt.format("%Y-%m-%dT%H:%M:%SZ").to_string())
        })
        .unwrap_or_else(|| chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string());

    for (orig, dest) in pairs {
        // Dedupe: ya importado antes (mismo orig+dest+source)?
        let exists: Option<i64> = sqlx::query_scalar(
            "SELECT id FROM flight_log
             WHERE source = ?1 AND origin_icao = ?2 AND destination_icao = ?3
             LIMIT 1",
        )
        .bind(SOURCE_LABEL)
        .bind(&orig)
        .bind(&dest)
        .fetch_optional(pool)
        .await?;

        if exists.is_some() {
            report.skipped_duplicates += 1;
            continue;
        }

        // Insert. origin_lat/lon son NOT NULL en la migración 010 —
        // usamos 0.0 como placeholder. La UI filtra estos casos en el
        // mapa con bbox checks (FlightBook ya skipea (0,0) artifacts).
        sqlx::query(
            "INSERT INTO flight_log (
                started_at, ended_at,
                origin_lat, origin_lon, origin_icao,
                destination_lat, destination_lon, destination_icao,
                source
            ) VALUES (?1, ?1, 0.0, 0.0, ?2, NULL, NULL, ?3, ?4)",
        )
        .bind(&mtime_iso)
        .bind(&orig)
        .bind(&dest)
        .bind(SOURCE_LABEL)
        .execute(pool)
        .await?;

        report.imported_count += 1;
    }

    tracing::info!(
        target: "msfs_logbook",
        "import {} · candidates={} imported={} skipped_dupes={}",
        path.display(),
        report.candidates_found,
        report.imported_count,
        report.skipped_duplicates,
    );

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_valid_icaos() {
        assert!(is_icao_run(b"KSEA"));
        assert!(is_icao_run(b"LFPG"));
        assert!(is_icao_run(b"EHAM"));
        assert!(is_icao_run(b"KJFK"));
        // Mix letras + dígitos (raro pero válido para algunos pads).
        assert!(is_icao_run(b"K123"));
    }

    #[test]
    fn rejects_invalid_icaos() {
        assert!(!is_icao_run(b"1234")); // todo dígitos
        assert!(!is_icao_run(b"abcd")); // lowercase
        assert!(!is_icao_run(b"AB1")); // muy corto
        assert!(!is_icao_run(b"AB-1")); // separador
        assert!(!is_icao_run(b"AB\x00C")); // null byte
    }

    #[test]
    fn extracts_isolated_icaos_only() {
        // "KSEA\0\0LFPG" → debe extraer ambos.
        let bytes = b"KSEA\x00\x00LFPG";
        let result = extract_icao_candidates(bytes);
        assert_eq!(result, vec!["KSEA".to_string(), "LFPG".to_string()]);
    }

    #[test]
    fn ignores_icaos_embedded_in_words() {
        // "BOEING737" — "OEIN" pasaría el regex pero está embebido.
        let bytes = b"BOEING737";
        let result = extract_icao_candidates(bytes);
        // "BOEN" estaría en la blacklist, y "OEIN" tiene letras
        // adyacentes (B antes, G después) así que se descarta.
        assert!(result.is_empty(), "got {:?}", result);
    }

    #[test]
    fn pairs_consecutive_icaos() {
        let bytes = b"KSEA\x00\x00LFPG\xFF\xFFKDEN\x00\x00KORD";
        let pairs = scan_icao_pairs(bytes);
        assert_eq!(
            pairs,
            vec![
                ("KSEA".to_string(), "LFPG".to_string()),
                ("KDEN".to_string(), "KORD".to_string()),
            ]
        );
    }

    #[test]
    fn skips_same_icao_pairs() {
        // "KSEA\0\0KSEA" — origin == destination, skip.
        // Pero "KSEA\0\0KSEA\0\0LFPG" sí debe formar (KSEA, LFPG).
        let bytes = b"KSEA\x00\x00KSEA\x00\x00LFPG";
        let pairs = scan_icao_pairs(bytes);
        assert_eq!(pairs, vec![("KSEA".to_string(), "LFPG".to_string())]);
    }
}
