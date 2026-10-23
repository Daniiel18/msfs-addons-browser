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
        GraphicsCaptureItemType, MinimumUpdateIntervalSettings, SecondaryWindowSettings,
        Settings,
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
        /// (v8.3.2) El encoder se creó SIN audio (el del dispositivo no le
        /// sirvió): el hilo de audio deja de empujar muestras.
        audio_off: AtomicBool,
        /// (v8.3.2) Motivo REAL por el que el handler paró (carpeta, encoder…).
        /// Antes se perdía y el log sólo decía "sin frames".
        last_error: Mutex<Option<String>>,
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
                let make = |audio: AudioSettingsBuilder| {
                    VideoEncoder::new(
                        VideoSettingsBuilder::new(w, h)
                            .frame_rate(sh.fps)
                            .bitrate(sh.bitrate)
                            .sub_type(VideoSettingsSubType::H264),
                        audio,
                        ContainerSettingsBuilder::new(),
                        &sh.temp_path,
                    )
                };
                let fmt = *sh.audio_fmt.lock().unwrap();
                let created = match fmt {
                    Some((rate, ch)) => make(
                        AudioSettingsBuilder::new()
                            .sample_rate(rate)
                            .channel_count(ch)
                            .bit_per_sample(16),
                    )
                    .or_else(|e| {
                        // (v8.3.2) Un audio que el encoder no acepta no debe
                        // costar el vídeo: clip sin audio.
                        tracing::warn!(target: "rec", "encoder con audio falló ({e}) — sigo sin audio");
                        sh.audio_off.store(true, Ordering::Relaxed);
                        make(AudioSettingsBuilder::new().disabled(true))
                    }),
                    None => make(AudioSettingsBuilder::new().disabled(true)),
                };
                let enc = match created {
                    Ok(enc) => enc,
                    Err(e) => {
                        let msg = crate::tr!("recorder.encoderFailed", e = e);
                        *sh.last_error.lock().unwrap() = Some(msg.clone());
                        return Err(msg.into());
                    }
                };
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
                if let Err(e) = enc.send_frame(frame) {
                    let msg = crate::tr!("recorder.encoderFailed", e = e);
                    *sh.last_error.lock().unwrap() = Some(msg.clone());
                    return Err(msg.into());
                }
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
        /// (v7.4.2) Ventana del sim que se está capturando (None = monitor).
        window: Option<Window>,
        interval: Duration,
        started_at: Instant,
        /// (v8.3.2) El fallo de esta sesión ya se avisó (una vez basta).
        pub reported: bool,
    }

    /// (v7.4.2) Procesos del simulador (mismos que `simconnect_watcher`).
    const SIM_PROCESSES: &[&str] = &["flightsimulator.exe", "flightsimulator2024.exe"];
    /// (v7.4.2) Si en este tiempo no llegó NINGÚN frame de la ventana, pasamos
    /// a capturar el monitor donde está el sim.
    const NO_FRAMES_FALLBACK: Duration = Duration::from_secs(6);

    /// (v7.4.2) Ventana principal del simulador: la ventana visible MÁS GRANDE
    /// de `FlightSimulator(2024).exe`; si no hay, la primera cuyo título
    /// contenga `title`. Antes era sólo lo segundo y se quedaba con la primera
    /// coincidencia en Z-order — que podía ser una ventana auxiliar del sim o
    /// de otra app con "Microsoft Flight Simulator" en el título → la captura
    /// no recibía NINGÚN frame (0 clips guardados en los logs).
    fn find_sim_window(title: &str) -> Option<Window> {
        let windows = Window::enumerate().ok()?;
        let by_process = windows
            .iter()
            .filter(|w| {
                w.process_name()
                    .map(|n| SIM_PROCESSES.contains(&n.to_ascii_lowercase().as_str()))
                    .unwrap_or(false)
            })
            .max_by_key(|w| {
                i64::from(w.width().unwrap_or(0)) * i64::from(w.height().unwrap_or(0))
            })
            .copied();
        by_process.or_else(|| {
            windows
                .into_iter()
                .find(|w| w.title().map(|t| t.contains(title)).unwrap_or(false))
        })
    }

    fn start_capture<T: TryInto<GraphicsCaptureItemType> + Send + 'static>(
        item: T,
        interval: Duration,
        shared: &Arc<Shared>,
    ) -> Result<CaptureControl<Handler, HandlerError>, String> {
        Handler::start_free_threaded(Settings::new(
            item,
            CursorCaptureSettings::WithoutCursor,
            DrawBorderSettings::WithoutBorder,
            SecondaryWindowSettings::Default,
            MinimumUpdateIntervalSettings::Custom(interval),
            DirtyRegionSettings::Default,
            ColorFormat::Bgra8,
            shared.clone(),
        ))
        .map_err(|e| format!("{e:?}"))
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
            // (v8.3.2) La carpeta del temporal TIENE que existir: el encoder
            // crea el archivo en el primer frame y, si falta la carpeta (PC
            // nueva, carpeta borrada), fallaba en silencio → "sin frames".
            if let Some(dir) = temp_path.parent() {
                std::fs::create_dir_all(dir).map_err(|e| {
                    anyhow!("{}", crate::tr!("recorder.folderFailed", path = dir.display(), e = e))
                })?;
            }
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
                audio_off: AtomicBool::new(false),
                last_error: Mutex::new(None),
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
            let window = find_sim_window(target_title);
            let control = match window {
                Some(win) => {
                    tracing::info!(
                        target: "rec",
                        "capturando ventana «{}» ({})",
                        win.title().unwrap_or_default(),
                        win.process_name().unwrap_or_default()
                    );
                    start_capture(win, interval, &shared)
                        .map_err(|e| anyhow!("{}", crate::tr!("recorder.captureStartWindowFailed", target = target_title, e = e)))?
                }
                None => {
                    let mon = Monitor::primary()
                        .map_err(|e| anyhow!("{}", crate::tr!("recorder.noWindowNoMonitor", target = target_title, e = format!("{e:?}"))))?;
                    tracing::warn!(
                        target: "rec",
                        "ventana «{target_title}» no encontrada — capturando el monitor primario"
                    );
                    start_capture(mon, interval, &shared)
                        .map_err(|e| anyhow!("{}", crate::tr!("recorder.captureMonitorFailed", e = e)))?
                }
            };

            Ok(Self {
                shared,
                control: Some(control),
                audio_stop,
                audio_join,
                done_rx,
                window,
                interval,
                started_at: Instant::now(),
                reported: false,
            })
        }

        /// (v8.3.2) Motivo del fallo que paró la captura, si lo hubo.
        pub fn last_error(&self) -> Option<String> {
            self.shared.last_error.lock().unwrap().clone()
        }

        /// (v7.4.2) Plan B anti "0 frames": si la captura de la VENTANA no
        /// entregó ni un frame en `NO_FRAMES_FALLBACK` (sim en pantalla
        /// completa exclusiva, ventana minimizada/ocluida…), la cambiamos por
        /// la del MONITOR donde está el sim. El encoder se crea con el primer
        /// frame, así que cambiar de origen antes de él no corrompe nada.
        /// Lo llama el hilo del grabador una vez por segundo.
        ///
        /// (v8.3.2) Devuelve el motivo si la captura acaba de PARAR por un
        /// error del handler (carpeta, encoder…): cambiar al monitor no lo
        /// arregla, así que se avisa en vez de esperar al "sin frames".
        pub fn watchdog(&mut self) -> Option<String> {
            let stopped = self.control.as_ref().is_some_and(|c| c.is_finished());
            if stopped {
                if let Some(c) = self.control.take() {
                    let _ = c.wait();
                }
                if let Some(e) = self.last_error() {
                    if self.reported {
                        return None;
                    }
                    self.reported = true;
                    tracing::warn!(target: "rec", "la captura se detuvo: {e}");
                    return Some(e);
                }
            }
            if self.shared.first_frame.load(Ordering::Acquire)
                || (!stopped && self.started_at.elapsed() < NO_FRAMES_FALLBACK)
            {
                return None;
            }
            // Sin ventana = ya estamos en el monitor.
            let win = self.window.take()?;
            let mon = win.monitor().or_else(|| Monitor::primary().ok())?;
            if let Some(c) = self.control.take() {
                let _ = c.stop();
            }
            match start_capture(mon, self.interval, &self.shared) {
                Ok(c) => {
                    tracing::warn!(
                        target: "rec",
                        "la ventana del sim no entregó frames en {}s — capturando su monitor",
                        NO_FRAMES_FALLBACK.as_secs()
                    );
                    self.control = Some(c);
                    None
                }
                Err(e) => {
                    tracing::warn!(target: "rec", "plan B (monitor) falló: {e}");
                    let msg = crate::tr!("recorder.captureMonitorFailed", e = e);
                    *self.shared.last_error.lock().unwrap() = Some(msg.clone());
                    self.reported = true;
                    Some(msg)
                }
            }
        }

        /// Finaliza el contenedor y mueve el temporal a `final_path`.
        pub fn finish_and_save(mut self, final_path: &Path) -> anyhow::Result<()> {
            self.shared.finish.store(true, Ordering::Relaxed);
            // El handler finaliza en el siguiente frame; si no llegan frames
            // (ventana minimizada / cerrada), caemos al plan B tras timeout.
            // Si la captura ya paró, no hay frame que esperar.
            let alive = self.control.as_ref().is_some_and(|c| !c.is_finished());
            let wait = if alive { Duration::from_secs(10) } else { Duration::ZERO };
            let res = self.done_rx.recv_timeout(wait);
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
                            let why = self.last_error().unwrap_or_else(|| crate::tr!("recorder.noFrames"));
                            return Err(anyhow!("{why}"));
                        }
                    }
                }
            }
            // Margen para que Media Foundation suelte el archivo.
            std::thread::sleep(Duration::from_millis(300));
            let tmp = &self.shared.temp_path;
            if std::fs::rename(tmp, final_path).is_err() {
                std::fs::copy(tmp, final_path)
                    .map_err(|e| anyhow!("{}", crate::tr!("recorder.clipMoveFailed", e = e)))?;
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
        if bytes.is_empty() || shared.audio_off.load(Ordering::Relaxed) {
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
        // El encoder SIEMPRE recibe estéreo PCM16. (v8.3.2) El AAC de Windows
        // sólo acepta 44,1/48 kHz: un dispositivo a 96/192 kHz (DACs USB,
        // algunos cascos) se remuestrea a 48 kHz en vez de romper el clip.
        let out_rate = super::encoder_sample_rate(lb.rate);
        let mut resampler = (out_rate != lb.rate).then(|| super::Resampler::new(lb.rate, out_rate));
        if resampler.is_some() {
            tracing::info!(target: "rec", "audio a {} Hz → remuestreado a {out_rate} Hz", lb.rate);
        }
        let _ = fmt_tx.send(Some((out_rate, 2)));

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
                    let bytes = match resampler.as_mut() {
                        Some(r) => r.process(&bytes),
                        None => bytes,
                    };
                    sent += (bytes.len() / 4) as u64;
                    push_audio(&shared, &bytes);
                }
                let _ = unsafe { lb.capture.ReleaseBuffer(frames) };
            }
            // Relleno de silencio si WASAPI no entregó nada (sin sesiones de
            // audio activas) — mantiene el A/V sync del reloj monotónico.
            // (`sent` cuenta muestras ya al rate del encoder.)
            let expected = (started.elapsed().as_secs_f64() * f64::from(out_rate)) as u64;
            if expected > sent + u64::from(out_rate) / 5 {
                let deficit = (expected - sent).min(u64::from(out_rate));
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

    #[cfg(test)]
    mod probe {
        use super::*;

        /// Diagnóstico manual: `REC_TARGET=<título> REC_AUDIO=0|1 cargo test
        /// --lib rec_probe -- --ignored --nocapture`.
        #[test]
        #[ignore]
        fn rec_probe() {
            let dir = std::env::var_os("REC_DIR").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
            let out = dir.join("simfleet-rec-probe.mp4");
            let _ = std::fs::remove_file(&out);
            let target = std::env::var("REC_TARGET").unwrap_or_else(|_| "__none__".into());
            let audio = std::env::var("REC_AUDIO").map(|v| v == "1").unwrap_or(true);
            let mut s = CaptureSession::start(
                &target,
                30,
                8_000_000,
                dir.join("simfleet-rec-probe.part.mp4"),
                audio,
            )
            .expect("start");
            eprintln!("audio_fmt={:?} window={:?}", s.shared.audio_fmt.lock().unwrap(), s.window.map(|w| w.title()));
            for i in 0..8 {
                std::thread::sleep(Duration::from_secs(1));
                s.watchdog();
                let finished = s.control.as_ref().map(|c| c.is_finished()).unwrap_or(true);
                eprintln!("t={} first_frame={} capture_finished={finished}", i + 1, s.shared.first_frame.load(Ordering::Acquire));
                if finished {
                    if let Some(c) = s.control.take() {
                        eprintln!("capture thread ended: {:?}", c.wait().err());
                    }
                }
            }
            let r = s.finish_and_save(&out);
            eprintln!("result={r:?} size={:?}", std::fs::metadata(&out).map(|m| m.len()));
        }
    }
}

