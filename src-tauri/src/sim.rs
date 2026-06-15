//! (v5.1.0) Versión de MSFS activa elegida por el usuario.
//!
//! Afecta:
//!   · El filtro del catálogo de SceneryAddons (mostrar addons de 2020
//!     vs 2024 según la versión elegida).
//!   · La preferencia al detectar la carpeta Community (2020 vs 2024).
//!
//! Es un flag global atómico para que cualquier módulo lo lea sin
//! pasar estado por todos lados. Se inicializa al arrancar desde el
//! setting `pref_sim_version` y se actualiza cuando el usuario lo
//! cambia en Ajustes (que pide reinicio para reaplicar todo).

use std::sync::atomic::{AtomicU8, Ordering};

/// 0 = MSFS 2020 (default), 1 = MSFS 2024.
static SIM: AtomicU8 = AtomicU8::new(0);

/// Fija la versión desde un string del setting (`"msfs2024"` / `"2024"`
/// → 2024; cualquier otra cosa → 2020).
pub fn set_from_str(value: &str) {
    let v = value.to_lowercase();
    let code = if v.contains("2024") { 1 } else { 0 };
    SIM.store(code, Ordering::Relaxed);
}

/// `true` si la versión activa es MSFS 2024.
pub fn is_2024() -> bool {
    SIM.load(Ordering::Relaxed) == 1
}

/// Etiqueta corta de la versión activa.
pub fn label() -> &'static str {
    if is_2024() {
        "MSFS 2024"
    } else {
        "MSFS 2020"
    }
}
