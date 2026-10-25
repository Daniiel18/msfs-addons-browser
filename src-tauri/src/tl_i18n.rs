//! (23/10) Español para lo que enseñan el Tech Log de SimFleet y Fenix/PMDG
//! TechLog: nombres de piezas, consumibles, sistemas, checks, títulos de
//! defectos/MEL y las frases de eventos, motivos y resúmenes.
//!
//! Se traduce AL MOSTRAR (no al guardar): así también salen en español los
//! vuelos y defectos ya guardados y lo que llega en inglés de TechLog, y en
//! inglés todo sigue igual. Los procedimientos MEL y los textos largos de los
//! hallazgos se quedan en inglés (decisión del usuario).
//!
//! Datos en `mx-catalogs/es.json`:
//! - `names`: nombre inglés (o plantilla con huecos `{pos}`, `{side}`…) → español.
//! - `values`: valores de hueco que cambian ("Captain's" → "del comandante").
//! - `sentences`: [plantilla inglesa, español]. Huecos: `{name}`/`{name2}` se
//!   traducen como nombre; los demás se copian (o pasan por `values`).

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use regex::Regex;
use serde::Deserialize;

use crate::i18n::Lang;
use crate::techlog::{TechLogAircraft, TechLogFlight};

#[derive(Deserialize, Default)]
#[serde(default)]
struct EsFile {
    names: HashMap<String, String>,
    values: HashMap<String, String>,
    sentences: Vec<(String, String)>,
}

struct Rule {
    re: Regex,
    slots: Vec<String>,
    to: String,
}

struct Tables {
    exact: HashMap<String, String>,
    names: Vec<Rule>,
    values: HashMap<String, String>,
    sentences: Vec<Rule>,
}

/// `"Brake {pos} wear"` → regex `^Brake (.+?) wear$` + huecos `["pos"]`.
fn compile(from: &str, to: &str) -> Option<Rule> {
    let slot = Regex::new(r"\{([a-z0-9_]+)\}").ok()?;
    let mut pattern = String::from("^");
    let mut slots = Vec::new();
    let mut last = 0;
    for m in slot.captures_iter(from) {
        let all = m.get(0)?;
        pattern.push_str(&regex::escape(&from[last..all.start()]));
        pattern.push_str("(.+?)");
        slots.push(m[1].to_string());
        last = all.end();
    }
    pattern.push_str(&regex::escape(&from[last..]));
    pattern.push('$');
    Some(Rule { re: Regex::new(&pattern).ok()?, slots, to: to.to_string() })
}

fn tables() -> &'static Tables {
    static T: OnceLock<Tables> = OnceLock::new();
    T.get_or_init(|| {
        let file: EsFile = serde_json::from_str(include_str!("../mx-catalogs/es.json")).unwrap_or_else(|e| {
            tracing::warn!(target: "i18n", "mx-catalogs/es.json ilegible: {e}");
            EsFile::default()
        });
        let mut exact = HashMap::new();
        let mut names = Vec::new();
        for (from, to) in &file.names {
            if from.contains('{') {
                names.extend(compile(from, to));
            } else {
                exact.insert(from.clone(), to.clone());
            }
        }
        // Las plantillas más largas primero: "Brake {pos} wear" antes que "Brake {pos}".
        names.sort_by_key(|r| std::cmp::Reverse(r.re.as_str().len()));
        let sentences = file.sentences.iter().filter_map(|(f, t)| compile(f, t)).collect();
        Tables { exact, names, values: file.values, sentences }
    })
}

fn spanish() -> bool {
    crate::i18n::current() == Lang::Es
}

fn fill(rule: &Rule, caps: &regex::Captures, t: &Tables) -> String {
    let mut out = rule.to.clone();
    for (i, slot) in rule.slots.iter().enumerate() {
        let v = caps.get(i + 1).map(|m| m.as_str()).unwrap_or_default();
        let v = if slot.starts_with("name") {
            name_in(v, t)
        } else {
            t.values.get(v).cloned().unwrap_or_else(|| v.to_string())
        };
        out = out.replace(&format!("{{{slot}}}"), &v);
    }
    out
}

fn name_in(s: &str, t: &Tables) -> String {
    let key = s.trim();
    if let Some(es) = t.exact.get(key) {
        return es.clone();
    }
    for r in &t.names {
        if let Some(c) = r.re.captures(key) {
            return fill(r, &c, t);
        }
    }
    s.to_string()
}

fn memo() -> &'static Mutex<HashMap<String, String>> {
    static M: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    M.get_or_init(|| Mutex::new(HashMap::new()))
}

fn sentence_in(s: &str, t: &Tables) -> String {
    let named = name_in(s, t);
    if named != s {
        return named;
    }
    // "Daily check overdue." → el nombre sin el punto final.
    if let Some(body) = s.trim_end().strip_suffix('.') {
        let named = name_in(body, t);
        if named != body {
            return format!("{named}.");
        }
    }
    for r in &t.sentences {
        if let Some(c) = r.re.captures(s.trim()) {
            return fill(r, &c, t);
        }
    }
    // Notas con símbolo delante ("⚠ VMO exceeded…"): traducir el resto.
    if let Some(i) = s.find(|c: char| c.is_ascii_alphanumeric()) {
        if i > 0 && !s[..i].contains(['?', '{']) {
            let rest = sentence_in(&s[i..], t);
            if rest != s[i..] {
                return format!("{}{rest}", &s[..i]);
            }
        }
    }
    // Resúmenes de TechLog: varias frases seguidas ("…block. VMO exceeded…").
    if s.contains(". ") {
        let parts: Vec<&str> = s.split(". ").collect();
        let done: Vec<String> = parts
            .iter()
            .map(|p| {
                let (body, dot) = p.strip_suffix('.').map_or((*p, ""), |b| (b, "."));
                format!("{}{dot}", sentence_in(body, t))
            })
            .collect();
        let joined = done.join(". ");
        if joined != s {
            return joined;
        }
    }
    s.to_string()
}

