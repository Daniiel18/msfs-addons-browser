//! (v8.3 · Etapa 6.2b) Base de aviones y flota real de cada aerolínea.
//!
//! - Base: `aircraftDatabase.zip` de OpenSky (2024), bajada UNA vez por PC
//!   (uso personal; no va en el instalador) y filtrada a aviones de
//!   transporte (bimotores o más, jet/turbohélice) y de aerolínea. Se mira
//!   cada 30 días si OpenSky publicó otra versión (Last-Modified).
//! - Los aviones vistos volando que no están (entregados después) se buscan
//!   en adsbdb: 1 consulta/s, hasta 300 al día, y se guardan (también los
//!   desconocidos, para no repetir en 30 días).
//! - Flota de una aerolínea = aviones que OpenSky la vio volar en 30 días +
//!   los aviones del usuario con esa aerolínea (aunque no se vean volar).

use crate::airline_economy::normalize_reg;
use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use futures_util::StreamExt;
use serde::Serialize;
use sqlx::{Row, SqlitePool};
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::time::Duration;

const DB_URL: &str = "https://s3.opensky-network.org/data-samples/metadata/aircraftDatabase.zip";
const KEY_VERSION: &str = "aircraft_db_version";
const KEY_CHECKED: &str = "aircraft_db_checked_at";
const CHECK_EVERY_DAYS: i64 = 30;
const ADSBDB: &str = "https://api.adsbdb.com/v0/aircraft";
const ADSBDB_DAILY: i64 = 300;
const ADSBDB_RETRY_DAYS: i64 = 30;
/// Ventana de "vistos volando" para la flota.
const FLEET_DAYS: i64 = 30;

// ───────────────────────────── Filtro del CSV ─────────────────────────────

#[derive(Debug, Clone, PartialEq)]
struct DbRow {
    icao24: String,
    registration: Option<String>,
    typecode: Option<String>,
    model: Option<String>,
    operator_icao: Option<String>,
    operator: Option<String>,
    built: Option<String>,
}

fn opt(s: &str) -> Option<String> {
    let t = s.trim();
    (!t.is_empty()).then(|| t.to_string())
}

/// Se guarda si es de aerolínea (operador ICAO) o de transporte
/// (`icaoaircrafttype` L2J/L3J/L4J/L2T…) y trae matrícula o tipo.
fn keep_row(icao24: &str, aircraft_type: &str, operator_icao: &str, registration: &str, typecode: &str) -> bool {
    let hex_ok = icao24.len() == 6 && icao24.chars().all(|c| c.is_ascii_hexdigit());
    let t = aircraft_type.as_bytes();
    let transport = t.len() == 3 && t[0] == b'L' && (b'2'..=b'4').contains(&t[1]) && (t[2] == b'J' || t[2] == b'T');
    hex_ok && (transport || !operator_icao.trim().is_empty()) && (!registration.trim().is_empty() || !typecode.trim().is_empty())
}