/// (v8.3.2) Sample rate que se le da al encoder: el del dispositivo si el AAC
/// de Windows lo acepta (44,1/48 kHz); si no, 48 kHz (se remuestrea).
fn encoder_sample_rate(device_rate: u32) -> u32 {
    if matches!(device_rate, 44_100 | 48_000) {
        device_rate
    } else {
        48_000
    }
}

/// (v8.3.2) Remuestreo lineal de PCM16 estéreo intercalado, con estado entre
/// paquetes (posición fraccionaria + última muestra) para no meter clics.
/// Calidad de sobra para el audio de un clip del sim.
struct Resampler {
    /// Muestras de entrada por muestra de salida.
    step: f64,
    /// Posición en la entrada del paquete actual; -1 = última del anterior.
    pos: f64,
    prev: (i16, i16),
}

impl Resampler {
    fn new(from: u32, to: u32) -> Self {
        Self {
            step: f64::from(from) / f64::from(to.max(1)),
            pos: 0.0,
            prev: (0, 0),
        }
    }

    fn process(&mut self, input: &[u8]) -> Vec<u8> {
        let n = input.len() / 4;
        if n == 0 {
            return Vec::new();
        }
        let prev = self.prev;
        let at = |i: isize| -> (i16, i16) {
            if i < 0 {
                return prev;
            }
            let o = i as usize * 4;
            (
                i16::from_le_bytes([input[o], input[o + 1]]),
                i16::from_le_bytes([input[o + 2], input[o + 3]]),
            )
        };
        let mut out = Vec::with_capacity(((n as f64 / self.step) as usize + 2) * 4);
        let mut pos = self.pos;
        while pos < (n - 1) as f64 {
            let i = pos.floor();
            let frac = pos - i;
            let (a, b) = (at(i as isize), at(i as isize + 1));
            let lerp = |x: i16, y: i16| (f64::from(x) + (f64::from(y) - f64::from(x)) * frac).round() as i16;
            out.extend_from_slice(&lerp(a.0, b.0).to_le_bytes());
            out.extend_from_slice(&lerp(a.1, b.1).to_le_bytes());
            pos += self.step;
        }
        self.prev = at(n as isize - 1);
        self.pos = pos - n as f64;
        out
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
    // (v7.4.2) En el entorno de pruebas NUNCA la carpeta real: el tope de
    // clips (`max_clips`) borra los más viejos y se llevaría clips reales.
    if !crate::test_env::is_active() {
        if let Some(p) = landingtoast_video_path() {
            return p;
        }
    }
    if let Some(p) = videos_landings_dir() {
        return p.to_string_lossy().into_owned();
    }
    fallback_data
        .join("landings")
        .to_string_lossy()
        .into_owned()
}

/// (v8.3.2) Carpeta Vídeos REAL de Windows, también si el usuario la movió a
/// otro disco u OneDrive (`User Shell Folders\My Video`; `RegGetValueW`
/// expande el `%USERPROFILE%`). Antes se asumía `%USERPROFILE%\Videos`.
fn videos_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        use windows::core::HSTRING;
        use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_SZ};
        let mut buf = [0u16; 1024];
        let mut size = (buf.len() * 2) as u32;
        let ok = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                &HSTRING::from(r"Software\Microsoft\Windows\CurrentVersion\Explorer\User Shell Folders"),
                &HSTRING::from("My Video"),
                RRF_RT_REG_SZ,
                None,
                Some(buf.as_mut_ptr() as *mut core::ffi::c_void),
                Some(&mut size),
            )
        }
        .is_ok();
        if ok {
            let len = (size as usize / 2).saturating_sub(1);
            let s = String::from_utf16_lossy(&buf[..len.min(buf.len())]);
            let s = s.trim();
            if !s.is_empty() && !s.contains('%') {
                return Some(PathBuf::from(s));
            }
        }
    }
    std::env::var_os("USERPROFILE").map(|p| PathBuf::from(p).join("Videos"))
}

