use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use sqlx::SqlitePool;
use tauri::{AppHandle, Emitter};
use tauri_plugin_opener::OpenerExt;
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

/// Tauri event name the manager emits on every job state change.
pub const EVENT_JOB_UPDATE: &str = "download://update";

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
    ) -> anyhow::Result<DownloadJob> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();

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
        };

        self.set_job(job.clone()).await;

        match method.kind {
            DownloadKind::Mirror | DownloadKind::Direct => {
                // Synchronous: hand off to the system browser.
                self.run_open_url(&id, method.url).await;
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
                j.message = Some("Cancelado por el usuario".into());
            }
        })
        .await;
        Ok(())
    }

    /// Pause the active torrent for `id`. Only meaningful for torrent
    /// jobs that have reached the `Downloading` phase — for everything
    /// else this is a no-op that still returns Ok (the UI button is
    /// only shown in the downloading state, but racing a phase change
    /// shouldn't surface a scary error).
    pub async fn pause(&self, id: &str) -> anyhow::Result<()> {
        // Engine errors are informative but not fatal — log and keep
        // the UI state honest (e.g. if the handle vanished between
        // the user's click and our read, just drop back to cancelled).
        if let Err(e) = self.inner.torrent_engine.pause(id).await {
            tracing::warn!("pause({id}) failed: {e:#}");
            return Ok(());
        }
        self.update(id, |j| {
            if j.phase == DownloadPhase::Downloading {
                j.phase = DownloadPhase::Paused;
                j.message = Some("Pausado".into());
            }
        })
        .await;
        Ok(())
    }

    /// Resume a previously-paused torrent. Mirrors [`Self::pause`].
    pub async fn resume(&self, id: &str) -> anyhow::Result<()> {
        if let Err(e) = self.inner.torrent_engine.unpause(id).await {
            tracing::warn!("resume({id}) failed: {e:#}");
            return Ok(());
        }
        self.update(id, |j| {
            if j.phase == DownloadPhase::Paused {
                j.phase = DownloadPhase::Downloading;
                j.message = None;
            }
        })
        .await;
        Ok(())
    }

    pub async fn clear(&self, id: &str) {
        let removed = {
            let mut map = self.inner.jobs.write().await;
            map.remove(id)
        };
        if let Some(mut job) = removed {
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
        let count = jobs.len();
        tracing::info!("downloads: restaurando {} job(s) persistido(s)", count);

        for job in jobs {
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
                    self.update(&job.id, |j| {
                        j.phase = DownloadPhase::Error;
                        j.error = Some(
                            "La app se cerró mientras se abría el enlace en el navegador. \
                             Vuelve a pulsar el botón de descarga si lo necesitas."
                                .into(),
                        );
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

    async fn update<F>(&self, id: &str, mutate: F)
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
                    let changed = j.phase != prev;
                    (Some(j.clone()), changed)
                }
                None => (None, false),
            }
        };
        if let Some(job) = snapshot {
            self.persist(&job, phase_changed).await;
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

    /// Mirror & Direct handler: hand the URL to the user's default
    /// browser. Matches legacy `Process.Start(ShellExecute)` behaviour,
    /// but through the Tauri opener plugin (cross-platform, sandboxed).
    async fn run_open_url(&self, id: &str, url: String) {
        self.update(id, |j| {
            j.phase = DownloadPhase::Downloading;
            j.message = Some("Abriendo en tu navegador…".into());
        })
        .await;

        let result = self.inner.app.opener().open_url(url, None::<&str>);

        match result {
            Ok(_) => {
                self.update(id, |j| {
                    j.phase = DownloadPhase::Completed;
                    j.message = Some(
                        "Abierto en el navegador. Completa la descarga ahí y luego usa \
                         «Instalar desde archivo…» con el archivo descargado."
                            .into(),
                    );
                })
                .await;
            }
            Err(e) => {
                self.update(id, |j| {
                    j.phase = DownloadPhase::Error;
                    j.error = Some(format!("No se pudo abrir la URL: {}", e));
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
            j.message = Some("En cola — esperando a que termine la descarga anterior…".into());
        })
        .await;

        let permit = tokio::select! {
            biased;
            _ = cancel.cancelled() => {
                self.update(id, |j| {
                    j.phase = DownloadPhase::Cancelled;
                    j.message = Some("Cancelado por el usuario antes de empezar".into());
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
                    j.error = Some("El gestor de descargas no está disponible".into());
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
            j.message = Some(
                "Obteniendo enlace magnet (puede tardar ~10 s en SceneryAddons)…".into(),
            );
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
                            j.message = Some("Cancelado por el usuario".into());
                        } else {
                            j.phase = DownloadPhase::Error;
                            j.error = Some(format!(
                                "No se pudo obtener el enlace magnet del hoster: {e:#}"
                            ));
                        }
                    })
                    .await;
                    return;
                }
            }
        };

        // Resolution succeeded — reflect the next stage in the UI.
        self.update(id, |j| {
            j.message = Some("Conectando con peers…".into());
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
                        j.message = Some("Cancelado por el usuario".into());
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
            j.message = Some("Extrayendo archivo…".into());
        })
        .await;

        let Some(archive) = torrent_result.primary_archive.clone() else {
            engine.cleanup_job_folder(&subfolder);
            self.update(id, |j| {
                j.phase = DownloadPhase::Error;
                j.error = Some(
                    "El torrent no contiene un archivo .zip/.rar/.7z que podamos \
                     auto-instalar. Abre la carpeta de descargas para revisarlo."
                        .into(),
                );
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
                    j.error = Some(
                        "No se detectó la carpeta Community de MSFS 2020. Configúrala \
                         manualmente desde Ajustes (próximamente) e intenta de nuevo."
                            .into(),
                    );
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
                    j.message = Some(format!(
                        "Se instalaron {} paquete{} ({:.1} MB)",
                        res.packages.len(),
                        if res.packages.len() == 1 { "" } else { "s" },
                        (res.total_bytes as f64) / (1024.0 * 1024.0),
                    ));
                    j.install_path = primary.map(|p| p.install_path);
                })
                .await;
            }
            Ok(Err(e)) => {
                self.update(id, |j| {
                    j.phase = DownloadPhase::Error;
                    j.error = Some(format!("Falló la instalación: {e:#}"));
                })
                .await;
            }
            Err(join_err) => {
                self.update(id, |j| {
                    j.phase = DownloadPhase::Error;
                    j.error = Some(format!("La tarea de instalación falló: {join_err}"));
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
