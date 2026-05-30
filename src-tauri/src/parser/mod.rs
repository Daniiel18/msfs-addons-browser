use once_cell::sync::Lazy;
use regex::Regex;

static VERSION_RE: Lazy<Regex> = Lazy::new(|| {
    // Versión `vX[.Y[.Z]][b2]` precedida de espacio y terminada
    // por boundary de palabra. Antes anclábamos al final del
    // string (`\s*$`) y eso fallaba con títulos de SceneryAddons
    // como `"EDDF Frankfurt Airport v1.1.2 MSFS 2020/2024"`, donde
    // el sufijo `MSFS 2020/2024` deja la versión en medio. El
    // resultado era `version = None` para 90% del catálogo y, por
    // tanto, ninguna update detectada.
    //
    // Con `\b` aceptamos versión en cualquier posición siempre que
    // venga precedida de un espacio + `v` y termine en un límite
    // de palabra. La porción posterior (típicamente metadata del
    // simulador) se descarta junto al name.
    Regex::new(r"\s+v(\d+(?:\.\d+)*(?:[A-Za-z]\d*)?)\b").unwrap()
});

static ICAO_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\b[A-Z]{4}\b").unwrap());

const SEPARATORS: &[&str] = &[" – ", " — ", " - ", " | "];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedAddon {
    pub developer: Option<String>,
    pub name: String,
    pub version: Option<String>,
    pub icao: Option<String>,
}

/// Parse a raw addon title like:
///   "SceneryTR Design – LTFJ Sabiha Gokcen International Airport v1.0.4"
/// into its developer / name / version / icao parts.
pub fn parse(title: &str) -> ParsedAddon {
    let title = title.trim();

    let (developer, rest) = split_on_first(title, SEPARATORS)
        .map(|(d, r)| (Some(d.trim().to_string()), r.to_string()))
        .unwrap_or((None, title.to_string()));

    let (name, version) = match VERSION_RE.captures(&rest) {
        Some(caps) => {
            let whole = caps.get(0).unwrap();
            let v = caps.get(1).map(|m| m.as_str().to_string());
            let n = rest[..whole.start()].trim().to_string();
            (n, v)
        }
        None => (rest.trim().to_string(), None),
    };

    let icao = ICAO_RE.find(&name).map(|m| m.as_str().to_string());

    ParsedAddon { developer, name, version, icao }
}

fn split_on_first<'a>(s: &'a str, seps: &[&str]) -> Option<(&'a str, &'a str)> {
    let mut best: Option<(usize, usize)> = None; // (index, sep_len)
    for sep in seps {
        if let Some(idx) = s.find(sep) {
            best = match best {
                Some((cur, _)) if cur < idx => best,
                _ => Some((idx, sep.len())),
            };
        }
    }
    best.map(|(idx, len)| (&s[..idx], &s[idx + len..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sceneryaddons_examples() {
        let cases = [
            (
                "SceneryTR Design – LTFJ Sabiha Gokcen International Airport v1.0.4",
                Some("SceneryTR Design"),
                "LTFJ Sabiha Gokcen International Airport",
                Some("1.0.4"),
                Some("LTFJ"),
            ),
            (
                "SiamFlight – RKSI Incheon International Airport v0.9.7",
                Some("SiamFlight"),
                "RKSI Incheon International Airport",
                Some("0.9.7"),
                Some("RKSI"),
            ),
            (
                "Simman – VTSY Betong International Airport v1.1.0",
                Some("Simman"),
                "VTSY Betong International Airport",
                Some("1.1.0"),
                Some("VTSY"),
            ),
        ];

        for (raw, dev, name, ver, icao) in cases {
            let p = parse(raw);
            assert_eq!(p.developer.as_deref(), dev, "raw={}", raw);
            assert_eq!(p.name, name, "raw={}", raw);
            assert_eq!(p.version.as_deref(), ver, "raw={}", raw);
            assert_eq!(p.icao.as_deref(), icao, "raw={}", raw);
        }
    }

    #[test]
    fn no_separator_still_extracts_icao() {
        let p = parse("KJFK John F Kennedy v2.1");
        assert_eq!(p.developer, None);
        assert_eq!(p.name, "KJFK John F Kennedy");
        assert_eq!(p.version.as_deref(), Some("2.1"));
        assert_eq!(p.icao.as_deref(), Some("KJFK"));
    }

    /// Regresión del bug que reportó el usuario el 2026-04-26: el
    /// regex anclado al final del string fallaba para los títulos
    /// canónicos de SceneryAddons, que tienen `MSFS 2020/2024`
    /// como sufijo. Sin extracción de versión no había updates.
    #[test]
    fn parses_sceneryaddons_titles_with_simulator_suffix() {
        let cases = [
            (
                "Aerosoft - EDDF Frankfurt Airport v1.1.2 MSFS 2020/2024",
                Some("Aerosoft"),
                "EDDF Frankfurt Airport",
                Some("1.1.2"),
                Some("EDDF"),
            ),
            (
                "Asobo Studio - EDDF Frankfurt Airport v1.0.2 MSFS 2020/2",
                Some("Asobo Studio"),
                "EDDF Frankfurt Airport",
                Some("1.0.2"),
                Some("EDDF"),
            ),
            (
                "FeelThere - EDDF Frankfurt Airport v0.3.0 MSFS 2020/2024",
                Some("FeelThere"),
                "EDDF Frankfurt Airport",
                Some("0.3.0"),
                Some("EDDF"),
            ),
            (
                "BravoAirspace - MDSD Las Americas International Airport v1.6.5 (Updated) MSFS 2020/2024",
                Some("BravoAirspace"),
                "MDSD Las Americas International Airport",
                Some("1.6.5"),
                Some("MDSD"),
            ),
        ];
        for (raw, dev, name, ver, icao) in cases {
            let p = parse(raw);
            assert_eq!(p.developer.as_deref(), dev, "raw={}", raw);
            assert_eq!(p.name, name, "raw={}", raw);
            assert_eq!(p.version.as_deref(), ver, "raw={}", raw);
            assert_eq!(p.icao.as_deref(), icao, "raw={}", raw);
        }
    }
}
