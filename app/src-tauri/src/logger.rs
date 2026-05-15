use std::path::Path;

use tracing_subscriber::prelude::*;
use tracing_subscriber::{fmt, EnvFilter};

pub fn init(app_data_dir: &Path) -> anyhow::Result<()> {
    let logs_dir = app_data_dir.join("logs");
    std::fs::create_dir_all(&logs_dir)?;

    let file_appender = tracing_appender::rolling::daily(&logs_dir, "app.log");

    // Filtro detallado por defecto:
    //   · `cmd=trace` — cada comando Tauri loguea entrada+salida
    //     con tiempo de ejecución (target "cmd").
    //   · `msfs_addons_browser_lib=debug` — debug de la app entera.
    //   · `info` para libs externas (sqlx, hyper, etc.) para que
    //     no se ahoguen los logs en chatter de bajo nivel.
    //
    // Override: el usuario puede setear `RUST_LOG` para subir/bajar
    // (ej. `RUST_LOG=trace` para verlo todo, o
    // `RUST_LOG=msfs_addons_browser_lib=trace,sqlx=warn` para
    // diagnosticar SQL específico).
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

    // En el archivo guardamos también número de línea + módulo —
    // útil para que cuando el usuario nos mande logs sepamos
    // exactamente dónde se generó cada mensaje.
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

    tracing::info!("logger inicializado — logs en {}", logs_dir.display());
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