/// (v8.3.2) `Vídeos\SimFleet Landings`: carpeta por defecto y de respaldo.
fn videos_landings_dir() -> Option<PathBuf> {
    // Los tests nunca tocan la carpeta Vídeos real del usuario.
    if cfg!(test) {
        return Some(std::env::temp_dir().join(format!("simfleet-videos-test-{}", std::process::id())));
    }
    videos_dir().map(|v| v.join(crate::test_env::landings_folder_name()))
}

/// (v8.3.2) Carpeta donde se graba DE VERDAD: la configurada (creándola si
/// falta — en una PC nueva o si se borró, antes la grabación fallaba en
/// silencio) o, si no se puede (disco desconectado…), la de Vídeos. Devuelve
/// además el aviso para el usuario cuando hubo que usar la de respaldo.
pub fn resolve_output_dir(
    configured: &str,
    fallback_data: &Path,
) -> Result<(PathBuf, Option<String>), String> {
    let cfg = PathBuf::from(configured);
    let err = match std::fs::create_dir_all(&cfg) {
        Ok(()) => return Ok((cfg, None)),
        Err(e) => e,
    };
    let backup = videos_landings_dir().unwrap_or_else(|| fallback_data.join("landings"));
    if backup != cfg && std::fs::create_dir_all(&backup).is_ok() {
        tracing::warn!(
            target: "rec",
            "carpeta «{}» no disponible ({err}) — grabo en «{}»",
            cfg.display(),
            backup.display()
        );
        let notice = crate::tr!("recorder.folderFallback", from = cfg.display(), to = backup.display());
        return Ok((backup, Some(notice)));
    }
    Err(crate::tr!("recorder.folderFailed", path = cfg.display(), e = err))
}

