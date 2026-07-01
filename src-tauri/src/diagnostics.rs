//! (v6.2.15) Diagnóstico de memoria — enumera el ÁRBOL de procesos de SimFleet
//! (el binario principal + los procesos hijo del WebView2 + cualquier proceso
//! que la app haya lanzado) con la RAM (working set) de cada uno. Sirve para
//! cazar fugas de memoria: el Task Manager agrupa los WebView2 bajo "SimFleet",
//! así que aquí los desglosamos para ver QUIÉN consume.
//!
//! El comando vuelca además el reporte al log (`target: "diag"`). En el frontend
//! está detrás de una clave de acceso (sección Diagnóstico de Ajustes).

use serde::Serialize;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};

// ─────────────────────────────────────────────────────────────────────────
// (v6.2.16) Métricas de ACTIVIDAD de los subsistemas internos. No es memoria
// por subsistema (todos comparten el heap del proceso), sino contadores en
// vivo para ver cuál está trabajando de más (o atascado). Cada subsistema
// llama a su `note_*` en su punto caliente; el panel de Diagnóstico los lee.
// ─────────────────────────────────────────────────────────────────────────

static SIMCONNECT_CONNECTED: AtomicBool = AtomicBool::new(false);
static SIMCONNECT_EMITS: AtomicU64 = AtomicU64::new(0);
static SIMCONNECT_LAST_EMIT_MS: AtomicI64 = AtomicI64::new(0);
static TRACK_ROWS_WRITTEN: AtomicU64 = AtomicU64::new(0);
static CLOUD_UPLOADS: AtomicU64 = AtomicU64::new(0);
static CLOUD_LAST_BYTES: AtomicU64 = AtomicU64::new(0);
static CLOUD_LAST_MS: AtomicI64 = AtomicI64::new(0);
static CLOUD_BUSY: AtomicBool = AtomicBool::new(false);
static RECORDER_ARMED: AtomicBool = AtomicBool::new(false);

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// SimConnect emitió un estado al frontend (con su flag de conexión real).
pub fn note_simconnect_emit(connected: bool) {
    SIMCONNECT_CONNECTED.store(connected, Ordering::Relaxed);
    SIMCONNECT_EMITS.fetch_add(1, Ordering::Relaxed);
    SIMCONNECT_LAST_EMIT_MS.store(now_ms(), Ordering::Relaxed);
}

/// Se escribieron `n` puntos de track en la DB.
pub fn note_track_rows(n: u64) {
    TRACK_ROWS_WRITTEN.fetch_add(n, Ordering::Relaxed);
}

/// La subida a la nube empezó (`true`) o terminó (`false`).
pub fn note_cloud_busy(busy: bool) {
    CLOUD_BUSY.store(busy, Ordering::Relaxed);
}

/// Se completó una subida del snapshot a la nube de `bytes`.
pub fn note_cloud_upload(bytes: u64) {
    CLOUD_UPLOADS.fetch_add(1, Ordering::Relaxed);
    CLOUD_LAST_BYTES.store(bytes, Ordering::Relaxed);
    CLOUD_LAST_MS.store(now_ms(), Ordering::Relaxed);
}

/// El grabador de aterrizajes está armado (`true`) o no.
pub fn note_recorder_armed(armed: bool) {
    RECORDER_ARMED.store(armed, Ordering::Relaxed);
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppMetrics {
    pub simconnect_connected: bool,
    pub simconnect_emits: u64,
    pub simconnect_last_emit_ms: i64,
    pub track_rows_written: u64,
    pub cloud_busy: bool,
    pub cloud_uploads: u64,
    pub cloud_last_bytes: u64,
    pub cloud_last_ms: i64,
    pub recorder_armed: bool,
    pub now_ms: i64,
}

/// Lee las métricas de actividad de los subsistemas internos (backend).
#[tauri::command]
pub fn diagnostics_app_metrics() -> AppMetrics {
    AppMetrics {
        simconnect_connected: SIMCONNECT_CONNECTED.load(Ordering::Relaxed),
        simconnect_emits: SIMCONNECT_EMITS.load(Ordering::Relaxed),
        simconnect_last_emit_ms: SIMCONNECT_LAST_EMIT_MS.load(Ordering::Relaxed),
        track_rows_written: TRACK_ROWS_WRITTEN.load(Ordering::Relaxed),
        cloud_busy: CLOUD_BUSY.load(Ordering::Relaxed),
        cloud_uploads: CLOUD_UPLOADS.load(Ordering::Relaxed),
        cloud_last_bytes: CLOUD_LAST_BYTES.load(Ordering::Relaxed),
        cloud_last_ms: CLOUD_LAST_MS.load(Ordering::Relaxed),
        recorder_armed: RECORDER_ARMED.load(Ordering::Relaxed),
        now_ms: now_ms(),
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcInfo {
    pub pid: u32,
    pub parent_pid: Option<u32>,
    pub name: String,
    pub memory_bytes: u64,
    /// true si es el proceso principal de SimFleet (el binario Rust).
    pub is_main: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcReport {
    pub total_bytes: u64,
    pub count: usize,
    pub processes: Vec<ProcInfo>,
}

/// Enumera el árbol de procesos propio (PID actual + todos sus descendientes) y
/// reporta la RAM de cada uno, ordenado de mayor a menor. Loguea el reporte.
#[tauri::command]
pub fn diagnostics_process_tree() -> ProcReport {
    use std::collections::HashMap;
    use sysinfo::{get_current_pid, ProcessRefreshKind, ProcessesToUpdate, System};

    let mut sys = System::new();
    sys.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::new().with_memory(),
    );

    let me = match get_current_pid() {
        Ok(p) => p.as_u32(),
        Err(_) => {
            return ProcReport {
                total_bytes: 0,
                count: 0,
                processes: Vec::new(),
            }
        }
    };

    // pid -> parent pid (para resolver descendencia por cadena de padres).
    let mut parent_of: HashMap<u32, Option<u32>> = HashMap::new();
    for (pid, proc) in sys.processes() {
        parent_of.insert(pid.as_u32(), proc.parent().map(|p| p.as_u32()));
    }
    // ¿`start` es el proceso actual o un descendiente suyo?
    let is_ours = |start: u32| -> bool {
        if start == me {
            return true;
        }
        let mut cur = start;
        for _ in 0..64 {
            match parent_of.get(&cur).copied().flatten() {
                Some(p) if p == me => return true,
                Some(p) => cur = p,
                None => return false,
            }
        }
        false
    };

    let mut processes: Vec<ProcInfo> = Vec::new();
    let mut total = 0u64;
    for (pid, proc) in sys.processes() {
        let pu = pid.as_u32();
        if is_ours(pu) {
            let mem = proc.memory();
            total += mem;
            processes.push(ProcInfo {
                pid: pu,
                parent_pid: proc.parent().map(|p| p.as_u32()),
                name: proc.name().to_string_lossy().into_owned(),
                memory_bytes: mem,
                is_main: pu == me,
            });
        }
    }
    processes.sort_by(|a, b| b.memory_bytes.cmp(&a.memory_bytes));

    tracing::info!(
        target: "diag",
        "Árbol de procesos SimFleet: {} procesos, total {:.1} MB",
        processes.len(),
        total as f64 / 1_048_576.0
    );
    for p in &processes {
        tracing::info!(
            target: "diag",
            "  {} (pid {}, parent {:?}) -> {:.1} MB",
            p.name,
            p.pid,
            p.parent_pid,
            p.memory_bytes as f64 / 1_048_576.0
        );
    }

    ProcReport {
        total_bytes: total,
        count: processes.len(),
        processes,
    }
}
