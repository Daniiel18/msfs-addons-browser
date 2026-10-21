//! (19/10) Etapa 3A — registrador en vuelo del Tech Log propio.
//!
//! Vive en memoria mientras el vuelo está abierto. El watcher de SimConnect
//! lo alimenta con cada muestra válida (no replay) y le avisa al abrir y al
//! cerrar el vuelo. Acumula contadores, momentos de estrés, excedencias (con
//! antirrebote, como el ExceedanceMonitor de TechLog) y aterrizajes; al
//! cerrar entrega un `FlightRecord` que `store::commit_flight` aplica.
//!
//! "Solo registro": no toca el avión en ningún momento.

use std::collections::HashMap;
use std::sync::Mutex;

use serde::Serialize;

use super::catalog::{catalog, detect_profile, Catalog, Profile};

/// Muestra que construye el watcher con su flujo principal (~4 Hz).
#[derive(Debug, Clone, Copy, Default)]
pub struct WatcherSample {
    pub sim_time_s: f64,
    pub on_ground: bool,
    pub ias: f64,
    pub gs: f64,
    pub alt_ft: f64,
    pub agl_ft: f64,
    pub vs_fpm: f64,
    /// Pitch con morro ARRIBA positivo (SimConnect lo da al revés).
    pub pitch_up_deg: f64,
    pub bank_deg: f64,
    pub g: f64,
    pub flaps_pct: f64,
    pub gear_handle_down: bool,
    pub eng_running: [bool; 2],
    pub n1: [f64; 2],
    pub egt: [f64; 2],
    pub overspeed_warning: bool,
    pub stall_warning: bool,
    pub precip_state: f64,
}

/// Flujo extra del Tech Log (definición aparte en el watcher).
#[derive(Debug, Clone, Copy, Default)]
pub struct Extra {
    pub valid: bool,
    pub mach: f64,
    pub flaps_index: f64,
    /// 0..1
    pub gear_pct: f64,
    /// 0..100
    pub rev_pct: [f64; 2],
    pub wheel_rpm: [f64; 2],
    /// 0..100
    pub apu_pct: f64,
    pub weight_lb: f64,
    pub body_vx_fps: f64,
    pub body_vz_fps: f64,
    pub on_runway: bool,
    pub oat_c: f64,
    /// 0..100
    pub spoilers_pct: f64,
}

/// Flujos opcionales (pueden no existir en algún simulador).
#[derive(Debug, Clone, Copy, Default)]
pub struct Optional {
    pub brakes_valid: bool,
    pub brake_temp_c: [f64; 2],
    pub warn_valid: bool,
    pub flap_speed_exceeded: bool,
    pub gear_speed_exceeded: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FlightEvent {
    pub phase: String,
    pub kind: String,
    /// "Info" | "Caution" | "Warning"
    pub severity: String,
    pub text: String,
}

/// A qué piezas afecta un abuso.
#[derive(Debug, Clone)]
pub enum Target {
    Key(&'static str),
    /// Piezas del tren (GEAR_*, LGCIU, amortiguadores).
    Gear,
    /// Sección caliente del motor (HPT, NGV, combustor).
    HotSection,
}

#[derive(Debug, Clone)]
pub enum Effect {
    /// Abre un defecto con el ítem MEL dado.
    Raise { mel: &'static str, pos: Option<String>, detail: String },
    /// Consume una fracción de vida (0..1).
    Abuse { target: Target, pos: Option<String>, share: f64, why: String },
    /// Daño directo a neumáticos (% de desgaste).
    Tyres { main: bool, pct: f64, why: String },
}

#[derive(Debug, Clone, Default)]
pub struct LandingRec {
    pub sink_fpm: f64,
    pub peak_g: f64,
    pub weight_kg: f64,
    pub crab_deg: f64,
    pub gs_kt: f64,
    pub pitch_up_deg: f64,
    pub bounces: u32,
    pub derot_deg_s: f64,
    pub brake_energy_mj: f64,
    pub wet: bool,
}

/// Resultado del vuelo para `store::commit_flight`.
#[derive(Debug, Clone, Default)]
pub struct FlightRecord {
    pub flight_id: i64,
    pub profile: Option<Profile>,
    pub block_h: f64,
    pub air_h: f64,
    pub eng_h: [f64; 2],
    pub eng_starts: [u32; 2],
    pub eng_ran: [bool; 2],
    pub apu_h: f64,
    pub apu_starts: u32,
    pub rev_deploys: [u32; 2],
    pub takeoffs: u32,
    pub toga_takeoffs: u32,
    pub landings: Vec<LandingRec>,
    /// Horas extra de motor por carga (TOGA/ascenso) → piezas calientes.
    pub workload_extra_h: [f64; 2],
    pub taxi_m: f64,
    pub stress: Vec<(String, Option<usize>)>,
    pub effects: Vec<Effect>,
    pub events: Vec<FlightEvent>,
}

#[derive(Debug, Clone, Copy)]
struct Episode {
    start: f64,
    peak: f64,
    limit: f64,
}

struct Session {
    rec: FlightRecord,
    last_t: Option<f64>,
    airborne: bool,
    eng_was: [bool; 2],
    start_window: [f64; 2],
    apu_was: bool,
    rev_was: [bool; 2],
    takeoff_stress_done: bool,
    climb_stress_done: bool,
    takeoff_max_n1: f64,
    prev_flaps: Option<i64>,
    prev_gear: Option<bool>,
    last_ground_t: f64,
    last_td_t: Option<f64>,
    // Aterrizaje en curso
    td: Option<LandingRec>,
    td_t: f64,
    td_rollout_s: f64,
    td_rev_s: f64,
    td_prev_pitch: Option<f64>,
    wheels_spun: [bool; 2],
    episodes: HashMap<String, Episode>,
    brakes: BrakeThermalModel,
}

/// Temperatura de frenos calculada a partir de la propia frenada, como el
/// `BrakeThermalModel` de TechLog. Se usa cuando el simulador no la da
/// (MSFS 2020, o aviones que dejan las simvars a 0).
#[derive(Debug, Clone, Copy)]
struct BrakeThermalModel {
    temp_c: [f64; 4],
    last_gs: Option<f64>,
    seeded: bool,
}

impl Default for BrakeThermalModel {
    fn default() -> Self {
        Self { temp_c: [15.0; 4], last_gs: None, seeded: false }
    }
}

impl BrakeThermalModel {
    /// Capacidad calorífica de un freno (MJ/°C).
    const HEAT_CAPACITY_MJ_PER_C: f64 = 0.075;