/// (v8.3.2) Carpetas que pueden tener clips: la configurada y la de respaldo.
pub fn clip_dirs(configured: &str) -> Vec<PathBuf> {
    let mut dirs = vec![PathBuf::from(configured)];
    if let Some(b) = videos_landings_dir() {
        if !dirs.contains(&b) {
            dirs.push(b);
        }
    }
    dirs
}

/// Carpeta (de `dirs`) cuyo manifiesto tiene el clip `id`.
pub fn clip_dir_of(dirs: &[PathBuf], id: &str) -> Option<PathBuf> {
    dirs.iter()
        .find(|d| load_clips(d).iter().any(|c| c.id == id))
        .cloned()
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

/// (v7.4.2) Ruta del lock, COMPARTIDA por todas las instancias. Antes vivía
/// en `temp_dir()`, pero desde v3.2.0 cada instancia redirige su TEMP a su
/// propio data dir → cada una tenía SU lock y el "entre procesos" no excluía
/// nada (la principal y la de pruebas grababan a la vez sobre el mismo sim).
#[cfg(target_os = "windows")]
fn shared_lock_path(name: &str) -> PathBuf {
    let dir = std::env::var_os("LOCALAPPDATA")
        .map(|p| PathBuf::from(p).join("SimFleet"))
        .unwrap_or_else(std::env::temp_dir);
    let _ = std::fs::create_dir_all(&dir);
    dir.join(name)
}

#[cfg(target_os = "windows")]
const MAIN_LOCK: &str = "simfleet_recorder.lock";
#[cfg(target_os = "windows")]
const TEST_LOCK: &str = "simfleet_recorder.pruebas.lock";

/// (v7.4.2) ¿La instancia PRINCIPAL está grabando ahora mismo? La de pruebas
/// le cede la captura: no arma mientras la principal graba y aborta si la
/// principal empieza (sólo hay un sim que capturar y su clip es el real).
pub fn main_instance_recording() -> bool {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::fs::OpenOptionsExt;
        let path = shared_lock_path(MAIN_LOCK);
        // Sin `create`: si el archivo no existe nadie lo tiene. Si existe y
        // la principal lo tiene abierto en exclusiva, abrirlo falla.
        path.exists()
            && std::fs::OpenOptions::new()
                .write(true)
                .share_mode(0)
                .open(&path)
                .is_err()
    }
    #[cfg(not(target_os = "windows"))]
    {
        false
    }
}

