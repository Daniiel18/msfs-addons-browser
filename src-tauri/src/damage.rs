//! (v3.26.0 — P7.10) Analizador de daños / "vuelo forzado".
//!
//! MSFS 2020 NO expone daño estructural ni fallo de tren/flaps por
//! SimConnect (eso es exclusivo de MSFS 2024). Lo que SÍ expone y usamos
//! como proxy del maltrato al avión, capturado por track sample:
//!   · OVERSPEED WARNING    → se voló por encima de Vmo/Mmo.
//!   · WARNING OIL PRESSURE  → presión de aceite fuera de rango (daño motor).
//!   · STALL WARNING         → entrada en pérdida (migración 031).
//!   · G FORCE               → sobre-G estructural (ya en g_force).
//!
//! El veredicto resume el vuelo en LIMPIO / FORZADO / DAÑADO:
//!   · clean   — sin avisos y G dentro de rango normal.
//!   · forced  — avisos transitorios o G alta (cerca del límite).
//!   · damaged — fallo de aceite sostenido, sobrevelocidad sostenida o
//!               G fuera de los límites estructurales (>+3.0 / <-1.0).
//!   · no_data — el vuelo no capturó estos datos (viejos / imports VAS):
//!               no se penaliza, sólo se informa.

use serde::Serialize;
use sqlx::SqlitePool;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DamageReport {
    /// "clean" | "forced" | "damaged" | "no_data".
    pub verdict: String,
    /// false cuando el vuelo no capturó ninguno de estos datos.
    pub has_data: bool,
    pub overspeed_samples: i64,
    pub stall_samples: i64,
    pub oil_press_samples: i64,
    pub max_g: Option<f64>,
    pub min_g: Option<f64>,
    /// Claves i18n de las razones (el frontend las traduce).
    pub reasons: Vec<String>,
}

/// Analiza la traza de un vuelo y devuelve el veredicto de daños.
pub async fn analyze(pool: &SqlitePool, flight_id: i64) -> anyhow::Result<DamageReport> {
    let rows = sqlx::query_as::<_, (Option<f64>, Option<i64>, Option<i64>, Option<i64>)>(
        r#"
        SELECT g_force, overspeed_warning, oil_press_warning, stall_warning
        FROM flight_log_track
        WHERE flight_id = ?1
        ORDER BY ts ASC, id ASC
        "#,
    )
    .bind(flight_id)
    .fetch_all(pool)
    .await?;

    let mut overspeed = 0i64;
    let mut oil = 0i64;
    let mut stall = 0i64;
    let mut max_g: Option<f64> = None;
    let mut min_g: Option<f64> = None;
    let mut any_warn_col = false;
    let mut any_g = false;

    for (g, ov, op, st) in &rows {
        if let Some(g) = g {
            if g.is_finite() {
                any_g = true;
                max_g = Some(max_g.map_or(*g, |m: f64| m.max(*g)));
                min_g = Some(min_g.map_or(*g, |m: f64| m.min(*g)));
            }
        }
        if let Some(v) = ov {
            any_warn_col = true;
            if *v != 0 {
                overspeed += 1;
            }
        }
        if let Some(v) = op {
            any_warn_col = true;
            if *v != 0 {
                oil += 1;
            }
        }
        if let Some(v) = st {
            any_warn_col = true;
            if *v != 0 {
                stall += 1;
            }
        }
    }

    let has_data = any_warn_col || any_g;
    if !has_data {
        return Ok(DamageReport {
            verdict: "no_data".into(),
            has_data: false,
            overspeed_samples: 0,
            stall_samples: 0,
            oil_press_samples: 0,
            max_g: None,
            min_g: None,
            reasons: vec![],
        });
    }

    let mut reasons: Vec<String> = Vec::new();
    let mut damaged = false;
    let mut forced = false;

    // Presión de aceite: fallo de sistema. Sostenido (>=3 samples ~30s)
    // = daño; un blip puntual = aviso (forzado).
    if oil >= 3 {
        damaged = true;
        reasons.push("fb.damage.reason.oil_press".into());
    } else if oil >= 1 {
        forced = true;
        reasons.push("fb.damage.reason.oil_press_brief".into());
    }

    // Sobrevelocidad. Sostenida (>=6 samples ~60s) = daño estructural; un
    // aviso puntual = forzado.
    if overspeed >= 6 {
        damaged = true;
        reasons.push("fb.damage.reason.overspeed_sustained".into());
    } else if overspeed >= 1 {
        forced = true;
        reasons.push("fb.damage.reason.overspeed".into());
    }

    // Pérdida (stall). Siempre cuenta como forzado.
    if stall >= 3 {
        forced = true;
        reasons.push("fb.damage.reason.stall".into());
    } else if stall >= 1 {
        forced = true;
        reasons.push("fb.damage.reason.stall_brief".into());
    }

    // Fuerza-G. Límites típicos de transporte: +2.5 / -1.0 g. Por encima
    // con margen = daño estructural.
    if let Some(mx) = max_g {
        if mx > 3.0 {
            damaged = true;
            reasons.push("fb.damage.reason.overg".into());
        } else if mx > 2.5 {
            forced = true;
            reasons.push("fb.damage.reason.high_g".into());
        }
    }
    if let Some(mn) = min_g {
        if mn < -1.0 {
            damaged = true;
            reasons.push("fb.damage.reason.neg_overg".into());
        } else if mn < 0.0 {
            forced = true;
            reasons.push("fb.damage.reason.neg_g".into());
        }
    }

    let verdict = if damaged {
        "damaged"
    } else if forced {
        "forced"
    } else {
        "clean"
    };

    Ok(DamageReport {
        verdict: verdict.into(),
        has_data: true,
        overspeed_samples: overspeed,
        stall_samples: stall,
        oil_press_samples: oil,
        max_g,
        min_g,
        reasons,
    })
}
