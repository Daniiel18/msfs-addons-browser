use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use sqlx::SqlitePool;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::{RwLock, Semaphore};
use tokio_util::sync::CancellationToken;

use super::torrent::{resolve_magnet_from_url, TorrentEngine, TorrentProgress};
use super::{DownloadJob, DownloadPhase};
use crate::community;
use crate::db::repo;
use crate::install;
use crate::sources::{DownloadKind, DownloadMethod};

/// Intervalo mínimo entre escrituras a DB para el mismo job cuando
/// el cambio es **sólo de progreso** (bytes/speed/eta). Las
/// transiciones de fase se persisten siempre, sin throttle.
///
/// 3 segundos es buen balance: granularidad suficiente para que el
/// usuario vea "estaba a 4.2 GB" tras un crash, y barato (~120
/// escrituras en una descarga de 6 minutos vs ~720 sin throttle).
const PERSIST_PROGRESS_THROTTLE_SECS: u64 = 3;

/// (25/10) Borra lo que hay en `dir` salvo los archivos de `keep` (y las
/// carpetas que los contienen, de las que sólo se borra lo demás).
fn purge_except(dir: &std::path::Path, keep: &[PathBuf]) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let path = e.path();
        if keep.iter().any(|k| k == &path) {
            continue;
        }
        if keep.iter().any(|k| k.starts_with(&path)) {
            purge_except(&path, keep);
            continue;
        }
        let _ = if path.is_dir() { std::fs::remove_dir_all(&path) } else { std::fs::remove_file(&path) };
    }
}

/// (25/10) Días que se queda el historial de descargas (terminadas,
/// canceladas o con error).
const HISTORY_DAYS: i64 = 7;

/// Tauri event name the manager emits on every job state change.
pub const EVENT_JOB_UPDATE: &str = "download://update";

/// (v8.3) Estado de una descarga de la ventana de hoster para el panel de
/// Descargas: la ventana se OCULTA en cuanto empieza a bajar, así que el
/// progreso y el resultado se ven aquí.
pub enum HosterStatus {
    /// Esperando el clic del usuario en la ventana ("Parte 2 de 2" si aplica).
    Waiting(Option<String>),
    /// Bytes bajando (`total` 0 si el servidor no da tamaño).
    Progress { done: u64, total: u64, bps: u64, part: Option<String> },
    /// Bajado: el instalador de la ventana principal lo procesa.
    Downloaded,
    Failed(String),
    Cancelled,
    /// (v8.3) Lo cuenta el frontend al procesar lo bajado: extrayendo…
    Processing,
    /// … listo, esperando al usuario (modal de instalación / elegir carpeta) …
    Ready,
    /// … y el final: instalado en Community o guardado en una carpeta.
    Installed,
    Saved,
}

#[derive(Clone)]
pub struct DownloadManager {
    inner: Arc<Inner>,
}

struct Inner {
    jobs: RwLock<HashMap<String, DownloadJob>>,
    /// Per-job cancel tokens — dropped (and the job removed from the map)
    /// once the job reaches a terminal state so cancelling a completed
    /// job is a no-op rather than a confusing new event.
    cancels: RwLock<HashMap<String, CancellationToken>>,
    app: AppHandle,
    torrent_engine: Arc<TorrentEngine>,
    /// Pool compartido para persistir el historial de instalaciones.
    /// Se inyecta desde `init_state` en lugar de buscarlo vía
    /// `app.state::<AppState>()` para evitar el ciclo: el manager se
    /// construye antes de que `AppState` exista.
    db: SqlitePool,
    /// Semáforo que serializa el trabajo *real* de torrent (resolve →
    /// download → install). Concurrencia 1: sólo un torrent puede estar
    /// trabajando a la vez. Las tareas en cola se quedan en
    /// `DownloadPhase::Queued` mientras esperan su turno y entran a
    /// `Resolving` cuando el permit se libera. Sin esta serialización
    /// el ancho de banda y el disco se saturaban con varias descargas
    /// concurrentes y la instalación posterior tropezaba con sí misma.
    torrent_slot: Semaphore,
    /// Última vez que persistimos cada job a la tabla `download_jobs`.
    /// Lo usa el throttle para no escribir DB en cada tick de
    /// progreso (sin él, una descarga de 5min hace ~600 inserts).
    /// Cambios de fase ignoran este map y persisten siempre.
    last_persisted: RwLock<HashMap<String, Instant>>,
    /// (v6 #1 fix) Compatibilidad de simulador por job_id (ej.
    /// "MSFS 2020/2024"). El `addon_title` del job es el nombre LIMPIO
    /// (sin los años), así que no sirve para detectar doble-compat; el
    /// frontend nos pasa el campo `simulator` autoritativo y lo
    /// guardamos aquí para la oferta de cross-link tras instalar.
    sim_compat: RwLock<HashMap<String, String>>,
}

impl DownloadManager {
    pub fn new(app: AppHandle, torrent_data_dir: PathBuf, db: SqlitePool) -> Self {
        Self {
            inner: Arc::new(Inner {
                jobs: RwLock::default(),
                cancels: RwLock::default(),
                app,
                torrent_engine: Arc::new(TorrentEngine::new(torrent_data_dir)),
                db,
                torrent_slot: Semaphore::new(1),
                last_persisted: RwLock::default(),
                sim_compat: RwLock::default(),
            }),
        }
    }

    pub async fn list(&self) -> Vec<DownloadJob> {
        let map = self.inner.jobs.read().await;
        let mut v: Vec<_> = map.values().cloned().collect();
        // newest first — UI wants a stack-ordered list.
        v.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        v
    }

    pub async fn start(
        &self,
        addon_id: String,
        addon_title: String,
        source: String,
        method: DownloadMethod,
        addon_simulator: String,
    ) -> anyhow::Result<DownloadJob> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();