/// Nombre (pieza, consumible, sistema, check, MEL, hallazgo) en el idioma de la app.
pub fn name(s: &str) -> String {
    if !spanish() || s.is_empty() {
        return s.to_string();
    }
    name_in(s, tables())
}

/// Frase (evento, detalle, motivo, mensaje, resumen) en el idioma de la app.
pub fn sentence(s: &str) -> String {
    if !spanish() || s.is_empty() {
        return s.to_string();
    }
    if let Some(hit) = memo().lock().ok().and_then(|m| m.get(s).cloned()) {
        return hit;
    }
    let out = sentence_in(s, tables());
    if let Ok(mut m) = memo().lock() {
        if m.len() > 4000 {
            m.clear();
        }
        m.insert(s.to_string(), out.clone());
    }
    out
}

fn opt(s: &mut Option<String>) {
    if let Some(v) = s.as_mut() {
        *v = sentence(v);
    }
}

/// Mantenimiento de un avión (Hangar, preflight, campana) en el idioma de la app.
pub fn localize_aircraft(a: &mut TechLogAircraft) {
    if !spanish() {
        return;
    }
    for r in &mut a.dispatch.reasons {
        *r = sentence(r);
    }
    opt(&mut a.pending_work);
    for d in &mut a.defects {
        d.title = sentence(&d.title);
        opt(&mut d.detail);
    }
    for p in &mut a.parts {
        p.name = name(&p.name);
        p.system = name(&p.system);
    }
    for c in &mut a.consumables {
        c.name = name(&c.name);
    }
    for c in &mut a.checks {
        c.name = name(&c.name);
    }
}

/// Lecturas del Fenix (grupos y etiquetas) en el idioma de la app.
pub fn localize_readings(r: &mut crate::techlog::TlReadings) {
    if !spanish() {
        return;
    }
    for g in &mut r.groups {
        g.name = name(&g.name);
        for i in &mut g.items {
            i.label = name(&i.label);
        }
    }
}

/// Registro de un vuelo (FlightBook) en el idioma de la app.
pub fn localize_flight(f: &mut TechLogFlight) {
    if !spanish() {
        return;
    }
    opt(&mut f.summary);
    for n in &mut f.score_notes {
        *n = sentence(n);
    }
    for e in &mut f.events {
        e.text = sentence(&e.text);
        if let Some(p) = e.phase.as_mut() {
            *p = name(p);
        }
    }
    for w in &mut f.wear {
        w.item = name(&w.item);
        w.unit = name(&w.unit);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn es<T>(f: impl FnOnce() -> T) -> T {
        crate::i18n::set_locale("es");
        f()
    }

    /// Cobertura con textos reales: `TL_TEXTS=<archivo, uno por línea> cargo
    /// test --lib tl_texts_coverage -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn tl_texts_coverage() {
        let path = std::env::var("TL_TEXTS").expect("TL_TEXTS");
        let text = std::fs::read_to_string(path).unwrap();
        crate::i18n::set_locale("es");
        let mut missing = 0;
        for line in text.lines().filter(|l| !l.trim().is_empty()) {
            let es = sentence(line);
            let mark = if es == line { missing += 1; "!!" } else { "ok" };
            eprintln!("{mark} {line}  =>  {es}");
        }
        eprintln!("sin traducir: {missing}");
    }

    #[test]
    fn es_json_parses_and_compiles() {
        let t = tables();
        assert!(t.exact.len() + t.names.len() > 300, "names: {}", t.exact.len() + t.names.len());
        assert!(t.sentences.len() > 40, "sentences: {}", t.sentences.len());
    }

    #[test]
    fn translates_names_sentences_and_summaries() {
        es(|| {
            assert_eq!(name("Brake 3"), "Freno 3");
            assert_eq!(name("Something unknown"), "Something unknown");
            assert_eq!(
                sentence("Engine 1 reverse thrust above idle below 70 kt: debris ingestion risk"),
                "Motor 1: reversa por encima de ralentí por debajo de 70 kt: riesgo de ingestión de objetos"
            );
            assert_eq!(sentence("0.8 h block, 1 landing(s)"), "0.8 h de bloque, 1 aterrizaje(s)");
            assert_eq!(sentence("Servicing required: Tyre 1 pressure"), "Servicio requerido: Presión del neumático 1");
            assert!(sentence("Daily check overdue").starts_with("Check diario"));
            let s = sentence("CYYC → CYEG, 0:46 block. Daily check overdue.");
            assert!(s.contains("de bloque") && s.contains("Check diario"), "{s}");
            // TechLog 2.2: avisos nuevos y lecturas.
            assert_eq!(
                sentence("Engine 2 oil pressure 11 psi (least 13 psi)"),
                "Motor 2: presión de aceite 11 psi (mínimo 13 psi)"
            );
            assert_eq!(name("Green pressure"), "Presión verde");
            assert_eq!(name("Battery 1"), "Batería 1");
            assert_eq!(sentence("⚠ Daily check overdue"), "⚠ Check diario vencido");
        });
    }
}
