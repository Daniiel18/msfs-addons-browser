//! (v6 #2b · v6.2.17) Grabación nativa de "Best Landings".
//!
//! Motor: **windows-capture 2.x** (Windows Graphics Capture + encoder
//! H.264 por HARDWARE vía Media Foundation) + captura WASAPI **loopback**
//! propia para el audio del sistema. Es el MISMO stack que usa
//! ScreenRecorderLib (la librería de LandingToast).
//!
//! Historia: el motor anterior (windows-record 0.1) tenía un replay buffer
//! en RAM que a 60fps acumulaba memoria sin control (hasta 20 GB → OOM →
//! crasheaba el sim). Con este motor el encoder **drena directo a disco**:
//! RAM plana a cualquier fps y calidad alta configurable.
//!
//! Semántica nueva: al armar (aproximación) se graba EN DISCO de forma
//! continua a un archivo temporal; al touchdown (+~12 s de rollout) el
//! archivo se finaliza y se renombra como clip → el clip cubre TODA la
//! aproximación final + el aterrizaje. Sin buffer en RAM que recortar.
//!
//! El objetivo de captura es una **ventana por título** (substring): MSFS para
//! los aterrizajes reales, y la ventana de la propia app ("SimFleet") para la
//! grabación de PRUEBA (testeable sin el sim). Fallback: monitor primario.
//!
//! Este módulo NO toca el `simconnect_watcher` (que procesa el vuelo en vivo).

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use sqlx::SqlitePool;

/// Nombre del manifiesto de clips dentro de la carpeta de salida.
const MANIFEST: &str = "simfleet_landings.json";

/// Substring del título de la ventana de MSFS (cubre 2020 y 2024).
pub const MSFS_WINDOW: &str = "Microsoft Flight Simulator";
/// Título de la ventana de la propia app — objetivo de la grabación de prueba.
pub const SELF_WINDOW: &str = "SimFleet";

// ─────────────────────────────────────────────────────────────────────────
// (v6.2.17) Motor de captura: WGC + Media Foundation (hardware) + WASAPI
// loopback. Ver el doc del módulo. Toda la sesión vive detrás de
// `CaptureSession` con tres operaciones: start / finish_and_save / abort.
// ─────────────────────────────────────────────────────────────────────────
#[cfg(windows)]
mod capture_engine {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{mpsc, Arc, Mutex};
    use std::thread::JoinHandle;
    use std::time::{Duration, Instant};

    use anyhow::anyhow;
    use windows_capture::capture::{CaptureControl, Context, GraphicsCaptureApiHandler};
    use windows_capture::graphics_capture_api::InternalCaptureControl;
    use windows_capture::encoder::{
        AudioSettingsBuilder, ContainerSettingsBuilder, VideoEncoder, VideoSettingsBuilder,
        VideoSettingsSubType,
    };
    use windows_capture::frame::Frame;
    use windows_capture::monitor::Monitor;
    use windows_capture::settings::{
        ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings,
        MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings,
    };
    use windows_capture::window::Window;

    type HandlerError = Box<dyn std::error::Error + Send + Sync>;

    /// Estado compartido entre el hilo de captura (frames), el hilo de audio
    /// (WASAPI loopback) y quien controla la sesión.
    struct Shared {
        fps: u32,
        bitrate: u32,
        temp_path: PathBuf,
        /// (sample_rate, canales) que el hilo de audio negoció con WASAPI —
        /// el encoder se configura con esto al crearse (primer frame).
        audio_fmt: Mutex<Option<(u32, u32)>>,
        /// El encoder se crea PEREZOSAMENTE en el primer frame (ahí sabemos
        /// las dimensiones reales de la ventana del sim).
        encoder: Mutex<Option<VideoEncoder>>,
        first_frame: AtomicBool,
        /// Señal de "finaliza el contenedor y para" — la procesa el handler
        /// en el siguiente frame (el encoder vive en su hilo).
        finish: AtomicBool,
        done_tx: Mutex<Option<mpsc::Sender<Result<(), String>>>>,
    }

    impl Shared {
        /// Finaliza el encoder (si existe) y notifica el resultado.
        fn finalize_encoder(&self) {
            if let Some(enc) = self.encoder.lock().unwrap().take() {
                let res = enc.finish().map_err(|e| format!("{e:?}"));
                if let Some(tx) = self.done_tx.lock().unwrap().take() {
                    let _ = tx.send(res);
                }
            }
        }
    }

    struct Handler {
        shared: Arc<Shared>,
    }

    impl GraphicsCaptureApiHandler for Handler {
        type Flags = Arc<Shared>;
        type Error = HandlerError;

        fn new(ctx: Context<Self::Flags>) -> Result<Self, Self::Error> {
            Ok(Self { shared: ctx.flags })
        }

        fn on_frame_arrived(
            &mut self,
            frame: &mut Frame,
            capture_control: InternalCaptureControl,
        ) -> Result<(), Self::Error> {
            let sh = &self.shared;
            if sh.finish.load(Ordering::Relaxed) {
                sh.finalize_encoder();
                capture_control.stop();
                return Ok(());
            }
            let mut guard = sh.encoder.lock().unwrap();
            if guard.is_none() {
                // Dimensiones PARES (requisito típico de H.264/NV12).
                let w = frame.width() & !1;
                let h = frame.height() & !1;
                let video = VideoSettingsBuilder::new(w, h)
                    .frame_rate(sh.fps)
                    .bitrate(sh.bitrate)
                    .sub_type(VideoSettingsSubType::H264);
                let audio = match *sh.audio_fmt.lock().unwrap() {
                    Some((rate, ch)) => AudioSettingsBuilder::new()
                        .sample_rate(rate)
                        .channel_count(ch)
                        .bit_per_sample(16),
                    None => AudioSettingsBuilder::new().disabled(true),
                };
                let enc = VideoEncoder::new(
                    video,
                    audio,
                    ContainerSettingsBuilder::new(),
                    &sh.temp_path,
                )?;
                *guard = Some(enc);
                sh.first_frame.store(true, Ordering::Release);
                tracing::info!(
                    target: "rec",
                    "encoder creado {w}x{h} @{}fps {} kbps (hardware MF)",
                    sh.fps,
                    sh.bitrate / 1000
                );
            }
            if let Some(enc) = guard.as_mut() {
                enc.send_frame(frame)?;
            }
            Ok(())
        }