    fn step(&mut self, s: &WatcherSample, x: &Extra, dt: f64) {
        if !self.seeded {
            self.temp_c = [x.oat_c; 4];
            self.seeded = true;
        }
        if dt <= 0.0 || dt > 5.0 {
            self.last_gs = Some(s.gs);
            return;
        }
        if let Some(last) = self.last_gs.filter(|_| s.on_ground && s.gs > 2.0) {
            let v = s.gs * KT_TO_MS;
            let decel = (last - s.gs) * KT_TO_MS / dt;
            let mut reverse = 0.0;
            for i in 0..2 {
                if x.rev_pct[i] > 50.0 {
                    reverse += (s.n1[i] - 20.0).max(0.0) / 50.0 * 0.6;
                }
            }
            let drag = 4.5e-5 * v * v * if x.spoilers_pct > 50.0 { 2.2 } else { 1.0 };
            let rolling = 0.15;
            let braking = decel - reverse - drag - rolling;
            if braking > 0.0 {
                let mass = if x.weight_lb * LB_TO_KG > 1000.0 { x.weight_lb * LB_TO_KG } else { 60000.0 };
                let mj = mass * braking * v * dt / 4.0 / 1_000_000.0;
                for (j, t) in self.temp_c.iter_mut().enumerate() {
                    // Los frenos interiores (2 y 3) se calientan un poco más.
                    *t += mj / Self::HEAT_CAPACITY_MJ_PER_C * if j == 1 || j == 2 { 1.02 } else { 0.98 };
                }
            }
        }
        let tau = if s.on_ground { 3600.0 } else if x.gear_pct > 0.5 { 900.0 } else { 1800.0 };
        let k = 1.0 - (-dt / tau).exp();
        for t in self.temp_c.iter_mut() {
            *t += (x.oat_c - *t) * k;
        }
        self.last_gs = Some(s.gs);
    }