/// Lee el CSV de OpenSky (dentro del zip) y devuelve las filas que interesan.
fn parse_zip(path: &std::path::Path) -> Result<Vec<DbRow>> {
    let file = std::fs::File::open(path)?;
    let mut zip = zip::ZipArchive::new(file)?;
    let idx = (0..zip.len())
        .find(|i| zip.by_index(*i).map(|f| f.name().to_ascii_lowercase().ends_with(".csv")).unwrap_or(false))
        .ok_or_else(|| anyhow!("el zip no trae CSV"))?;
    let entry = zip.by_index(idx)?;
    let mut rdr = csv::ReaderBuilder::new().flexible(true).from_reader(entry);
    let headers = rdr.headers()?.clone();
    let col = |name: &str| headers.iter().position(|h| h.trim().eq_ignore_ascii_case(name));
    let (Some(c_icao), Some(c_reg), Some(c_type), Some(c_aty)) =
        (col("icao24"), col("registration"), col("typecode"), col("icaoaircrafttype"))
    else {
        return Err(anyhow!("columnas inesperadas en aircraftDatabase.csv"));
    };
    let (c_model, c_opicao, c_op, c_built) = (col("model"), col("operatoricao"), col("operator"), col("built"));
    fn get(r: &csv::StringRecord, c: Option<usize>) -> &str {
        c.and_then(|c| r.get(c)).unwrap_or("")
    }
    let mut out = Vec::with_capacity(150_000);
    for rec in rdr.records() {
        let Ok(r) = rec else { continue };
        let icao24 = get(&r, Some(c_icao)).trim().to_ascii_lowercase();
        if !keep_row(&icao24, get(&r, Some(c_aty)), get(&r, c_opicao), get(&r, Some(c_reg)), get(&r, Some(c_type))) {
            continue;
        }
        out.push(DbRow {
            icao24,
            registration: opt(get(&r, Some(c_reg))).map(|s| s.to_ascii_uppercase()),
            typecode: opt(get(&r, Some(c_type))).map(|s| s.to_ascii_uppercase()),
            model: opt(get(&r, c_model)),
            operator_icao: opt(get(&r, c_opicao)).map(|s| s.to_ascii_uppercase()),
            operator: opt(get(&r, c_op)),
            built: opt(get(&r, c_built)),
        });
    }
    Ok(out)
}

// ───────────────────────────── Descarga ─────────────────────────────

async fn setting(pool: &SqlitePool, key: &str) -> Option<String> {
    sqlx::query_scalar::<_, String>("SELECT value FROM settings WHERE key = ?")
        .bind(key)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
}

async fn set_setting(pool: &SqlitePool, key: &str, value: &str) -> Result<()> {
    sqlx::query("INSERT OR REPLACE INTO settings (key, value) VALUES (?, ?)")
        .bind(key)
        .bind(value)
        .execute(pool)
        .await?;
    Ok(())
}

async fn opensky_rows(pool: &SqlitePool) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM aircraft_db WHERE source = 'opensky'")
        .fetch_one(pool)
        .await
        .unwrap_or(0)
}

/// Baja la base de OpenSky si falta o si hay versión nueva (mirado cada 30
/// días). Devuelve cuántos aviones se cargaron, o `None` si no hizo falta.
pub async fn ensure_db(pool: &SqlitePool, http: &reqwest::Client) -> Result<Option<usize>> {
    let have = opensky_rows(pool).await > 0;
    let checked_recently = setting(pool, KEY_CHECKED)
        .await
        .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
        .is_some_and(|t| Utc::now() - t.with_timezone(&Utc) < ChronoDuration::days(CHECK_EVERY_DAYS));
    if have && checked_recently {
        return Ok(None);
    }
    let head = http.head(DB_URL).timeout(Duration::from_secs(30)).send().await?;
    let version = head
        .headers()
        .get(reqwest::header::LAST_MODIFIED)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    if have && !version.is_empty() && setting(pool, KEY_VERSION).await.as_deref() == Some(version.as_str()) {
        set_setting(pool, KEY_CHECKED, &Utc::now().to_rfc3339()).await?;
        return Ok(None);
    }

    tracing::info!(target: "aircraft_db", "bajando la base de aviones de OpenSky ({version})");
    let tmp = std::env::temp_dir().join(format!("simfleet-aircraftdb-{}.zip", std::process::id()));
    let resp = http.get(DB_URL).timeout(Duration::from_secs(600)).send().await?.error_for_status()?;
    {
        let mut f = std::fs::File::create(&tmp)?;
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            f.write_all(&chunk.context("descarga de la base de aviones")?)?;
        }
    }
    let path = tmp.clone();
    let rows = tokio::task::spawn_blocking(move || parse_zip(&path)).await?;
    let _ = std::fs::remove_file(&tmp);
    let rows = rows?;
    if rows.len() < 10_000 {
        return Err(anyhow!("base de aviones demasiado pequeña ({} filas)", rows.len()));
    }

    let mut tx = pool.begin().await?;
    // Los de adsbdb (más nuevos) se conservan; los "desconocidos" se reintentan.
    sqlx::query("DELETE FROM aircraft_db WHERE source IN ('opensky', 'none')").execute(&mut *tx).await?;
    for r in &rows {
        sqlx::query(
            "INSERT OR IGNORE INTO aircraft_db
                (icao24, registration, typecode, model, operator_icao, operator, built, source)
             VALUES (?,?,?,?,?,?,?, 'opensky')",
        )
        .bind(&r.icao24)
        .bind(&r.registration)
        .bind(&r.typecode)
        .bind(&r.model)
        .bind(&r.operator_icao)
        .bind(&r.operator)
        .bind(&r.built)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    set_setting(pool, KEY_VERSION, &version).await?;
    set_setting(pool, KEY_CHECKED, &Utc::now().to_rfc3339()).await?;
    tracing::info!(target: "aircraft_db", "base de aviones cargada: {} aviones", rows.len());
    Ok(Some(rows.len()))
}