        fn on_closed(&mut self) -> Result<(), Self::Error> {
            // La ventana capturada se cerró — finalizamos para no corromper
            // el mp4 (quien espere el Save recibirá el resultado).
            self.shared.finalize_encoder();
            Ok(())
        }
    }

    /// Sesión de grabación activa (captura + audio + encoder).
    pub struct CaptureSession {
        shared: Arc<Shared>,
        control: Option<CaptureControl<Handler, HandlerError>>,
        audio_stop: Arc<AtomicBool>,
        audio_join: Option<JoinHandle<()>>,
        done_rx: mpsc::Receiver<Result<(), String>>,
    }

    impl CaptureSession {
        /// Arranca la captura de la ventana cuyo título contiene
        /// `target_title` (fallback: monitor primario), grabando a
        /// `temp_path`. No bloquea.
        pub fn start(
            target_title: &str,
            fps: u32,
            bitrate: u32,
            temp_path: PathBuf,
            capture_audio: bool,
        ) -> anyhow::Result<Self> {
            let _ = std::fs::remove_file(&temp_path);
            let (done_tx, done_rx) = mpsc::channel();
            let shared = Arc::new(Shared {
                fps: fps.max(1),
                bitrate,
                temp_path,
                audio_fmt: Mutex::new(None),
                encoder: Mutex::new(None),
                first_frame: AtomicBool::new(false),
                finish: AtomicBool::new(false),
                done_tx: Mutex::new(Some(done_tx)),
            });

            // ── Audio (WASAPI loopback) ── se inicializa en SU hilo (COM) y
            // nos manda el formato negociado antes de arrancar la captura.
            let audio_stop = Arc::new(AtomicBool::new(false));
            let mut audio_join = None;
            if capture_audio {
                let (fmt_tx, fmt_rx) = mpsc::channel();
                let sh = shared.clone();
                let stop = audio_stop.clone();
                audio_join = Some(std::thread::spawn(move || {
                    audio_thread_main(sh, stop, fmt_tx);
                }));
                match fmt_rx.recv_timeout(Duration::from_secs(3)) {
                    Ok(Some(fmt)) => {
                        *shared.audio_fmt.lock().unwrap() = Some(fmt);
                    }
                    _ => tracing::warn!(
                        target: "rec",
                        "audio loopback no disponible — el clip irá sin audio"
                    ),
                }
            }

            // ── Captura ── throttle de frames al fps pedido (menos GPU/encoder).
            let interval = Duration::from_micros(1_000_000u64 / shared.fps as u64);
            let control = match Window::from_contains_name(target_title) {
                Ok(win) => Handler::start_free_threaded(Settings::new(
                    win,
                    CursorCaptureSettings::WithoutCursor,
                    DrawBorderSettings::WithoutBorder,
                    SecondaryWindowSettings::Default,
                    MinimumUpdateIntervalSettings::Custom(interval),
                    DirtyRegionSettings::Default,
                    ColorFormat::Bgra8,
                    shared.clone(),
                ))
                .map_err(|e| anyhow!("no se pudo iniciar la captura de «{target_title}»: {e:?}"))?,
                Err(_) => {
                    let mon = Monitor::primary()
                        .map_err(|e| anyhow!("sin ventana «{target_title}» ni monitor primario: {e:?}"))?;
                    tracing::warn!(
                        target: "rec",
                        "ventana «{target_title}» no encontrada — capturando el monitor primario"
                    );
                    Handler::start_free_threaded(Settings::new(
                        mon,
                        CursorCaptureSettings::WithoutCursor,
                        DrawBorderSettings::WithoutBorder,
                        SecondaryWindowSettings::Default,
                        MinimumUpdateIntervalSettings::Custom(interval),
                        DirtyRegionSettings::Default,
                        ColorFormat::Bgra8,
                        shared.clone(),
                    ))
                    .map_err(|e| anyhow!("no se pudo iniciar la captura del monitor: {e:?}"))?
                }
            };

            Ok(Self {
                shared,
                control: Some(control),
                audio_stop,
                audio_join,
                done_rx,
            })
        }

        /// Finaliza el contenedor y mueve el temporal a `final_path`.
        pub fn finish_and_save(mut self, final_path: &Path) -> anyhow::Result<()> {
            self.shared.finish.store(true, Ordering::Relaxed);
            // El handler finaliza en el siguiente frame; si no llegan frames
            // (ventana minimizada / cerrada), caemos al plan B tras timeout.
            let res = self.done_rx.recv_timeout(Duration::from_secs(10));
            self.audio_stop.store(true, Ordering::Relaxed);
            if let Some(j) = self.audio_join.take() {
                let _ = j.join();
            }
            if let Some(c) = self.control.take() {
                let _ = c.stop();
            }
            match res {
                Ok(Ok(())) => {}
                Ok(Err(e)) => return Err(anyhow!("encoder: {e}")),
                Err(_) => {
                    // Plan B: finalizamos directamente (ya no hay hilos tocando
                    // el encoder — audio parado y captura detenida).
                    match self.shared.encoder.lock().unwrap().take() {
                        Some(enc) => enc
                            .finish()
                            .map_err(|e| anyhow!("finish (plan B): {e:?}"))?,
                        None => {
                            return Err(anyhow!(
                                "la captura no recibió ningún frame (¿ventana visible?)"
                            ))
                        }
                    }
                }
            }
            // Margen para que Media Foundation suelte el archivo.
            std::thread::sleep(Duration::from_millis(300));
            let tmp = &self.shared.temp_path;
            if std::fs::rename(tmp, final_path).is_err() {
                std::fs::copy(tmp, final_path)
                    .map_err(|e| anyhow!("no se pudo mover el clip: {e}"))?;
                let _ = std::fs::remove_file(tmp);
            }
            Ok(())
        }

        /// Descarta la grabación (sin clip) y borra el temporal.
        pub fn abort(mut self) {
            self.shared.finish.store(true, Ordering::Relaxed);
            self.audio_stop.store(true, Ordering::Relaxed);
            if let Some(j) = self.audio_join.take() {
                let _ = j.join();
            }
            if let Some(c) = self.control.take() {
                let _ = c.stop();
            }
            drop(self.shared.encoder.lock().unwrap().take());
            std::thread::sleep(Duration::from_millis(200));
            let _ = std::fs::remove_file(&self.shared.temp_path);
        }
    }