    fn hottest(&self) -> f64 {
        self.temp_c.iter().copied().fold(f64::MIN, f64::max)
    }
}

/// Temperaturas del simulador sólo si son creíbles (misma regla que TechLog:
/// MSFS 2020 no tiene la simvar y devuelve 0 en las dos).
fn usable_sim_brakes(o: &Optional) -> Option<f64> {
    let [l, r] = o.brake_temp_c;
    let ok = |v: f64| v.is_finite() && v > -70.0 && v < 2000.0;
    (o.brakes_valid && ok(l) && ok(r) && (l != 0.0 || r != 0.0)).then(|| l.max(r))
}

static SESSION: Mutex<Option<Session>> = Mutex::new(None);
static EXTRA: Mutex<Extra> = Mutex::new(Extra {
    valid: false,
    mach: 0.0,
    flaps_index: 0.0,
    gear_pct: 0.0,
    rev_pct: [0.0; 2],
    wheel_rpm: [0.0; 2],
    apu_pct: 0.0,
    weight_lb: 0.0,
    body_vx_fps: 0.0,
    body_vz_fps: 0.0,
    on_runway: false,
    oat_c: 15.0,
    spoilers_pct: 0.0,
});
static OPTIONAL: Mutex<Optional> = Mutex::new(Optional {
    brakes_valid: false,
    brake_temp_c: [0.0; 2],
    warn_valid: false,
    flap_speed_exceeded: false,
    gear_speed_exceeded: false,
});
static AIRCRAFT: Mutex<Option<(Option<String>, Option<String>)>> = Mutex::new(None);

pub fn set_extra(e: Extra) {
    if let Ok(mut g) = EXTRA.lock() {
        *g = Extra { valid: true, ..e };
    }
}
pub fn set_brakes(temps: [f64; 2]) {
    if let Ok(mut g) = OPTIONAL.lock() {
        g.brakes_valid = true;
        g.brake_temp_c = temps;
    }
}
pub fn set_speed_warnings(flaps: bool, gear: bool) {
    if let Ok(mut g) = OPTIONAL.lock() {
        g.warn_valid = true;
        g.flap_speed_exceeded = flaps;
        g.gear_speed_exceeded = gear;
    }
}
/// Título / tipo ATC del avión cargado (del flujo de metadatos).
pub fn set_aircraft(title: Option<String>, atc_type: Option<String>) {
    if let Ok(mut g) = AIRCRAFT.lock() {
        *g = Some((title, atc_type));
    }
}

fn current_profile() -> Option<Profile> {
    let g = AIRCRAFT.lock().ok()?;
    let (t, a) = g.as_ref()?;
    Some(detect_profile(&[t.as_deref(), a.as_deref()]))
}

/// El watcher abrió un vuelo (OUT u OFF sin OUT).
pub fn flight_opened(flight_id: i64) {
    let rec = FlightRecord {
        flight_id,
        profile: current_profile(),
        ..Default::default()
    };
    if let Ok(mut g) = SESSION.lock() {
        *g = Some(Session {
            rec,
            last_t: None,
            airborne: false,
            eng_was: [false; 2],
            start_window: [0.0; 2],
            apu_was: false,
            rev_was: [false; 2],
            takeoff_stress_done: false,
            climb_stress_done: false,
            takeoff_max_n1: 0.0,
            prev_flaps: None,
            prev_gear: None,
            last_ground_t: 0.0,
            last_td_t: None,
            td: None,
            td_t: 0.0,
            td_rollout_s: 0.0,
            td_rev_s: 0.0,
            td_prev_pitch: None,
            wheels_spun: [false; 2],
            episodes: HashMap::new(),
            brakes: BrakeThermalModel::default(),
        });
    }
    tracing::info!(target: "mx", "registro de mantenimiento abierto (vuelo {flight_id})");
}

/// Cada muestra válida del watcher (no replay).
pub fn feed(s: WatcherSample) {
    let extra = EXTRA.lock().map(|g| *g).unwrap_or_default();
    let opt = OPTIONAL.lock().map(|g| *g).unwrap_or_default();
    let Ok(mut g) = SESSION.lock() else { return };
    let Some(sess) = g.as_mut() else { return };
    if sess.rec.profile.is_none() {
        sess.rec.profile = current_profile();
    }
    sess.step(&s, &extra, &opt);
}

/// Cierra el vuelo: entrega el registro (si había sesión para él).
pub fn take(flight_id: i64) -> Option<FlightRecord> {
    let mut g = SESSION.lock().ok()?;
    let sess = g.as_mut()?;
    if sess.rec.flight_id != flight_id {
        return None;
    }
    let mut sess = g.take()?;
    sess.finish();
    Some(sess.rec)
}

const KT_TO_MS: f64 = 0.514444;
const LB_TO_KG: f64 = 0.453592;

impl Session {
    fn cat(&self) -> &'static Catalog {
        catalog(self.rec.profile.as_ref().map(|p| p.family).unwrap_or(super::catalog::Family::Generic))
    }

    fn phase(&self, s: &WatcherSample) -> String {
        if s.on_ground {
            if s.gs > 40.0 {
                if self.rec.takeoffs > 0 && !self.airborne && self.rec.landings.len() as u32 + u32::from(self.td.is_some()) >= self.rec.takeoffs {
                    "LANDING".into()
                } else {
                    "TAKEOFF".into()
                }
            } else {
                "TAXI".into()
            }
        } else if s.vs_fpm > 500.0 {
            "CLIMB".into()
        } else if s.vs_fpm < -500.0 {
            if s.agl_ft < 3000.0 { "APPROACH".into() } else { "DESCENT".into() }
        } else {
            "CRUISE".into()
        }
    }

    fn event(&mut self, s: &WatcherSample, kind: &str, severity: &str, text: String) {
        let phase = self.phase(s);
        self.rec.events.push(FlightEvent {
            phase,
            kind: kind.into(),
            severity: severity.into(),
            text,
        });
    }

    fn stress(&mut self, moment: &str, engine: Option<usize>) {
        self.rec.stress.push((moment.to_string(), engine));
    }

