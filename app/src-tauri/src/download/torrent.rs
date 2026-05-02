//! Torrent download engine built on `librqbit`.
//!
//! Responsibilities of this module:
//! * Lazy-init a single shared `Session` (DHT bootstrap + port bind are
//!   expensive; doing it per job wastes time and UDP ports).
//! * Provide an ergonomic `download_magnet()` API that emits progress
//!   snapshots on a callback and honours a `CancellationToken`.
//! * Always release file handles before the caller runs the installer —
//!   on Windows the installer cannot read/move files while librqbit
//!   still has them mmapped, which caused sharing-violation failures
//!   during early development.
//!
//! Out-of-scope (for now): DHT persistence, partial file selection.
//! Easy to add later on top of this.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context};
use librqbit::{AddTorrent, AddTorrentOptions, ManagedTorrent, Session};

/// Re-alias locally — librqbit defines this as `Arc<ManagedTorrent>`
/// but doesn't re-export the typedef at the crate root.
type ManagedTorrentHandle = Arc<ManagedTorrent>;
use tokio::sync::{OnceCell, RwLock};
use tokio_util::sync::CancellationToken;

/// Archive extensions we'll hand off to the installer. Lowercase, no dot.
const ARCHIVE_EXTS: &[&str] = &["zip", "rar", "7z"];

/// How long to wait between magnet-resolution retries. Matches the
/// legacy .NET client; both SceneryAddons and Simplaza sometimes take
/// a few seconds to prepare the torrent entry on their side.
const RESOLVE_RETRY_DELAY: Duration = Duration::from_secs(2);
/// SceneryAddons' `dl.sceneryaddons.org/get.php` endpoint shows a
/// "Read First" interstitial with a 9-second countdown before the
/// submit button is enabled. Submitting again with the session cookie
/// after ≥9s returns the actual magnet page. Wait 12 s for safety
/// margin against clock skew / slow origin.
const RESOLVE_READFIRST_DELAY: Duration = Duration::from_secs(12);
const RESOLVE_MAX_ATTEMPTS: u32 = 10;
/// Under this length the response is almost certainly an error page
/// or a still-loading placeholder — legacy used 200 bytes as the floor.
const RESOLVE_MIN_BODY: usize = 200;

/// Outcome of a single fetch attempt inside [`resolve_magnet_from_url`].
/// Split out so the retry loop can pick the right back-off — a generic
/// transient error deserves a quick 2 s retry, but the SceneryAddons
/// "Read First" interstitial *requires* a ≥9 s wait server-side.
enum FetchOutcome {
    /// Resolved magnet URI, caller should return it.
    Magnet(String),
    /// SceneryAddons' Read-First interstitial. We already accepted the
    /// session cookie; the next retry just needs to wait out the
    /// countdown and re-submit with the same cookie jar.
    ReadFirstGate,
    /// Anything else — transient server error, short body, missing
    /// magnet in the HTML, etc. Retries quickly.
    Transient(anyhow::Error),
}

