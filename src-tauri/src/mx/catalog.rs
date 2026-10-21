//! (19/10) Etapa 3A — catálogos del Tech Log propio de SimFleet.
//!
//! Los catálogos (piezas con vida, consumibles, límites, MEL, checks) están
//! en `src-tauri/mx-catalogs/<familia>.json`, adaptados de Fenix TechLog
//! (Airbus) y PMDG TechLog (Boeing) para uso personal. Se embeben al
//! compilar. `detect_profile` elige familia, variante y motor a partir del
//! título / tipo ATC del avión.

use std::collections::HashMap;
use std::sync::OnceLock;

use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Family {
    A320,
    B737,
    B777,
    AWide,
    Generic,
}

impl Family {
    pub fn id(self) -> &'static str {
        match self {
            Family::A320 => "a320",
            Family::B737 => "b737",
            Family::B777 => "b777",
            Family::AWide => "awide",
            Family::Generic => "generic",
        }
    }
    pub fn parse(s: &str) -> Family {
        match s {
            "a320" => Family::A320,
            "b737" => Family::B737,
            "b777" => Family::B777,
            "awide" => Family::AWide,
            _ => Family::Generic,
        }
    }
    fn json(self) -> &'static str {
        match self {
            Family::A320 => include_str!("../../mx-catalogs/a320.json"),
            Family::B737 => include_str!("../../mx-catalogs/b737.json"),
            Family::B777 => include_str!("../../mx-catalogs/b777.json"),
            Family::AWide => include_str!("../../mx-catalogs/awide.json"),
            Family::Generic => include_str!("../../mx-catalogs/generic.json"),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CompDef {
    pub key: String,
    pub name: String,
    pub ata: i64,
    pub system: String,
    pub positions: Option<Vec<String>>,
    pub basis: Option<String>,
    pub life: f64,
    pub kind: Option<String>,
    pub mel: String,
    pub symptom: Option<String>,
    pub stress: Option<String>,
    pub degraded_mel: Option<String>,
    pub degraded_symptom: Option<String>,
    pub variants: Option<Vec<String>>,
}

impl CompDef {
    pub fn positions_or_single(&self) -> Vec<String> {
        match &self.positions {
            Some(p) if !p.is_empty() => p.clone(),
            _ => vec![String::new()],
        }
    }
    pub fn name_for(&self, pos: &str) -> String {
        self.name.replace("{pos}", pos).trim().to_string()
    }
    /// "life" | "wear" | "cabin"
    pub fn kind(&self) -> &str {
        self.kind.as_deref().unwrap_or("life")
    }
    pub fn basis(&self) -> &str {
        self.basis.as_deref().unwrap_or("flightHours")
    }
    pub fn applies_to(&self, variant: &str) -> bool {
        match &self.variants {
            Some(v) if !v.is_empty() => v.iter().any(|x| x.eq_ignore_ascii_case(variant)),
            _ => true,
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ConsDef {
    pub key: String,
    pub name: String,
    pub unit: String,
    pub positions: Option<Vec<String>>,
    pub capacity: Option<f64>,
    pub capacity_by_engine: Option<HashMap<String, f64>>,
    pub min_dispatch: Option<f64>,
    pub min_dispatch_by_engine: Option<HashMap<String, f64>>,
    pub low: Option<f64>,
    pub low_by_engine: Option<HashMap<String, f64>>,
    pub critical: Option<f64>,
    pub fills: bool,
    pub usage_basis: Option<String>,
    pub use_per_unit: f64,
    pub use_per_pax_hour: f64,
    pub service_label: Option<String>,
}

impl ConsDef {
    fn pick(single: Option<f64>, by: &Option<HashMap<String, f64>>, engine: &str) -> Option<f64> {
        by.as_ref()
            .and_then(|m| m.get(engine).copied().or_else(|| m.values().next().copied()))
            .or(single)
    }
    pub fn capacity(&self, engine: &str) -> f64 {
        Self::pick(self.capacity, &self.capacity_by_engine, engine).unwrap_or(100.0)
    }
    pub fn min_dispatch(&self, engine: &str) -> f64 {
        Self::pick(self.min_dispatch, &self.min_dispatch_by_engine, engine).unwrap_or(0.0)
    }
    pub fn low(&self, engine: &str) -> f64 {
        Self::pick(self.low, &self.low_by_engine, engine).unwrap_or(0.0)
    }
    pub fn positions_or_single(&self) -> Vec<String> {
        match &self.positions {
            Some(p) if !p.is_empty() => p.clone(),
            _ => vec![String::new()],
        }
    }
    pub fn name_for(&self, pos: &str) -> String {
        self.name.replace("{pos}", pos).trim().to_string()
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct VariantLimits {
    pub vmo: f64,
    pub mmo: f64,
    pub vle: f64,
    pub vlo_extend: f64,
    pub vlo_retract: f64,
    pub vfe: Vec<f64>,
    pub mlw_kg: f64,
    pub mtow_kg: f64,
    pub tailstrike_pitch: f64,
    pub seats: f64,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct EngineLimits {
    pub egt_takeoff_c: f64,
    pub egt_start_c: f64,
    pub egt_tolerance_c: f64,
    pub egt_over_limit_sec: f64,
    pub n1_max_pct: f64,
    pub egt_margin_new_c: f64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CommonLimits {
    pub repair_to_pct: Vec<f64>,
    pub worn_below_pct: f64,
    pub hard_landing_fpm: f64,
    pub severe_hard_landing_fpm: f64,
    pub hard_landing_g: f64,
    pub severe_hard_landing_g: f64,
    pub extreme_landing_fpm: f64,
    pub extreme_landing_g: f64,
    pub tyre_speed_kt: f64,
    pub brake_takeoff_max_c: f64,
    pub taxi_max_kt: f64,
    pub turn_max_kt: f64,
    pub reverse_min_kt: f64,
    pub g_max_clean: f64,
    pub g_min_clean: f64,
    pub g_max_flaps: f64,
    pub bank_near_ground_deg: f64,
}

impl Default for CommonLimits {
    fn default() -> Self {
        Self {
            repair_to_pct: vec![70.0, 50.0, 35.0],
            worn_below_pct: 25.0,
            hard_landing_fpm: 600.0,
            severe_hard_landing_fpm: 840.0,
            hard_landing_g: 2.6,
            severe_hard_landing_g: 2.86,
            extreme_landing_fpm: 1200.0,
            extreme_landing_g: 3.5,
            tyre_speed_kt: 195.0,
            brake_takeoff_max_c: 300.0,
            taxi_max_kt: 30.0,
            turn_max_kt: 15.0,
            reverse_min_kt: 70.0,
            g_max_clean: 2.5,
            g_min_clean: -1.0,
            g_max_flaps: 2.0,
            bank_near_ground_deg: 7.0,
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CheckDef {
    pub key: String,
    pub name: String,
    pub hours: Option<f64>,
    pub days: Option<f64>,
    pub flight_hours: Option<f64>,
    pub cycles: Option<f64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Limits {
    pub variants: HashMap<String, VariantLimits>,
    pub engines: HashMap<String, EngineLimits>,
    pub common: CommonLimits,
    pub checks: Vec<CheckDef>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MelItem {
    pub id: String,
    pub title: String,
    pub ata: i64,
    /// "noGo" | "a" | "b" | "c" | "d"
    pub category: String,
    pub fix_hours: Option<f64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Spares {
    pub common_ata: Vec<i64>,
    pub rare_ata: Vec<i64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Cabin {
    pub rows: HashMap<String, f64>,
    pub seat_letters: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Catalog {
    pub components: Vec<CompDef>,
    pub consumables: Vec<ConsDef>,
    pub limits: Limits,
    pub mel: Vec<MelItem>,
    pub spares: Spares,
    pub cabin: Cabin,
}

impl Catalog {
    pub fn component(&self, key: &str) -> Option<&CompDef> {
        self.components.iter().find(|c| c.key == key)
    }
    pub fn consumable(&self, key: &str) -> Option<&ConsDef> {
        self.consumables.iter().find(|c| c.key == key)
    }
    pub fn mel_item(&self, id: &str) -> Option<&MelItem> {
        self.mel.iter().find(|m| m.id == id)
    }
    pub fn variant(&self, v: &str) -> VariantLimits {
        self.limits
            .variants
            .get(v)
            .or_else(|| self.limits.variants.values().next())
            .cloned()
            .unwrap_or_default()
    }
    pub fn engine(&self, e: &str) -> EngineLimits {
        self.limits
            .engines
            .get(e)
            .or_else(|| self.limits.engines.values().next())
            .cloned()
            .unwrap_or_default()
    }
    /// Asientos del variante (para estimar pasajeros sin dato de carga).
    pub fn seats(&self, variant: &str) -> f64 {
        let v = self.variant(variant);
        if v.seats > 0.0 {
            return v.seats;
        }
        self.cabin.rows.get(variant).copied().unwrap_or(0.0) * self.cabin.seat_letters.len() as f64
    }
}

pub fn catalog(f: Family) -> &'static Catalog {
    static CACHE: OnceLock<HashMap<Family, Catalog>> = OnceLock::new();
    let map = CACHE.get_or_init(|| {
        [Family::A320, Family::B737, Family::B777, Family::AWide, Family::Generic]
            .into_iter()
            .map(|f| {
                let cat: Catalog = serde_json::from_str(f.json()).unwrap_or_else(|e| {
                    tracing::error!(target: "mx", "catálogo {} ilegible: {e}", f.id());
                    Catalog::default()
                });
                (f, cat)
            })
            .collect()
    });
    &map[&f]
}

/// Familia + variante + motor de un avión.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    pub family: Family,
    pub variant: String,
    pub engine: String,
}

/// Elige el perfil por el título / tipo ATC / modelo del avión.
pub fn detect_profile(parts: &[Option<&str>]) -> Profile {
    let hay: String = parts
        .iter()
        .flatten()
        .map(|s| s.to_lowercase())
        .collect::<Vec<_>>()
        .join(" ");
    let has = |n: &str| hay.contains(n);
    let p = |family, variant: &str, engine: &str| Profile {
        family,
        variant: variant.to_string(),
        engine: engine.to_string(),
    };
    // FlyByWire A32NX / A320neo.
    if has("a32nx") || has("flybywire") || has("fly by wire") || (has("a320") && has("neo")) {
        let v = if has("a321") { "A321" } else { "A320" };
        return p(Family::A320, v, "LEAP-1A");
    }
    // 737 MAX (iFly). "737-8" sólo si no sigue otra cifra ("737-800" es NG).
    let max8 = hay.match_indices("737-8").any(|(i, m)| {
        !hay[i + m.len()..].chars().next().is_some_and(|c| c.is_ascii_digit())
    });
    if has("737 max") || has("737max") || has("737-max") || max8 || has("b38m") || has("ifly") {
        return p(Family::B737, "737-8", "LEAP-1B");
    }
    if has("777") || has("b77") {
        let v = if has("300er") || has("77w") || has("-300") {
            "777-300ER"
        } else if has("200lr") || has("77l") {
            "777-200LR"
        } else if has("777f") || has("77f") || has("freighter") {
            "777F"
        } else {
            "777-200ER"
        };
        return p(Family::B777, v, "GE90");
    }
    if has("a350") || has("a359") {
        return p(Family::AWide, "A350-900", "TRENT-XWB");
    }
    if has("a310") {
        return p(Family::AWide, "A310-300", "CF6-80C2");
    }
    if has("a300") {
        return p(Family::AWide, "A300-600", "CF6-80C2");
    }
    if has("737") || has("b73") {
        let v = if has("-600") || has("736") {
            "737-600"
        } else if has("-700") || has("737-7") || has("73g") {
            "737-700"
        } else if has("-900") || has("737-9") || has("739") {
            "737-900"
        } else {
            "737-800"
        };
        return p(Family::B737, v, "CFM");
    }
    if has("a319") || has("a320") || has("a321") {
        let v = if has("a319") {
            "A319"
        } else if has("a321") {
            "A321"
        } else {
            "A320"
        };
        let e = if has("iae") || has("v2500") { "IAE" } else { "CFM" };
        return p(Family::A320, v, e);
    }
    p(Family::Generic, "GENERIC", "GENERIC")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalogs_parse() {
        for f in [Family::A320, Family::B737, Family::B777, Family::AWide, Family::Generic] {
            let c = catalog(f);
            assert!(c.components.len() > 100, "{} piezas", f.id());
            assert!(!c.limits.checks.is_empty(), "{} checks", f.id());
            assert!(c.mel_item("05-51-01").is_some(), "{} MEL", f.id());
        }
    }

    #[test]
    fn detects_profiles() {
        let d = |t: &str| detect_profile(&[Some(t)]);
        assert_eq!(d("FlyByWire A320neo (LEAP)").family, Family::A320);
        assert_eq!(d("iFly 737-MAX8 Ryanair").variant, "737-8");
        assert_eq!(d("PMDG 777-300ER Air Canada").variant, "777-300ER");
        assert_eq!(d("PMDG 777F FedEx").variant, "777F");
        assert_eq!(d("iniBuilds A350-900 Air France").variant, "A350-900");
        assert_eq!(d("Airbus A310-300 Lufthansa").variant, "A310-300");
        assert_eq!(d("PMDG 737-800 Ryanair").variant, "737-800");
        assert_eq!(d("Cessna 172 Skyhawk").family, Family::Generic);
    }
}