    fn step(&mut self, s: &WatcherSample, x: &Extra, o: &Optional) {
        let gap = match self.last_t {
            Some(prev) if s.sim_time_s > prev => s.sim_time_s - prev,
            _ => 0.0,
        };
        let dt = gap.min(5.0);
        self.last_t = Some(s.sim_time_s);
        // Huecos de más de 5 s (pausa, carga) no calientan los frenos.
        self.brakes.step(s, x, gap);
        let t = s.sim_time_s;
        let cat = self.cat();
        let profile = self.rec.profile.clone();
        let (variant, engine) = profile
            .as_ref()
            .map(|p| (p.variant.clone(), p.engine.clone()))
            .unwrap_or_else(|| ("GENERIC".into(), "GENERIC".into()));
        let lim = cat.variant(&variant);
        let eng = cat.engine(&engine);
        let common = cat.limits.common.clone();

        // --- Contadores ---------------------------------------------------
        let h = dt / 3600.0;
        self.rec.block_h += h;
        if self.airborne {
            self.rec.air_h += h;
        }
        for i in 0..2 {
            if s.eng_running[i] {
                self.rec.eng_h[i] += h;
                self.rec.eng_ran[i] = true;
                // Carga del motor: TOGA ×3, ascenso ×1.3 (TechLog: el
                // despegue gasta varias veces su tiempo, el crucero 1:1).
                let n1 = s.n1[i];
                let factor = if n1 >= 95.0 { 3.0 } else if n1 >= 88.0 && !s.on_ground { 1.3 } else { 1.0 };
                self.rec.workload_extra_h[i] += h * (factor - 1.0);
                if !self.eng_was[i] {
                    self.rec.eng_starts[i] += 1;
                    self.start_window[i] = 90.0;
                    self.stress("start", Some(i + 1));
                }
            }
            self.eng_was[i] = s.eng_running[i];
            if self.start_window[i] > 0.0 {
                self.start_window[i] -= dt;
            }
            let rev_on = x.valid && x.rev_pct[i] > 50.0;
            if rev_on && !self.rev_was[i] {
                self.rec.rev_deploys[i] += 1;
                self.stress("reverse", Some(i + 1));
            }
            self.rev_was[i] = rev_on;
        }
        if x.valid {
            let apu_on = x.apu_pct > 90.0;
            if apu_on {
                self.rec.apu_h += h;
            }
            if apu_on && !self.apu_was {
                self.rec.apu_starts += 1;
                self.stress("apu", None);
            }
            self.apu_was = apu_on;
        }

        // --- Mandos: tren y flaps ------------------------------------------
        let flaps_idx = if x.valid { x.flaps_index.round() as i64 } else { (s.flaps_pct * 4.0).round() as i64 };
        if let Some(p) = self.prev_flaps {
            if p != flaps_idx {
                self.stress("flaps", None);
            }
        }
        self.prev_flaps = Some(flaps_idx);
        if let Some(p) = self.prev_gear {
            if p != s.gear_handle_down {
                self.stress("gear", None);
            }
        }
        self.prev_gear = Some(s.gear_handle_down);

        // --- Despegue / aterrizaje ------------------------------------------
        if s.on_ground {
            self.last_ground_t = t;
            if s.gs > 30.0 && s.n1[0].max(s.n1[1]) > 75.0 && !self.takeoff_stress_done && self.td.is_none() {
                self.takeoff_stress_done = true;
                self.stress("takeoff", None);
            }
            if s.gs > 30.0 && self.td.is_none() {
                self.takeoff_max_n1 = self.takeoff_max_n1.max(s.n1[0].max(s.n1[1]));
            }
            if !x.on_runway && s.gs > 1.0 && s.gs < 45.0 {
                self.rec.taxi_m += s.gs * KT_TO_MS * dt;
            }
        }
        if !s.on_ground && s.agl_ft > 15.0 && !self.airborne {
            // Rebote: despega de nuevo a los pocos segundos del toque.
            if let (Some(td), Some(tdt)) = (self.td.as_mut(), self.last_td_t) {
                if t - tdt < 6.0 {
                    td.bounces += 1;
                    self.airborne = true;
                    return;
                }
            }
            self.airborne = true;
            self.rec.takeoffs += 1;
            if self.takeoff_max_n1 >= 95.0 {
                self.rec.toga_takeoffs += 1;
            }
            self.takeoff_max_n1 = 0.0;
            self.climb_stress_done = false;
            self.takeoff_stress_done = true;
        }
        if self.airborne && !self.climb_stress_done && s.agl_ft > 1500.0 && s.vs_fpm > 0.0 {
            self.climb_stress_done = true;
            self.stress("climb", None);
        }
        if self.airborne && s.on_ground {
            self.airborne = false;
            let is_bounce = self.last_td_t.is_some_and(|tdt| t - tdt < 6.0) && self.td.is_some();
            if !is_bounce {
                // Toque nuevo.
                self.finish_landing();
                let crab = if x.valid && x.body_vz_fps.abs() >= 1.0 {
                    x.body_vx_fps.atan2(x.body_vz_fps).to_degrees()
                } else {
                    0.0
                };
                self.td = Some(LandingRec {
                    sink_fpm: (-s.vs_fpm).max(0.0),
                    peak_g: s.g,
                    weight_kg: x.weight_lb * LB_TO_KG,
                    crab_deg: crab,
                    gs_kt: s.gs,
                    pitch_up_deg: s.pitch_up_deg,
                    wet: s.precip_state >= 4.0,
                    ..Default::default()
                });
                self.td_t = t;
                self.td_rollout_s = 0.0;
                self.td_rev_s = 0.0;
                self.td_prev_pitch = Some(s.pitch_up_deg);
                self.stress("landing", None);
                self.takeoff_stress_done = false;
            }
            self.last_td_t = Some(t);
        }
        if let Some(td) = self.td.as_mut() {
            let since = t - self.td_t;
            if since <= 3.0 {
                td.peak_g = td.peak_g.max(s.g);
                td.pitch_up_deg = td.pitch_up_deg.max(s.pitch_up_deg);
            }
            if s.on_ground && since <= 15.0 && dt > 0.0 {
                if let Some(pp) = self.td_prev_pitch {
                    if s.pitch_up_deg > 0.3 {
                        td.derot_deg_s = td.derot_deg_s.max((pp - s.pitch_up_deg) / dt);
                    }
                }
                self.td_prev_pitch = Some(s.pitch_up_deg);
            }
            if s.on_ground {
                self.td_rollout_s += dt;
                if x.valid && (x.rev_pct[0] > 50.0 || x.rev_pct[1] > 50.0) {
                    self.td_rev_s += dt;
                }
            }
            if s.on_ground && (s.gs < 30.0 || since > 60.0) {
                self.finish_landing();
            }
        }

        // --- Excedencias -----------------------------------------------------
        let airborne = self.airborne;
        let ias = s.ias;
        // VMO / MMO (o el aviso del simulador si no hay límite).
        if lim.vmo > 0.0 {
            self.check(s, "VMO", airborne && ias - lim.vmo > 2.0, t, ias, lim.vmo, 2.0);
            if lim.mmo > 0.0 && x.valid {
                self.check(s, "MMO", airborne && x.mach - lim.mmo > 0.004, t, x.mach, lim.mmo, 2.0);
            }
        } else {
            self.check(s, "OVERSPEED", airborne && s.overspeed_warning, t, ias, 0.0, 2.0);
        }
        // VFE
        if !lim.vfe.is_empty() && x.valid && flaps_idx > 0 {
            let i = (flaps_idx as usize).min(lim.vfe.len() - 1);
            let vfe = lim.vfe[i];
            if vfe > 0.0 {
                self.check(s, "VFE", airborne && ias > vfe + 3.0, t, ias, vfe, 2.0);
            }
        } else if o.warn_valid {
            self.check(s, "VFE_WARN", airborne && o.flap_speed_exceeded, t, ias, 0.0, 2.0);
        }
        // VLE / VLO
        let gear_pct = if x.valid { x.gear_pct } else if s.gear_handle_down { 1.0 } else { 0.0 };
        if lim.vle > 0.0 {
            self.check(s, "VLE", airborne && gear_pct >= 0.95 && ias > lim.vle + 3.0, t, ias, lim.vle, 2.0);
            let vlo = if s.gear_handle_down { lim.vlo_extend } else { lim.vlo_retract };
            if x.valid && vlo > 0.0 {
                self.check(s, "VLO", airborne && gear_pct > 0.05 && gear_pct < 0.95 && ias > vlo + 3.0, t, ias, vlo, 1.0);
            }
        } else if o.warn_valid {
            self.check(s, "VLE_WARN", airborne && o.gear_speed_exceeded, t, ias, 0.0, 2.0);
        }
        // Factor de carga
        let gmax = if flaps_idx > 0 { common.g_max_flaps } else { common.g_max_clean };
        let in_air_settled = airborne && s.agl_ft > 15.0 && t - self.last_ground_t > 3.0;
        self.check(s, "GMAX", in_air_settled && s.g > gmax + 0.05, t, s.g, gmax, 0.3);
        self.check(s, "GMIN", in_air_settled && s.g < common.g_min_clean - 0.05, t, -s.g, -common.g_min_clean, 0.3);
        // Cerca del suelo
        self.check(s, "POD", s.agl_ft < 3.0 && s.gs > 40.0 && s.bank_deg.abs() > common.bank_near_ground_deg, t, s.bank_deg.abs(), common.bank_near_ground_deg, 0.2);
        if lim.tailstrike_pitch > 0.0 {
            self.check(s, "TAILSTRIKE", s.on_ground && s.gs > 40.0 && s.pitch_up_deg > lim.tailstrike_pitch, t, s.pitch_up_deg, lim.tailstrike_pitch, 0.1);
        }
        self.check(s, "TYRE_SPEED", s.on_ground && s.gs > common.tyre_speed_kt, t, s.gs, common.tyre_speed_kt, 0.2);
        if x.valid {
            for i in 0..2 {
                if x.wheel_rpm[i] > 100.0 {
                    self.wheels_spun[i] = true;
                }
            }
            if s.on_ground && s.gs < 3.0 {
                self.wheels_spun = [false; 2];
            }
            let key_l = "SKID_L";
            let key_r = "SKID_R";
            let (sl, sr) = (self.wheels_spun[0], self.wheels_spun[1]);
            self.check(s, key_l, s.on_ground && s.gs > 25.0 && sl && x.wheel_rpm[0] < 5.0, t, s.gs, 25.0, 1.0);
            self.check(s, key_r, s.on_ground && s.gs > 25.0 && sr && x.wheel_rpm[1] < 5.0, t, s.gs, 25.0, 1.0);
            self.check(s, "TAXI_SPEED", s.on_ground && !x.on_runway && s.gs > common.taxi_max_kt, t, s.gs, common.taxi_max_kt, 5.0);
            for i in 0..2 {
                let key = if i == 0 { "REV_LOW_1" } else { "REV_LOW_2" };
                self.check(s, key, s.on_ground && x.rev_pct[i] > 50.0 && s.n1[i] > 55.0 && s.gs < common.reverse_min_kt - 10.0, t, s.gs, common.reverse_min_kt, 1.5);
            }
        }
        // Motores
        for i in 0..2 {
            let (k_toga, k_n1, k_egt, k_hot) = match i {
                0 => ("TOGA_TIME_1", "N1_1", "EGT_1", "HOT_START_1"),
                _ => ("TOGA_TIME_2", "N1_2", "EGT_2", "HOT_START_2"),
            };
            self.check(s, k_toga, s.eng_running[i] && s.n1[i] >= 95.0, t, s.n1[i], 95.0, 0.0);
            if eng.n1_max_pct > 0.0 {
                self.check(s, k_n1, s.n1[i] > eng.n1_max_pct + 0.5, t, s.n1[i], eng.n1_max_pct, 1.0);
            }
            if eng.egt_takeoff_c > 0.0 && s.egt[i] > 0.0 {
                let starting = self.start_window[i] > 0.0;
                self.check(s, k_hot, starting && s.egt[i] > eng.egt_start_c + eng.egt_tolerance_c, t, s.egt[i], eng.egt_start_c, 0.5);
                let over = eng.egt_over_limit_sec.max(1.0);
                self.check(s, k_egt, !starting && s.eng_running[i] && s.egt[i] > eng.egt_takeoff_c + eng.egt_tolerance_c, t, s.egt[i], eng.egt_takeoff_c, over);
            }
        }
        // Frenos: los del simulador si son creíbles; si no, el modelo térmico.
        let hot = usable_sim_brakes(o).unwrap_or_else(|| self.brakes.hottest());
        self.check(s, "HOT_BRAKES_TO", s.on_ground && s.gs > 30.0 && self.td.is_none() && !self.airborne && self.rec.landings.is_empty() && hot > common.brake_takeoff_max_c, t, hot, common.brake_takeoff_max_c, 0.5);
        // Avisos de operación (sin daño, como el FDM de TechLog)
        self.check(s, "BANK", airborne && s.bank_deg.abs() > 45.0, t, s.bank_deg.abs(), 45.0, 1.0);
        self.check(s, "SPEED250", airborne && s.alt_ft < 9500.0 && ias > 262.0 && s.agl_ft > 1500.0, t, ias, 250.0, 10.0);
        self.check(s, "FLAPS_ALT", airborne && flaps_idx > 0 && s.alt_ft > 20000.0, t, s.alt_ft, 20000.0, 3.0);
        self.check(s, "STALL", airborne && s.stall_warning, t, ias, 0.0, 1.0);
    }