/// Resolve a magnet URI from a hoster "get.php" URL.
///
/// SceneryAddons and Simplaza back their "Torrent Download" buttons
/// with very different redirector shapes:
///
///   * **SceneryAddons** (`dl.sceneryaddons.org/get.php`): first GET
///     returns a 200 OK HTML "Read First" page with a 9-second
///     countdown and sets a `scad-readfirst-timestamp` cookie. A
///     second GET of the same URL, with the cookie and ≥9 s later,
///     returns the magnet page.
///   * **Simplaza** (`simplaza.org/...`): a single 200 OK HTML page
///     with the magnet embedded in an `<a href="magnet:?…">` anchor.
///
/// We handle both with one retry loop. Cookies persist across attempts
/// via reqwest's cookie jar (this is what the legacy .NET client got
/// for free via `HttpClientHandler.UseCookies = true`). When we
/// recognise the Read-First gate, we sleep 12 s before the next retry
/// instead of the usual 2 s — otherwise we'd churn attempts inside
/// the 9 s window and run out of retries before the gate opens.
///
/// Honours the `CancellationToken` at every step so the user can
/// back out of a stubborn URL without waiting for the full retry
/// window.
pub async fn resolve_magnet_from_url(
    url: &str,
    cancel: CancellationToken,
) -> anyhow::Result<String> {
    // Build a fresh client per invocation so the cookie jar is scoped
    // to this one resolution. Cookie persistence is the critical bit
    // for SceneryAddons — without it every attempt is treated as a
    // first visit and we never cross the Read-First gate.
    //
    // Disable auto-redirect: reqwest can't follow a `magnet:?` Location
    // anyway, and if a site ever moves to that shape we want to catch
    // it in the response we have, not lose it inside reqwest's
    // redirect handler. A browser-ish UA avoids naive bot-blockers.
    let client = reqwest::Client::builder()
        .user_agent(
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
             (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36",
        )
        .timeout(Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::none())
        .cookie_store(true)
        .build()
        .context("building magnet-resolver HTTP client")?;

    let mut last_err: Option<anyhow::Error> = None;

    for attempt in 1..=RESOLVE_MAX_ATTEMPTS {
        if cancel.is_cancelled() {
            anyhow::bail!("Cancelled by user");
        }

        let outcome = tokio::select! {
            _ = cancel.cancelled() => {
                anyhow::bail!("Cancelled by user");
            }
            res = fetch_magnet_once(&client, url) => res,
        };

        let delay = match outcome {
            FetchOutcome::Magnet(m) => {
                tracing::info!(
                    "torrent: resolved magnet on attempt {attempt}/{}",
                    RESOLVE_MAX_ATTEMPTS
                );
                return Ok(m);
            }
            FetchOutcome::ReadFirstGate => {
                tracing::info!(
                    "torrent: attempt {attempt}/{} hit SceneryAddons Read-First gate; \
                     waiting {}s before retry",
                    RESOLVE_MAX_ATTEMPTS,
                    RESOLVE_READFIRST_DELAY.as_secs(),
                );
                last_err = Some(anyhow!(
                    "SceneryAddons Read-First gate (waiting for {}s countdown)",
                    RESOLVE_READFIRST_DELAY.as_secs()
                ));
                RESOLVE_READFIRST_DELAY
            }
            FetchOutcome::Transient(e) => {
                tracing::warn!(
                    "torrent: magnet resolve attempt {attempt}/{} failed: {e:#}",
                    RESOLVE_MAX_ATTEMPTS
                );
                last_err = Some(e);
                RESOLVE_RETRY_DELAY
            }
        };

        // Don't sleep after the final attempt.
        if attempt < RESOLVE_MAX_ATTEMPTS {
            tokio::select! {
                _ = cancel.cancelled() => {
                    anyhow::bail!("Cancelled by user");
                }
                _ = tokio::time::sleep(delay) => {}
            }
        }
    }

    Err(last_err.unwrap_or_else(|| {
        anyhow!("magnet resolver exhausted {RESOLVE_MAX_ATTEMPTS} attempts")
    }))
}

/// Single pass: GET the URL and classify the response as a resolved
/// magnet, a recognised "wait then retry" gate, or a transient error.
/// The retry loop uses the classification to pick the right back-off.
async fn fetch_magnet_once(client: &reqwest::Client, url: &str) -> FetchOutcome {
    match fetch_magnet_inner(client, url).await {
        Ok(outcome) => outcome,
        Err(e) => FetchOutcome::Transient(e),
    }
}

/// The `Result<FetchOutcome>` shape here is internal: any error we
/// return bubbles out of `fetch_magnet_once` as [`FetchOutcome::Transient`].
/// Intentional gates (Read-First) are returned as `Ok(ReadFirstGate)`
/// so we don't conflate "server said wait" with "something broke".
async fn fetch_magnet_inner(
    client: &reqwest::Client,
    url: &str,
) -> anyhow::Result<FetchOutcome> {
    let resp = client.get(url).send().await.context("GET failed")?;
    let status = resp.status();

    // Case 1 — redirect with magnet as Location. Unlikely in current
    // SceneryAddons/Simplaza shape but cheap to support for future-proofing.
    if status.is_redirection() {
        if let Some(loc) = resp
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|v| v.to_str().ok())
        {
            let trimmed = loc.trim();
            if trimmed.len() >= 8 && trimmed[..8].eq_ignore_ascii_case("magnet:?") {
                let params = trimmed[8..].replace("&amp;", "&");
                return Ok(FetchOutcome::Magnet(format!("magnet:?{params}")));
            }
            anyhow::bail!("redirect Location is not a magnet URI: {trimmed}");
        }
        anyhow::bail!("redirect with no Location header (status {status})");
    }

    if !status.is_success() {
        anyhow::bail!("HTTP {status}");
    }

    let body = resp.text().await.context("reading body")?;
    if body.len() < RESOLVE_MIN_BODY {
        anyhow::bail!("response too short ({} bytes)", body.len());
    }

    // Case 2 — SceneryAddons Read-First interstitial. Recognised by
    // the page title; the reqwest client's cookie jar has already
    // captured the `scad-readfirst-timestamp` cookie that the next
    // request needs to satisfy the 9-second gate.
    if body.contains("SceneryAddons - Read First") {
        return Ok(FetchOutcome::ReadFirstGate);
    }

    // Case 3 — plain HTML with the magnet embedded somewhere
    // (Simplaza, or SceneryAddons after the gate is cleared).
    let lower = body.to_ascii_lowercase();
    let Some(idx) = lower.find("magnet:?") else {
        // Log a short body sample to aid debugging when a page shape
        // changes (new challenge, captcha, etc.) — 300 chars is
        // enough to spot the obvious without flooding the log.
        let sample: String = body.chars().take(300).collect();
        tracing::debug!("torrent: no magnet link; body head = {sample:?}");
        anyhow::bail!("no magnet link in HTML");
    };

    // Slice everything after `magnet:?` and stop at the first char
    // that would close an href/src attribute value or a bare URL.
    let rest = &body[idx + "magnet:?".len()..];
    let end = rest
        .find(|c: char| matches!(c, '"' | '\'' | '<' | '>' | ' ' | '\t' | '\n' | '\r'))
        .unwrap_or(rest.len());
    let params = rest[..end].trim();
    if params.is_empty() {
        anyhow::bail!("magnet link found but empty");
    }

    // HTML-escaped ampersands are extremely common when the magnet is
    // embedded inside an attribute. Decode so `&xt=…&tr=…` params
    // parse correctly downstream.
    let params = params.replace("&amp;", "&");

    // Normalise the scheme to lowercase so librqbit's strict parser
    // accepts it regardless of how the page formatted it.
    Ok(FetchOutcome::Magnet(format!("magnet:?{params}")))
}

