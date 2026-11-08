//! (03/11) Etapa 7 — modos de lanzamiento y conjuntos de addons.
//!
//! Qué se enciende y qué se apaga lo calcula el front, con la misma
//! clasificación que el usuario ve en Instalados (`src/lib/dispatch.ts`). Aquí
//! viven los conjuntos, la lista «Nunca apagar» y el punto de restauración, y
//! se aplican los cambios con el mismo interruptor de siempre (`layout.json`).

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};
use tauri::{AppHandle, Emitter};

use crate::airline_economy::normalize_reg;
use crate::db::repo;
use crate::my_fleet::installed_liveries;
use crate::package_ops;

const KEY_MODE: &str = "dispatch_mode";
const KEY_SET: &str = "dispatch_set_id";
const KEY_PENDING: &str = "dispatch_pending_restore";
const KEY_ALTERNATE: &str = "dispatch_use_alternate";
const KEY_TRIP: &str = "dispatch_trip";
const KEY_ACTIVE: &str = "dispatch_active_set";
/// Un día de vuelos no pasa de aquí, y la lista de la revisión se hace eterna.
const MAX_STOPS: usize = 20;
/// Puntos de restauración que se conservan.
const MAX_RESTORE_POINTS: i64 = 5;
/// El índice de liveries lee cientos de archivos: no se repite en cada apertura.
const LIVERIES_TTL: Duration = Duration::from_secs(600);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContentSet {
    pub id: i64,
    pub name: String,
    /// `user` | `restore`.
    pub kind: String,
    /// Encendidos al guardar.
    pub enabled: Vec<String>,
    /// Todos los que existían al guardar.
    pub known: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DispatchState {
    /// `just` | `add` | `focus` | `set`.
    pub mode: String,
    pub set_id: Option<i64>,
    pub keep: Vec<String>,
    pub sets: Vec<ContentSet>,
    /// Punto de restauración que falta por ofrecer o aplicar.
    pub pending_restore: Option<i64>,
    /// (03/11) Encender también el escenario del alternativo del OFP.
    pub use_alternate: bool,
    /// (04/11) Viaje de varias paradas activo.
    pub trip: Option<DispatchTrip>,
    /// (05/11) Escenario elegido por aeropuerto cuando hay varios encendidos.
    pub scenery_choices: Vec<SceneryChoice>,
    /// (06/11) Último conjunto aplicado o guardado.
    pub active_set: Option<i64>,
}

/// (05/11) Qué hacer en un aeropuerto con varios escenarios encendidos.
#[derive(Debug, Clone, PartialEq, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct SceneryChoice {
    pub icao: String,
    /// Carpeta del escenario elegido.
    pub folder: Option<String>,
    /// No avisar en este aeropuerto: se quedan todos encendidos.
    pub ignored: bool,
}

/// (04/11) Varios vuelos en un día: paradas en orden y tipo de avión.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DispatchTrip {
    pub stops: Vec<String>,
    #[serde(default)]
    pub aircraft: Option<String>,
}

/// (03/11) Una livery instalada con su matrícula.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveryEntry {
    pub folder: String,
    pub title: String,
    /// Sin guiones ni espacios, en mayúsculas.
    pub registration: String,
}

/// Cambios ya confirmados por el usuario.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DispatchChanges {
    #[serde(default)]
    pub on: Vec<String>,
    #[serde(default)]
    pub off: Vec<String>,
    /// Nombre del punto de restauración si esto apaga algo.
    #[serde(default)]
    pub restore_name: Option<String>,
    /// Ese punto queda pendiente: se ofrecerá al cerrar el simulador.
    #[serde(default)]
    pub mark_pending: bool,
    /// Esto sustituye a lo pendiente (aplicar un conjunto o restaurar).
    #[serde(default)]
    pub clear_pending: bool,
    /// (06/11) Conjunto que queda como activo al aplicar esto.
    #[serde(default)]
    pub active_set: Option<i64>,
}

impl DispatchChanges {
    pub fn is_empty(&self) -> bool {
        self.on.is_empty() && self.off.is_empty() && !self.clear_pending && self.active_set.is_none()
    }
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyReport {
    pub enabled: Vec<String>,
    pub disabled: Vec<String>,
    /// Rutas que no se pudieron cambiar.
    pub failed: Vec<String>,
    pub pending_restore: Option<i64>,
}

// ---------------------------------------------------------------------------
// Ajustes
// ---------------------------------------------------------------------------

async fn get_setting(pool: &SqlitePool, key: &str) -> Result<Option<String>> {
    let row: Option<(String,)> = sqlx::query_as("SELECT value FROM settings WHERE key = ?1")
        .bind(key)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|r| r.0))
}