    // ─────────────────────────────────────────────────────────────────────
    // WASAPI loopback (audio del sistema). Corre en su propio hilo con COM
    // MTA. Convierte el mix format del dispositivo (típicamente float32
    // 48 kHz N canales) a PCM16 ESTÉREO y lo empuja al encoder, rellenando
    // con silencio los huecos para mantener el A/V sync (el encoder usa un
    // reloj de audio monotónico por muestras).
    // ─────────────────────────────────────────────────────────────────────

    struct Loopback {
        client: windows::Win32::Media::Audio::IAudioClient,
        capture: windows::Win32::Media::Audio::IAudioCaptureClient,
        rate: u32,
        channels: usize,
        is_float: bool,
    }

    unsafe fn loopback_new() -> anyhow::Result<Loopback> {
        use windows::Win32::Media::Audio::{
            eConsole, eRender, IAudioCaptureClient, IAudioClient, IMMDeviceEnumerator,
            MMDeviceEnumerator, AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_LOOPBACK,
            WAVEFORMATEXTENSIBLE,
        };
        use windows::Win32::System::Com::{CoCreateInstance, CoTaskMemFree, CLSCTX_ALL};

        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
                .map_err(|e| anyhow!("MMDeviceEnumerator: {e}"))?;
        let device = enumerator
            .GetDefaultAudioEndpoint(eRender, eConsole)
            .map_err(|e| anyhow!("GetDefaultAudioEndpoint: {e}"))?;
        let client: IAudioClient = device
            .Activate(CLSCTX_ALL, None)
            .map_err(|e| anyhow!("IAudioClient::Activate: {e}"))?;
        let fmt_ptr = client
            .GetMixFormat()
            .map_err(|e| anyhow!("GetMixFormat: {e}"))?;
        let fmt = *fmt_ptr;
        let rate = fmt.nSamplesPerSec;
        let channels = fmt.nChannels as usize;
        let bits = fmt.wBitsPerSample;
        // wFormatTag: 1 = PCM · 3 = IEEE float · 0xFFFE = EXTENSIBLE (mirar
        // el SubFormat: data1 == 3 → float, == 1 → PCM).
        let tag = fmt.wFormatTag as u32;
        let is_float = match tag {
            3 => true,
            0xFFFE => {
                let ext = &*(fmt_ptr as *const WAVEFORMATEXTENSIBLE);
                ext.SubFormat.data1 == 3
            }
            _ => false,
        };
        if !(is_float && bits == 32) && !(!is_float && bits == 16) {
            CoTaskMemFree(Some(fmt_ptr as *const _));
            return Err(anyhow!(
                "formato de mezcla no soportado (tag={tag} bits={bits})"
            ));
        }
        // Buffer de 200 ms (unidades de 100 ns).
        client
            .Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                AUDCLNT_STREAMFLAGS_LOOPBACK,
                2_000_000,
                0,
                fmt_ptr,
                None,
            )
            .map_err(|e| anyhow!("IAudioClient::Initialize(loopback): {e}"))?;
        CoTaskMemFree(Some(fmt_ptr as *const _));
        let capture: IAudioCaptureClient = client
            .GetService()
            .map_err(|e| anyhow!("IAudioCaptureClient: {e}"))?;
        Ok(Loopback {
            client,
            capture,
            rate,
            channels,
            is_float,
        })
    }

    impl Loopback {
        /// Convierte `frames` muestras intercaladas del mix format a PCM16
        /// ESTÉREO little-endian (toma los 2 primeros canales).
        unsafe fn to_stereo_i16(&self, data: *const u8, frames: usize) -> Vec<u8> {
            let ch = self.channels.max(1);
            let mut out = Vec::with_capacity(frames * 4);
            if self.is_float {
                let src = std::slice::from_raw_parts(data as *const f32, frames * ch);
                for f in 0..frames {
                    let l = src[f * ch];
                    let r = if ch > 1 { src[f * ch + 1] } else { l };
                    out.extend_from_slice(
                        &((l.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes(),
                    );
                    out.extend_from_slice(
                        &((r.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes(),
                    );
                }
            } else {
                let src = std::slice::from_raw_parts(data as *const i16, frames * ch);
                for f in 0..frames {
                    let l = src[f * ch];
                    let r = if ch > 1 { src[f * ch + 1] } else { l };
                    out.extend_from_slice(&l.to_le_bytes());
                    out.extend_from_slice(&r.to_le_bytes());
                }
            }
            out
        }
    }

    fn push_audio(shared: &Shared, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        if let Some(enc) = shared.encoder.lock().unwrap().as_mut() {
            // El timestamp se ignora (reloj monotónico interno del encoder).
            let _ = enc.send_audio_buffer(bytes, 0);
        }
    }

    fn audio_thread_main(
        shared: Arc<Shared>,
        stop: Arc<AtomicBool>,
        fmt_tx: mpsc::Sender<Option<(u32, u32)>>,
    ) {
        use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        let lb = match unsafe { loopback_new() } {
            Ok(l) => l,
            Err(e) => {
                tracing::warn!(target: "rec", "WASAPI loopback falló: {e:#}");
                let _ = fmt_tx.send(None);
                unsafe { CoUninitialize() };
                return;
            }
        };
        // El encoder SIEMPRE recibe estéreo PCM16 al sample rate del mix.
        let _ = fmt_tx.send(Some((lb.rate, 2)));

        // Espera al primer frame de vídeo — el reloj de audio del encoder
        // empieza en la primera muestra, así que arrancar antes desincroniza.
        while !stop.load(Ordering::Relaxed) && !shared.first_frame.load(Ordering::Acquire) {
            std::thread::sleep(Duration::from_millis(10));
        }
        if stop.load(Ordering::Relaxed) {
            unsafe { CoUninitialize() };
            return;
        }
        if unsafe { lb.client.Start() }.is_err() {
            tracing::warn!(target: "rec", "IAudioClient::Start falló — clip sin audio");
            unsafe { CoUninitialize() };
            return;
        }

        let started = Instant::now();
        let mut sent: u64 = 0; // muestras (frames de audio) enviadas
        while !stop.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(10));
            // Drenar todos los paquetes pendientes.
            loop {
                let pkt = unsafe { lb.capture.GetNextPacketSize() }.unwrap_or(0);
                if pkt == 0 {
                    break;
                }
                let mut data: *mut u8 = std::ptr::null_mut();
                let mut frames: u32 = 0;
                let mut flags: u32 = 0;
                if unsafe {
                    lb.capture
                        .GetBuffer(&mut data, &mut frames, &mut flags, None, None)
                }
                .is_err()
                {
                    break;
                }
                if frames > 0 {
                    // AUDCLNT_BUFFERFLAGS_SILENT = 0x2 → el paquete es silencio.
                    let bytes = if flags & 0x2 != 0 {
                        vec![0u8; frames as usize * 4]
                    } else {
                        unsafe { lb.to_stereo_i16(data, frames as usize) }
                    };
                    push_audio(&shared, &bytes);
                    sent += frames as u64;
                }
                let _ = unsafe { lb.capture.ReleaseBuffer(frames) };
            }
            // Relleno de silencio si WASAPI no entregó nada (sin sesiones de
            // audio activas) — mantiene el A/V sync del reloj monotónico.
            let expected = (started.elapsed().as_secs_f64() * f64::from(lb.rate)) as u64;
            if expected > sent + u64::from(lb.rate) / 5 {
                let deficit = (expected - sent).min(u64::from(lb.rate));
                push_audio(&shared, &vec![0u8; deficit as usize * 4]);
                sent += deficit;
            }
        }
        unsafe {
            let _ = lb.client.Stop();
        }
        drop(lb);
        unsafe { CoUninitialize() };
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Config
// ─────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingConfig {
    pub enabled: bool,
    /// Posición del OSD: 0 = Arriba · 1 = Abajo (igual que LandingToast).
    pub osd_position: i64,
    pub output_path: String,
    // (v6.2.18) `clip_seconds` y `unlimited` ELIMINADOS: el motor nuevo graba
    // a disco toda la aproximación final (no hay buffer que recortar) y
    // `unlimited` jamás se llegó a usar en ningún motor.
    /// Capturar también el micrófono (no soportado por el motor nuevo).
    pub capture_microphone: bool,
    pub max_clips: i64,
    /// Mostrar el OSD de aterrizaje (toast con el FPM) al tocar pista.
    pub osd_enabled: bool,
    /// FPS de grabación (30 = rendimiento, 60 = fluido). Clamp 24..120.
    pub fps: i64,
}

async fn kv(pool: &SqlitePool, key: &str) -> Option<String> {
    sqlx::query_as::<_, (String,)>("SELECT value FROM settings WHERE key = ?1")
        .bind(key)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
        .map(|(v,)| v)
        .filter(|s| !s.trim().is_empty())
}

async fn kv_i64(pool: &SqlitePool, key: &str, default: i64) -> i64 {
    kv(pool, key)
        .await
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

/// Carpeta de salida por defecto (hereda de LandingToast si existe).
fn default_output_path(fallback_data: &Path) -> String {
    if let Some(p) = landingtoast_video_path() {
        return p;
    }
    if let Ok(profile) = std::env::var("USERPROFILE") {
        return PathBuf::from(profile)
            .join("Videos")
            .join("SimFleet Landings")
            .to_string_lossy()
            .into_owned();
    }
    fallback_data
        .join("landings")
        .to_string_lossy()
        .into_owned()
}

fn landingtoast_config() -> Option<serde_json::Value> {
    let appdata = std::env::var("APPDATA").ok()?;
    let cfg = PathBuf::from(appdata).join("LandingToast").join("config.json");
    serde_json::from_str(&std::fs::read_to_string(cfg).ok()?).ok()
}

fn landingtoast_video_path() -> Option<String> {
    landingtoast_config()?
        .get("VideoOutputPath")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .filter(|s| !s.trim().is_empty())
}

fn lt_i64(lt: &Option<serde_json::Value>, key: &str, fallback: i64) -> i64 {
    lt.as_ref()
        .and_then(|j| j.get(key))
        .and_then(|v| v.as_i64())
        .unwrap_or(fallback)
}

pub async fn load_config(pool: &SqlitePool, fallback_data: &Path) -> RecordingConfig {
    let lt = landingtoast_config();
    RecordingConfig {
        enabled: kv(pool, "rec_enabled")
            .await
            .map(|s| matches!(s.as_str(), "1" | "true" | "yes"))
            .unwrap_or(false),
        osd_position: match kv(pool, "rec_osd_position").await {
            Some(s) => s.parse().unwrap_or(0),
            None => lt_i64(&lt, "Position", 0),
        },
        output_path: kv(pool, "rec_output_path")
            .await
            .unwrap_or_else(|| default_output_path(fallback_data)),
        capture_microphone: kv(pool, "rec_microphone")
            .await
            .map(|s| matches!(s.as_str(), "1" | "true" | "yes"))
            .unwrap_or(false),
        max_clips: kv_i64(pool, "rec_max_clips", 20).await,
        osd_enabled: kv(pool, "rec_osd_enabled")
            .await
            .map(|s| matches!(s.as_str(), "1" | "true" | "yes"))
            .unwrap_or(true),
        fps: kv_i64(pool, "rec_fps", DEFAULT_FPS).await.clamp(24, 120),
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Motor de grabación
// ─────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineStatus {
    /// true si el motor de grabación está disponible (Windows).
    pub available: bool,
    pub engine: String,
}

pub fn engine_status() -> EngineStatus {
    EngineStatus {
        available: cfg!(windows),
        engine: "windows-record".into(),
    }
}

/// FPS de captura por defecto. 30 = más seguro (menos carga de captura/copia de
/// GPU que compite con MSFS en escenarios pesados). El usuario puede subir a 60.
const DEFAULT_FPS: i64 = 30;

/// (v6 #2b) Solo puede haber UNA grabación activa a la vez. Desktop Duplication
/// permite una sola duplicación por monitor — si el auto-disparo (replay buffer)
/// está grabando y se lanza la prueba, la 2ª captura se desconecta
/// ("Error receiving video sample: Disconnected"). Este flag las serializa.
static RECORDING_BUSY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Lock-file CROSS-PROCESO de la grabación. El `AtomicBool` de arriba solo
/// serializa dentro de UN proceso; si el usuario corre DOS instancias de
/// SimFleet (p.ej. dev:safe + la build instalada), cada una tiene su propio
/// flag y ambas intentan capturar MSFS → la 2ª falla al armar ("file in use")
/// o la 1ª guarda "No video frames" por contención de Desktop Duplication.
/// Un handle de archivo abierto en modo EXCLUSIVO (share_mode 0) actúa de
/// mutex entre procesos: mientras una instancia lo tiene, las demás no pueden
/// abrirlo. Es crash-safe: el SO cierra el handle al terminar el proceso, así
/// que un cierre abrupto no deja el lock colgado.
static RECORDING_LOCK_FILE: std::sync::Mutex<Option<std::fs::File>> =
    std::sync::Mutex::new(None);

/// Intenta tomar el lock entre procesos. `true` si lo conseguimos.
fn acquire_cross_process_lock() -> bool {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::fs::OpenOptionsExt;
        let path = std::env::temp_dir().join("simfleet_recorder.lock");
        match std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .share_mode(0) // 0 = sin compartir → exclusivo entre procesos
            .open(&path)
        {
            Ok(f) => {
                *RECORDING_LOCK_FILE.lock().unwrap() = Some(f);
                true
            }
            Err(_) => false, // otra instancia de SimFleet ya está grabando
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        true
    }
}

/// Suelta el lock entre procesos (cierra el handle).
fn release_cross_process_lock() {
    *RECORDING_LOCK_FILE.lock().unwrap() = None;
}

/// Intenta reservar la grabación. `false` si ya hay una en curso — en ESTE
/// proceso (flag) o en OTRA instancia de SimFleet (lock-file).
pub fn try_acquire_recording() -> bool {
    if RECORDING_BUSY
        .compare_exchange(
            false,
            true,
            std::sync::atomic::Ordering::SeqCst,
            std::sync::atomic::Ordering::SeqCst,
        )
        .is_err()
    {
        return false;
    }
    // Captura de MSFS = una sola por máquina. Si otra instancia la tiene, no
    // armamos (evita el conflicto de doble-instancia: "file in use" / 0 frames).
    if !acquire_cross_process_lock() {
        RECORDING_BUSY.store(false, std::sync::atomic::Ordering::SeqCst);
        return false;
    }
    true
}

/// Libera la reserva de grabación (flag + lock entre procesos).
pub fn release_recording() {
    release_cross_process_lock();
    RECORDING_BUSY.store(false, std::sync::atomic::Ordering::SeqCst);
}

/// Bitrate de vídeo según resolución y fps (~0.30 bpp). El default de la crate
/// (5 Mbps) deja el vídeo pixelado en movimiento; como la 0.1.0 sólo expone
/// bitrate fijo (CBR, sin VBR/Quality como ScreenRecorderLib de LandingToast),
/// compensamos con un bitrate ALTO para que las escenas con mucho movimiento
/// no pierdan detalle. Ej.: 1080p60 ≈ 37 Mbps, 1440p60 ≈ 66 Mbps.
fn compute_bitrate(width: u32, height: u32, fps: u32) -> u32 {
    // (v6.2.17) ~0.12 bits/píxel/frame — calidad alta para H.264 por hardware:
    // 1080p30 ≈ 8 Mbps · 1080p60 ≈ 15 Mbps · 1440p60 ≈ 26 Mbps · 4K60 → tope 45.
    // El valor anterior (0.30 bpp, mín 25 Mbps) era desproporcionado y ayudaba
    // a inflar la cola del encoder del motor viejo.
    let raw = (width as u64) * (height as u64) * (fps as u64) * 12 / 100;
    raw.clamp(8_000_000, 45_000_000) as u32
}

/// Resolución del monitor objetivo, para igualar la salida a la NATIVA y NO
/// reescalar (el reescalado emborrona). `prefer_current`=true usa el monitor
/// donde está la ventana de la app (para la prueba); false usa el primario
/// (mejor apuesta para dónde corre MSFS). Fallback 1920x1080.
#[cfg(windows)]
pub fn target_monitor_size(app: &tauri::AppHandle, prefer_current: bool) -> (u32, u32) {
    use tauri::Manager;
    let Some(win) = app.get_webview_window("main") else {
        return (1920, 1080);
    };
    let mon = if prefer_current {
        win.current_monitor().ok().flatten()
    } else {
        None
    }
    .or_else(|| win.primary_monitor().ok().flatten())
    .or_else(|| win.current_monitor().ok().flatten());
    mon.map(|m| (m.size().width, m.size().height))
        .filter(|(w, h)| *w > 0 && *h > 0)
        .unwrap_or((1920, 1080))
}

#[cfg(not(windows))]
pub fn target_monitor_size(_app: &tauri::AppHandle, _prefer_current: bool) -> (u32, u32) {
    (1920, 1080)
}

/// Graba `duration_s` s de la ventana cuyo título contiene `target_title`
/// (substring, case-insensitive), con audio del sistema. Bloqueante
/// (~duration_s) — llamar desde `spawn_blocking`.
///
/// (v6.2.17) Motor nuevo (WGC + MF hardware): `width`/`height` solo se usan
/// para calcular el bitrate — el tamaño real sale del primer frame. El
/// micrófono no está soportado por el motor nuevo (solo audio del sistema).
#[cfg(windows)]
pub fn record_window_clip(
    target_title: &str,
    out: &Path,
    duration_s: i64,
    capture_microphone: bool,
    width: u32,
    height: u32,
    fps: u32,
) -> anyhow::Result<()> {
    use std::time::Duration;

    if capture_microphone {
        tracing::warn!(
            target: "rec",
            "micrófono aún no soportado por el motor nuevo — se graba solo el audio del sistema"
        );
    }
    let temp = out.with_extension("part.mp4");
    let session = capture_engine::CaptureSession::start(
        target_title,
        fps,
        compute_bitrate(width, height, fps),
        temp,
        true,
    )?;

    std::thread::sleep(Duration::from_secs(duration_s.clamp(2, 600) as u64));

    session.finish_and_save(out)?;

    if !out.is_file() {
        anyhow::bail!("la grabación no produjo ningún archivo");
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn record_window_clip(
    _target_title: &str,
    _out: &Path,
    _duration_s: i64,
    _capture_microphone: bool,
    _width: u32,
    _height: u32,
    _fps: u32,
) -> anyhow::Result<()> {
    anyhow::bail!("la grabación solo está soportada en Windows")
}

// ─────────────────────────────────────────────────────────────────────────
// Librería de clips (manifiesto JSON en la carpeta de salida)
// ─────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LandingClip {
    pub id: String,
    pub path: String,
    pub recorded_at: String,
    pub fpm: Option<i64>,
    pub grade: Option<String>,
    pub airport_icao: Option<String>,
    pub airport_name: Option<String>,
    pub model: Option<String>,
    pub registration: Option<String>,
    pub favorite: bool,
    pub duration_s: i64,
    /// Clip de prueba (botón "grabar prueba"), no de un aterrizaje real.
    pub is_test: bool,
}

fn manifest_path(output_dir: &Path) -> PathBuf {
    output_dir.join(MANIFEST)
}

pub fn load_clips(output_dir: &Path) -> Vec<LandingClip> {
    let Ok(text) = std::fs::read_to_string(manifest_path(output_dir)) else {
        return Vec::new();
    };
    let mut clips: Vec<LandingClip> = serde_json::from_str(&text).unwrap_or_default();
    clips.retain(|c| Path::new(&c.path).is_file());
    clips
}

fn save_clips(output_dir: &Path, clips: &[LandingClip]) -> anyhow::Result<()> {
    std::fs::create_dir_all(output_dir)?;
    std::fs::write(manifest_path(output_dir), serde_json::to_string_pretty(clips)?)?;
    Ok(())
}

pub fn add_clip(output_dir: &Path, clip: LandingClip, max_clips: i64) -> anyhow::Result<()> {
    let mut clips = load_clips(output_dir);
    clips.push(clip);
    apply_retention(&mut clips, max_clips);
    save_clips(output_dir, &clips)
}

/// Conserva favoritos + clips de prueba; del resto, los de mejor FPM hasta
/// `max_clips`. Borra del disco lo descartado.
fn apply_retention(clips: &mut Vec<LandingClip>, max_clips: i64) {
    let max = max_clips.max(1) as usize;
    let mut prunable: Vec<usize> = clips
        .iter()
        .enumerate()
        .filter(|(_, c)| !c.favorite && !c.is_test)
        .map(|(i, _)| i)
        .collect();
    let keep_real =
        max.saturating_sub(clips.iter().filter(|c| c.favorite || c.is_test).count());
    if prunable.len() <= keep_real {
        return;
    }
    // Peor FPM (más negativo) primero = primeros en caer.
    prunable.sort_by(|&a, &b| {
        clips[a]
            .fpm
            .unwrap_or(i64::MIN)
            .cmp(&clips[b].fpm.unwrap_or(i64::MIN))
    });
    let remove_count = prunable.len().saturating_sub(keep_real);
    let remove_set: std::collections::HashSet<usize> =
        prunable.into_iter().take(remove_count).collect();
    let mut idx = 0;
    clips.retain(|c| {
        let keep = !remove_set.contains(&idx);
        if !keep {
            let _ = std::fs::remove_file(&c.path);
        }
        idx += 1;
        keep
    });
}

pub fn set_favorite(output_dir: &Path, id: &str, favorite: bool) -> anyhow::Result<()> {
    let mut clips = load_clips(output_dir);
    if let Some(c) = clips.iter_mut().find(|c| c.id == id) {
        c.favorite = favorite;
    }
    save_clips(output_dir, &clips)
}

pub fn delete_clip(output_dir: &Path, id: &str) -> anyhow::Result<()> {
    let mut clips = load_clips(output_dir);
    if let Some(pos) = clips.iter().position(|c| c.id == id) {
        let _ = std::fs::remove_file(&clips[pos].path);
        clips.remove(pos);
    }
    save_clips(output_dir, &clips)
}

#[allow(clippy::too_many_arguments)]
fn update_clip_meta(
    out_dir: &Path,
    id: &str,
    fpm: Option<i64>,
    grade: Option<String>,
    icao: Option<String>,
    name: Option<String>,
    model: Option<String>,
    reg: Option<String>,
) -> anyhow::Result<()> {
    let mut clips = load_clips(out_dir);
    if let Some(c) = clips.iter_mut().find(|c| c.id == id) {
        if c.fpm.is_none() {
            c.fpm = fpm;
        }
        if c.grade.is_none() {
            c.grade = grade;
        }
        if c.airport_icao.is_none() {
            c.airport_icao = icao;
        }
        if c.airport_name.is_none() {
            c.airport_name = name;
        }
        if c.model.is_none() {
            c.model = model;
        }
        if c.registration.is_none() {
            c.registration = reg;
        }
    }
    save_clips(out_dir, &clips)
}

// ─────────────────────────────────────────────────────────────────────────
// (v6 #2b) Auto-disparo: replay buffer armado en aproximación → al detectar el
// touchdown se guarda el clip. NO toca el watcher: una tarea async sondea el
// `FlightStatus` compartido (fase/on_ground/altitud) y manda comandos a un
// hilo dedicado que es dueño del `Recorder` (windows-record es !Send / COM).
// Opt-in: solo corre si la grabación está activada en Ajustes.
// ─────────────────────────────────────────────────────────────────────────

#[cfg(windows)]
enum RecCmd {
    Arm {
        output_path: String,
        width: u32,
        height: u32,
        fps: u32,
    },
    Save {
        path: String,
        reply: tokio::sync::oneshot::Sender<Result<(), String>>,
    },
    Stop,
}

/// Hilo dedicado dueño de la sesión de captura. Recibe comandos por un canal
/// sync — aislar la sesión aquí evita líos de `Send`/COM con tokio.
///
/// (v6.2.17) Con el motor nuevo NO hay replay buffer en RAM: `Arm` empieza a
/// grabar EN DISCO a un temporal; `Save` finaliza el contenedor y lo renombra
/// como clip (cubre toda la aproximación final); `Stop` descarta y borra.
#[cfg(windows)]
fn recorder_thread(rx: std::sync::mpsc::Receiver<RecCmd>) {
    let mut session: Option<capture_engine::CaptureSession> = None;
    while let Ok(cmd) = rx.recv() {
        match cmd {
            RecCmd::Arm {
                output_path,
                width,
                height,
                fps,
            } => {
                if session.is_some() {
                    continue;
                }
                // Solo armamos si no hay otra grabación (p.ej. una prueba).
                if !try_acquire_recording() {
                    tracing::warn!(target: "rec", "ya hay una grabación activa; no se arma la captura");
                    continue;
                }
                let base = Path::new(&output_path).join("simfleet_buffer.mp4");
                match capture_engine::CaptureSession::start(
                    MSFS_WINDOW,
                    fps,
                    compute_bitrate(width, height, fps),
                    base,
                    true,
                ) {
                    Ok(s) => {
                        tracing::info!(target: "rec", "captura armada (MSFS, {fps} fps, disco)");
                        session = Some(s);
                    }
                    Err(e) => {
                        release_recording();
                        tracing::warn!(target: "rec", "no se pudo armar la captura: {e:#}");
                    }
                }
            }
            RecCmd::Save { path, reply } => {
                let res = match session.take() {
                    Some(s) => {
                        let r = s
                            .finish_and_save(Path::new(&path))
                            .map_err(|e| format!("{e:#}"));
                        release_recording();
                        tracing::info!(target: "rec", "clip guardado: {path}");
                        r
                    }
                    None => Err("recorder no armado".to_string()),
                };
                let _ = reply.send(res);
            }
            RecCmd::Stop => {
                if let Some(s) = session.take() {
                    s.abort();
                    release_recording();
                    tracing::info!(target: "rec", "captura detenida (sin clip)");
                }
            }
        }
    }
}

#[cfg(windows)]
pub fn spawn_controller(pool: SqlitePool, app: tauri::AppHandle, data_dir: PathBuf) {
    let (tx, rx) = std::sync::mpsc::channel::<RecCmd>();
    let _ = std::thread::Builder::new()
        .name("landing-recorder".into())
        .spawn(move || recorder_thread(rx));
    tokio::spawn(controller_loop(pool, app, data_dir, tx));
}

#[cfg(windows)]
async fn controller_loop(
    pool: SqlitePool,
    app: tauri::AppHandle,
    data_dir: PathBuf,
    tx: std::sync::mpsc::Sender<RecCmd>,
) {
    use std::time::Duration;
    use tauri::Manager;

    let mut armed = false;
    let mut prev_on_ground = true;
    let mut saved = false;
    // (v6.2.14) Bug crítico de RAM: el fallback por altitud (`alt < 3000`) también
    // se cumple en el ASCENSO tras despegar, así que el grabador se armaba al
    // subir y corría TODO el vuelo (windows-record 0.1 filtra memoria → OOM →
    // crashea el sim). Dos defensas:
    //  · `reached_cruise`: solo permitimos el fallback por altitud DESPUÉS de
    //    haber estado alto (>6000ft) en este vuelo — así el climb-out nunca lo
    //    dispara, solo el descenso de vuelta.
    //  · `armed_at`: tope de tiempo armado; si lleva demasiado (holding largo o
    //    fase que no transiciona) lo paramos para acotar el leak nativo.
    let mut reached_cruise = false;
    let mut armed_at: Option<std::time::Instant> = None;
    const MAX_ARMED: Duration = Duration::from_secs(12 * 60);
    // (v6.2.16) Fix "save_replay falló: recorder no armado" (visto en los logs
    // EN CADA aterrizaje): `schedule_save` duerme ~12s tras el touchdown para
    // que el rollout entre en el buffer, pero la fase pasaba a taxi_in antes y
    // el Stop llegaba PRIMERO → el Save encontraba el recorder apagado y el
    // clip se perdía. Mientras haya un guardado pendiente NO se desarma.
    let mut save_pending_until: Option<std::time::Instant> = None;
    const SAVE_GRACE: Duration = Duration::from_secs(16);

    loop {
        tokio::time::sleep(Duration::from_secs(1)).await;

        let cfg = load_config(&pool, &data_dir).await;

        let status = match app.try_state::<crate::simconnect_watcher::SharedState>() {
            Some(s) => s.lock().await.status.clone(),
            None => continue,
        };
        // ¿Hay un guardado de clip en vuelo? (bloquea cualquier desarme)
        let save_pending =
            save_pending_until.map_or(false, |t| std::time::Instant::now() < t);
        if !save_pending {
            save_pending_until = None;
        }
        if !status.sim_running {
            if armed && !save_pending {
                let _ = tx.send(RecCmd::Stop);
                armed = false;
                armed_at = None;
                saved = false;
                crate::diagnostics::note_recorder_armed(false);
            }
            prev_on_ground = true;
            reached_cruise = false;
            continue;
        }

        let phase = status.phase_label.clone().unwrap_or_default();
        let on_ground = status.on_ground.unwrap_or(true);
        // Marca que el avión ya subió de verdad (este vuelo) — distingue el
        // descenso de vuelta del climb-out inicial. Se resetea al tocar tierra.
        if status.current_alt_ft.map_or(false, |a| a > 6_000) {
            reached_cruise = true;
        }
        if on_ground {
            reached_cruise = false;
        }
        // (v6.1 perf · v6.2.14) Armamos LO MÁS TARDE posible — solo en aproximación
        // real. La fase "approach"/"descent" del watcher es por ALTURA SOBRE EL
        // TERRENO. El fallback por altitud MSL (<3000ft) SOLO vale si ya estuvimos
        // en crucero (`reached_cruise`) y NO estamos en fase de ascenso — así el
        // paso por 3000ft tras despegar nunca arma el grabador.
        let descending_phase = matches!(phase.as_str(), "approach" | "descent");
        let climb_phase = matches!(phase.as_str(), "takeoff" | "climbing" | "cruise");
        let approaching = descending_phase
            || (!on_ground
                && reached_cruise
                && !climb_phase
                && status.current_alt_ft.map_or(false, |a| a < 3_000));

        // ── Grabación (solo si está activada) ──
        if cfg.enabled {
            // Armar el replay buffer al entrar en aproximación.
            if approaching && !on_ground && !armed {
                let (w, h) = target_monitor_size(&app, false);
                let _ = tx.send(RecCmd::Arm {
                    output_path: cfg.output_path.clone(),
                    width: w,
                    height: h,
                    // (v6.2.17) 60 fps de vuelta: el motor nuevo (WGC + encoder
                    // por hardware) drena a disco — sin buffer en RAM que
                    // acumular. El tope de 30 era del motor viejo (windows-record).
                    fps: cfg.fps.clamp(24, 60) as u32,
                });
                armed = true;
                armed_at = Some(std::time::Instant::now());
                saved = false;
                crate::diagnostics::note_recorder_armed(true);
            }
            // Tope de tiempo armado: si lleva demasiado sin aterrizar (holding,
            // fase atascada), paramos para liberar memoria del grabador. Se
            // rearmará en el siguiente tick si seguimos en aproximación.
            if armed
                && !save_pending
                && armed_at.map_or(false, |t| t.elapsed() > MAX_ARMED)
            {
                let _ = tx.send(RecCmd::Stop);
                armed = false;
                armed_at = None;
                saved = false;
                crate::diagnostics::note_recorder_armed(false);
                tracing::warn!(target: "rec", "replay buffer detenido por tope de tiempo armado (anti-leak)");
            }
        } else if armed && !save_pending {
            let _ = tx.send(RecCmd::Stop);
            armed = false;
            armed_at = None;
            saved = false;
            crate::diagnostics::note_recorder_armed(false);
        }

        // ── Touchdown: transición airborne → on_ground ──
        if on_ground && !prev_on_ground {
            // (El OSD lo emite el watcher al instante; aquí solo grabación.)
            // Guardar el clip si el replay buffer está armado.
            if armed && !saved {
                saved = true;
                // El Save real corre ~12s después (rollout) — protegemos el
                // buffer de cualquier Stop hasta que termine (SAVE_GRACE).
                save_pending_until = Some(std::time::Instant::now() + SAVE_GRACE);
                // (v6.2.18) Duración REAL del clip: desde que se armó (inicio
                // de la grabación a disco) + ~12s de rollout. Antes se escribía
                // el `clip_seconds` configurado, que ya no recorta nada.
                let duration_s = armed_at
                    .map(|t| t.elapsed().as_secs() as i64 + 12)
                    .unwrap_or(60);
                schedule_save(
                    &pool,
                    &tx,
                    &cfg.output_path,
                    duration_s,
                    cfg.max_clips,
                    status.destination_icao.clone(),
                    status.destination_name.clone(),
                    status.aircraft_icao.clone(),
                );
            }
        }

        // Desarmar la grabación tras llegar a tierra (taxi/parking) — pero
        // NUNCA mientras el guardado del clip siga pendiente (v6.2.16).
        if armed
            && !save_pending
            && on_ground
            && matches!(phase.as_str(), "taxi_in" | "parking" | "deboarding")
        {
            let _ = tx.send(RecCmd::Stop);
            armed = false;
            armed_at = None;
            saved = false;
            crate::diagnostics::note_recorder_armed(false);
        }
        prev_on_ground = on_ground;
    }
}

#[cfg(windows)]
#[allow(clippy::too_many_arguments)]
fn schedule_save(
    pool: &SqlitePool,
    tx: &std::sync::mpsc::Sender<RecCmd>,
    output_path: &str,
    // Duración estimada del clip (armado→touchdown + rollout) para el manifest.
    clip_seconds: i64,
    max_clips: i64,
    dest_icao: Option<String>,
    dest_name: Option<String>,
    aircraft_icao: Option<String>,
) {
    use std::time::Duration;
    let pool = pool.clone();
    let tx = tx.clone();
    let out_dir = PathBuf::from(output_path);
    tokio::spawn(async move {
        // ~12s para que el rollout completo entre en el buffer antes de
        // guardar (antes se cortaba muy temprano el vídeo).
        tokio::time::sleep(Duration::from_secs(12)).await;
        let millis = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let path = out_dir
            .join(format!("simfleet_landing_{millis}.mp4"))
            .to_string_lossy()
            .into_owned();
        let (rtx, rrx) = tokio::sync::oneshot::channel();
        if tx.send(RecCmd::Save { path: path.clone(), reply: rtx }).is_err() {
            return;
        }
        match rrx.await {
            Ok(Ok(())) => {
                let recorded_at = sqlx::query_as::<_, (String,)>("SELECT datetime('now')")
                    .fetch_one(&pool)
                    .await
                    .map(|(s,)| s)
                    .unwrap_or_default();
                let clip = LandingClip {
                    id: uuid::Uuid::new_v4().to_string(),
                    path,
                    recorded_at,
                    fpm: None,
                    grade: None,
                    airport_icao: dest_icao,
                    airport_name: dest_name,
                    model: aircraft_icao,
                    registration: None,
                    favorite: false,
                    duration_s: clip_seconds,
                    is_test: false,
                };
                let id = clip.id.clone();
                if let Err(e) = add_clip(&out_dir, clip, max_clips) {
                    tracing::warn!(target: "rec", "add_clip falló: {e:#}");
                }
                tracing::info!(target: "rec", "Best Landing guardado ({id})");
                // Enriquecer con metadata del flight_log ya finalizado (~20s).
                tokio::time::sleep(Duration::from_secs(20)).await;
                enrich_clip_from_flightlog(&pool, &out_dir, &id).await;
            }
            Ok(Err(e)) => tracing::warn!(target: "rec", "save_replay falló: {e}"),
            Err(_) => tracing::warn!(target: "rec", "recorder no respondió al guardar"),
        }
    });
}

#[cfg(windows)]
async fn enrich_clip_from_flightlog(pool: &SqlitePool, out_dir: &Path, id: &str) {
    let row = sqlx::query_as::<
        _,
        (
            Option<i64>,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
        ),
    >(
        "SELECT landing_fpm, destination_icao, destination_name, \
                aircraft_model, aircraft_atc_type, aircraft_registration \
         FROM flight_log WHERE status = 'completed' ORDER BY started_at DESC LIMIT 1",
    )
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();
    let Some((fpm, dico, dname, model, atc, reg)) = row else {
        return;
    };
    let grade = fpm.map(|f| crate::flight_log::landing_grade(f).to_string());
    let model = model.or(atc);
    let _ = update_clip_meta(out_dir, id, fpm, grade, dico, dname, model, reg);
}

#[cfg(not(windows))]
pub fn spawn_controller(_pool: SqlitePool, _app: tauri::AppHandle, _data_dir: PathBuf) {}
