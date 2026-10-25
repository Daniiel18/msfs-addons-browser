//! (v7.4.2) ENTORNO DE PRUEBAS — 2ª instancia de SimFleet que corre junto a
//! la principal (la instalada) SIN mezclar datos con ella.
//!
//! Se activa con `SIMFLEET_SAFE_MODE=1` (lo pone `dev-safe.ps1`) o con el
//! argumento `--pruebas` (lo añade el autoarranque de la instancia de pruebas,
//! que Windows lanza sin variables de entorno). Cada recurso compartido de la
//! máquina recibe su PROPIO espacio de pruebas:
//!
//!   · Datos (DB, logs, temp, torrents): `%LOCALAPPDATA%\SimFleet-Pruebas\data`
//!     (o `SIMFLEET_DATA_DIR`), marcado con `ENTORNO_DE_PRUEBAS.txt`.
//!   · WebView2 (localStorage, cookies, sesiones): `<datos>\webview2`.
//!   · Google Drive: SUBE a `msfs-addons-data.PRUEBAS.json`; BAJA del
//!     snapshot real (sólo lectura). Auto-sync diario y tras cada vuelo
//!     activos, como en la principal.
//!   · Supabase: tablas `*_test` y canales `simfleet-test-vs-*` (frontend,
//!     ver `src/lib/testEnv.ts`).
//!   · Autoarranque: entrada propia «SimFleet Pruebas» en `HKCU\…\Run`.
//!   · Clips de Best Landings: `Videos\SimFleet Landings - PRUEBAS`.
//!
//! Lo que NO se aísla (decisión del usuario): la carpeta Community real y
//! SimConnect (el watcher corre también en pruebas; se puede apagar con
//! `SIMFLEET_DISABLE_WATCHER=1`). Discord RPC tampoco.
//!
//! Doble candado sobre el dir de datos: la instancia de pruebas NUNCA adopta un
//! dir que ya tenga una DB sin la marca de pruebas, y la principal NUNCA abre
//! un dir marcado como de pruebas.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context};

/// Argumento de línea de comandos que activa el entorno de pruebas.
pub const CLI_FLAG: &str = "--pruebas";
/// Archivo marca que identifica un dir de datos como de pruebas.
pub const MARKER_FILE: &str = "ENTORNO_DE_PRUEBAS.txt";
/// Nombre de la entrada de autoarranque (distinta de «SimFleet»).
pub const AUTOSTART_NAME: &str = "SimFleet Pruebas";
/// Nombre de la DB (mismo que usa `db::init`).
const DB_FILE: &str = "msfs-addons.db";

const MARKER_TEXT: &str = "Este directorio pertenece al ENTORNO DE PRUEBAS de SimFleet.\r\n\
La instancia principal (instalada) se niega a abrirlo, y la de pruebas solo\r\n\
escribe aquí. Puedes borrarlo entero para empezar las pruebas desde cero.\r\n";

/// `true` si este proceso es la instancia de pruebas.
pub fn is_active() -> bool {
    static ACTIVE: once_cell::sync::Lazy<bool> = once_cell::sync::Lazy::new(|| {
        std::env::var_os("SIMFLEET_SAFE_MODE").is_some()
            || std::env::args().any(|a| a == CLI_FLAG)
    });
    *ACTIVE
}

/// `true` si el watcher de SimConnect debe arrancar. En pruebas está
/// ENCENDIDO por defecto (pedido del usuario); se apaga con
/// `SIMFLEET_DISABLE_WATCHER=1`. La instancia principal siempre lo arranca.
pub fn watcher_enabled() -> bool {
    !is_active() || std::env::var_os("SIMFLEET_DISABLE_WATCHER").is_none()
}

/// Dir de datos del entorno de pruebas. Lo crea y lo marca la primera vez.
/// Falla (en vez de adoptarlo) si el dir ya contiene una DB sin la marca:
/// eso significa que alguien apuntó `SIMFLEET_DATA_DIR` a datos reales.
pub fn data_dir() -> anyhow::Result<PathBuf> {
    let dir = match std::env::var_os("SIMFLEET_DATA_DIR") {
        Some(p) if !p.is_empty() => PathBuf::from(p),
        _ => {
            let local = std::env::var_os("LOCALAPPDATA")
                .context("LOCALAPPDATA no disponible para el entorno de pruebas")?;
            PathBuf::from(local).join("SimFleet-Pruebas").join("data")
        }
    };
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("no se pudo crear el dir de pruebas {}", dir.display()))?;
    let marker = dir.join(MARKER_FILE);
    if !marker.is_file() {
        if dir.join(DB_FILE).exists() {
            bail!(
                "ENTORNO DE PRUEBAS: {} ya contiene una base de datos que NO es de \
                 pruebas (falta {}). Me niego a usarla para no pisar datos reales.",
                dir.display(),
                MARKER_FILE
            );
        }
        std::fs::write(&marker, MARKER_TEXT)
            .with_context(|| format!("no se pudo marcar {}", dir.display()))?;
    }
    Ok(dir)
}

/// Candado de la instancia PRINCIPAL: nunca abrir un dir de pruebas.
pub fn ensure_not_test_dir(dir: &Path) -> anyhow::Result<()> {
    if dir.join(MARKER_FILE).exists() {
        bail!(
            "{} es un dir del ENTORNO DE PRUEBAS ({}). La instancia principal no \
             lo abre para no mezclar datos de pruebas con los reales.",
            dir.display(),
            MARKER_FILE
        );
    }
    Ok(())
}

const REAL_SYNC_FILE: &str = "msfs-addons-data.json";
const TEST_SYNC_FILE: &str = "msfs-addons-data.PRUEBAS.json";

/// Snapshot de cloud sync (Drive appDataFolder y carpeta local) donde se
/// ESCRIBE: subir, auto-upload tras cada vuelo, purgar. En pruebas va a un
/// archivo propio y JAMÁS al real.
pub fn sync_upload_file_name() -> &'static str {
    if is_active() {
        TEST_SYNC_FILE
    } else {
        REAL_SYNC_FILE
    }
}

/// Snapshot que se LEE al bajar. En pruebas es el REAL (decisión del usuario:
/// probar la bajada con sus vuelos de verdad) — sólo lectura: ninguna ruta de
/// escritura usa este nombre.
pub fn sync_download_file_name() -> &'static str {
    REAL_SYNC_FILE
}

/// Nombre de la carpeta por defecto de los clips de Best Landings.
pub fn landings_folder_name() -> &'static str {
    if is_active() {
        "SimFleet Landings - PRUEBAS"
    } else {
        "SimFleet Landings"
    }
}

/// Script inyectado en TODOS los webviews de la instancia de pruebas antes de
/// cargar la página: el frontend lo lee de forma síncrona (sin esperar un IPC)
/// para elegir tablas/canales de Supabase de pruebas desde el primer render.
/// (25/10) En una compilación de desarrollo (`tauri dev`, la de
/// `dev-safe.ps1`) además se salta el bloqueo de Windows Hello (pedido del
/// usuario: sólo en pruebas). Un build de release lo pide siempre, aunque se
/// fuerce el modo pruebas.
pub fn js_init_script() -> &'static str {
    if cfg!(debug_assertions) {
        "window.__SIMFLEET_TEST_ENV__ = true; window.__SIMFLEET_NO_LOCK__ = true;"
    } else {
        "window.__SIMFLEET_TEST_ENV__ = true;"
    }
}
