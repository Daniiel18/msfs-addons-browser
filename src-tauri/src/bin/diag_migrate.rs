//! Binario de diagnóstico: corre las migraciones sobre el DB del
//! usuario y reporta exactamente qué falla. Útil cuando la app
//! crashea silencioso en `db::init`.
//!
//! Uso:
//!   cargo run --bin diag_migrate --manifest-path src-tauri/Cargo.toml \
//!     -- "C:\Users\n0xful\AppData\Local\Temp\user-db-sqlx-test.db"

use std::path::PathBuf;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("usage: diag_migrate <db_dir>");
        std::process::exit(2);
    }
    let path = PathBuf::from(&args[1]);
    println!("[diag] data dir: {}", path.display());

    // Llama exactamente la misma `db::init` que usa la app real, para
    // que el diagnóstico cubra el path de normalize_migration_checksums.
    match msfs_addons_browser_lib::db::init(&path).await {
        Ok(_pool) => println!("[diag] init OK"),
        Err(e) => {
            eprintln!("[diag] FALLÓ: {e:#}");
            std::process::exit(1);
        }
    }
    Ok(())
}