/// Snapshot of torrent progress pushed to the caller roughly twice a
/// second. Values are raw so the UI layer owns any human-friendly
/// formatting.
#[derive(Debug, Clone, Copy)]
pub struct TorrentProgress {
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub speed_bps: u64,
    pub eta_seconds: Option<u64>,
}

/// Paths returned after a successful download, ready for the installer.
#[derive(Debug, Clone)]
pub struct TorrentResult {
    /// Root folder the torrent's contents were written into (ours to
    /// delete once the install pipeline is done).
    pub root_dir: PathBuf,
    /// The archive file the installer should extract, if one was found.
    /// `None` means the torrent payload isn't a scenery archive we can
    /// auto-install — the caller surfaces guidance to the user.
    pub primary_archive: Option<PathBuf>,
}

pub struct TorrentEngine {
    session: OnceCell<Arc<Session>>,
    default_output_dir: PathBuf,
    /// Active torrent handles, keyed by DownloadManager job id. The
    /// engine itself doesn't know or care about job ids — they're
    /// just an opaque token the caller uses to later call
    /// [`Self::pause`]/[`Self::unpause`] on the same torrent.
    /// Inserted on `download_magnet` entry, removed on exit (success
    /// or failure) so a pause call against a finished job is a clean
    /// error rather than acting on a dangling handle.
    handles: RwLock<HashMap<String, ManagedTorrentHandle>>,
}

impl TorrentEngine {
    pub fn new(default_output_dir: PathBuf) -> Self {
        Self {
            session: OnceCell::new(),
            default_output_dir,
            handles: RwLock::new(HashMap::new()),
        }
    }

    async fn ensure_session(&self) -> anyhow::Result<&Arc<Session>> {
        self.session
            .get_or_try_init(|| async {
                // Directory must exist before `Session::new` tries to write
                // inside it.
                std::fs::create_dir_all(&self.default_output_dir)
                    .context("could not create torrent data dir")?;
                tracing::info!(
                    "torrent: creating session at {}",
                    self.default_output_dir.display()
                );
                Session::new(self.default_output_dir.clone())
                    .await
                    .context("librqbit Session::new failed")
            })
            .await
    }