/// Intenta tomar el lock entre procesos. `true` si lo conseguimos.
fn acquire_cross_process_lock() -> bool {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::fs::OpenOptionsExt;
        let test = crate::test_env::is_active();
        if test && main_instance_recording() {
            return false;
        }
        let path = shared_lock_path(if test { TEST_LOCK } else { MAIN_LOCK });
        match std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
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
    // (v8.3) `get_window`: también con el webview de flightsim.to dentro.
    let Some(win) = app.get_window("main") else {
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
        anyhow::bail!("{}", crate::tr!("recorder.noOutputFile"));
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
    anyhow::bail!("{}", crate::tr!("recorder.windowsOnly"))
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
/// (v8.3.2) Aviso dentro de SimFleet (toast) — el usuario eligió enterarse
/// de los fallos de grabación sólo en la app. `kind`: "error" | "folder".
#[cfg(windows)]
fn notify(app: &tauri::AppHandle, kind: &str, message: String) {
    use tauri::Emitter;
    let _ = app.emit(
        "recorder://notice",
        serde_json::json!({ "kind": kind, "message": message }),
    );
}

#[cfg(windows)]
fn recorder_thread(rx: std::sync::mpsc::Receiver<RecCmd>, app: tauri::AppHandle) {
    let mut session: Option<capture_engine::CaptureSession> = None;
    loop {
        // (v7.4.2) Despertamos cada segundo para el watchdog de "0 frames".
        let cmd = match rx.recv_timeout(std::time::Duration::from_secs(1)) {
            Ok(cmd) => cmd,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                if let Some(e) = session.as_mut().and_then(|s| s.watchdog()) {
                    notify(&app, "error", e);
                }
                continue;
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        };
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
                        notify(&app, "error", format!("{e:#}"));
                    }
                }
            }
            RecCmd::Save { path, reply } => {
                let res = match session.take() {
                    Some(s) => {
                        let reported = s.reported;
                        let r = s
                            .finish_and_save(Path::new(&path))
                            .map_err(|e| format!("{e:#}"));
                        release_recording();
                        // (v7.4.2) Antes se logueaba "clip guardado" SIEMPRE,
                        // también cuando el guardado fallaba.
                        match &r {
                            Ok(()) => tracing::info!(target: "rec", "clip guardado: {path}"),
                            // (v8.3.2) Si el watchdog ya avisó del motivo, no repetir.
                            Err(e) if !reported => notify(&app, "error", e.clone()),
                            Err(_) => {}
                        }
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
    let app_rec = app.clone();
    let _ = std::thread::Builder::new()
        .name("landing-recorder".into())
        .spawn(move || recorder_thread(rx, app_rec));
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
    // (v8.3.2) Carpeta donde se armó la grabación (la configurada o la de
    // respaldo): el clip se guarda AHÍ. Y si no hubo carpeta posible, no se
    // reintenta (ni se re-avisa) cada segundo hasta tocar tierra.
    let mut armed_dir: Option<PathBuf> = None;
    let mut arm_blocked = false;

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
            arm_blocked = false;
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

        // (v7.4.2) En el entorno de pruebas la instancia principal tiene
        // prioridad sobre la captura del sim (ver `main_instance_recording`).
        let yield_to_main = crate::test_env::is_active() && main_instance_recording();

        // ── Grabación (solo si está activada) ──
        if cfg.enabled {
            if armed && !save_pending && yield_to_main {
                let _ = tx.send(RecCmd::Stop);
                armed = false;
                armed_at = None;
                saved = false;
                crate::diagnostics::note_recorder_armed(false);
                tracing::warn!(target: "rec", "pruebas: la instancia principal está grabando — cedo la captura");
            }
            // Armar el replay buffer al entrar en aproximación.
            if approaching && !on_ground && !armed && !yield_to_main && !arm_blocked {
                match resolve_output_dir(&cfg.output_path, &data_dir) {
                    Ok((dir, notice)) => {
                        if let Some(n) = notice {
                            notify(&app, "folder", n);
                        }
                        let (w, h) = target_monitor_size(&app, false);
                        let _ = tx.send(RecCmd::Arm {
                            output_path: dir.to_string_lossy().into_owned(),
                            width: w,
                            height: h,
                            // (v6.2.17) 60 fps de vuelta: el motor nuevo (WGC + encoder
                            // por hardware) drena a disco — sin buffer en RAM que
                            // acumular. El tope de 30 era del motor viejo (windows-record).
                            fps: cfg.fps.clamp(24, 60) as u32,
                        });
                        armed_dir = Some(dir);
                        armed = true;
                        armed_at = Some(std::time::Instant::now());
                        saved = false;
                        crate::diagnostics::note_recorder_armed(true);
                    }
                    Err(e) => {
                        tracing::warn!(target: "rec", "no se arma la captura: {e}");
                        notify(&app, "error", e);
                        arm_blocked = true;
                    }
                }
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
                let out_dir = armed_dir
                    .clone()
                    .unwrap_or_else(|| PathBuf::from(&cfg.output_path));
                schedule_save(
                    &pool,
                    &tx,
                    &out_dir,
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
    out_dir: &Path,
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
    let out_dir = out_dir.to_path_buf();
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

#[cfg(test)]
mod tests {
    use super::*;

    fn pcm(frames: &[(i16, i16)]) -> Vec<u8> {
        frames
            .iter()
            .flat_map(|(l, r)| l.to_le_bytes().into_iter().chain(r.to_le_bytes()))
            .collect()
    }

    fn frames(bytes: &[u8]) -> Vec<(i16, i16)> {
        bytes
            .chunks_exact(4)
            .map(|c| (i16::from_le_bytes([c[0], c[1]]), i16::from_le_bytes([c[2], c[3]])))
            .collect()
    }

    #[test]
    fn encoder_rate_only_changes_unsupported_rates() {
        assert_eq!(encoder_sample_rate(48_000), 48_000);
        assert_eq!(encoder_sample_rate(44_100), 44_100);
        assert_eq!(encoder_sample_rate(96_000), 48_000);
        assert_eq!(encoder_sample_rate(192_000), 48_000);
        assert_eq!(encoder_sample_rate(32_000), 48_000);
    }

    #[test]
    fn resampler_halves_96k_across_packets_without_gaps() {
        let mut r = Resampler::new(96_000, 48_000);
        // Rampa 0..2000 partida en paquetes de tamaños raros.
        let ramp: Vec<(i16, i16)> = (0..2000).map(|i| (i as i16, -(i as i16))).collect();
        let mut out = Vec::new();
        for chunk in ramp.chunks(333) {
            out.extend(frames(&r.process(&pcm(chunk))));
        }
        // ~la mitad de muestras, y la rampa sigue siendo continua (paso 2).
        assert!((out.len() as i64 - 1000).abs() <= 1, "len {}", out.len());
        for w in out.windows(2) {
            assert_eq!(w[1].0 - w[0].0, 2);
            assert_eq!(w[1].1, -w[1].0);
        }
    }

    #[test]
    fn resampler_upsamples_32k_to_48k() {
        let mut r = Resampler::new(32_000, 48_000);
        let input: Vec<(i16, i16)> = (0..3200).map(|i| (i as i16, 0)).collect();
        let out = frames(&r.process(&pcm(&input)));
        assert!((out.len() as i64 - 4800).abs() <= 2, "len {}", out.len());
        assert!(out.windows(2).all(|w| w[1].0 >= w[0].0));
    }

    #[test]
    fn output_dir_is_created_when_missing() {
        let base = std::env::temp_dir().join(format!("simfleet-rec-test-{}", std::process::id()));
        let missing = base.join("Videos").join("SimFleet Landings");
        let _ = std::fs::remove_dir_all(&base);
        let (dir, notice) = resolve_output_dir(&missing.to_string_lossy(), &base).unwrap();
        assert_eq!(dir, missing);
        assert!(notice.is_none());
        assert!(dir.is_dir());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn unavailable_output_dir_falls_back_with_notice() {
        // Una ruta imposible: un ARCHIVO hace de carpeta padre.
        let base = std::env::temp_dir().join(format!("simfleet-rec-fb-{}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        let file = base.join("no-soy-carpeta");
        std::fs::write(&file, b"x").unwrap();
        let bad = file.join("clips");
        let (dir, notice) = resolve_output_dir(&bad.to_string_lossy(), &base).unwrap();
        assert_ne!(dir, bad);
        assert!(dir.is_dir());
        assert!(notice.is_some());
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[cfg(windows)]
    #[test]
    fn videos_dir_is_expanded() {
        let v = videos_dir().expect("videos dir");
        assert!(!v.to_string_lossy().contains('%'));
        assert!(v.is_absolute());
    }
}
