use std::path::Path;

use tracing_subscriber::prelude::*;
use tracing_subscriber::{fmt, EnvFilter};

pub fn init(app_data_dir: &Path) -> anyhow::Result<()> {
    let logs_dir = app_data_dir.join("logs");
    std::fs::create_dir_all(&logs_dir)?;

    let file_appender = tracing_appender::rolling::daily(&logs_dir, "app.log");

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,msfs_addons_browser_lib=debug"));

    let stdout_layer = fmt::layer()
        .with_target(true)
        .with_timer(fmt::time::ChronoLocal::rfc_3339());

    let file_layer = fmt::layer()
        .with_ansi(false)
        .with_target(true)
        .with_timer(fmt::time::ChronoLocal::rfc_3339())
        .with_writer(file_appender);

    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(stdout_layer)
        .with(file_layer)
        .try_init();

    Ok(())
}