    /// Drive a magnet URL to completion inside `job_subfolder`.
    ///
    /// The `job_id` is an opaque caller-owned token used for
    /// pause/unpause dispatch; it's never inspected. The `on_progress`
    /// closure is polled ~every 500 ms on the calling task — callers
    /// that need to emit from a different task should push through a
    /// channel from inside the closure.
    pub async fn download_magnet(
        &self,
        job_id: &str,
        magnet: &str,
        job_subfolder: &str,
        cancel: CancellationToken,
        mut on_progress: impl FnMut(TorrentProgress),
    ) -> anyhow::Result<TorrentResult> {
        let session = self.ensure_session().await?.clone();

        let add_response = session
            .add_torrent(
                AddTorrent::from_url(magnet),
                Some(AddTorrentOptions {
                    // `overwrite: true` lets librqbit resume over pre-existing
                    // partial files — important when a job is retried after
                    // a cancellation or crash.
                    overwrite: true,
                    // Route every job into its own folder; simpler cleanup
                    // and avoids cross-job filename collisions.
                    sub_folder: Some(job_subfolder.to_string()),
                    ..Default::default()
                }),
            )
            .await
            .context("librqbit add_torrent failed")?;

        let handle = add_response
            .into_handle()
            .ok_or_else(|| anyhow!("add_torrent returned ListOnly; expected an active torrent"))?;
        let torrent_id = handle.id();

        // Register the handle so pause/unpause calls from the outside
        // world can find it. `Arc`-cloning a `ManagedTorrentHandle` is
        // cheap (it's just a typedef for `Arc<ManagedTorrent>`).
        self.handles
            .write()
            .await
            .insert(job_id.to_string(), handle.clone());

        tracing::info!(
            "torrent: added id={} folder={} job={}",
            torrent_id,
            job_subfolder,
            job_id,
        );

        // We need to poll stats AND watch for completion AND honour the
        // cancel token. Split it into two futures and let `select!`
        // resolve whichever finishes first.
        let poll_fut = async {
            loop {
                tokio::time::sleep(Duration::from_millis(500)).await;
                let stats = handle.stats();
                // `librqbit` reports speed in MiB/s (MiB = 1024*1024).
                // We expose raw bytes/sec to the UI so it can format
                // however it wants (and for numeric ETA math).
                let speed_bps = stats
                    .live
                    .as_ref()
                    .map(|l| (l.download_speed.mbps * 1024.0 * 1024.0) as u64)
                    .unwrap_or(0);
                // librqbit exposes `time_remaining` only through a tuple-
                // struct with a private field. Compute ETA ourselves from
                // speed + remaining bytes — also lets us skip the frame
                // when we've just started and speed is 0 (noisy "∞ ETA").
                let remaining = stats.total_bytes.saturating_sub(stats.progress_bytes);
                let eta_seconds = if speed_bps > 0 {
                    Some(remaining / speed_bps)
                } else {
                    None
                };
                on_progress(TorrentProgress {
                    bytes_done: stats.progress_bytes,
                    bytes_total: stats.total_bytes,
                    speed_bps,
                    eta_seconds,
                });
                if let Some(err) = &stats.error {
                    anyhow::bail!("torrent error: {err}");
                }
                // `wait_until_completed` is the authoritative signal;
                // `finished` is a belt-and-braces guard so the poll loop
                // can exit in case that future is held up by a state
                // transition.
                if stats.finished {
                    return Ok::<_, anyhow::Error>(());
                }
            }
        };

        let completion_fut = handle.wait_until_completed();

        let outcome = tokio::select! {
            res = poll_fut => res,
            res = completion_fut => res.map(|_| ()).map_err(anyhow::Error::from),
            _ = cancel.cancelled() => Err(anyhow!("Cancelled by user")),
        };

        // Drop the handle from the pause/resume registry before the
        // session deletes it — any racing pause call gets a clean
        // "no active torrent" error instead of operating on a zombie.
        self.handles.write().await.remove(job_id);

        // Always remove the torrent from the session before returning so
        // file handles are released. `delete_files: false` keeps the
        // downloaded bytes on disk for the installer to read.
        if let Err(e) = session.delete(torrent_id.into(), false).await {
            tracing::warn!("torrent: session.delete failed: {e:#}");
        }

        outcome?;

        let root_dir = self.default_output_dir.join(job_subfolder);
        let primary_archive = find_primary_archive(&root_dir);
        tracing::info!(
            "torrent: finished id={} archive={:?}",
            torrent_id,
            primary_archive
        );

        Ok(TorrentResult {
            root_dir,
            primary_archive,
        })
    }