// ───────────────────────────── adsbdb ─────────────────────────────

/// Extrae (matrícula, tipo ICAO, modelo, operador ICAO, operador) de una
/// respuesta de adsbdb. `None` si no conoce el avión.
fn parse_adsbdb(body: &str) -> Option<DbRow> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    let a = v.get("response")?.get("aircraft")?;
    let s = |k: &str| a.get(k).and_then(|x| x.as_str()).and_then(opt);
    Some(DbRow {
        icao24: s("mode_s")?.to_ascii_lowercase(),
        registration: s("registration").map(|x| x.to_ascii_uppercase()),
        typecode: s("icao_type").map(|x| x.to_ascii_uppercase()),
        model: s("type"),
        operator_icao: s("registered_owner_operator_flag_code").map(|x| x.to_ascii_uppercase()),
        operator: s("registered_owner"),
        built: None,
    })
}

async fn adsbdb_today(pool: &SqlitePool) -> i64 {
    let day = Utc::now().format("%Y-%m-%d").to_string();
    sqlx::query_scalar(
        "SELECT COUNT(*) FROM aircraft_db WHERE source IN ('adsbdb', 'none') AND substr(fetched_at, 1, 10) = ?",
    )
    .bind(day)
    .fetch_one(pool)
    .await
    .unwrap_or(0)
}

/// Busca en adsbdb los aviones vistos volando que no están en la base.
pub async fn enrich_unknown(pool: &SqlitePool, http: &reqwest::Client) -> Result<usize> {
    let budget = ADSBDB_DAILY - adsbdb_today(pool).await;
    if budget <= 0 {
        return Ok(0);
    }
    let since = Utc::now().timestamp() - FLEET_DAYS * 86_400;
    let retry_before = (Utc::now() - ChronoDuration::days(ADSBDB_RETRY_DAYS)).to_rfc3339();
    let todo: Vec<String> = sqlx::query_scalar(
        "SELECT rf.icao24 FROM real_flight rf
           LEFT JOIN aircraft_db a ON a.icao24 = rf.icao24
          WHERE rf.first_seen >= ?
            AND (a.icao24 IS NULL OR (a.source = 'none' AND a.fetched_at < ?))
          GROUP BY rf.icao24
          ORDER BY COUNT(*) DESC
          LIMIT ?",
    )
    .bind(since)
    .bind(&retry_before)
    .bind(budget)
    .fetch_all(pool)
    .await?;
    let mut done = 0usize;
    for icao24 in todo {
        let resp = match http
            .get(format!("{ADSBDB}/{icao24}"))
            .timeout(Duration::from_secs(20))
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                tracing::debug!(target: "aircraft_db", "adsbdb {icao24}: {e}");
                break;
            }
        };
        let status = resp.status();
        if status.as_u16() == 429 || status.is_server_error() {
            tracing::info!(target: "aircraft_db", "adsbdb respondió {status}: se sigue otro día");
            break;
        }
        let body = resp.text().await.unwrap_or_default();
        let row = parse_adsbdb(&body).filter(|r| r.icao24 == icao24);
        let now = Utc::now().to_rfc3339();
        match row {
            Some(r) => {
                sqlx::query(
                    "INSERT OR REPLACE INTO aircraft_db
                        (icao24, registration, typecode, model, operator_icao, operator, built, source, fetched_at)
                     VALUES (?,?,?,?,?,?,NULL,'adsbdb',?)",
                )
                .bind(&icao24)
                .bind(&r.registration)
                .bind(&r.typecode)
                .bind(&r.model)
                .bind(&r.operator_icao)
                .bind(&r.operator)
                .bind(&now)
                .execute(pool)
                .await?;
            }
            None => {
                sqlx::query("INSERT OR REPLACE INTO aircraft_db (icao24, source, fetched_at) VALUES (?, 'none', ?)")
                    .bind(&icao24)
                    .bind(&now)
                    .execute(pool)
                    .await?;
            }
        }
        done += 1;
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    if done > 0 {
        tracing::info!(target: "aircraft_db", "adsbdb: {done} aviones consultados");
    }
    Ok(done)
}

