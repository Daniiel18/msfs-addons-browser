use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use tracing::{Event, Subscriber};
use tracing_subscriber::fmt::format::{FormatEvent, FormatFields, Writer};
use tracing_subscriber::fmt::{FmtContext, FormattedFields};
use tracing_subscriber::prelude::*;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::{fmt, EnvFilter};

/// (v3.37.0 #11) Formatter custom para logs legibles.
///
/// Output: `[YYYY/MM/DD HH:MM:SS] [LEVEL]    [categoria] mensaje key=val`
///
/// · Fecha PRIMERO entre corchetes (año completo) — pedido del usuario.
/// · `[LEVEL]` con padding a 9 chars para alinear columnas. Además de
///   ERROR/WARN/INFO/DEBUG/TRACE soportamos un nivel sintético
///   **SUCCESS**: se emite con `success!(...)` (un `info!` con el campo
///   marcador `success = true`) y el formatter lo pinta como
///   `[SUCCESS]` (el marcador NO se imprime como campo).
/// · `[categoria]` = el `target` de tracing (último segmento si es un
///   path de módulo): cmd, scan, download, cloud_sync, settings, …
struct CompactFormat;

/// (v3.37.0 #11) Visitor que separa el campo especial `message` (el
/// texto del log) del resto de campos (`key=value`) y detecta el
/// marcador `success`. Los tipos numéricos/error no se sobreescriben:
/// el `Visit` por defecto los reenvía a `record_debug`.
#[derive(Default)]
struct EventVisitor {
    message: String,
    fields: String,
    is_success: bool,
}

impl tracing::field::Visit for EventVisitor {
    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        if field.name() == "success" {
            self.is_success = self.is_success || value;
            return;
        }
        use std::fmt::Write;
        let _ = write!(self.fields, " {}={}", field.name(), value);
    }
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.message.push_str(value);
            return;
        }
        use std::fmt::Write;
        let _ = write!(self.fields, " {}={}", field.name(), value);
    }
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        use std::fmt::Write;
        match field.name() {
            "message" => {
                let _ = write!(self.message, "{:?}", value);
            }
            // por si `success` llega como debug en vez de bool.
            "success" => {}
            name => {
                let _ = write!(self.fields, " {}={:?}", name, value);
            }
        }
    }
}

impl<S, N> FormatEvent<S, N> for CompactFormat
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        ctx: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> std::fmt::Result {
        let meta = event.metadata();
        let mut visitor = EventVisitor::default();
        event.record(&mut visitor);

        // [YYYY/MM/DD HH:MM:SS] local — fecha primero, año completo.
        let now = chrono::Local::now();
        write!(writer, "[{}] ", now.format("%Y/%m/%d %H:%M:%S"))?;

        // [LEVEL] con SUCCESS sintético; padding a 9 para alinear.
        let level_str = if visitor.is_success {
            "[SUCCESS]"
        } else {
            match *meta.level() {
                tracing::Level::ERROR => "[ERROR]",
                tracing::Level::WARN => "[WARN]",
                tracing::Level::INFO => "[INFO]",
                tracing::Level::DEBUG => "[DEBUG]",
                tracing::Level::TRACE => "[TRACE]",
            }
        };
        write!(writer, "{:<9} ", level_str)?;

        // [categoria] — último segmento del target de tracing.
        let target = meta.target();
        let category = target.rsplit("::").next().unwrap_or(target);
        write!(writer, "[{}] ", category)?;

        // Mensaje + campos extra (key=value). El marcador `success` ya
        // fue consumido por el visitor y no se imprime.
        write!(writer, "{}", visitor.message)?;
        write!(writer, "{}", visitor.fields)?;

        // Spans si los hay (info!() dentro de un span getea el nombre).
        if let Some(scope) = ctx.event_scope() {
            for span in scope.from_root() {
                let ext = span.extensions();
                let fields = ext.get::<FormattedFields<N>>();
                if let Some(f) = fields {
                    if !f.fields.is_empty() {
                        write!(writer, " {{{}}}", f.fields)?;
                    }
                }
            }
        }
        writeln!(writer)
    }
}

/// (v4.5.0) Logs **diarios** con retención de 30 días — pedido del
/// usuario. Cada día se crea un archivo nuevo `simfleet.<YYYY-MM-DD>.log`
/// y `tracing-appender` borra automáticamente los más antiguos cuando
/// hay más de `LOG_RETENTION_DAYS`. Así un día de logs no contamina al
/// siguiente y el historial se mantiene acotado (~1 mes) sin crecer sin
/// fin.
///
/// (Antes: un único `simfleet.log` con rotación por tamaño.)
const LOG_FILE_PREFIX: &str = "simfleet";
const LOG_FILE_SUFFIX: &str = "log";
const LOG_RETENTION_DAYS: usize = 30;