    /// Best-effort cleanup for a job's download folder. Swallows errors —
    /// the caller should already have logged the root cause of the job
    /// failure; a dangling folder is a secondary concern.
    pub fn cleanup_job_folder(&self, job_subfolder: &str) {
        let dir = self.default_output_dir.join(job_subfolder);
        if dir.exists() {
            if let Err(e) = std::fs::remove_dir_all(&dir) {
                tracing::warn!("torrent: could not remove {}: {e}", dir.display());
            } else {
                tracing::info!("torrent: cleaned {}", dir.display());
            }
        }
    }

    /// Pause an in-flight torrent by job id. No-ops (errors) if the
    /// job hasn't yet reached the active download phase (e.g. still
    /// resolving the magnet) or has already finished.
    pub async fn pause(&self, job_id: &str) -> anyhow::Result<()> {
        let handle = self
            .handles
            .read()
            .await
            .get(job_id)
            .cloned()
            .ok_or_else(|| anyhow!("no active torrent for job {job_id}"))?;
        let session = self.ensure_session().await?;
        session.pause(&handle).await.context("librqbit pause failed")
    }

    /// Resume a previously-paused torrent. Mirrors [`Self::pause`].
    pub async fn unpause(&self, job_id: &str) -> anyhow::Result<()> {
        let handle = self
            .handles
            .read()
            .await
            .get(job_id)
            .cloned()
            .ok_or_else(|| anyhow!("no active torrent for job {job_id}"))?;
        let session = self.ensure_session().await?.clone();
        session
            .unpause(&handle)
            .await
            .context("librqbit unpause failed")
    }
}

/// Recolecta todos los archivos comprimidos bajo `root` y elige cuál
/// entregar al instalador.
///
/// La regla antigua ("el más grande") funcionaba con torrents de un solo
/// .zip, pero se rompe con RAR multi-volumen: en un set
/// `foo.part1.rar` (2 GB) + `foo.part2.rar` (1 GB) el más grande es
/// `part1`, lo cual da la casualidad de ser correcto — pero en
/// `foo.part01.rar` + `foo.part02.rar` del mismo tamaño el orden de
/// iteración del FS decide, y si cae en `part02` el extractor no puede
/// empezar porque los motores RAR quieren el primer volumen.
///
/// Reglas de selección (en orden):
///   1. Si hay ZIP, devolver el ZIP más grande (los .zip de la escena
///      MSFS son de un solo archivo — la ambigüedad no aplica).
///   2. Si hay 7z, preferir `foo.7z.001`; si no, el `foo.7z` más grande.
///   3. Si hay RAR, preferir el primer volumen:
///      · `foo.part1.rar` / `foo.part01.rar` / `foo.part001.rar` (nuevo estilo)
///      · o un `.rar` sin infijo `.partN.` (viejo estilo: `foo.rar` es
///        el primero, luego `foo.r00`, `foo.r01`…)
///      · fallback al más grande si no reconocemos el patrón.
fn find_primary_archive(root: &Path) -> Option<PathBuf> {
    // Recolectar todos los archivos con extensión comprimida; luego
    // aplicar las reglas encima del conjunto — más simple de razonar
    // que un "best-so-far" con tres heurísticas mezcladas.
    let mut archives: Vec<(PathBuf, u64)> = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(iter) = std::fs::read_dir(&dir) else { continue };
        for entry in iter.flatten() {
            let path = entry.path();
            let Ok(ft) = entry.file_type() else { continue };
            if ft.is_dir() {
                stack.push(path);
                continue;
            }
            let Some(ext) = path.extension().and_then(|s| s.to_str()) else { continue };
            if !ARCHIVE_EXTS.contains(&ext.to_ascii_lowercase().as_str()) {
                continue;
            }
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            archives.push((path, size));
        }
    }
    if archives.is_empty() {
        return None;
    }

    // Clasificar por extensión para poder aplicar reglas específicas.
    let ext_of = |p: &Path| {
        p.extension()
            .and_then(|s| s.to_str())
            .map(|s| s.to_ascii_lowercase())
            .unwrap_or_default()
    };

    // Regla 1: ZIP — el más grande gana.
    let zips: Vec<_> = archives
        .iter()
        .filter(|(p, _)| ext_of(p) == "zip")
        .collect();
    if !zips.is_empty() {
        return zips
            .iter()
            .max_by_key(|(_, s)| *s)
            .map(|(p, _)| p.clone());
    }

    // Regla 2: 7z — preferir `.7z.001` si existe, si no el `.7z` más grande.
    let sevens: Vec<_> = archives
        .iter()
        .filter(|(p, _)| ext_of(p) == "7z" || is_sevenz_volume(p))
        .collect();
    if !sevens.is_empty() {
        if let Some((first, _)) = sevens.iter().find(|(p, _)| is_sevenz_first_volume(p)) {
            return Some(first.clone());
        }
        return sevens
            .iter()
            .max_by_key(|(_, s)| *s)
            .map(|(p, _)| p.clone());
    }

    // Regla 3: RAR — el primer volumen es el correcto.
    let rars: Vec<_> = archives
        .iter()
        .filter(|(p, _)| ext_of(p) == "rar" || is_old_style_rar_volume(p))
        .collect();
    if !rars.is_empty() {
        // 3a: partN.rar con N == 1 (contando ceros a la izquierda).
        if let Some((first, _)) = rars
            .iter()
            .find(|(p, _)| is_new_style_rar_first_volume(p))
        {
            return Some(first.clone());
        }
        // 3b: .rar sin infijo `.partN.` (primer volumen viejo estilo).
        if let Some((plain, _)) = rars.iter().find(|(p, _)| {
            ext_of(p) == "rar" && !has_part_infix(p)
        }) {
            return Some(plain.clone());
        }
        // 3c: último recurso — el más grande.
        return rars
            .iter()
            .max_by_key(|(_, s)| *s)
            .map(|(p, _)| p.clone());
    }

    // No debería alcanzarse (archives no vacío implica alguna regla arriba),
    // pero por seguridad devolvemos el más grande.
    archives.into_iter().max_by_key(|(_, s)| *s).map(|(p, _)| p)
}