// ───────────────────────────── Flota ─────────────────────────────

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FleetAircraft {
    pub icao24: Option<String>,
    pub registration: Option<String>,
    pub typecode: Option<String>,
    pub model: Option<String>,
    /// Vuelos reales vistos en los últimos 30 días.
    pub flights_30d: i64,
    /// Último aterrizaje visto (Unix, s) y su aeropuerto (si OpenSky lo estimó).
    pub last_seen: Option<i64>,
    pub last_airport: Option<String>,
    /// Es un avión del usuario (lo voló con esta aerolínea).
    pub mine: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AirlineFleet {
    pub airline_icao: String,
    pub aircraft: Vec<FleetAircraft>,
    /// Aviones vistos volando en 30 días.
    pub seen: i64,
    /// Aviones del usuario en la flota.
    pub mine: i64,
    /// Registrados a nombre de la aerolínea (base 2024) sin vuelos vistos.
    pub registered_not_seen: i64,
    /// Vistos sin matrícula ni tipo conocidos (aún).
    pub unknown: i64,
    /// La base de aviones de OpenSky ya está cargada.
    pub db_ready: bool,
}

pub async fn fleet(pool: &SqlitePool, airline_icao: &str) -> Result<AirlineFleet> {
    let icao = airline_icao.trim().to_ascii_uppercase();
    let since = Utc::now().timestamp() - FLEET_DAYS * 86_400;
    let rows = sqlx::query(
        "SELECT rf.icao24 AS icao24, COUNT(*) AS n, MAX(rf.last_seen) AS ls,
                a.registration AS reg, a.typecode AS tc, a.model AS model
           FROM real_flight rf
           LEFT JOIN aircraft_db a ON a.icao24 = rf.icao24
          WHERE rf.airline_icao = ? AND rf.first_seen >= ?
          GROUP BY rf.icao24",
    )
    .bind(&icao)
    .bind(since)
    .fetch_all(pool)
    .await?;
    // Último aeropuerto de cada avión (el del vuelo que terminó más tarde).
    let mut last_airport: HashMap<String, (i64, Option<String>)> = HashMap::new();
    for r in sqlx::query(
        "SELECT icao24, last_seen, arr_icao FROM real_flight WHERE airline_icao = ? AND first_seen >= ?",
    )
    .bind(&icao)
    .bind(since)
    .fetch_all(pool)
    .await?
    {
        let ls: i64 = r.get("last_seen");
        let e = last_airport.entry(r.get("icao24")).or_insert((i64::MIN, None));
        if ls > e.0 {
            *e = (ls, r.get("arr_icao"));
        }
    }
    // (6.2d) Tuyos: los que volaste con ella + los de TechLog/pinturas cuyo
    // operador (base de aviones) es esta aerolínea. Y cualquier avión tuyo que
    // se vea volando con ella.
    let mut mine_regs = crate::airline_economy::user_registrations(pool, &icao).await.unwrap_or_default();
    if let Some(by_op) = crate::my_fleet::mine_by_operator(pool).await.unwrap_or_default().remove(&icao) {
        for (reg, a) in by_op {
            mine_regs.entry(reg).or_insert((a.registration, a.model));
        }
    }
    let all_mine = crate::my_fleet::my_aircraft(pool).await.unwrap_or_default();
    let mut seen_mine: HashSet<String> = HashSet::new();
    let mut aircraft: Vec<FleetAircraft> = rows
        .into_iter()
        .map(|r| {
            let icao24: String = r.get("icao24");
            let reg: Option<String> = r.get("reg");
            let mine = reg
                .as_deref()
                .map(normalize_reg)
                .is_some_and(|n| mine_regs.contains_key(&n) || all_mine.contains_key(&n));
            if mine {
                seen_mine.extend(reg.as_deref().map(normalize_reg));
            }
            FleetAircraft {
                last_airport: last_airport.get(&icao24).and_then(|(_, a)| a.clone()),
                icao24: Some(icao24),
                registration: reg,
                typecode: r.get("tc"),
                model: r.get("model"),
                flights_30d: r.get("n"),
                last_seen: r.get("ls"),
                mine,
            }
        })
        .collect();
    let seen = aircraft.len() as i64;
    let unknown = aircraft.iter().filter(|a| a.registration.is_none() && a.typecode.is_none()).count() as i64;
    // Los del usuario que no se vieron volar también son de la flota.
    for (reg, (display, model)) in &mine_regs {
        if !seen_mine.contains(reg) {
            aircraft.push(FleetAircraft {
                icao24: None,
                registration: Some(display.clone()),
                typecode: None,
                model: model.clone(),
                flights_30d: 0,
                last_seen: None,
                last_airport: None,
                mine: true,
            });
        }
    }
    aircraft.sort_by(|a, b| b.mine.cmp(&a.mine).then(b.flights_30d.cmp(&a.flights_30d)));
    let registered_not_seen: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM aircraft_db
          WHERE operator_icao = ? AND icao24 NOT IN
                (SELECT icao24 FROM real_flight WHERE airline_icao = ? AND first_seen >= ?)",
    )
    .bind(&icao)
    .bind(&icao)
    .bind(since)
    .fetch_one(pool)
    .await?;
    Ok(AirlineFleet {
        airline_icao: icao,
        mine: aircraft.iter().filter(|a| a.mine).count() as i64,
        aircraft,
        seen,
        registered_not_seen,
        unknown,
        db_ready: opensky_rows(pool).await > 0,
    })
}