async fn set_setting(pool: &SqlitePool, key: &str, value: &str) -> Result<()> {
    sqlx::query(
        r#"INSERT INTO settings (key, value, updated_at)
           VALUES (?1, ?2, datetime('now'))
           ON CONFLICT(key) DO UPDATE SET
               value = excluded.value,
               updated_at = excluded.updated_at"#,
    )
    .bind(key)
    .bind(value)
    .execute(pool)
    .await?;
    Ok(())
}

async fn clear_setting(pool: &SqlitePool, key: &str) -> Result<()> {
    sqlx::query("DELETE FROM settings WHERE key = ?1").bind(key).execute(pool).await?;
    Ok(())
}

pub async fn pending_restore(pool: &SqlitePool) -> Result<Option<i64>> {
    let id = get_setting(pool, KEY_PENDING).await?.and_then(|v| v.parse::<i64>().ok());
    let Some(id) = id else { return Ok(None) };
    let exists: Option<(i64,)> = sqlx::query_as("SELECT id FROM content_set WHERE id = ?1")
        .bind(id)
        .fetch_optional(pool)
        .await?;
    Ok(exists.map(|r| r.0))
}

// ---------------------------------------------------------------------------
// Estado
// ---------------------------------------------------------------------------

fn parse_list(raw: &str) -> Vec<String> {
    serde_json::from_str(raw).unwrap_or_default()
}

async fn list_sets(pool: &SqlitePool) -> Result<Vec<ContentSet>> {
    let rows = sqlx::query(
        "SELECT id, name, kind, enabled, known, created_at, updated_at
         FROM content_set ORDER BY id DESC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|r| ContentSet {
            id: r.get("id"),
            name: r.get("name"),
            kind: r.get("kind"),
            enabled: parse_list(r.get("enabled")),
            known: parse_list(r.get("known")),
            created_at: r.get("created_at"),
            updated_at: r.get("updated_at"),
        })
        .collect())
}

async fn list_keep(pool: &SqlitePool) -> Result<Vec<String>> {
    let rows: Vec<(String,)> = sqlx::query_as("SELECT folder_name FROM dispatch_keep ORDER BY folder_name")
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(|r| r.0).collect())
}

async fn list_scenery_choices(pool: &SqlitePool) -> Result<Vec<SceneryChoice>> {
    Ok(sqlx::query_as("SELECT icao, folder_name AS folder, ignored FROM scenery_choice ORDER BY icao")
        .fetch_all(pool)
        .await?)
}

/// (05/11) Recuerda el escenario de un aeropuerto. Sin escenario y sin ignorar,
/// lo olvida y se vuelve a preguntar.
pub async fn set_scenery_choice(
    pool: &SqlitePool,
    icao: &str,
    folder: Option<&str>,
    ignored: bool,
) -> Result<Vec<SceneryChoice>> {
    let icao = icao.trim().to_ascii_uppercase();
    if icao.is_empty() {
        bail!("empty_icao");
    }
    if folder.is_none() && !ignored {
        sqlx::query("DELETE FROM scenery_choice WHERE icao = ?1")
            .bind(&icao)
            .execute(pool)
            .await?;
    } else {
        sqlx::query(
            r#"INSERT INTO scenery_choice (icao, folder_name, ignored, updated_at)
               VALUES (?1, ?2, ?3, datetime('now'))
               ON CONFLICT(icao) DO UPDATE SET folder_name = excluded.folder_name,
                   ignored = excluded.ignored, updated_at = excluded.updated_at"#,
        )
        .bind(&icao)
        .bind(folder)
        .bind(ignored)
        .execute(pool)
        .await?;
    }
    list_scenery_choices(pool).await
}