/// `true` si el nombre es del tipo `foo.part1.rar`, `foo.part01.rar`,
/// `foo.part001.rar`, etc. — es decir, el primer volumen del set
/// multi-parte al estilo nuevo de WinRAR.
fn is_new_style_rar_first_volume(path: &Path) -> bool {
    let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
        return false;
    };
    // `stem` aún termina en `.partN` porque `file_stem()` sólo recorta
    // la extensión final.
    let Some(part_idx) = stem.to_ascii_lowercase().rfind(".part") else {
        return false;
    };
    let digits = &stem[part_idx + ".part".len()..];
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    digits.parse::<u32>().map(|n| n == 1).unwrap_or(false)
}

/// `true` si el nombre contiene el infijo `.partN.` (cualquier N) — o
/// sea, pertenece a un set multi-parte estilo nuevo. Usado para
/// distinguir el "primer volumen viejo estilo" (plain `.rar` SIN
/// `.partN.`) del primer volumen nuevo estilo.
fn has_part_infix(path: &Path) -> bool {
    let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
        return false;
    };
    let Some(idx) = stem.to_ascii_lowercase().rfind(".part") else {
        return false;
    };
    let digits = &stem[idx + ".part".len()..];
    !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit())
}

/// Un volumen viejo estilo es `foo.r00`, `foo.r01`, etc.
fn is_old_style_rar_volume(path: &Path) -> bool {
    let Some(ext) = path.extension().and_then(|s| s.to_str()) else {
        return false;
    };
    let ext = ext.to_ascii_lowercase();
    ext.len() == 3
        && ext.starts_with('r')
        && ext[1..].chars().all(|c| c.is_ascii_digit())
}

/// `true` si el archivo es un volumen .7z multi-parte (.7z.001, .7z.002…).
fn is_sevenz_volume(path: &Path) -> bool {
    let Some(ext) = path.extension().and_then(|s| s.to_str()) else {
        return false;
    };
    // Para `foo.7z.001`, `extension()` devuelve `"001"` y `file_stem()`
    // devuelve `"foo.7z"` — así que el indicio es: extensión 100%
    // dígitos + stem termina en `.7z`.
    if !(ext.len() >= 2 && ext.chars().all(|c| c.is_ascii_digit())) {
        return false;
    }
    path.file_stem()
        .and_then(|s| s.to_str())
        .map(|s| s.to_ascii_lowercase().ends_with(".7z"))
        .unwrap_or(false)
}

/// `true` si es el primer volumen multi-parte: `.7z.001`.
fn is_sevenz_first_volume(path: &Path) -> bool {
    if !is_sevenz_volume(path) {
        return false;
    }
    path.extension()
        .and_then(|s| s.to_str())
        .map(|s| s.parse::<u32>().ok() == Some(1))
        .unwrap_or(false)
}