    /// Antirrebote: abre el episodio con la condición y lo evalúa al cerrarse
    /// (si duró al menos `debounce` s). El pico se guarda mientras dura.
    #[allow(clippy::too_many_arguments)]
    fn check(&mut self, s: &WatcherSample, key: &str, cond: bool, t: f64, value: f64, limit: f64, debounce: f64) {
        if cond {
            let e = self.episodes.entry(key.to_string()).or_insert(Episode { start: t, peak: value, limit });
            e.peak = e.peak.max(value);
            return;
        }
        if let Some(e) = self.episodes.remove(key) {
            let dur = t - e.start;
            if dur >= debounce {
                self.exceedance(s, key, e.peak, e.limit, dur);
            }
        }
    }

    fn finish(&mut self) {
        let keys: Vec<String> = self.episodes.keys().cloned().collect();
        let t = self.last_t.unwrap_or(0.0);
        let s = WatcherSample::default();
        for k in keys {
            if let Some(e) = self.episodes.remove(&k) {
                let dur = t - e.start;
                self.exceedance(&s, &k, e.peak, e.limit, dur);
            }
        }
        self.finish_landing();
    }

    fn finish_landing(&mut self) {
        let Some(mut td) = self.td.take() else { return };
        let share = if self.td_rollout_s > 0.0 {
            (self.td_rev_s / self.td_rollout_s).clamp(0.0, 0.6)
        } else {
            0.0
        };
        let mass = if td.weight_kg > 1000.0 { td.weight_kg } else { 60_000.0 };
        let v1 = td.gs_kt * KT_TO_MS;
        let v2 = 30.0 * KT_TO_MS;
        td.brake_energy_mj = 0.5 * mass * (v1 * v1 - v2 * v2).max(0.0) * (1.0 - share) / 4.0 / 1_000_000.0;
        self.rec.landings.push(td);
    }