/// Estado de la base (para Ajustes).
pub async fn db_status(pool: &SqlitePool) -> (i64, Option<String>, i64) {
    let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM aircraft_db WHERE source IN ('opensky', 'adsbdb')")
        .fetch_one(pool)
        .await
        .unwrap_or(0);
    (rows, setting(pool, KEY_VERSION).await.filter(|v| !v.is_empty()), adsbdb_today(pool).await)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_transport_and_airline_aircraft_only() {
        assert!(keep_row("a1b2c3", "L2J", "", "N123AA", "A321"));
        assert!(keep_row("A1B2C3", "L2T", "", "", "AT76"), "turbohélice con tipo");
        assert!(keep_row("4ca123", "", "RYR", "EI-ABC", ""), "de aerolínea aunque falte el tipo");
        assert!(!keep_row("aa3487", "L1P", "", "N757F", "BE36"), "avioneta privada");
        assert!(!keep_row("", "L2J", "", "N1", "A320"), "sin icao24");
        assert!(!keep_row("zz0000", "L2J", "", "N1", "A320"), "icao24 no hex");
        assert!(!keep_row("a1b2c3", "L2J", "", "", ""), "sin matrícula ni tipo");
        assert!(!keep_row("a1b2c3", "H2T", "", "N1", "S92"), "helicóptero");
    }

    #[test]
    fn reads_the_opensky_csv_inside_the_zip() {
        let csv = concat!(
            "\"icao24\",\"registration\",\"manufacturericao\",\"manufacturername\",\"model\",\"typecode\",",
            "\"serialnumber\",\"linenumber\",\"icaoaircrafttype\",\"operator\",\"operatorcallsign\",",
            "\"operatoricao\",\"operatoriata\",\"owner\",\"testreg\",\"registered\",\"reguntil\",\"status\",",
            "\"built\",\"firstflightdate\",\"seatconfiguration\",\"engines\",\"modes\",\"adsb\",\"acars\",",
            "\"notes\",\"categoryDescription\"\n",
            "\"\",\"\",\"\",\"\",\"\",\"\",\"\",\"\",\"\",\"\",\"\",\"\",\"\",\"\",\"\",\"\",\"\",\"\",\"\",\"\",\"\",\"\",\"false\",\"false\",\"false\",\"\",\"\"\n",
            "\"aa3487\",\"N757F\",\"RAYTHEON\",\"Raytheon\",\"A36\",\"BE36\",\"\",\"\",\"L1P\",\"\",\"\",\"\",\"\",\"x\",\"\",\"\",\"\",\"\",\"\",\"\",\"\",\"\",\"false\",\"false\",\"false\",\"\",\"\"\n",
            "\"C05F1A\",\"c-gorp\",\"BOEING\",\"Boeing\",\"737-8 MAX\",\"b38m\",\"\",\"\",\"L2J\",\"WestJet\",\"WESTJET\",\"wja\",\"WS\",\"\",\"\",\"\",\"\",\"\",\"2019-05-01\",\"\",\"\",\"\",\"false\",\"false\",\"false\",\"\",\"\"\n",
        );
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("aircraftDatabase.zip");
        {
            let f = std::fs::File::create(&path).unwrap();
            let mut z = zip::ZipWriter::new(f);
            z.start_file("aircraftDatabase.csv", zip::write::SimpleFileOptions::default()).unwrap();
            z.write_all(csv.as_bytes()).unwrap();
            z.finish().unwrap();
        }
        let rows = parse_zip(&path).unwrap();
        assert_eq!(rows.len(), 1, "sólo el 737 de WestJet");
        let r = &rows[0];
        assert_eq!(r.icao24, "c05f1a");
        assert_eq!(r.registration.as_deref(), Some("C-GORP"));
        assert_eq!(r.typecode.as_deref(), Some("B38M"));
        assert_eq!(r.operator_icao.as_deref(), Some("WJA"));
        assert_eq!(r.built.as_deref(), Some("2019-05-01"));
    }

    #[test]
    fn parses_adsbdb_aircraft_and_unknown() {
        let ok = r#"{"response":{"aircraft":{"type":"A321-271NX","icao_type":"a21n","manufacturer":"Airbus",
            "mode_s":"A1B2C3","registration":"n123aa","registered_owner_operator_flag_code":"AAL",
            "registered_owner":"American Airlines"}}}"#;
        let r = parse_adsbdb(ok).unwrap();
        assert_eq!(r.icao24, "a1b2c3");
        assert_eq!(r.registration.as_deref(), Some("N123AA"));
        assert_eq!(r.typecode.as_deref(), Some("A21N"));
        assert_eq!(r.operator_icao.as_deref(), Some("AAL"));
        assert!(parse_adsbdb(r#"{"response":"unknown aircraft"}"#).is_none());
        assert!(parse_adsbdb("no json").is_none());
    }
}
