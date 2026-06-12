//! (v4.23.0) Extracción INTELIGENTE de aerolínea y matrícula desde los
//! metadatos de la livery.
//!
//! Problema (reportado por el usuario con el A320 FBW "LATAM PR-XBD"):
//! los creadores de liveries rellenan `atc.cfg` con valores random —
//! `ATC AIRLINE` suele traer el nombre del DESARROLLADOR ("Fly By Wire",
//! "Fenix") y `ATC ID` basura ("'", "ASOBO-AI"). Sin embargo el TITLE de
//! la livery casi siempre contiene la aerolínea y la matrícula reales:
//! "Airbus A320 Neo (FBW) LATAM PR-XBD - Combustível do Futuro".
//!
//! Estrategia:
//!   · `smart_registration`: valida el ATC ID contra los formatos
//!     reales de matrícula (XX-YYY, N-numbers, etc.); si es basura,
//!     la extrae del TITLE por regex.
//!   · `smart_airline`: descarta nombres de desarrolladores conocidos
//!     y, en ese caso, busca una aerolínea real en el TITLE contra un
//!     diccionario curado (~90 aerolíneas).

use once_cell::sync::Lazy;
use regex::Regex;

/// Marcas/desarrolladores que NO son aerolíneas — si `ATC AIRLINE`
/// matchea una de estas, el valor se considera placeholder.
const DEV_BRANDS: &[&str] = &[
    "FLYBYWIRE", "FLY BY WIRE", "FBW", "FENIX", "PMDG", "ASOBO",
    "MICROSOFT", "INIBUILDS", "INI BUILDS", "AEROSOFT", "HEADWIND",
    "HORIZON SIM", "HORIZONSIM", "LATINVFR", "CAPTAIN SIM", "JUSTFLIGHT",
    "JUST FLIGHT", "LEONARDO", "FELIS", "CARENADO", "MILVIZ", "BREDOK3D",
    "WORKING TITLE", "SALTY", "SYNAPTIC", "DIGITAL FLIGHT DYNAMICS",
    "LVFR", "BLACKSQUARE", "BLACK SQUARE",
];

/// Diccionario de aerolíneas: (keyword en MAYÚSCULAS para buscar en el
/// title, nombre display). Keywords distintivos — multi-palabra o
/// marcas inequívocas — para evitar falsos positivos.
const AIRLINES: &[(&str, &str)] = &[
    ("LATAM", "LATAM"),
    ("CITILINK", "Citilink"),
    ("GARUDA", "Garuda Indonesia"),
    ("LION AIR", "Lion Air"),
    ("BATIK", "Batik Air"),
    ("SRIWIJAYA", "Sriwijaya Air"),
    ("AMERICAN AIRLINES", "American Airlines"),
    ("DELTA", "Delta Air Lines"),
    ("UNITED", "United Airlines"),
    ("SOUTHWEST", "Southwest Airlines"),
    ("ALASKA", "Alaska Airlines"),
    ("JETBLUE", "JetBlue"),
    ("SPIRIT", "Spirit Airlines"),
    ("FRONTIER", "Frontier Airlines"),
    ("HAWAIIAN", "Hawaiian Airlines"),
    ("ALLEGIANT", "Allegiant Air"),
    ("SUN COUNTRY", "Sun Country"),
    ("AIR CANADA", "Air Canada"),
    ("WESTJET", "WestJet"),
    ("AEROMEXICO", "Aeroméxico"),
    ("AEROMÉXICO", "Aeroméxico"),
    ("VOLARIS", "Volaris"),
    ("VIVA AEROBUS", "Viva Aerobus"),
    ("VIVAAEROBUS", "Viva Aerobus"),
    ("COPA", "Copa Airlines"),
    ("AVIANCA", "Avianca"),
    ("ARAJET", "Arajet"),
    ("CARIBBEAN AIRLINES", "Caribbean Airlines"),
    ("LIAT", "LIAT"),
    ("GOL", "GOL Linhas Aéreas"),
    ("AZUL", "Azul"),
    ("AEROLINEAS ARGENTINAS", "Aerolíneas Argentinas"),
    ("AEROLÍNEAS ARGENTINAS", "Aerolíneas Argentinas"),
    ("SKY AIRLINE", "SKY Airline"),
    ("JETSMART", "JetSMART"),
    ("BOLIVIANA", "Boliviana de Aviación"),
    ("IBERIA", "Iberia"),
    ("VUELING", "Vueling"),
    ("AIR EUROPA", "Air Europa"),
    ("RYANAIR", "Ryanair"),
    ("EASYJET", "easyJet"),
    ("BRITISH AIRWAYS", "British Airways"),
    ("VIRGIN ATLANTIC", "Virgin Atlantic"),
    ("LUFTHANSA", "Lufthansa"),
    ("CONDOR", "Condor"),
    ("EUROWINGS", "Eurowings"),
    ("AIR FRANCE", "Air France"),
    ("TRANSAVIA", "Transavia"),
    ("KLM", "KLM"),
    ("TAP ", "TAP Air Portugal"),
    ("TAP AIR", "TAP Air Portugal"),
    ("SWISS", "SWISS"),
    ("AUSTRIAN", "Austrian Airlines"),
    ("BRUSSELS AIRLINES", "Brussels Airlines"),
    ("ITA AIRWAYS", "ITA Airways"),
    ("ALITALIA", "Alitalia"),
    ("WIZZ", "Wizz Air"),
    ("TURKISH", "Turkish Airlines"),
    ("PEGASUS", "Pegasus"),
    ("AEGEAN", "Aegean Airlines"),
    ("LOT ", "LOT Polish Airlines"),
    ("FINNAIR", "Finnair"),
    ("NORWEGIAN", "Norwegian"),
    ("ICELANDAIR", "Icelandair"),
    ("AER LINGUS", "Aer Lingus"),
    ("SAS ", "SAS"),
    ("SCANDINAVIAN", "SAS"),
    ("EMIRATES", "Emirates"),
    ("QATAR", "Qatar Airways"),
    ("ETIHAD", "Etihad"),
    ("SAUDIA", "Saudia"),
    ("FLYDUBAI", "flydubai"),
    ("EL AL", "El Al"),
    ("ROYAL JORDANIAN", "Royal Jordanian"),
    ("QANTAS", "Qantas"),
    ("JETSTAR", "Jetstar"),
    ("AIR NEW ZEALAND", "Air New Zealand"),
    ("ANA ", "ANA"),
    ("ALL NIPPON", "ANA"),
    ("JAPAN AIRLINES", "Japan Airlines"),
    ("KOREAN AIR", "Korean Air"),
    ("ASIANA", "Asiana"),
    ("CHINA SOUTHERN", "China Southern"),
    ("CHINA EASTERN", "China Eastern"),
    ("AIR CHINA", "Air China"),
    ("CATHAY", "Cathay Pacific"),
    ("EVA AIR", "EVA Air"),
    ("SINGAPORE AIRLINES", "Singapore Airlines"),
    ("MALAYSIA AIRLINES", "Malaysia Airlines"),
    ("AIRASIA", "AirAsia"),
    ("VIETNAM AIRLINES", "Vietnam Airlines"),
    ("VIETJET", "VietJet"),
    ("CEBU PACIFIC", "Cebu Pacific"),
    ("PHILIPPINE AIRLINES", "Philippine Airlines"),
    ("THAI AIRWAYS", "Thai Airways"),
    ("INDIGO", "IndiGo"),
    ("AIR INDIA", "Air India"),
    ("VISTARA", "Vistara"),
    ("ETHIOPIAN", "Ethiopian Airlines"),
    ("KENYA AIRWAYS", "Kenya Airways"),
    ("EGYPTAIR", "EGYPTAIR"),
    ("ROYAL AIR MAROC", "Royal Air Maroc"),
    ("SOUTH AFRICAN", "South African Airways"),
    ("TUI", "TUI"),
];