pub async fn state(pool: &SqlitePool) -> Result<DispatchState> {
    let sets = list_sets(pool).await?;
    let set_id = get_setting(pool, KEY_SET).await?.and_then(|v| v.parse::<i64>().ok());
    let set_id = set_id.filter(|id| sets.iter().any(|s| s.id == *id && s.kind == "user"));
    let mode = get_setting(pool, KEY_MODE).await?.unwrap_or_default();
    let mode = match mode.as_str() {
        "add" | "focus" => mode,
        "set" if set_id.is_some() => mode,
        _ => "just".to_string(),
    };
    let active_set = get_setting(pool, KEY_ACTIVE).await?.and_then(|v| v.parse::<i64>().ok());
    let active_set = active_set.filter(|id| sets.iter().any(|s| s.id == *id && s.kind == "user"));
    Ok(DispatchState {
        mode,
        set_id,
        keep: list_keep(pool).await?,
        sets,
        pending_restore: pending_restore(pool).await?,
        use_alternate: get_setting(pool, KEY_ALTERNATE).await?.as_deref() != Some("0"),
        trip: get_setting(pool, KEY_TRIP).await?.and_then(|v| serde_json::from_str(&v).ok()),
        scenery_choices: list_scenery_choices(pool).await?,
        active_set,
    })
}

/// Códigos válidos en mayúsculas. También valen los de 3 y 5 caracteres: hay
/// aeródromos sin ICAO de 4, como S50 o 0S9.
fn clean_trip(trip: DispatchTrip) -> Result<DispatchTrip> {
    let code = |s: &str, range: std::ops::RangeInclusive<usize>| {
        let s = s.trim().to_ascii_uppercase();
        (range.contains(&s.len()) && s.chars().all(|c| c.is_ascii_alphanumeric())).then_some(s)
    };
    let stops: Vec<String> = trip.stops.iter().filter_map(|s| code(s, 3..=5)).collect();
    if stops.len() < 2 {
        bail!("too_few_stops");
    }
    if stops.len() > MAX_STOPS {
        bail!("too_many_stops");
    }
    Ok(DispatchTrip { stops, aircraft: trip.aircraft.as_deref().and_then(|a| code(a, 2..=4)) })
}

/// Guarda el viaje hasta que se quite (`None`) y devuelve el que quedó.
pub async fn set_trip(pool: &SqlitePool, trip: Option<DispatchTrip>) -> Result<Option<DispatchTrip>> {
    let Some(trip) = trip else {
        clear_setting(pool, KEY_TRIP).await?;
        return Ok(None);
    };
    let trip = clean_trip(trip)?;
    set_setting(pool, KEY_TRIP, &serde_json::to_string(&trip)?).await?;
    Ok(Some(trip))
}

pub async fn set_use_alternate(pool: &SqlitePool, on: bool) -> Result<()> {
    set_setting(pool, KEY_ALTERNATE, if on { "1" } else { "0" }).await
}

// ---------------------------------------------------------------------------
// Liveries por matrícula
// ---------------------------------------------------------------------------

type LiveryCache = Mutex<Option<(Instant, Vec<LiveryEntry>)>>;

fn livery_cache() -> &'static LiveryCache {
    static CACHE: OnceLock<LiveryCache> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(None))
}

/// Tras un escaneo puede haber liveries nuevas.
pub fn invalidate_liveries() {
    *livery_cache().lock().unwrap_or_else(|e| e.into_inner()) = None;
}

/// Liveries instaladas con matrícula, sin los paquetes de tráfico IA.
pub async fn liveries(pool: &SqlitePool) -> Result<Vec<LiveryEntry>> {
    if let Some((at, list)) = livery_cache().lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        if at.elapsed() < LIVERIES_TTL {
            return Ok(list.clone());
        }
    }
    let list: Vec<LiveryEntry> = installed_liveries(pool)
        .await?
        .into_iter()
        .filter_map(|(folder, l)| {
            let registration = normalize_reg(l.registration.as_deref()?);
            (!registration.is_empty()).then_some(LiveryEntry { folder, title: l.title, registration })
        })
        .collect();
    *livery_cache().lock().unwrap_or_else(|e| e.into_inner()) = Some((Instant::now(), list.clone()));
    Ok(list)
}

pub async fn set_mode(pool: &SqlitePool, mode: &str, set_id: Option<i64>) -> Result<()> {
    if !matches!(mode, "just" | "add" | "focus" | "set") {
        bail!("modo desconocido: {mode}");
    }
    set_setting(pool, KEY_MODE, mode).await?;
    match set_id {
        Some(id) => set_setting(pool, KEY_SET, &id.to_string()).await,
        None => clear_setting(pool, KEY_SET).await,
    }
}