/// (v3.4.0) Guard contra doble init. Antes usábamos `try_init()` que
/// es idempotente pero SILENCIOSO — si por error invocábamos
/// `logger::init` dos veces (p.ej. plugin que también lo hace, hot
/// reload de Tauri en dev), el segundo intento fallaba sin avisar y
/// teníamos comportamiento confuso. Con este flag al menos
/// reportamos por `eprintln!` (no podemos usar `tracing::warn!`
/// porque eso es lo que estamos inicializando) y abortamos.
static LOGGER_INITIALIZED: AtomicBool = AtomicBool::new(false);

pub fn init(app_data_dir: &Path) -> anyhow::Result<()> {
    if LOGGER_INITIALIZED
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        eprintln!(
            "[logger] init() llamado dos veces — ignorando segundo intento. Esto sugiere un bug; reporta si lo ves."
        );
        return Ok(());
    }
    let logs_dir = app_data_dir.join("logs");
    std::fs::create_dir_all(&logs_dir)?;

    // (v4.5.0) Appender diario con retención de 30 archivos. Genera
    // `simfleet.<YYYY-MM-DD>.log` y poda los más viejos automáticamente.
    let file_appender = tracing_appender::rolling::Builder::new()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix(LOG_FILE_PREFIX)
        .filename_suffix(LOG_FILE_SUFFIX)
        .max_log_files(LOG_RETENTION_DAYS)
        .build(&logs_dir)?;

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new(
            "info,msfs_addons_browser_lib=debug,cmd=trace,scan=debug,download=debug,install=debug,updates=debug,airports=debug",
        )
    });

    // (v3.2.0) Formato compacto custom: `[LEVEL] YY/MM/DD HH:MM:SS
    // target:line message`. Aplicado a stdout y al archivo.
    let stdout_layer = fmt::layer().event_format(CompactFormat);
    let file_layer = fmt::layer()
        .with_ansi(false)
        .event_format(CompactFormat)
        .with_writer(file_appender);

    // (v3.4.0) `try_init` antes era silencioso ante fallo. Ahora
    // verificamos el `Result` y lo logueamos por stderr — si el global
    // dispatcher ya estaba seteado (ej. otro plugin lo registró antes
    // que nosotros), sabemos del conflicto. NO restablecemos el guard
    // porque la subscripción no quedó "nuestra".
    if let Err(e) = tracing_subscriber::registry()
        .with(filter)
        .with(stdout_layer)
        .with(file_layer)
        .try_init()
    {
        eprintln!(
            "[logger] try_init falló — global dispatcher ya seteado: {e:#}"
        );
    }

    tracing::info!(
        "logger inicializado — logs diarios (retención {} días) en {}",
        LOG_RETENTION_DAYS,
        logs_dir.display()
    );
    Ok(())
}

/// Macro para loguear comandos Tauri con un patrón estandarizado.
/// Genera un mensaje al entrar y otro al salir con la duración.
///
/// Uso:
///   ```ignore
///   #[tauri::command]
///   pub async fn foo(...) -> Result<X, String> {
///       crate::cmd_log!("foo", "param1={}", v1);
///       let _t = crate::CmdTimer::start("foo");
///       // ... lógica ...
///   }
///   ```
///
/// El `_t` cuando se sale del scope (drop) loguea la duración.
#[macro_export]
macro_rules! cmd_log {
    ($cmd:literal) => {
        tracing::info!(target: "cmd", "▶ {}", $cmd);
    };
    ($cmd:literal, $($arg:tt)+) => {
        tracing::info!(target: "cmd", "▶ {} | {}", $cmd, format_args!($($arg)+));
    };
}

/// (v3.37.0 #11) Loguea una línea de ÉXITO — nivel sintético SUCCESS.
/// Es un `info!` con el campo marcador `success = true` que el
/// `CompactFormat` pinta como `[SUCCESS]`. Uso idéntico a
/// `tracing::info!`:
///   `success!(target: "cloud_sync", "Subida OK: {}", id);`
#[macro_export]
macro_rules! success {
    (target: $target:expr, $($arg:tt)+) => {
        tracing::info!(target: $target, success = true, $($arg)+);
    };
    ($($arg:tt)+) => {
        tracing::info!(success = true, $($arg)+);
    };
}

/// Timer RAII — loguea duración del comando al hacer drop.
/// Construido con `CmdTimer::start("nombre")` al inicio del comando.
pub struct CmdTimer {
    name: &'static str,
    start: std::time::Instant,
}

impl CmdTimer {
    pub fn start(name: &'static str) -> Self {
        Self {
            name,
            start: std::time::Instant::now(),
        }
    }
}

impl Drop for CmdTimer {
    fn drop(&mut self) {
        let elapsed = self.start.elapsed();
        let level = if elapsed.as_millis() > 1000 {
            tracing::Level::WARN
        } else {
            tracing::Level::INFO
        };
        if level == tracing::Level::WARN {
            tracing::warn!(
                target: "cmd",
                "◀ {} ({}ms — LENTO)",
                self.name,
                elapsed.as_millis()
            );
        } else {
            tracing::info!(
                target: "cmd",
                "◀ {} ({}ms)",
                self.name,
                elapsed.as_millis()
            );
        }
    }
}