/// Regex de matrícula: `PR-XBD`, `PK-GQN`, `D-AIZA`, `CC-BAW`,
/// `HK-5123`, `N123AB`, `N45DX`… Grupo 1 = la matrícula.
static REG_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"\b([A-Z]{1,2}-[A-Z0-9]{2,5}|N[0-9]{1,5}[A-Z]{0,2})\b").unwrap()
});

fn is_dev_brand(s: &str) -> bool {
    let up = s.trim().to_uppercase();
    DEV_BRANDS.iter().any(|b| up == *b || up.contains(b))
}

/// ¿El string parece una matrícula real?
fn looks_like_registration(s: &str) -> bool {
    let t = s.trim();
    // Match COMPLETO contra el patrón (no substring).
    REG_RE
        .find(t)
        .map(|m| m.start() == 0 && m.end() == t.len())
        .unwrap_or(false)
}

/// Matrícula final: el ATC ID si es válido; si es basura, la primera
/// matrícula reconocible dentro del TITLE; si no hay, None.
pub fn smart_registration(
    raw_reg: Option<&str>,
    title: Option<&str>,
) -> Option<String> {
    if let Some(r) = raw_reg {
        let t = r.trim().trim_matches(['\'', '"', '`']).trim();
        if looks_like_registration(&t.to_uppercase()) {
            return Some(t.to_uppercase());
        }
    }
    let title = title?;
    REG_RE
        .find(&title.to_uppercase())
        .map(|m| m.as_str().to_string())
}

/// Aerolínea final: el ATC AIRLINE si NO es un desarrollador conocido y
/// tiene contenido; si es placeholder, la primera aerolínea del
/// diccionario que aparezca en el TITLE; si no, el raw original (mejor
/// algo que nada).
pub fn smart_airline(
    raw_airline: Option<&str>,
    title: Option<&str>,
) -> Option<String> {
    let raw = raw_airline.map(|s| s.trim()).filter(|s| s.len() > 1);
    let raw_is_dev = raw.map(is_dev_brand).unwrap_or(true);
    if let (Some(r), false) = (raw, raw_is_dev) {
        return Some(r.to_string());
    }
    if let Some(t) = title {
        let up = t.to_uppercase();
        for (kw, display) in AIRLINES {
            if up.contains(kw) {
                return Some((*display).to_string());
            }
        }
    }
    raw.map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fbw_latam_title() {
        let title = Some("Airbus A320 Neo (FBW) LATAM PR-XBD - Combustível do Futuro");
        assert_eq!(
            smart_airline(Some("Fly By Wire"), title).as_deref(),
            Some("LATAM")
        );
        assert_eq!(
            smart_registration(Some("'"), title).as_deref(),
            Some("PR-XBD")
        );
    }

    #[test]
    fn valid_raw_passthrough() {
        assert_eq!(
            smart_registration(Some("PK-GQN"), None).as_deref(),
            Some("PK-GQN")
        );
        assert_eq!(
            smart_airline(Some("Citilink"), None).as_deref(),
            Some("Citilink")
        );
    }

    #[test]
    fn n_number() {
        let title = Some("Boeing 737-800 United N37267");
        assert_eq!(
            smart_registration(Some(""), title).as_deref(),
            Some("N37267")
        );
    }
}