pub async fn set_keep(pool: &SqlitePool, folder: &str, keep: bool) -> Result<Vec<String>> {
    if keep {
        sqlx::query("INSERT OR IGNORE INTO dispatch_keep (folder_name) VALUES (?1)")
            .bind(folder)
            .execute(pool)
            .await?;
    } else {
        sqlx::query("DELETE FROM dispatch_keep WHERE folder_name = ?1")
            .bind(folder)
            .execute(pool)
            .await?;
    }
    list_keep(pool).await
}

// ---------------------------------------------------------------------------
// Conjuntos
// ---------------------------------------------------------------------------

/// Lo que hay ahora: (encendidos, todos). Sólo addons que se pueden apagar.
async fn current_lists(pool: &SqlitePool) -> Result<(Vec<String>, Vec<String>)> {
    let pkgs = repo::list_community_packages(pool).await?;
    let mut enabled = Vec::new();
    let mut known = Vec::new();
    for p in pkgs.iter().filter(|p| p.has_manifest) {
        known.push(p.folder_name.clone());
        if p.enabled {
            enabled.push(p.folder_name.clone());
        }
    }
    Ok((enabled, known))
}

async fn insert_set(pool: &SqlitePool, name: &str, kind: &str) -> Result<i64> {
    let (enabled, known) = current_lists(pool).await?;
    let res = sqlx::query("INSERT INTO content_set (name, kind, enabled, known) VALUES (?1, ?2, ?3, ?4)")
        .bind(name)
        .bind(kind)
        .bind(serde_json::to_string(&enabled)?)
        .bind(serde_json::to_string(&known)?)
        .execute(pool)
        .await?;
    Ok(res.last_insert_rowid())
}

pub async fn save_set(pool: &SqlitePool, name: &str) -> Result<i64> {
    let name = name.trim();
    if name.is_empty() {
        bail!("empty_name");
    }
    let id = insert_set(pool, name, "user").await?;
    set_setting(pool, KEY_ACTIVE, &id.to_string()).await?;
    Ok(id)
}