        // (v6 #1 fix) Recordamos la compatibilidad de simulador para la oferta
        // de cross-link post-instalación (el addon_title viene sin los años).
        if !addon_simulator.trim().is_empty() {
            self.inner
                .sim_compat
                .write()
                .await
                .insert(id.clone(), addon_simulator);
        }

        let kind_is_torrent = matches!(method.kind, DownloadKind::Torrent);
        let addon_id_img = addon_id.clone();
        let kind_str = match method.kind {
            DownloadKind::Torrent => "torrent",
            DownloadKind::Mirror => "mirror",
            DownloadKind::Direct => "direct",
        }
        .to_string();

        let job = DownloadJob {
            id: id.clone(),
            addon_id,
            addon_title,
            source,
            method_kind: kind_str,
            method_name: method.name.clone(),
            url: method.url.clone(),
            phase: DownloadPhase::Queued,
            bytes_total: 0,
            bytes_done: 0,
            speed_bps: 0,
            eta_seconds: None,
            message: None,
            error: None,
            install_path: None,
            created_at: now,
            image_url: repo::addon_image(&self.inner.db, &addon_id_img).await,
            pausable: false,
            resumable: kind_is_torrent,
            resume: None,
        };

        self.set_job(job.clone()).await;

        match method.kind {
            DownloadKind::Mirror | DownloadKind::Direct => {
                // (v7) En vez de abrir el navegador del sistema y que el
                // usuario baje + instale a mano, abrimos un navegador
                // EMBEBIDO (con ad blocker + bloqueo de popups) que captura la
                // descarga y la auto-instala en la Community correcta. Si el
                // método trae `parts` (Skybound multi-parte), baja todas en
                // orden y las une antes de instalar.
                self.run_embedded_download(&id, method.url, method.parts, method.password)
                    .await;
            }
            DownloadKind::Torrent => {
                // Asynchronous: the torrent flow runs in the background so
                // the UI can track multiple concurrent jobs. `start` returns
                // as soon as the job is registered and transitioned to
                // Resolving.
                let cancel = CancellationToken::new();
                self.inner
                    .cancels
                    .write()
                    .await
                    .insert(id.clone(), cancel.clone());

                let this = self.clone();
                let job_id = id.clone();
                // `method.url` may be either a direct `magnet:?` URI or an
                // HTTPS hoster URL that resolves to one — `run_torrent`
                // handles both cases.
                let torrent_url = method.url.clone();
                tauri::async_runtime::spawn(async move {
                    this.run_torrent(&job_id, torrent_url, cancel).await;
                    // Clean up the cancel token regardless of outcome.
                    this.inner.cancels.write().await.remove(&job_id);
                });
            }
        }

