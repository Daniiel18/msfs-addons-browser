//! (v6.2.8) Avisos de actualización del **AIRAC** (datos de navegación).
//!
//! A diferencia de GSX, el AIRAC NO necesita scrapear: los ciclos siguen un
//! calendario FIJO de 28 días anclado a una fecha de referencia conocida
//! (AIRAC 2301 efectivo el 2023-01-26, validado contra 2501 = 2025-01-23). Así
//! calculamos el ciclo vigente para HOY de forma determinística y lo comparamos
//! con el que el usuario tiene instalado. El frontend lo muestra en
//! Notificaciones + Dashboard y, al actualizar, abre Navigraph Hub.
//!
//! (v7.4.2) El ciclo INSTALADO ya no es sólo una marca manual: se lee del
//! paquete de navdata que Navigraph Hub instala en la Community
//! (`navigraph-nav-*`, manifest `"title": "AIRAC Cycle 2610 rev.1"`). Antes la
//! app sólo sabía lo que el usuario había pulsado en SU botón — si actualizaba
//! desde Navigraph Hub (p.ej. el 2610 llegó el 27/09, 4 días antes de ser
//! efectivo) la app seguía avisando días después, y sólo revisaba al arrancar.
//! La marca manual queda como respaldo cuando no hay navdata de Navigraph.

use std::path::Path;

use chrono::{Datelike, Duration, Local, NaiveDate};
use serde::Serialize;
use sqlx::SqlitePool;

use crate::AppState;

const AIRAC_INSTALLED_KEY: &str = "airac_installed_cycle";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiracUpdateInfo {
    pub installed_cycle: String,
    pub latest_cycle: String,
    pub effective_date: String, // ISO YYYY-MM-DD del ciclo vigente
    pub has_update: bool,
    /// (v7.4.2) De dónde sale `installed_cycle`: `"navigraph"` (leído de la
    /// navdata instalada en la Community) o `"manual"` (marca guardada al
    /// pulsar "Actualizar" en la app).
    pub installed_source: &'static str,
    /// (v7.4.2) Próximo ciclo y su fecha efectiva (ISO).
    pub next_cycle: String,
    pub next_effective_date: String,
}

/// Ancla: AIRAC 2301 efectivo 2023-01-26.
fn anchor() -> NaiveDate {
    NaiveDate::from_ymd_opt(2023, 1, 26).unwrap()
}

/// Ciclo AIRAC ("YYNN") y su fecha efectiva para una fecha dada.
fn airac_for(date: NaiveDate) -> (String, NaiveDate) {
    let days = (date - anchor()).num_days();
    let periods = days.div_euclid(28);
    let eff = anchor() + Duration::days(periods * 28);
    let year = eff.year();
    // Ordinal dentro del año = nº de fechas efectivas de ese año hasta `eff`.
    let mut first = eff;
    while (first - Duration::days(28)).year() == year {
        first -= Duration::days(28);
    }
    let ordinal = ((eff - first).num_days() / 28) + 1;
    (format!("{:02}{:02}", year % 100, ordinal), eff)
}

/// (v7.4.2) Fecha efectiva de un ciclo "YYNN", o `None` si ese ciclo no
/// existe en el calendario (p.ej. "2615", o un "0001" de un paquete base).
/// Es la validación de días: todo ciclo que la app muestra o compara pasa por
/// aquí y se ancla a su fecha real.
fn cycle_effective_date(cycle: &str) -> Option<NaiveDate> {
    let c = cycle.trim();
    if c.len() != 4 || !c.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    let yy: i32 = c[..2].parse().ok()?;
    let nn: i64 = c[2..].parse().ok()?;
    // Nada anterior a 2020 (MSFS 2020): descarta basura como el "0.1.0" del
    // paquete base de Navigraph, que se leería como "0001".
    if yy < 20 || nn < 1 {
        return None;
    }
    // Primera fecha efectiva de 20YY: el primer múltiplo de 28 días desde el
    // ancla que cae dentro de ese año.
    let jan1 = NaiveDate::from_ymd_opt(2000 + yy, 1, 1)?;
    let (_, eff_before) = airac_for(jan1 - Duration::days(1));
    let first = eff_before + Duration::days(28);
    let eff = first + Duration::days((nn - 1) * 28);
    // El ordinal no puede desbordar al año siguiente (13 o 14 ciclos/año).
    (eff.year() == 2000 + yy).then_some(eff)
}