/// Vuelve a guardar el conjunto con lo que hay encendido ahora.
pub async fn update_set(pool: &SqlitePool, id: i64) -> Result<()> {
    let (enabled, known) = current_lists(pool).await?;
    sqlx::query(
        "UPDATE content_set SET enabled = ?2, known = ?3, updated_at = datetime('now')
         WHERE id = ?1 AND kind = 'user'",
    )
    .bind(id)
    .bind(serde_json::to_string(&enabled)?)
    .bind(serde_json::to_string(&known)?)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn rename_set(pool: &SqlitePool, id: i64, name: &str) -> Result<()> {
    let name = name.trim();
    if name.is_empty() {
        bail!("empty_name");
    }
    sqlx::query("UPDATE content_set SET name = ?2, updated_at = datetime('now') WHERE id = ?1")
        .bind(id)
        .bind(name)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn delete_set(pool: &SqlitePool, id: i64) -> Result<()> {
    sqlx::query("DELETE FROM content_set WHERE id = ?1").bind(id).execute(pool).await?;
    if pending_restore(pool).await?.is_none() {
        clear_setting(pool, KEY_PENDING).await?;
    }
    Ok(())
}

/// Conserva los últimos puntos de restauración (y siempre el pendiente).
async fn prune_restore_points(pool: &SqlitePool) -> Result<()> {
    let pending = pending_restore(pool).await?.unwrap_or(0);
    sqlx::query(
        "DELETE FROM content_set
         WHERE kind = 'restore' AND id <> ?1 AND id NOT IN (
             SELECT id FROM content_set WHERE kind = 'restore' ORDER BY id DESC LIMIT ?2
         )",
    )
    .bind(pending)
    .bind(MAX_RESTORE_POINTS)
    .execute(pool)
    .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Aplicar
// ---------------------------------------------------------------------------

fn unique(list: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    list.into_iter().filter(|f| seen.insert(f.to_lowercase())).collect()
}

/// Enciende primero y apaga después. Antes de apagar nada guarda el punto de
/// restauración; si ya hay uno pendiente se conserva, porque ese es el estado
/// al que el usuario querrá volver.
pub async fn apply(pool: &SqlitePool, ch: DispatchChanges) -> Result<ApplyReport> {
    let keep: HashSet<String> = list_keep(pool).await?.iter().map(|f| f.to_lowercase()).collect();
    let on = unique(ch.on);
    let off: Vec<String> = unique(ch.off)
        .into_iter()
        .filter(|f| !keep.contains(&f.to_lowercase()))
        .collect();

    if ch.clear_pending {
        clear_setting(pool, KEY_PENDING).await?;
    }
    if let Some(id) = ch.active_set {
        set_setting(pool, KEY_ACTIVE, &id.to_string()).await?;
    }
    if !off.is_empty() {
        let pending = pending_restore(pool).await?;
        if let Some(name) = ch.restore_name.as_deref().filter(|_| !(ch.mark_pending && pending.is_some())) {
            let id = insert_set(pool, name, "restore").await?;
            if ch.mark_pending {
                set_setting(pool, KEY_PENDING, &id.to_string()).await?;
            }
            prune_restore_points(pool).await?;
        }
    }

    let turned_on = package_ops::set_packages_enabled(pool, &on, true).await?;
    let turned_off = package_ops::set_packages_enabled(pool, &off, false).await?;
    Ok(ApplyReport {
        enabled: turned_on.changed,
        disabled: turned_off.changed,
        failed: turned_on.failed.iter().chain(&turned_off.failed).map(|f| f.path.clone()).collect(),
        pending_restore: pending_restore(pool).await?,
    })
}

/// Aplica, avisa al front y vigila el cierre del simulador si queda pendiente.
pub async fn apply_and_notify(app: &AppHandle, pool: &SqlitePool, ch: DispatchChanges) -> Result<ApplyReport> {
    let report = apply(pool, ch).await?;
    let _ = app.emit("dispatch://applied", &report);
    if report.pending_restore.is_some() {
        ensure_restore_watch(app.clone(), pool.clone());
    }
    Ok(report)
}

static WATCH: AtomicBool = AtomicBool::new(false);

/// Con un punto pendiente, avisa cuando el simulador se cierre (tras haberlo
/// visto abierto). Termina si deja de haber nada pendiente.
pub fn ensure_restore_watch(app: AppHandle, pool: SqlitePool) {
    if WATCH.swap(true, Ordering::SeqCst) {
        return;
    }
    tauri::async_runtime::spawn(async move {
        let mut seen = false;
        let mut gone = 0;
        loop {
            let Some(id) = pending_restore(&pool).await.ok().flatten() else { break };
            if crate::launch_flow::sim_running_now().await {
                seen = true;
                gone = 0;
            } else if seen {
                gone += 1;
                if gone >= 2 {
                    tracing::info!(target: "dispatch", "simulador cerrado: se ofrece restaurar {id}");
                    let _ = app.emit("dispatch://sim-closed", id);
                    crate::desktop_notice::show(
                        &app,
                        crate::tr!("dispatch.restoreTitle"),
                        Some(crate::tr!("dispatch.restoreBody")),
                        Some("info".into()),
                        false,
                    );
                    break;
                }
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
        WATCH.store(false, Ordering::SeqCst);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[tokio::test]
    async fn a_scenery_choice_is_remembered_changed_and_forgotten() {
        let pool = db().await;

        let saved = set_scenery_choice(&pool, " cyow ", Some("simaddons-airport-cyow"), false).await.unwrap();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].icao, "CYOW");
        assert_eq!(saved[0].folder.as_deref(), Some("simaddons-airport-cyow"));
        assert!(!saved[0].ignored);

        let saved = set_scenery_choice(&pool, "CYOW", None, true).await.unwrap();
        assert_eq!(saved.len(), 1, "un aeropuerto, una fila");
        assert!(saved[0].ignored);
        assert_eq!(saved[0].folder, None);
        assert_eq!(state(&pool).await.unwrap().scenery_choices.len(), 1);

        assert!(set_scenery_choice(&pool, "CYOW", None, false).await.unwrap().is_empty());
        assert!(set_scenery_choice(&pool, "  ", None, true).await.is_err());
    }

    #[tokio::test]
    async fn the_active_set_is_the_last_one_saved_or_applied() {
        let pool = db().await;
        assert_eq!(state(&pool).await.unwrap().active_set, None);

        let first = save_set(&pool, "Largo radio").await.unwrap();
        let second = save_set(&pool, "Corto radio").await.unwrap();
        assert_eq!(state(&pool).await.unwrap().active_set, Some(second));

        let apply_first = DispatchChanges { active_set: Some(first), ..Default::default() };
        assert!(!apply_first.is_empty(), "aplicar un conjunto idéntico también cuenta");
        apply(&pool, apply_first).await.unwrap();
        assert_eq!(state(&pool).await.unwrap().active_set, Some(first));

        delete_set(&pool, first).await.unwrap();
        assert_eq!(state(&pool).await.unwrap().active_set, None, "un conjunto borrado deja de estar activo");
    }

    async fn db() -> SqlitePool {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool
    }

    /// Paquete real en disco, con su `layout.json`.
    async fn pkg(pool: &SqlitePool, root: &Path, folder: &str) {
        let dir = root.join(folder);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("layout.json"), "{}").unwrap();
        sqlx::query("INSERT INTO community_packages (folder_name, install_path, title) VALUES (?1, ?2, ?1)")
            .bind(folder)
            .bind(dir.to_string_lossy().into_owned())
            .execute(pool)
            .await
            .unwrap();
    }

    fn is_on(root: &Path, folder: &str) -> bool {
        root.join(folder).join("layout.json").is_file()
    }

    fn focus(on: &[&str], off: &[&str], name: &str) -> DispatchChanges {
        DispatchChanges {
            on: on.iter().map(|s| s.to_string()).collect(),
            off: off.iter().map(|s| s.to_string()).collect(),
            restore_name: Some(name.into()),
            mark_pending: true,
            clear_pending: false,
            active_set: None,
        }
    }

    #[tokio::test]
    async fn focus_saves_the_state_before_turning_anything_off() {
        let pool = db().await;
        let tmp = tempfile::tempdir().unwrap();
        for f in ["a", "b", "c"] {
            pkg(&pool, tmp.path(), f).await;
        }
        let report = apply(&pool, focus(&[], &["b", "c"], "Antes")).await.unwrap();
        assert_eq!(report.disabled, vec!["b", "c"]);
        assert!(report.failed.is_empty());
        assert!(is_on(tmp.path(), "a"));
        assert!(!is_on(tmp.path(), "b"));

        let st = state(&pool).await.unwrap();
        let point = &st.sets[0];
        assert_eq!(point.kind, "restore");
        assert_eq!(st.pending_restore, Some(point.id));
        // Antes de apagar: todo encendido.
        assert_eq!(point.enabled.len(), 3);
    }

    #[tokio::test]
    async fn a_second_focus_keeps_the_original_restore_point() {
        let pool = db().await;
        let tmp = tempfile::tempdir().unwrap();
        for f in ["a", "b", "c"] {
            pkg(&pool, tmp.path(), f).await;
        }
        apply(&pool, focus(&[], &["b"], "Primero")).await.unwrap();
        apply(&pool, focus(&["b"], &["c"], "Segundo")).await.unwrap();
        let st = state(&pool).await.unwrap();
        assert_eq!(st.sets.len(), 1);
        assert_eq!(st.sets[0].name, "Primero");
        assert_eq!(st.pending_restore, Some(st.sets[0].id));
    }

    #[tokio::test]
    async fn never_turn_off_wins_even_if_asked() {
        let pool = db().await;
        let tmp = tempfile::tempdir().unwrap();
        pkg(&pool, tmp.path(), "kjfk").await;
        pkg(&pool, tmp.path(), "egll").await;
        set_keep(&pool, "KJFK", true).await.unwrap();
        let report = apply(&pool, focus(&[], &["kjfk", "egll"], "x")).await.unwrap();
        assert_eq!(report.disabled, vec!["egll"]);
        assert!(is_on(tmp.path(), "kjfk"));
        assert!(!is_on(tmp.path(), "egll"));
    }

    #[tokio::test]
    async fn nothing_turned_off_means_no_restore_point() {
        let pool = db().await;
        let tmp = tempfile::tempdir().unwrap();
        pkg(&pool, tmp.path(), "a").await;
        apply(&pool, focus(&[], &["a"], "x")).await.unwrap();
        sqlx::query("DELETE FROM content_set").execute(&pool).await.unwrap();
        clear_setting(&pool, KEY_PENDING).await.unwrap();
        let report = apply(&pool, focus(&["a"], &[], "y")).await.unwrap();
        assert_eq!(report.enabled, vec!["a"]);
        assert!(is_on(tmp.path(), "a"));
        let st = state(&pool).await.unwrap();
        assert!(st.sets.is_empty());
        assert_eq!(st.pending_restore, None);
    }

    #[tokio::test]
    async fn only_the_last_restore_points_stay() {
        let pool = db().await;
        let tmp = tempfile::tempdir().unwrap();
        pkg(&pool, tmp.path(), "a").await;
        pkg(&pool, tmp.path(), "b").await;
        for n in 0..8 {
            let ch = DispatchChanges {
                on: vec!["b".into()],
                off: vec!["a".into()],
                restore_name: Some(format!("punto {n}")),
                mark_pending: false,
                clear_pending: false,
                active_set: None,
            };
            apply(&pool, ch).await.unwrap();
            package_ops::set_packages_enabled(&pool, &["a".to_string()], true).await.unwrap();
        }
        let st = state(&pool).await.unwrap();
        assert_eq!(st.sets.len(), MAX_RESTORE_POINTS as usize);
        assert_eq!(st.sets[0].name, "punto 7");
    }

    #[tokio::test]
    async fn a_set_keeps_what_was_on_and_what_existed() {
        let pool = db().await;
        let tmp = tempfile::tempdir().unwrap();
        for f in ["a", "b"] {
            pkg(&pool, tmp.path(), f).await;
        }
        package_ops::set_packages_enabled(&pool, &["b".to_string()], false).await.unwrap();
        let id = save_set(&pool, "  Corto radio ").await.unwrap();
        // Sin manifest no se puede apagar: no cuenta.
        sqlx::query("UPDATE community_packages SET has_manifest = 0 WHERE folder_name = 'a'")
            .execute(&pool)
            .await
            .unwrap();
        pkg(&pool, tmp.path(), "nuevo").await;
        let st = state(&pool).await.unwrap();
        let set = st.sets.iter().find(|s| s.id == id).unwrap();
        assert_eq!(set.name, "Corto radio");
        assert_eq!(set.enabled, vec!["a"]);
        assert_eq!(set.known, vec!["a", "b"]);

        update_set(&pool, id).await.unwrap();
        let st = state(&pool).await.unwrap();
        let set = st.sets.iter().find(|s| s.id == id).unwrap();
        assert_eq!(set.enabled, vec!["nuevo"]);
        assert_eq!(set.known, vec!["b", "nuevo"]);
        assert!(save_set(&pool, "   ").await.is_err());
    }

    #[tokio::test]
    async fn a_deleted_set_stops_being_the_chosen_one() {
        let pool = db().await;
        let id = save_set(&pool, "Largo radio").await.unwrap();
        set_mode(&pool, "set", Some(id)).await.unwrap();
        assert_eq!(state(&pool).await.unwrap().mode, "set");
        delete_set(&pool, id).await.unwrap();
        let st = state(&pool).await.unwrap();
        assert_eq!(st.mode, "just");
        assert_eq!(st.set_id, None);
        assert!(set_mode(&pool, "otro", None).await.is_err());
    }

    #[tokio::test]
    async fn a_trip_stays_until_it_is_removed() {
        let pool = db().await;
        let trip = DispatchTrip {
            stops: vec![" kjfk".into(), "S50".into(), "x!".into(), "0s9".into(), "kjfk".into()],
            aircraft: Some("a20n ".into()),
        };
        let saved = set_trip(&pool, Some(trip)).await.unwrap().unwrap();
        // Volver al origen es un viaje normal: no se quitan repetidos.
        assert_eq!(saved.stops, vec!["KJFK", "S50", "0S9", "KJFK"]);
        assert_eq!(saved.aircraft.as_deref(), Some("A20N"));
        assert_eq!(state(&pool).await.unwrap().trip, Some(saved));

        assert_eq!(set_trip(&pool, None).await.unwrap(), None);
        assert_eq!(state(&pool).await.unwrap().trip, None);
    }

    #[tokio::test]
    async fn a_trip_needs_between_two_and_twenty_stops() {
        let pool = db().await;
        let few = DispatchTrip { stops: vec!["KJFK".into()], aircraft: None };
        assert!(set_trip(&pool, Some(few)).await.is_err());
        let many = DispatchTrip { stops: vec!["KJFK".into(); MAX_STOPS + 1], aircraft: None };
        assert!(set_trip(&pool, Some(many)).await.is_err());
        let no_type = DispatchTrip { stops: vec!["KJFK".into(), "CYYZ".into()], aircraft: Some("?".into()) };
        assert_eq!(set_trip(&pool, Some(no_type)).await.unwrap().unwrap().aircraft, None);
    }

    #[tokio::test]
    async fn ratings_reach_the_package_list() {
        let pool = db().await;
        let tmp = tempfile::tempdir().unwrap();
        pkg(&pool, tmp.path(), "EHAM").await;
        repo::set_addon_rating(&pool, "eham", Some(4)).await.unwrap();
        assert_eq!(repo::list_community_packages(&pool).await.unwrap()[0].rating, Some(4));
        repo::set_addon_rating(&pool, "EHAM", Some(5)).await.unwrap();
        assert_eq!(repo::list_community_packages(&pool).await.unwrap()[0].rating, Some(5));
        repo::set_addon_rating(&pool, "EHAM", None).await.unwrap();
        assert_eq!(repo::list_community_packages(&pool).await.unwrap()[0].rating, None);
        assert!(repo::set_addon_rating(&pool, "EHAM", Some(6)).await.is_err());
    }

    #[tokio::test]
    async fn the_alternate_is_on_until_turned_off() {
        let pool = db().await;
        assert!(state(&pool).await.unwrap().use_alternate);
        set_use_alternate(&pool, false).await.unwrap();
        assert!(!state(&pool).await.unwrap().use_alternate);
        set_use_alternate(&pool, true).await.unwrap();
        assert!(state(&pool).await.unwrap().use_alternate);
    }

    #[tokio::test]
    async fn liveries_are_found_by_registration_without_traffic() {
        let pool = db().await;
        let tmp = tempfile::tempdir().unwrap();
        let add = |folder: &str, file: &str, cfg: &str| {
            let dir = tmp.path().join(folder).join("SimObjects").join("Airplanes").join("X");
            let full = dir.join(file);
            std::fs::create_dir_all(full.parent().unwrap()).unwrap();
            std::fs::write(full, cfg).unwrap();
        };
        add("ifly-wja", "aircraft.cfg", "[FLTSIM.0]\ntitle = \"WestJet\"\natc_id = \"C-GORP\"\n");
        add("fnx-jbu", "liveries/fnx/JBU/livery.cfg", "[GENERAL]\nname = \"JetBlue\"\n[FLTSIM]\natc_id = \"N657JB\"\n");
        add("fsltl-traffic-x", "aircraft.cfg", "[FLTSIM.0]\ntitle = \"IA\"\natc_id = \"N1\"\n");
        add("sin-matricula", "aircraft.cfg", "[FLTSIM.0]\ntitle = \"Sin\"\n");
        for f in ["ifly-wja", "fnx-jbu", "fsltl-traffic-x", "sin-matricula"] {
            sqlx::query("INSERT INTO community_packages (folder_name, install_path, title, content_type) VALUES (?1, ?2, ?1, 'AIRCRAFT')")
                .bind(f)
                .bind(tmp.path().join(f).to_string_lossy().into_owned())
                .execute(&pool)
                .await
                .unwrap();
        }
        invalidate_liveries();
        let mut list = liveries(&pool).await.unwrap();
        invalidate_liveries();
        list.sort_by(|a, b| a.folder.cmp(&b.folder));
        let got: Vec<(&str, &str, &str)> =
            list.iter().map(|l| (l.folder.as_str(), l.title.as_str(), l.registration.as_str())).collect();
        assert_eq!(got, vec![("fnx-jbu", "JetBlue", "N657JB"), ("ifly-wja", "WestJet", "CGORP")]);
    }

    #[tokio::test]
    async fn restoring_clears_what_was_pending() {
        let pool = db().await;
        let tmp = tempfile::tempdir().unwrap();
        for f in ["a", "b"] {
            pkg(&pool, tmp.path(), f).await;
        }
        apply(&pool, focus(&[], &["b"], "Antes")).await.unwrap();
        let ch = DispatchChanges { on: vec!["b".into()], clear_pending: true, ..Default::default() };
        let report = apply(&pool, ch).await.unwrap();
        assert_eq!(report.pending_restore, None);
        assert!(is_on(tmp.path(), "b"));
    }
}
