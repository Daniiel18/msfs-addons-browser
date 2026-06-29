//! (v6.2.15) Diagnóstico de memoria — enumera el ÁRBOL de procesos de SimFleet
//! (el binario principal + los procesos hijo del WebView2 + cualquier proceso
//! que la app haya lanzado) con la RAM (working set) de cada uno. Sirve para
//! cazar fugas de memoria: el Task Manager agrupa los WebView2 bajo "SimFleet",
//! así que aquí los desglosamos para ver QUIÉN consume.
//!
//! El comando vuelca además el reporte al log (`target: "diag"`). En el frontend
//! está detrás de una clave de acceso (sección Diagnóstico de Ajustes).

use serde::Serialize;

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
