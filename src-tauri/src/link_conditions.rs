//! (05/11) Condición de un enlace del Link Map: el enlace solo vale en ciertos
//! vuelos. Dentro de un tipo basta con uno; entre tipos, hay que cumplirlos
//! todos. La evalúa el plan del vuelo (`src/lib/dispatch.ts`).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LinkConditions {
    /// Código ISO del país: `BR`.
    pub country: Vec<String>,
    /// Aeropuerto de salida, parada o llegada: `SBGR`.
    pub airport: Vec<String>,
    /// Tipo ICAO del plan: `B738`.
    pub aircraft: Vec<String>,
    /// Aerolínea ICAO del indicativo: `WJA`.
    pub airline: Vec<String>,
}

impl LinkConditions {
    /// En mayúsculas y sin repetidos. `Ok(None)` si no queda nada que exigir;
    /// `Err` con el primer valor que no sirve.
    pub fn normalized(self) -> Result<Option<Self>, String> {
        let clean = Self {
            country: clean(self.country, |v| v.len() == 2 && letters(v))?,
            airport: clean(self.airport, |v| (3..=5).contains(&v.len()) && alphanumeric(v))?,
            aircraft: clean(self.aircraft, |v| (2..=4).contains(&v.len()) && alphanumeric(v))?,
            airline: clean(self.airline, |v| v.len() == 3 && letters(v))?,
        };
        Ok((clean != Self::default()).then_some(clean))
    }
}

fn letters(value: &str) -> bool {
    value.chars().all(|c| c.is_ascii_alphabetic())
}

fn alphanumeric(value: &str) -> bool {
    value.chars().all(|c| c.is_ascii_alphanumeric())
}

fn clean(list: Vec<String>, valid: fn(&str) -> bool) -> Result<Vec<String>, String> {
    let mut out: Vec<String> = Vec::new();
    for raw in list {
        let value = raw.trim().to_ascii_uppercase();
        if value.is_empty() {
            continue;
        }
        if !valid(&value) {
            return Err(raw.trim().to_string());
        }
        if !out.contains(&value) {
            out.push(value);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| v.to_string()).collect()
    }

    #[test]
    fn values_are_tidied_up() {
        let clean = LinkConditions {
            country: list(&[" br", "BR", "ca "]),
            airport: list(&["sbgr", ""]),
            aircraft: list(&["b738"]),
            airline: list(&["wja"]),
        }
        .normalized()
        .unwrap()
        .unwrap();
        assert_eq!(clean.country, ["BR", "CA"]);
        assert_eq!(clean.airport, ["SBGR"]);
        assert_eq!(clean.aircraft, ["B738"]);
        assert_eq!(clean.airline, ["WJA"]);
    }

    #[test]
    fn nothing_to_ask_for_means_no_condition() {
        let empty = LinkConditions { country: list(&["  "]), ..Default::default() };
        assert_eq!(empty.normalized(), Ok(None));
    }

    #[test]
    fn a_value_that_makes_no_sense_is_named() {
        let cases = [
            LinkConditions { country: list(&["Brasil"]), ..Default::default() },
            LinkConditions { airport: list(&["SB"]), ..Default::default() },
            LinkConditions { aircraft: list(&["B737-800"]), ..Default::default() },
            LinkConditions { airline: list(&["W1A"]), ..Default::default() },
        ];
        let wrong: Vec<String> = cases.into_iter().map(|c| c.normalized().unwrap_err()).collect();
        assert_eq!(wrong, ["Brasil", "SB", "B737-800", "W1A"]);
    }

    #[test]
    fn missing_lists_are_read_as_empty() {
        let read: LinkConditions = serde_json::from_str(r#"{"country":["BR"]}"#).unwrap();
        assert_eq!(read.country, ["BR"]);
        assert!(read.airline.is_empty());
    }
}