        // Return the post-dispatch snapshot so synchronous Mirror/Direct
        // callers see the terminal state; Torrent callers see `Resolving`
        // set by `run_torrent`'s first update.
        let map = self.inner.jobs.read().await;
        Ok(map.get(&id).cloned().unwrap_or(job))
    }

    pub async fn cancel(&self, id: &str) -> anyhow::Result<()> {
        // A paused torrent won't be sitting inside librqbit's download
        // loop, so the cancel token alone isn't enough to unstick it —
        // unpause first so the regular cancellation path can run.
        // Best-effort: ignore errors (e.g. already not paused).
        let _ = self.inner.torrent_engine.unpause(id).await;

        // Trigger any per-job cancel token first — the torrent flow
        // watches this and unwinds cleanly.
        if let Some(tok) = self.inner.cancels.read().await.get(id).cloned() {
            tok.cancel();
        }
        // Also flip the UI state immediately for synchronous flows
        // (Mirror/Direct) where there's no cancel token to honour.
        self.update(id, |j| {
            if !matches!(
                j.phase,
                DownloadPhase::Completed | DownloadPhase::Error | DownloadPhase::Cancelled
            ) {
                j.phase = DownloadPhase::Cancelled;
                j.message = Some(crate::tr!("manager.cancelledByUser"));
            }
        })
        .await;
        // (v8.3) Descarga del navegador embebido: cerrar su ventana (oculta
        // mientras baja) la corta, también la de reqwest, que vigila la ventana.
        // DESPUÉS de marcar "cancelado": el corte llega como fallo y un job
        // cancelado ya no cambia (no sale "falló la descarga").
        let label = format!("hoster-dl-{id}");
        if let Some(w) = self.inner.app.get_webview_window(&label) {
            let _ = w.close();
            let _ = std::fs::remove_dir_all(crate::commands::livery::hoster_dl_dir(&label));
        }
        // (v8.3) Descarga de la web de flightsim.to: la corta su manejador de
        // progreso en cuanto ve la petición.
        crate::commands::livery::request_external_cancel(id);
        // (25/10) Directa en pausa (sin tarea viva): fuera lo bajado.
        if let Some(spec) = self.inner.jobs.read().await.get(id).and_then(|j| j.resume.clone()) {
            crate::commands::livery::forget_direct(id, &spec);
        }
        Ok(())
    }

    /// Pause the active torrent for `id`. Only meaningful for torrent
    /// jobs that have reached the `Downloading` phase — for everything
    /// else this is a no-op that still returns Ok (the UI button is
    /// only shown in the downloading state, but racing a phase change
    /// shouldn't surface a scary error).
    pub async fn pause(&self, id: &str) -> anyhow::Result<()> {
        let Some(job) = self.inner.jobs.read().await.get(id).cloned() else { return Ok(()) };
        if job.phase != DownloadPhase::Downloading {
            return Ok(());
        }
        // (25/10) Torrent (librqbit), descarga directa de SimFleet (corta el
        // bucle de bytes; lo bajado se queda) o la del navegador de la
        // ventana de hoster (WebView2 la pausa de verdad).
        let paused = if job.method_kind == "torrent" {
            // Engine errors are informative but not fatal — log and keep
            // the UI state honest.
            match self.inner.torrent_engine.pause(id).await {
                Ok(()) => true,
                Err(e) => {
                    tracing::warn!("pause({id}) failed: {e:#}");
                    false
                }
            }
        } else if job.resume.is_some() {
            crate::commands::livery::pause_direct(id);
            true
        } else {
            crate::commands::livery::set_webview_download_paused(&self.inner.app, id, true).await
        };
        if paused {
            self.update(id, |j| {
                if j.phase == DownloadPhase::Downloading {
                    j.phase = DownloadPhase::Paused;
                    j.speed_bps = 0;
                    j.eta_seconds = None;
                    j.message = Some(crate::tr!("manager.paused"));
                }
            })
            .await;
        }
        Ok(())
    }

    /// Resume a paused download. (25/10) Also retries a direct download that
    /// failed, from what was already downloaded.
    pub async fn resume(&self, id: &str) -> anyhow::Result<()> {
        let Some(job) = self.inner.jobs.read().await.get(id).cloned() else { return Ok(()) };
        let resumed = if job.method_kind == "torrent" {
            if job.phase != DownloadPhase::Paused {
                return Ok(());
            }
            if let Err(e) = self.inner.torrent_engine.unpause(id).await {
                // Sin torrent vivo (pausado antes de reiniciar SimFleet): se
                // relanza; librqbit sigue desde los archivos parciales.
                tracing::info!("resume({id}): sin torrent vivo ({e:#}) → lo relanzo");
                self.spawn_torrent(&job);
            }
            true
        } else if let Some(spec) = job.resume.clone() {
            if !matches!(job.phase, DownloadPhase::Paused | DownloadPhase::Error) {
                return Ok(());
            }
            crate::commands::livery::resume_direct(&self.inner.app, id, &spec)
        } else {
            job.phase == DownloadPhase::Paused
                && crate::commands::livery::set_webview_download_paused(&self.inner.app, id, false).await
        };
        if resumed {
            self.update(id, |j| {
                j.phase = DownloadPhase::Downloading;
                j.message = None;
                j.error = None;
            })
            .await;
        }
        Ok(())
    }

    /// (25/10) Purga de arranque de las carpetas temporales de descargas
    /// (`simfleet-mirror-dl`, `simfleet-livery-dl`) sin tocar los archivos
    /// parciales de las que siguen pendientes.
    pub async fn purge_temp(&self, temp_dir: &std::path::Path) {
        let keep: Vec<PathBuf> = self
            .inner
            .jobs
            .read()
            .await
            .values()
            .filter(|j| !matches!(j.phase, DownloadPhase::Completed | DownloadPhase::Cancelled | DownloadPhase::Error)
                || j.resume.is_some() && j.phase == DownloadPhase::Error)
            .filter_map(|j| j.resume.as_deref().and_then(crate::commands::livery::resume_target))
            .collect();
        for dir in ["simfleet-mirror-dl", "simfleet-livery-dl"] {
            purge_except(&temp_dir.join(dir), &keep);
        }
    }

    /// (25/10) Guarda cómo retomar la descarga directa `id` (JSON de
    /// `livery::ResumeSpec`): pausable, reanudable y sigue tras reiniciar.
    pub async fn set_resume(&self, id: &str, spec: Option<String>) {
        self.update_with(id, true, |j| j.resume = spec).await;
    }

    /// (25/10) La descarga del navegador de `id` se puede (o ya no) pausar.
    pub async fn set_pausable(&self, id: &str, pausable: bool) {
        self.update(id, |j| j.pausable = pausable).await;
    }

    fn spawn_torrent(&self, job: &DownloadJob) {
        let this = self.clone();
        let job_id = job.id.clone();
        let torrent_url = job.url.clone();
        tauri::async_runtime::spawn(async move {
            let cancel = CancellationToken::new();
            this.inner.cancels.write().await.insert(job_id.clone(), cancel.clone());
            this.run_torrent(&job_id, torrent_url, cancel).await;
            this.inner.cancels.write().await.remove(&job_id);
        });
    }

    pub async fn clear(&self, id: &str) {
        let removed = {
            let mut map = self.inner.jobs.write().await;
            map.remove(id)
        };
        if let Some(mut job) = removed {
            // (25/10) Una directa que no terminó: fuera lo bajado.
            if let Some(spec) = job.resume.as_deref().filter(|_| job.phase != DownloadPhase::Completed) {
                crate::commands::livery::forget_direct(id, spec);
            }
            // Emit one last update with a synthetic "removed" marker so
            // the UI reconciliation logic can drop the row.
            job.phase = DownloadPhase::Cancelled;
            job.message = Some("__removed__".into());
            let _ = self.inner.app.emit(EVENT_JOB_UPDATE, &job);
        }
        // Also drop any lingering cancel token (defensive).
        self.inner.cancels.write().await.remove(id);
        // Best-effort: borra la fila persistida. Si falla, sólo logueamos
        // — el usuario verá la fila reaparecer al próximo restart, que es
        // mejor que crashear el clear.
        if let Err(e) = repo::delete_download_job(&self.inner.db, id).await {
            tracing::warn!("clear: no se pudo borrar download_job {id} de DB: {e:#}");
        }
        self.inner.last_persisted.write().await.remove(id);
    }

    /// Restaura los jobs persistidos al arrancar la app — los carga
    /// en memoria, emite el evento de update para que la UI los
    /// pinte y re-lanza `run_torrent` para los que estaban activos.
    ///
    /// Llamado UNA vez desde `init_state` después de construir el
    /// manager. Idempotente sólo en el sentido de que ejecutarlo dos
    /// veces persiste cada job dos veces — no spawnea downloads
    /// duplicados porque la segunda llamada vería los jobs ya en el
    /// HashMap, pero la lógica de re-spawn no chequea eso. Por eso
    /// llamarlo dos veces es bug, no idempotente. **No lo llames
    /// dos veces.**
    ///
    /// Política de restauración por fase:
    /// - `Queued | Resolving | Downloading | Paused | Installing`:
    ///   torrent → re-spawn `run_torrent`. librqbit detecta los
    ///   ficheros parciales en `job-{id}/` y resume (gracias a
    ///   `overwrite: true`).
    /// - Mirror/Direct no-terminales: pasaron por `run_open_url` que
    ///   completa al instante. Si los vemos no-terminales es porque
    ///   la app crasheó *durante* el opener — los marcamos como
    ///   `Error` con mensaje explicativo y dejamos que el usuario
    ///   re-clique si quiere.
    /// - Terminales (`Completed | Cancelled | Error`): se quedan en
    ///   memoria como historial visible; no se acciona nada.
    pub async fn restore_persisted(&self) -> anyhow::Result<usize> {
        let jobs = repo::list_download_jobs(&self.inner.db).await?;
        // (25/10) El historial (terminadas, canceladas, con error) se queda una
        // semana; lo más viejo se borra.
        let cutoff = chrono::Utc::now() - chrono::Duration::days(HISTORY_DAYS);
        let (jobs, old): (Vec<_>, Vec<_>) = jobs.into_iter().partition(|j| {
            let terminal = matches!(
                j.phase,
                DownloadPhase::Completed | DownloadPhase::Cancelled | DownloadPhase::Error
            );
            let at = chrono::DateTime::parse_from_rfc3339(&j.created_at).map(|d| d.with_timezone(&chrono::Utc));
            !terminal || at.map_or(true, |d| d >= cutoff)
        });
        for j in &old {
            let _ = repo::delete_download_job(&self.inner.db, &j.id).await;
        }
        let count = jobs.len();
        tracing::info!(
            "downloads: restaurando {} job(s) persistido(s) ({} del historial de más de {HISTORY_DAYS} días borrado(s))",
            count,
            old.len()
        );

        for mut job in jobs {
            // (25/10) Jobs de antes de guardar la imagen: la del catálogo.
            if job.image_url.is_none() && !job.addon_id.is_empty() {
                job.image_url = repo::addon_image(&self.inner.db, &job.addon_id).await;
            }
            // Cargar en memoria + emitir para que el frontend
            // muestre la entrada inmediatamente al abrir el panel.
            {
                let mut map = self.inner.jobs.write().await;
                map.insert(job.id.clone(), job.clone());
            }
            let _ = self.inner.app.emit(EVENT_JOB_UPDATE, &job);

            let is_terminal = matches!(
                job.phase,
                DownloadPhase::Completed | DownloadPhase::Cancelled | DownloadPhase::Error
            );
            if is_terminal {
                continue;
            }

            // (25/10) Pausada antes de cerrar: sigue en pausa (Reanudar la relanza).
            if job.phase == DownloadPhase::Paused && job.resumable {
                tracing::info!("downloads: {} sigue en pausa ({})", job.id, job.method_kind);
                continue;
            }
            // (25/10) Descarga directa de SimFleet: sigue desde lo ya bajado.
            if let Some(spec) = job.resume.clone() {
                tracing::info!("downloads: reanudo la descarga directa {} (bytes_done={})", job.id, job.bytes_done);
                self.update(&job.id, |j| {
                    j.phase = DownloadPhase::Downloading;
                    j.message = None;
                })
                .await;
                if !crate::commands::livery::resume_direct(&self.inner.app, &job.id, &spec) {
                    self.update(&job.id, |j| {
                        j.phase = DownloadPhase::Error;
                        j.error = Some(crate::tr!("manager.interruptedRestart"));
                    })
                    .await;
                }
                continue;
            }

            match job.method_kind.as_str() {
                "torrent" => {
                    tracing::info!(
                        "downloads: resume torrent {} (phase={:?}, bytes_done={})",
                        job.id, job.phase, job.bytes_done
                    );
                    let cancel = CancellationToken::new();
                    self.inner
                        .cancels
                        .write()
                        .await
                        .insert(job.id.clone(), cancel.clone());

                    let this = self.clone();
                    let job_id = job.id.clone();
                    let torrent_url = job.url.clone();
                    tauri::async_runtime::spawn(async move {
                        this.run_torrent(&job_id, torrent_url, cancel).await;
                        this.inner.cancels.write().await.remove(&job_id);
                    });
                }
                _ => {
                    // Mirror/Direct: no hay nada que reanudar — un
                    // open_url no tiene estado intermedio. Marcamos
                    // como Error para no dejar al usuario mirando un
                    // spinner eterno.
                    tracing::warn!(
                        "downloads: job {} ({}) estaba en phase={:?} pero no es resumible — marcado Error",
                        job.id, job.method_kind, job.phase
                    );
                    // (25/10) La del navegador de la ventana no se puede seguir
                    // (su enlace era de un solo uso): "Reintentar".
                    self.update(&job.id, |j| {
                        j.phase = DownloadPhase::Error;
                        j.error = Some(crate::tr!("manager.interruptedRestart"));
                    })
                    .await;
                }
            }
        }
        Ok(count)
    }

    // ───────────────── internals ─────────────────

    async fn set_job(&self, job: DownloadJob) {
        {
            let mut map = self.inner.jobs.write().await;
            map.insert(job.id.clone(), job.clone());
        }
        // Persistir siempre en la creación — el throttle no aplica
        // porque es la primera vez que vemos el job.
        self.persist(&job, true).await;
        let _ = self.inner.app.emit(EVENT_JOB_UPDATE, &job);
    }

    /// (v8.3) Refleja en el job el estado de su ventana de hoster (ver
    /// `HosterStatus`). Un job cancelado ya no cambia.
    pub async fn hoster_status(&self, id: &str, status: HosterStatus) {
        self.update(id, |j| {
            if j.phase == DownloadPhase::Cancelled {
                return;
            }
            match status {
                HosterStatus::Waiting(part) => {
                    j.phase = DownloadPhase::Resolving;
                    j.bytes_done = 0;
                    j.bytes_total = 0;
                    j.speed_bps = 0;
                    j.eta_seconds = None;
                    j.error = None;
                    j.message = Some(match part {
                        Some(p) => crate::tr!("manager.partWaiting", part = p),
                        None => crate::tr!("manager.waitingInWindow"),
                    });
                }
                // (25/10) Un aviso de bytes que llega justo tras pausar no la
                // despausa.
                HosterStatus::Progress { done, total, .. } if j.phase == DownloadPhase::Paused => {
                    j.bytes_done = done;
                    j.bytes_total = total;
                }
                HosterStatus::Progress { done, total, bps, part } => {
                    j.phase = DownloadPhase::Downloading;
                    j.bytes_done = done;
                    j.bytes_total = total;
                    j.speed_bps = bps;
                    j.eta_seconds = (total > done && bps > 0).then(|| (total - done) / bps);
                    j.message = part;
                }
                HosterStatus::Downloaded => {
                    j.phase = DownloadPhase::Completed;
                    if j.bytes_total > 0 {
                        j.bytes_done = j.bytes_total;
                    }
                    j.speed_bps = 0;
                    j.eta_seconds = None;
                    j.message = Some(crate::tr!("manager.downloadedProcessing"));
                }
                HosterStatus::Failed(msg) => {
                    j.phase = DownloadPhase::Error;
                    j.error = Some(msg);
                    j.message = None;
                }
                HosterStatus::Cancelled => {
                    j.phase = DownloadPhase::Cancelled;
                    j.error = None;
                    j.message = Some(crate::tr!("manager.cancelledByUser"));
                }
                HosterStatus::Processing => {
                    j.phase = DownloadPhase::Installing;
                    j.speed_bps = 0;
                    j.eta_seconds = None;
                    j.message = Some(crate::tr!("manager.processing"));
                }
                HosterStatus::Ready => {
                    j.phase = DownloadPhase::Installing;
                    j.message = Some(crate::tr!("manager.readyForYou"));
                }
                HosterStatus::Installed => {
                    j.phase = DownloadPhase::Completed;
                    j.message = Some(crate::tr!("manager.installedDone"));
                }
                HosterStatus::Saved => {
                    j.phase = DownloadPhase::Completed;
                    j.message = Some(crate::tr!("manager.savedDone"));
                }
            }
        })
        .await;
    }

    /// (v8.3) Job para una descarga que NO arrancó este gestor (la web de
    /// flightsim.to embebida / la ventana de liveries): así también se ve en
    /// el panel de Descargas, con su progreso, y se puede cancelar.
    pub async fn register_external(
        &self,
        id: String,
        title: String,
        source: String,
        method_name: String,
        url: String,
    ) {
        let job = DownloadJob {
            id,
            addon_id: String::new(),
            addon_title: title,
            source,
            method_kind: "direct".to_string(),
            method_name,
            url,
            phase: DownloadPhase::Downloading,
            bytes_total: 0,
            bytes_done: 0,
            speed_bps: 0,
            eta_seconds: None,
            message: None,
            error: None,
            install_path: None,
            created_at: chrono::Utc::now().to_rfc3339(),
            image_url: None,
            pausable: false,
            resumable: false,
            resume: None,
        };
        self.set_job(job).await;
    }

    async fn update<F>(&self, id: &str, mutate: F)
    where
        F: FnOnce(&mut DownloadJob),
    {
        self.update_with(id, false, mutate).await;
    }

    /// `update` que además persiste sin throttle si `force`.
    async fn update_with<F>(&self, id: &str, force: bool, mutate: F)
    where
        F: FnOnce(&mut DownloadJob),
    {
        // Capturamos la fase previa para decidir si la persistencia
        // a DB es "cambio de fase" (siempre) o "sólo progreso"
        // (throttled).
        let (snapshot, phase_changed) = {
            let mut map = self.inner.jobs.write().await;
            match map.get_mut(id) {
                Some(j) => {
                    let prev = j.phase;
                    mutate(j);
                    j.refresh_flags();
                    let changed = j.phase != prev;
                    (Some(j.clone()), changed)
                }
                None => (None, false),
            }
        };
        if let Some(job) = snapshot {
            self.persist(&job, phase_changed || force).await;
            let _ = self.inner.app.emit(EVENT_JOB_UPDATE, &job);
        }
    }

    /// Persiste el job a DB respetando el throttle de progreso.
    /// `force=true` salta el throttle — usado en transiciones de
    /// fase (Queued→Resolving→Downloading→Completed/Error/etc.).
    async fn persist(&self, job: &DownloadJob, force: bool) {
        if !force {
            let mut map = self.inner.last_persisted.write().await;
            let now = Instant::now();
            if let Some(prev) = map.get(&job.id) {
                if now.duration_since(*prev).as_secs() < PERSIST_PROGRESS_THROTTLE_SECS {
                    return;
                }
            }
            map.insert(job.id.clone(), now);
        } else {
            self.inner
                .last_persisted
                .write()
                .await
                .insert(job.id.clone(), Instant::now());
        }
        if let Err(e) = repo::upsert_download_job(&self.inner.db, job).await {
            tracing::warn!("persist: upsert download_job {} falló: {e:#}", job.id);
        }
    }

    /// (v7) Mirror & Direct handler: abre la descarga en un navegador
    /// EMBEBIDO de SimFleet (no el del sistema). Ese navegador tiene ad
    /// blocker + bloqueo de popups, captura la descarga y emite
    /// `embedded-download://finished` → el instalador de drop la instala sola
    /// en la Community correcta y borra el temporal.
    ///
    /// El `DownloadJob` sólo refleja "abierta": no podemos seguir el progreso
    /// dentro del WebView (WebView2 no expone bytes), y la instalación la
    /// reporta el propio instalador de drop en la ventana principal. Por eso
    /// marcamos `Completed` con un mensaje claro en cuanto la ventana abre —
    /// igual que el viejo flujo, pero ahora todo ocurre dentro de la app.
    async fn run_embedded_download(
        &self,
        id: &str,
        url: String,
        parts: Vec<String>,
        password: Option<String>,
    ) {
        self.update(id, |j| {
            j.phase = DownloadPhase::Downloading;
            j.message = Some(if parts.is_empty() {
                crate::tr!("manager.openingDownload")
            } else {
                crate::tr!("manager.openingDownloadParts", n = parts.len() + 1)
            });
        })
        .await;

        let win_label = format!("hoster-dl-{id}");
        let title = crate::tr!("manager.hosterWindowTitle");
        // Idioma de la app para los textos del overlay.
        let lang: String = sqlx::query_as::<_, (String,)>(
            "SELECT value FROM settings WHERE key='pref_language'",
        )
        .fetch_optional(&self.inner.db)
        .await
        .ok()
        .flatten()
        .map(|r| r.0)
        .unwrap_or_default();
        let result = crate::commands::livery::open_hoster_download(
            &self.inner.app,
            win_label,
            url,
            title,
            &lang,
            parts,
            password,
        );

        match result {
            // (v8.3) La ventana está abierta esperando al usuario; el progreso
            // y el final los va contando la ventana (`hoster_status`).
            Ok(()) => self.hoster_status(id, HosterStatus::Waiting(None)).await,
            Err(e) => {
                self.update(id, |j| {
                    j.phase = DownloadPhase::Error;
                    j.error = Some(crate::tr!("manager.openDownloadFailed", e = e));
                })
                .await;
            }
        }
    }

    /// Torrent + auto-install pipeline:
    /// 1. Hand the magnet to `TorrentEngine` and stream progress into
    ///    the job state (UI re-renders reactively via emitted events).
    /// 2. On completion, look for the primary archive in the output
    ///    folder and run the installer in a blocking task (it does zip
    ///    extraction + file copy, both CPU/IO-bound).
    /// 3. Always wipe the torrent output folder so the user's AppData
    ///    doesn't grow forever — fixes the legacy "dangling %TEMP%" bug
    ///    for this code path too.
    async fn run_torrent(&self, id: &str, url: String, cancel: CancellationToken) {
        // Espera el slot global de torrents — concurrencia 1.
        // Mientras esperamos, mostramos al usuario que el job está
        // en cola con un mensaje explícito. Si el job se cancela
        // mientras espera el permit, salimos sin tocar nada.
        self.update(id, |j| {
            j.phase = DownloadPhase::Queued;
            j.message = Some(crate::tr!("manager.queuedWaiting"));
        })
        .await;

        let permit = tokio::select! {
            biased;
            _ = cancel.cancelled() => {
                self.update(id, |j| {
                    j.phase = DownloadPhase::Cancelled;
                    j.message = Some(crate::tr!("manager.cancelledBeforeStart"));
                })
                .await;
                return;
            }
            permit = self.inner.torrent_slot.acquire() => permit,
        };
        // El permit se mantiene vivo mientras `_permit` esté en
        // scope. Al salir de la función (return o panic) se libera y
        // el siguiente job en cola arranca automáticamente.
        let _permit = match permit {
            Ok(p) => p,
            Err(e) => {
                tracing::error!("torrent_slot cerrado: {e}");
                self.update(id, |j| {
                    j.phase = DownloadPhase::Error;
                    j.error = Some(crate::tr!("manager.mgrUnavailable"));
                })
                .await;
                return;
            }
        };

        self.update(id, |j| {
            j.phase = DownloadPhase::Resolving;
            // SceneryAddons gates this behind a ~10 s "Read First"
            // countdown — set the message up-front so the UI doesn't
            // look like it's stuck for those 10 seconds.
            j.message = Some(crate::tr!("manager.resolvingMagnet"));
        })
        .await;

        // Both SceneryAddons and Simplaza hand us an HTTPS redirector URL
        // (`.../get.php?hoster=torrent&…`) that renders an HTML page with
        // the real magnet inside it. Only actual `magnet:?` URIs can be
        // passed straight to librqbit; anything else needs scraping first.
        let magnet = if url.starts_with("magnet:") {
            url
        } else {
            match resolve_magnet_from_url(&url, cancel.clone()).await {
                Ok(m) => m,
                Err(e) => {
                    let cancelled = cancel.is_cancelled();
                    self.update(id, |j| {
                        if cancelled {
                            j.phase = DownloadPhase::Cancelled;
                            j.message = Some(crate::tr!("manager.cancelledByUser"));
                        } else {
                            j.phase = DownloadPhase::Error;
                            j.error =
                                Some(crate::tr!("manager.magnetResolveFailed", e = format!("{e:#}")));
                        }
                    })
                    .await;
                    return;
                }
            }
        };

        // Resolution succeeded — reflect the next stage in the UI.
        self.update(id, |j| {
            j.message = Some(crate::tr!("manager.connectingPeers"));
        })
        .await;

        let subfolder = format!("job-{id}");
        let engine = self.inner.torrent_engine.clone();

        // Download phase. The engine pushes progress snapshots on each
        // poll; we translate them into job-state updates inline so the
        // UI sees live bytes/speed.
        let download_result = {
            let this = self.clone();
            let id_for_progress = id.to_string();
            let first_byte_seen = std::sync::atomic::AtomicBool::new(false);
            engine
                .download_magnet(
                    id,
                    &magnet,
                    &subfolder,
                    cancel.clone(),
                    move |p: TorrentProgress| {
                        let id = id_for_progress.clone();
                        let this = this.clone();
                        // Promote to `Downloading` on the first progress tick
                        // with real bytes — keeps "Resolving…" visible until
                        // metadata + first peer are actually live.
                        let promote = p.bytes_done > 0
                            && !first_byte_seen.swap(true, std::sync::atomic::Ordering::Relaxed);
                        // The closure is sync, but `update` is async — spawn
                        // to avoid blocking the poll loop on state locks.
                        tauri::async_runtime::spawn(async move {
                            this.update(&id, |j| {
                                if promote || j.phase == DownloadPhase::Resolving {
                                    j.phase = DownloadPhase::Downloading;
                                    j.message = None;
                                }
                                // Don't stomp a user-initiated pause with
                                // a stale progress tick — librqbit keeps
                                // polling stats while paused. Byte counts
                                // still update so the UI shows how far we
                                // got before the pause.
                                if j.phase != DownloadPhase::Paused {
                                    j.bytes_done = p.bytes_done;
                                    j.bytes_total = p.bytes_total;
                                    j.speed_bps = p.speed_bps;
                                    j.eta_seconds = p.eta_seconds;
                                } else {
                                    // While paused, only update totals
                                    // (if they just became known) and
                                    // zero speed/eta to match reality.
                                    if j.bytes_total == 0 {
                                        j.bytes_total = p.bytes_total;
                                    }
                                    j.speed_bps = 0;
                                    j.eta_seconds = None;
                                }
                            })
                            .await;
                        });
                    },
                )
                .await
        };

        // Check cancellation explicitly: the engine also returns
        // "Cancelled by user" via Err, but keeping the phase correct
        // here means we don't briefly flash "Error" before reconciling.
        let cancelled = cancel.is_cancelled();

        let torrent_result = match download_result {
            Ok(res) => res,
            Err(e) => {
                engine.cleanup_job_folder(&subfolder);
                self.update(id, |j| {
                    if cancelled {
                        j.phase = DownloadPhase::Cancelled;
                        j.message = Some(crate::tr!("manager.cancelledByUser"));
                    } else {
                        j.phase = DownloadPhase::Error;
                        j.error = Some(format!("{e:#}"));
                    }
                })
                .await;
                return;
            }
        };

        // Install phase.
        self.update(id, |j| {
            j.phase = DownloadPhase::Installing;
            j.message = Some(crate::tr!("manager.extracting"));
        })
        .await;

        let Some(archive) = torrent_result.primary_archive.clone() else {
            engine.cleanup_job_folder(&subfolder);
            self.update(id, |j| {
                j.phase = DownloadPhase::Error;
                j.error = Some(crate::tr!("manager.noArchiveInTorrent"));
            })
            .await;
            return;
        };

        // Community folder is required for install. Surface a clear
        // error (not a panic) if detection fails.
        let community_path = match community::detect_community_folder() {
            Ok(Some(info)) => {
                if !info.exists {
                    // Best-effort create so a freshly installed sim
                    // without addons still works.
                    let _ = std::fs::create_dir_all(&info.path);
                }
                std::path::PathBuf::from(&info.path)
            }
            _ => {
                engine.cleanup_job_folder(&subfolder);
                self.update(id, |j| {
                    j.phase = DownloadPhase::Error;
                    j.error = Some(crate::tr!("manager.communityNotDetected"));
                })
                .await;
                return;
            }
        };

        // `install_archive` is synchronous (fs IO + zip extraction).
        // `spawn_blocking` keeps it off the async reactor.
        let archive_for_task = archive.clone();
        let community_for_task = community_path.clone();
        let install_outcome = tokio::task::spawn_blocking(move || {
            install::install_archive(&archive_for_task, &community_for_task)
        })
        .await;

        // Clean up the torrent output folder regardless of install
        // outcome — the archive is either already copied into Community
        // or irrelevant on failure.
        engine.cleanup_job_folder(&subfolder);

        match install_outcome {
            Ok(Ok(res)) => {
                let primary = res.packages.first().cloned();

                // **Persistir ANTES de emitir `Completed`**.
                //
                // El frontend escucha el evento `download://update`. Cuando ve
                // phase=Completed, refresca `installed_addons` y `community_packages`
                // para mostrar el nuevo paquete inmediatamente. Si emitiéramos el
                // evento antes del INSERT, el refresh del frontend leería la tabla
                // antes de que la fila exista — y el usuario tendría que pulsar F5
                // para verla. Persistir primero garantiza que la query del
                // frontend encuentre lo que busca.
                if let Some(snapshot) = self.snapshot(id).await {
                    for pkg in &res.packages {
                        let rec = repo::InstallRecord {
                            id: uuid::Uuid::new_v4().to_string(),
                            addon_id: Some(snapshot.addon_id.clone()),
                            source: Some(snapshot.source.clone()),
                            title: snapshot.addon_title.clone(),
                            // Campos finos: en el flujo torrent no
                            // tenemos developer/version aquí; el row
                            // de `addons` con ese `addon_id` los tiene
                            // y se puede unir si hace falta. `name`
                            // sí lo conocemos — es el directorio del
                            // paquete dentro de Community.
                            name: Some(pkg.name.clone()),
                            developer: None,
                            version: None,
                            install_path: pkg.install_path.clone(),
                            size_bytes: Some(pkg.size_bytes as i64),
                        };
                        if let Err(e) = repo::record_install(&self.inner.db, &rec).await {
                            tracing::warn!(
                                "no se pudo registrar instalación en DB ({}): {e:#}",
                                pkg.name
                            );
                        }
                    }
                }

                // Ahora sí — emitimos el evento Completed con la cadena ya
                // visible en DB.
                self.update(id, |j| {
                    j.phase = DownloadPhase::Completed;
                    if res.packages.is_empty() && res.installer_payload.is_some() {
                        // (fix) El torrent traía un INSTALADOR (.exe/.msi), no un
                        // paquete auto-instalable. Antes reportaba "Se instalaron
                        // 0 paquetes" (parecía éxito) y el instalador se borraba
                        // con la limpieza. Ahora se avisa claro y se redirige al
                        // método Mirror/Direct, que SÍ maneja instaladores (los
                        // abre en SimFleet y te deja guardarlos/ejecutarlos).
                        j.message = Some(crate::tr!("manager.torrentIsInstaller"));
                    } else {
                        j.message = Some(crate::tr!(
                            "manager.installedPackages",
                            n = res.packages.len(),
                            mb = format!("{:.1}", (res.total_bytes as f64) / (1024.0 * 1024.0)),
                        ));
                    }
                    j.install_path = primary.map(|p| p.install_path);
                })
                .await;

                // (v6 #1) Cross-link 2020/2024: si el addon es doble-compat
                // ("MSFS 2020/2024") y el usuario tiene AMBAS versiones, emite
                // una oferta para enlazar el paquete a la otra Community vía
                // junction NTFS. El frontend muestra un modal post-instalación.
                if let Some(snapshot) = self.snapshot(id).await {
                    // La etiqueta de compatibilidad autoritativa es el campo
                    // `simulator` ("MSFS 2020/2024"); el addon_title es el
                    // nombre limpio. Caemos al título sólo si no se guardó.
                    let compat = self
                        .inner
                        .sim_compat
                        .read()
                        .await
                        .get(id)
                        .cloned()
                        .filter(|s| !s.trim().is_empty())
                        .unwrap_or_else(|| snapshot.addon_title.clone());
                    if let Some(offer) = crate::cross_link::build_offer(&compat, &res.packages) {
                        let _ = self.inner.app.emit("cross-link://offer", &offer);
                    }
                }
                // Liberamos el registro de compat de este job (ya consumido).
                self.inner.sim_compat.write().await.remove(id);

                // (v5.0.0) Motor de rendimiento: al DESCARGAR un aeropuerto
                // bajamos la nota "Optional Configuration" de su página en
                // SceneryAddons y escribimos `config/simfleet_perf.json`
                // (sólo si la nota existe). NUNCA renombra archivos: el
                // addon queda en su estado por defecto del creador. Flujo
                // invisible; registra SIEMPRE en logs el resultado.
                if let Some(snapshot) = self.snapshot(id).await {
                    let db = self.inner.db.clone();
                    let addon_id = snapshot.addon_id.clone();
                    let packages: Vec<(String, String)> = res
                        .packages
                        .iter()
                        .map(|p| (p.install_path.clone(), p.name.clone()))
                        .collect();
                    tokio::task::spawn(async move {
                        // Resolver la página + ICAO del addon de catálogo.
                        let page = repo::get_addon_page(&db, &addon_id)
                            .await
                            .ok()
                            .flatten();
                        let Some((page_url, icao)) = page else {
                            for (_, folder) in &packages {
                                tracing::info!(
                                    "perf: {folder} descargado sin página de catálogo — sin nota"
                                );
                            }
                            return;
                        };
                        // Bajar el HTML de la página (best-effort).
                        let http = reqwest::Client::builder()
                            .user_agent("SimFleet")
                            .build()
                            .unwrap_or_default();
                        let html = match http.get(&page_url).send().await {
                            Ok(r) => r.text().await.unwrap_or_default(),
                            Err(e) => {
                                tracing::warn!("perf: no se pudo bajar {page_url}: {e}");
                                String::new()
                            }
                        };
                        for (install_path, folder) in packages {
                            let dir = std::path::PathBuf::from(&install_path);
                            let icao2 = icao.clone();
                            let html2 = html.clone();
                            let folder2 = folder.clone();
                            let _ = tokio::task::spawn_blocking(move || {
                                if html2.is_empty() {
                                    crate::perf_config::generate_on_install(
                                        &dir, &folder2, icao2, None,
                                    );
                                } else {
                                    match crate::perf_config::enrich_page_note_only(
                                        &dir, &folder2, icao2, &html2,
                                    ) {
                                        Some(cfg) => tracing::info!(
                                            "perf: nota OK al descargar {folder2} → {} opción(es)",
                                            cfg.options.len()
                                        ),
                                        None => tracing::info!(
                                            "perf: {folder2} sin 'Optional Configuration' en su página"
                                        ),
                                    }
                                }
                            })
                            .await;
                        }
                    });
                }
            }
            Ok(Err(e)) => {
                self.update(id, |j| {
                    j.phase = DownloadPhase::Error;
                    j.error = Some(crate::tr!("manager.installFailed", e = format!("{e:#}")));
                })
                .await;
            }
            Err(join_err) => {
                self.update(id, |j| {
                    j.phase = DownloadPhase::Error;
                    j.error = Some(crate::tr!("manager.installTaskFailed", err = join_err));
                })
                .await;
            }
        }
    }

    /// Devuelve una copia del job actual sin tomar prestada la
    /// estructura — útil cuando necesitamos leer varios campos para
    /// construir un registro de DB.
    async fn snapshot(&self, id: &str) -> Option<DownloadJob> {
        let map = self.inner.jobs.read().await;
        map.get(id).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// (25/10) La purga de arranque respeta lo ya bajado de una descarga
    /// pendiente y su carpeta, y borra todo lo demás.
    #[test]
    fn the_startup_purge_keeps_pending_partial_files() {
        let base = std::env::temp_dir().join(format!("simfleet-purge-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let job_dir = base.join("hoster-dl-abc");
        std::fs::create_dir_all(&job_dir).unwrap();
        std::fs::create_dir_all(base.join("hoster-dl-old")).unwrap();
        let keep = job_dir.join("pmdg-aircraft-77f.rar");
        std::fs::write(&keep, b"partial").unwrap();
        std::fs::write(job_dir.join("leftover.tmp"), b"x").unwrap();
        std::fs::write(base.join("hoster-dl-old").join("a.zip"), b"x").unwrap();
        std::fs::write(base.join("loose.zip"), b"x").unwrap();
        purge_except(&base, std::slice::from_ref(&keep));
        assert!(keep.exists());
        assert!(!job_dir.join("leftover.tmp").exists());
        assert!(!base.join("hoster-dl-old").exists() && !base.join("loose.zip").exists());
        let _ = std::fs::remove_dir_all(&base);
    }
}
