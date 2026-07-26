//! (v6.2.52) Descarga de liveries de flightsim.to con la cuenta del usuario,
//! por un mecanismo FIABLE (el WebView2 embebido renderizaba en blanco y no era
//! depurable en vivo):
//!
//!   1. El front abre flightsim.to en el **navegador real** del usuario
//!      (`openExternal`) — ahí ya está logueado y la página carga bien.
//!   2. `start_livery_download_watch` **vigila la carpeta de Descargas**: cuando
//!      aparece un archivo NUEVO (`.zip/.rar/.7z`) y termina de escribirse, lo
//!      inspecciona; si contiene liveries PMDG/iFly, emite
//!      `livery-download://finished` → el front lo instala con el MISMO flujo del
//!      drag&drop (nested-zip, variante, carpeta `pmdg-livery-*`).
//!
//! Sólo actúa sobre archivos que REALMENTE son liveries — cualquier otra descarga
//! se ignora en silencio. La vigilancia dura ~25 min por click (se extiende si
//! vuelves a pulsar "Buscar liveries").

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tauri::{Emitter, Manager};

static WATCHING: AtomicBool = AtomicBool::new(false);
/// Epoch secs hasta cuando vigilar. Cada click lo empuja +25 min.
static DEADLINE: AtomicU64 = AtomicU64::new(0);

const WATCH_WINDOW_SECS: u64 = 25 * 60;

/// Arranca (o extiende) la vigilancia de la carpeta de Descargas para instalar
/// liveries automáticamente. Idempotente: si ya hay un vigilante, sólo extiende
/// la ventana.
#[tauri::command]
pub fn start_livery_download_watch(app: tauri::AppHandle) -> Result<(), String> {
    let deadline = now_secs() + WATCH_WINDOW_SECS;
    DEADLINE.store(deadline, Ordering::SeqCst);

    // Ya hay un vigilante corriendo → sólo extendimos la ventana.
    if WATCHING.swap(true, Ordering::SeqCst) {
        tracing::info!(target: "livery", "watch: ventana extendida hasta {}", deadline);
        return Ok(());
    }

    let dirs = downloads_dirs();
    if dirs.is_empty() {
        WATCHING.store(false, Ordering::SeqCst);
        return Err("No pude localizar la carpeta de Descargas".into());
    }
    tracing::info!(
        target: "livery",
        "watch: vigilando {:?} para liveries descargadas",
        dirs
    );

    std::thread::Builder::new()
        .name("livery-download-watch".into())
        .spawn(move || watch_loop(app, dirs))
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn watch_loop(app: tauri::AppHandle, dirs: Vec<PathBuf>) {
    // Baseline: los archivos que YA existen no se procesan (sólo descargas
    // nuevas tras pulsar el botón).
    let mut processed: HashSet<PathBuf> = HashSet::new();
    for d in &dirs {
        for f in list_archives(d) {
            processed.insert(f);
        }
    }
    // Tamaño visto por archivo, para detectar cuándo terminó de escribirse.
    let mut sizes: HashMap<PathBuf, u64> = HashMap::new();

    loop {
        std::thread::sleep(Duration::from_secs(3));
        if now_secs() > DEADLINE.load(Ordering::SeqCst) {
            break;
        }
        for d in &dirs {
            for f in list_archives(d) {
                if processed.contains(&f) {
                    continue;
                }
                let size = std::fs::metadata(&f).map(|m| m.len()).unwrap_or(0);
                let prev = sizes.insert(f.clone(), size);
                // Esperamos a que el tamaño se estabilice (2 lecturas iguales,
                // > 0) → descarga terminada.
                if prev != Some(size) || size == 0 {
                    continue;
                }
                processed.insert(f.clone());
                sizes.remove(&f);
                handle_new_archive(&app, &f);
            }
        }
    }

    WATCHING.store(false, Ordering::SeqCst);
    tracing::info!(target: "livery", "watch: vigilancia terminada");
}

/// Inspecciona un archivo recién descargado; si contiene liveries, dispara la
/// instalación en el front. Si no, lo ignora en silencio.
fn handle_new_archive(app: &tauri::AppHandle, path: &Path) {
    let state = app.state::<crate::AppState>();
    let insp = match crate::drop_install::inspect(path, None, &state.drop_sessions) {
        Ok(i) => i,
        Err(e) => {
            tracing::debug!(target: "livery", "watch: inspect falló {}: {}", path.display(), e);
            return;
        }
    };
    let has_livery = insp
        .items
        .iter()
        .any(|i| matches!(i.kind.as_str(), "pmdg_livery" | "ifly_livery"));
    // Cerramos la sesión del vigilante — el front re-inspecciona con su flujo.
    crate::drop_install::cancel(&insp.session_id, &state.drop_sessions);

    if has_livery {
        tracing::info!(
            target: "livery",
            "watch: livery detectada en descarga {} → instalando",
            path.display()
        );
        let _ = app.emit(
            "livery-download://finished",
            path.to_string_lossy().to_string(),
        );
    } else {
        tracing::debug!(
            target: "livery",
            "watch: descarga {} no es livery, ignorada",
            path.display()
        );
    }
}

/// Archivos comprimidos directamente en `dir` (no recursivo).
fn list_archives(dir: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    rd.flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.extension()
                    .and_then(|s| s.to_str())
                    .map(|e| matches!(e.to_ascii_lowercase().as_str(), "zip" | "rar" | "7z"))
                    .unwrap_or(false)
        })
        .collect()
}

/// Carpeta(s) de Descargas del usuario. Respeta la reubicación del known-folder
/// (el usuario puede tenerla en `D:\Downloads`) leyendo el registro, con
/// fallback a `%USERPROFILE%\Downloads`.
fn downloads_dirs() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    #[cfg(windows)]
    if let Some(p) = downloads_from_registry() {
        out.push(p);
    }
    if let Some(up) = std::env::var_os("USERPROFILE") {
        let p = Path::new(&up).join("Downloads");
        if !out.contains(&p) {
            out.push(p);
        }
    }
    out.into_iter().filter(|p| p.is_dir()).collect()
}

#[cfg(windows)]
fn downloads_from_registry() -> Option<PathBuf> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let key = hkcu
        .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Explorer\Shell Folders")
        .ok()?;
    // GUID del known-folder Downloads.
    let val: String = key
        .get_value("{374DE290-123F-4565-9164-39C4925E467B}")
        .ok()?;
    let trimmed = val.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(PathBuf::from(trimmed))
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
