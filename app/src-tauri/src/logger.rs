use std::path::Path;

use tracing_subscriber::prelude::*;
use tracing_subscriber::{fmt, EnvFilter};

/// (v3.1.3) Logs unificados — un solo archivo `simfleet.log` en la
/// carpeta de logs de la app, con rotación SIMPLE por tamaño.
///
/// Antes usábamos `tracing_appender::rolling::daily` que generaba un
/// archivo por día (`app.log.2026-05-22`, `app.log.2026-05-23`, etc).
/// El usuario pidió "un único archivo de log global" para auditar sin
/// saltar entre archivos.
///
/// Estrategia:
///   · Archivo principal: `simfleet.log` (todos los logs nuevos).
///   · Al arrancar, si `simfleet.log` supera `MAX_LOG_BYTES` (5 MB),
///     lo rotamos a `simfleet.log.1` (sobrescribiendo el anterior).
///   · Sólo 1 backup — los logs históricos antiguos quedan en
///     `app.log.YYYY-MM-DD` por compat con instalaciones previas.
const MAX_LOG_BYTES: u64 = 5 * 1024 * 1024;
const LOG_FILE_NAME: &str = "simfleet.log";

pub fn init(app_data_dir: &Path) -> anyhow::Result<()> {
    let logs_dir = app_data_dir.join("logs");
    std::fs::create_dir_all(&logs_dir)?;

    // Rotación por tamaño al arrancar — barato y suficiente para
    // los volúmenes que generamos (~50 MB/día en debug, <5 MB/día
    // en INFO production).
    let log_path = logs_dir.join(LOG_FILE_NAME);
    if let Ok(meta) = std::fs::metadata(&log_path) {
        if meta.len() > MAX_LOG_BYTES {
            let backup = logs_dir.join(format!("{}.1", LOG_FILE_NAME));
            let _ = std::fs::remove_file(&backup); // ignora si no existe
            let _ = std::fs::rename(&log_path, &backup);
        }
    }

    // never daily — usamos un appender que escribe siempre al mismo
    // archivo. `rolling::never` cumple esto y soporta WriteGuard
    // implícito para flush ordenado.
    let file_appender = tracing_appender::rolling::never(&logs_dir, LOG_FILE_NAME);

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new(
            "info,msfs_addons_browser_lib=debug,cmd=trace,scan=debug,download=debug,install=debug,updates=debug,airports=debug",
        )
    });

    let stdout_layer = fmt::layer()
        .with_target(true)
        .with_thread_ids(false)
        .with_thread_names(false)
        .with_line_number(true)
        .with_file(false)
        .with_timer(fmt::time::ChronoLocal::rfc_3339());

    let file_layer = fmt::layer()
        .with_ansi(false)
        .with_target(true)
        .with_thread_ids(false)
        .with_thread_names(false)
        .with_line_number(true)
        .with_file(false)
        .with_timer(fmt::time::ChronoLocal::rfc_3339())
        .with_writer(file_appender);

    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(stdout_layer)
        .with(file_layer)
        .try_init();

    tracing::info!(
        "logger inicializado — logs unificados en {}",
        log_path.display()
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