/// (v7.4.2) Ciclo AIRAC REAL instalado en el sim por Navigraph Hub. Se busca
/// en las Community detectadas (`navigraph-nav-*`), prefiriendo la del sim
/// elegido en la app y, dentro de ella, el ciclo más nuevo.
fn detect_navigraph_cycle() -> Option<String> {
    let folders = crate::community::detect_all_community_folders().ok()?;
    let prefer = if crate::sim::is_2024() { "2024" } else { "2020" };
    let mut found: Vec<(bool, String)> = Vec::new();
    for f in folders.iter().filter(|f| f.exists) {
        let Ok(rd) = std::fs::read_dir(&f.path) else {
            continue;
        };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_ascii_lowercase();
            if !name.starts_with("navigraph-nav") {
                continue;
            }
            if let Some(c) = read_manifest_cycle(&e.path().join("manifest.json")) {
                found.push((f.variant.contains(prefer), c));
            }
        }
    }
    let newest = |preferred_only: bool| {
        found
            .iter()
            .filter(|(p, _)| *p || !preferred_only)
            .filter_map(|(_, c)| cycle_effective_date(c).map(|d| (d, c.clone())))
            .max()
            .map(|(_, c)| c)
    };
    newest(true).or_else(|| newest(false))
}

/// Lee el ciclo del manifest de un paquete de navdata de Navigraph:
/// `"title": "AIRAC Cycle 2610 rev.1"` o, si no, `"package_version": "26.10.1"`.
/// Descarta lo que no sea un ciclo real (el paquete base dice
/// "AIRAC Cycle Base" / "0.1.0").
fn read_manifest_cycle(path: &Path) -> Option<String> {
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
    static TITLE_RE: once_cell::sync::Lazy<regex::Regex> =
        once_cell::sync::Lazy::new(|| regex::Regex::new(r"(?i)AIRAC\s+Cycle\s+(\d{4})\b").unwrap());
    let from_title = v
        .get("title")
        .and_then(|t| t.as_str())
        .and_then(|t| TITLE_RE.captures(t))
        .map(|c| c[1].to_string());
    let from_version = || {
        let pv = v.get("package_version")?.as_str()?;
        let mut it = pv.split('.');
        let yy: u32 = it.next()?.parse().ok()?;
        let nn: u32 = it.next()?.parse().ok()?;
        Some(format!("{yy:02}{nn:02}"))
    };
    from_title
        .or_else(from_version)
        .filter(|c| cycle_effective_date(c).is_some())
}

async fn read_installed(pool: &SqlitePool, latest: &str, today: NaiveDate) -> String {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT value FROM settings WHERE key = ?1")
            .bind(AIRAC_INSTALLED_KEY)
            .fetch_optional(pool)
            .await
            .ok()
            .flatten();
    match row.map(|r| r.0).filter(|s| cycle_effective_date(s).is_some()) {
        Some(v) => v,
        // Semilla por defecto: el ciclo ANTERIOR al vigente, para que la primera
        // corrida muestre la notificación (también sirve de prueba). Al
        // actualizar se reemplaza por el real.
        None => {
            let prev = airac_for(today - Duration::days(28)).0;
            if prev == latest {
                latest.to_string()
            } else {
                prev
            }
        }
    }
}

async fn save_installed(pool: &SqlitePool, cycle: &str) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO settings (key, value, updated_at)
        VALUES (?1, ?2, datetime('now'))
        ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at
        "#,
    )
    .bind(AIRAC_INSTALLED_KEY)
    .bind(cycle.trim())
    .execute(pool)
    .await
    .map(|_| ())
}

/// Calcula el estado AIRAC para `today` dado el ciclo instalado. Puro (sin
/// IO) para poder testear la lógica de fechas.
fn evaluate(today: NaiveDate, installed: &str) -> (String, NaiveDate, bool, String, NaiveDate) {
    let (latest_cycle, eff) = airac_for(today);
    let (next_cycle, next_eff) = airac_for(eff + Duration::days(28));
    // "Más nuevo" se decide por FECHA efectiva, no por el número (cruces de
    // año 2613→2701, ciclos inválidos…). Un ciclo instalado inválido cuenta
    // como desactualizado.
    let has_update = match cycle_effective_date(installed) {
        Some(installed_eff) => installed_eff < eff,
        None => true,
    };
    (latest_cycle, eff, has_update, next_cycle, next_eff)
}

#[tauri::command]
pub async fn airac_check_update(
    state: tauri::State<'_, AppState>,
) -> Result<AiracUpdateInfo, String> {
    // (v7.2.4) Fecha LOCAL, no UTC: los ciclos AIRAC son efectivos a las 0000Z,
    // pero el usuario compara contra SU calendario. Con UTC, un usuario en
    // UTC-4 (Rep. Dominicana) veía el ciclo nuevo la noche ANTERIOR a la fecha
    // efectiva impresa (a las 20:00 locales ya era el día siguiente en UTC).
    // Usando la fecha local, el aviso aparece el mismo día que la fecha mostrada.
    let today = Local::now().date_naive();
    let latest_now = airac_for(today).0;
    let detected = tokio::task::spawn_blocking(detect_navigraph_cycle)
        .await
        .ok()
        .flatten();
    let (installed, installed_source) = match detected {
        Some(c) => {
            // Mantiene la marca manual alineada con la realidad.
            let _ = save_installed(&state.db, &c).await;
            (c, "navigraph")
        }
        None => (read_installed(&state.db, &latest_now, today).await, "manual"),
    };
    let (latest_cycle, eff, has_update, next_cycle, next_eff) = evaluate(today, &installed);
    Ok(AiracUpdateInfo {
        installed_cycle: installed,
        latest_cycle,
        effective_date: eff.format("%Y-%m-%d").to_string(),
        has_update,
        installed_source,
        next_cycle,
        next_effective_date: next_eff.format("%Y-%m-%d").to_string(),
    })
}