    fn exceedance(&mut self, s: &WatcherSample, key: &str, peak: f64, limit: f64, dur: f64) {
        let pos_of = |k: &str| k.rsplit('_').next().filter(|p| p.chars().all(|c| c.is_ascii_digit())).map(String::from);
        let fx = &mut Vec::new();
        let raise = |fx: &mut Vec<Effect>, mel: &'static str, pos: Option<String>, detail: String| {
            fx.push(Effect::Raise { mel, pos, detail })
        };
        let (kind, sev, text): (&str, &str, String) = match key {
            "VMO" | "MMO" => {
                let mach = key == "MMO";
                let e = peak - limit;
                let flutter = if mach { e >= 0.07 } else { e >= 31.0 };
                let damage = flutter || if mach { e >= 0.04 } else { e >= 25.0 };
                let insp = damage || (if mach { e >= 0.02 } else { e >= 10.0 }) || ((if mach { e >= 0.015 } else { e >= 8.0 }) && dur > 10.0);
                let text = if mach {
                    format!("MMO exceeded: M{peak:.3} (limit M{limit:.2})")
                } else {
                    format!("VMO exceeded: {peak:.0} kt (limit {limit:.0})")
                };
                if insp {
                    raise(fx, "05-51-03", None, text.clone());
                }
                if damage {
                    for k in ["ELEV_SERVO", "THS_ACTUATOR"] {
                        fx.push(Effect::Abuse { target: Target::Key(k), pos: None, share: 0.2, why: "overspeed".into() });
                    }
                }
                if flutter {
                    raise(fx, "27-34-02", None, format!("{text}: beyond the dive speed, elevator flutter damage"));
                    raise(fx, "51-00-01", None, text.clone());
                }
                (key, "Warning", text)
            }
            "OVERSPEED" => {
                let text = format!("Overspeed warning for {dur:.0} s ({peak:.0} kt)");
                if dur > 8.0 {
                    raise(fx, "05-51-03", None, text.clone());
                }
                ("VMO", "Warning", text)
            }
            "VFE" | "VFE_WARN" => {
                let e = if key == "VFE" { peak - limit } else { 12.0 };
                let locked = e >= 25.0 || dur > 30.0;
                let damage = locked || e >= 15.0;
                let insp = damage || e > 10.0 || dur > 8.0;
                let text = if key == "VFE" {
                    format!("Flap overspeed: {peak:.0} kt (VFE {limit:.0})")
                } else {
                    format!("Flap overspeed warning for {dur:.0} s ({peak:.0} kt)")
                };
                if insp {
                    raise(fx, "05-51-04", None, text.clone());
                }
                if damage {
                    for k in ["SFCC_FLAP", "SFCC_SLAT"] {
                        fx.push(Effect::Abuse { target: Target::Key(k), pos: None, share: 0.15, why: "flap overspeed".into() });
                    }
                    raise(fx, "27-50-02", None, text.clone());
                }
                if locked {
                    raise(fx, "27-50-01", Some("Flaps".into()), text.clone());
                }
                ("VFE", "Warning", text)
            }
            "VLE" | "VLO" | "VLE_WARN" => {
                let big = key != "VLE_WARN" && peak - limit >= 30.0;
                let text = if key == "VLE_WARN" {
                    format!("Gear overspeed warning for {dur:.0} s ({peak:.0} kt)")
                } else if key == "VLE" {
                    format!("Gear-down speed exceeded: {peak:.0} kt (VLE {limit:.0})")
                } else {
                    format!("Gear operated above VLO: {peak:.0} kt (VLO {limit:.0})")
                };
                raise(fx, "05-51-05", None, text.clone());
                if big {
                    raise(fx, "32-30-01", None, format!("{text}: gear doors damaged"));
                    fx.push(Effect::Abuse { target: Target::Gear, pos: None, share: 0.15, why: "gear overspeed".into() });
                }
                ("VLE", "Warning", text)
            }
            "GMAX" => {
                let text = format!("Load factor {peak:.2} g (limit {limit:.1} g)");
                raise(fx, "05-51-08", None, text.clone());
                if peak >= limit + 0.5 {
                    raise(fx, "51-00-02", None, text.clone());
                }
                if peak >= limit + 1.0 {
                    raise(fx, "28-11-02", None, text.clone());
                }
                ("G", "Warning", text)
            }
            "GMIN" => {
                let text = format!("Negative load factor {:.2} g (limit {:.1} g)", -peak, -limit);
                raise(fx, "05-51-08", None, text.clone());
                if peak >= limit + 0.5 {
                    raise(fx, "51-00-02", None, text.clone());
                }
                ("G", "Warning", text)
            }
            "POD" => {
                let text = format!("Engine pod / wingtip strike: bank {peak:.1}° with the wheels on the runway");
                raise(fx, "05-51-09", None, text.clone());
                ("POD_STRIKE", "Warning", text)
            }
            "TAILSTRIKE" => {
                let text = format!("Tail strike: pitch {peak:.1}° on the runway (limit {limit:.1}°)");
                raise(fx, "05-51-06", None, text.clone());
                ("TAILSTRIKE", "Warning", text)
            }
            "TYRE_SPEED" => {
                let text = format!("Tyre speed rating exceeded: {peak:.0} kt (rating {limit:.0} kt)");
                raise(fx, "32-41-04", None, text.clone());
                fx.push(Effect::Tyres { main: true, pct: 100.0, why: "tyre speed".into() });
                ("TYRE_SPEED", "Warning", text)
            }
            "SKID_L" | "SKID_R" => {
                let side = if key == "SKID_L" { "Left" } else { "Right" };
                let pos = if key == "SKID_L" { "1" } else { "3" };
                let text = format!("{side} main wheels locked at {peak:.0} kt: tyres flat-spotted");
                raise(fx, "32-41-03", Some(pos.into()), text.clone());
                fx.push(Effect::Tyres { main: true, pct: 60.0, why: "flat spot".into() });
                ("FLAT_SPOT", "Warning", text)
            }
            "TAXI_SPEED" => ("TAXI_SPEED", "Caution", format!("Fast taxi: {peak:.0} kt (guide {limit:.0} kt)")),
            k if k.starts_with("REV_LOW") => {
                let pos = pos_of(k);
                let text = format!("Engine {} reverse thrust above idle below {limit:.0} kt: debris ingestion risk", pos.clone().unwrap_or_default());
                raise(fx, "72-00-01", pos, text.clone());
                ("REV_LOW_SPEED", "Caution", text)
            }
            k if k.starts_with("TOGA_TIME") => {
                let pos = pos_of(k);
                if dur < 300.0 {
                    return;
                }
                let over = dur > 600.0;
                let text = format!("Engine {} at takeoff thrust for {:.1} min (limit 5 min)", pos.clone().unwrap_or_default(), dur / 60.0);
                raise(fx, "72-00-02", pos.clone(), text.clone());
                if over {
                    raise(fx, "72-00-09", pos.clone(), text.clone());
                    fx.push(Effect::Abuse { target: Target::HotSection, pos, share: 0.1, why: "takeoff thrust held".into() });
                }
                ("TOGA_TIME", "Warning", text)
            }
            k if k.starts_with("N1_") => {
                let pos = pos_of(k);
                let text = format!("Engine {} N1 {peak:.1}% (limit {limit:.1}%)", pos.clone().unwrap_or_default());
                raise(fx, "72-00-02", pos, text.clone());
                ("N1", "Warning", text)
            }
            k if k.starts_with("EGT_") => {
                let pos = pos_of(k);
                let text = format!("Engine {} EGT {peak:.0} °C (limit {limit:.0} °C)", pos.clone().unwrap_or_default());
                raise(fx, "72-00-02", pos.clone(), text.clone());
                fx.push(Effect::Abuse { target: Target::HotSection, pos, share: 0.05, why: "EGT overlimit".into() });
                ("EGT", "Warning", text)
            }
            k if k.starts_with("HOT_START") => {
                let pos = pos_of(k);
                let text = format!("Engine {} start EGT {peak:.0} °C (limit {limit:.0} °C)", pos.clone().unwrap_or_default());
                raise(fx, "72-00-03", pos, text.clone());
                ("HOT_START", "Warning", text)
            }
            "HOT_BRAKES_TO" => {
                let text = format!("Takeoff with brakes at {peak:.0} °C (limit {limit:.0} °C)");
                raise(fx, "05-51-10", None, text.clone());
                ("HOT_BRAKES_TAKEOFF", "Warning", text)
            }
            "BANK" => ("BANK", "Caution", format!("Bank angle {peak:.0}° (FDM limit {limit:.0}°)")),
            "SPEED250" => ("SPEED_250", "Caution", format!("{peak:.0} kt below FL100 (limit {limit:.0} kt)")),
            "FLAPS_ALT" => ("FLAPS_ALTITUDE", "Caution", format!("Flaps extended at {peak:.0} ft (max {limit:.0} ft)")),
            "STALL" => ("STALL", "Warning", format!("Stall warning for {dur:.0} s")),
            _ => return,
        };
        let effects = std::mem::take(fx);
        self.rec.effects.extend(effects);
        self.event(s, kind, sev, text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brake_model_heats_on_a_hard_stop_and_cools_in_the_air() {
        let mut m = BrakeThermalModel::default();
        let x = Extra { valid: true, weight_lb: 140_000.0, oat_c: 15.0, ..Default::default() };
        // 140 kt → 0 frenando a ~2,5 m/s² (sin reversas), muestras de 0,25 s.
        let mut s = WatcherSample { on_ground: true, gs: 140.0, ..Default::default() };
        m.step(&s, &x, 0.0);
        while s.gs > 3.0 {
            s.gs -= 2.5 / KT_TO_MS * 0.25;
            m.step(&s, &x, 0.25);
        }
        let hot = m.hottest();
        assert!(hot > 150.0 && hot < 600.0, "freno a {hot:.0} °C");
        // Los interiores se calientan algo más.
        assert!(m.temp_c[1] > m.temp_c[0]);
        // Media hora en vuelo con el tren arriba: se enfría hacia la OAT.
        let air = WatcherSample { on_ground: false, gs: 450.0, ..Default::default() };
        for _ in 0..1800 {
            m.step(&air, &x, 1.0);
        }
        assert!(m.hottest() < hot * 0.5);
        // Un hueco largo (pausa) no calienta nada.
        let before = m.hottest();
        m.step(&WatcherSample { on_ground: true, gs: 0.0, ..Default::default() }, &x, 30.0);
        assert!((m.hottest() - before).abs() < 1e-9);
    }

    #[test]
    fn sim_brake_temps_need_to_be_believable() {
        let o = |l: f64, r: f64| Optional { brakes_valid: true, brake_temp_c: [l, r], ..Default::default() };
        // MSFS 2020: la simvar no existe y llega 0/0 → se usa el modelo.
        assert_eq!(usable_sim_brakes(&o(0.0, 0.0)), None);
        assert_eq!(usable_sim_brakes(&o(f64::NAN, 80.0)), None);
        assert_eq!(usable_sim_brakes(&o(2500.0, 80.0)), None);
        assert_eq!(usable_sim_brakes(&Optional::default()), None);
        assert_eq!(usable_sim_brakes(&o(120.0, 180.0)), Some(180.0));
    }
}