/// Marca un ciclo como instalado (tras actualizar en Navigraph Hub).
#[tauri::command]
pub async fn airac_set_installed_cycle(
    cycle: String,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    if cycle_effective_date(&cycle).is_none() {
        return Err(format!("ciclo AIRAC inválido: {cycle}"));
    }
    save_installed(&state.db, &cycle).await.map_err(|e| e.to_string())
}

/// Resuelve la ruta del actualizador (Navigraph Hub) POR USUARIO. Devuelve el
/// exe o el acceso directo si existe; `None` si no se encuentra (el frontend
/// abre entonces la web). No se hardcodea la ruta porque vive en el perfil del
/// usuario (otro PC = otro usuario).
#[tauri::command]
pub fn airac_updater_path() -> Option<String> {
    let candidates: Vec<std::path::PathBuf> = {
        let mut v = Vec::new();
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            v.push(
                std::path::Path::new(&local)
                    .join("Programs")
                    .join("navigraph-hub")
                    .join("Navigraph Hub.exe"),
            );
        }
        if let Ok(appdata) = std::env::var("APPDATA") {
            v.push(
                std::path::Path::new(&appdata)
                    .join(r"Microsoft\Windows\Start Menu\Programs\Navigraph Hub\Navigraph Hub.lnk"),
            );
        }
        v
    };
    candidates
        .into_iter()
        .find(|p| p.exists())
        .map(|p| p.to_string_lossy().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    /// Fechas efectivas oficiales (calendario AIRAC de la OACI).
    const KNOWN: &[(&str, (i32, u32, u32))] = &[
        ("2001", (2020, 1, 2)),
        ("2014", (2020, 12, 31)), // año de 14 ciclos
        ("2101", (2021, 1, 28)),
        ("2301", (2023, 1, 26)),
        ("2313", (2023, 12, 28)),
        ("2401", (2024, 1, 25)),
        ("2501", (2025, 1, 23)),
        ("2513", (2025, 12, 25)),
        ("2601", (2026, 1, 22)),
        ("2609", (2026, 9, 3)),
        ("2610", (2026, 10, 1)),
        ("2611", (2026, 10, 29)),
        ("2613", (2026, 12, 24)),
        ("2701", (2027, 1, 21)),
    ];

    #[test]
    fn effective_dates_match_official_calendar() {
        for (cycle, (y, m, day)) in KNOWN {
            assert_eq!(cycle_effective_date(cycle), Some(d(*y, *m, *day)), "{cycle}");
            // Ida y vuelta: el día efectivo y el anterior caen en el ciclo correcto.
            assert_eq!(airac_for(d(*y, *m, *day)).0, *cycle, "{cycle} el día efectivo");
            assert_ne!(airac_for(d(*y, *m, *day) - Duration::days(1)).0, *cycle);
        }
    }

    #[test]
    fn invalid_cycles_are_rejected() {
        for bad in ["2615", "2614", "0000", "2600", "26", "abcd", "0001"] {
            assert_eq!(cycle_effective_date(bad), None, "{bad}");
        }
    }

    #[test]
    fn update_flag_follows_effective_dates() {
        // 2610 instalado (llegó el 27/09): sin aviso hasta el 29/10.
        assert!(!evaluate(d(2026, 9, 27), "2610").2, "instalado antes de ser efectivo");
        assert!(!evaluate(d(2026, 10, 18), "2610").2);
        assert!(!evaluate(d(2026, 10, 28), "2610").2, "la víspera del 2611");
        assert!(evaluate(d(2026, 10, 29), "2610").2, "el mismo día del 2611");
        // 2609 instalado: el aviso empieza EXACTAMENTE el 01/10, no antes.
        assert!(!evaluate(d(2026, 9, 30), "2609").2);
        assert!(evaluate(d(2026, 10, 1), "2609").2);
        // Cruce de año.
        assert!(evaluate(d(2027, 1, 21), "2613").2);
        assert!(!evaluate(d(2027, 1, 20), "2613").2);
        // Próximo ciclo.
        let (_, _, _, next, next_eff) = evaluate(d(2026, 10, 18), "2610");
        assert_eq!((next.as_str(), next_eff), ("2611", d(2026, 10, 29)));
    }
}
