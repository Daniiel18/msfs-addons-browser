//! (8.4.x) Servicios de escala que cobran dos sistemas a la vez.
//!
//! El agua potable, los baños y el catering se pueden pedir en el Tech Log
//! (el de SimFleet o Fenix/PMDG TechLog) Y en GSX: el avión queda servido una
//! vez, pero Finanzas los cobraba dos veces (el Tech Log al pedirlo y el
//! recibo de GSX al repartirse por vuelo). Decisión del usuario: cobra GSX.
//!
//! Cómo: el cobro del Tech Log se queda en el historial y, si en la MISMA
//! escala GSX cobra ese servicio, se añade justo con él una línea de
//! reembolso (`refund:gsx:<componente original>`, importe negativo, misma
//! fecha). Contabilidad trazable: nada se borra y el neto es un solo cobro.
//!
//! Escala = entre la llegada del avión (último vuelo terminado antes del
//! cobro) y su siguiente salida. Una línea de GSX es de esa escala si GSX la
//! repartió a ese vuelo de llegada (lado "arr") o al de salida (lado "dep"):
//! el mismo reparto que usa la economía, así que sólo cuenta lo que GSX
//! REALMENTE cobra (no los intentos abortados ni lo que aún no tiene vuelo).
//!
//! Es idempotente y va en las dos direcciones: añade los reembolsos que
//! faltan, ajusta su importe si el cobro original cambió y quita (anotándolo
//! para la nube) los que ya no valen — p. ej. si se borró el vuelo con el
//! recibo de GSX, el cobro del Tech Log vuelve a contar.

use std::collections::{HashMap, HashSet};

use anyhow::Result;
use chrono::{DateTime, Utc};
use sqlx::{Row, SqlitePool};

/// Prefijo del componente de un reembolso en `maintenance_log`.
pub const REFUND_PREFIX: &str = "refund:gsx:";

/// Servicio de escala que pueden cobrar el Tech Log y GSX.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Svc {
    Water,
    Waste,
    Catering,
}

impl Svc {
    /// Clave para los textos (`economy.svc*`).
    pub fn label_key(self) -> &'static str {
        match self {
            Svc::Water => "economy.svcWater",
            Svc::Waste => "economy.svcWaste",
            Svc::Catering => "economy.svcCatering",
        }
    }
}

/// Servicio de un cobro del Tech Log (`mx:service:WATER`,
/// `techlog:service:CATERING`…). `None` si no es uno de estos.
pub fn svc_of_charge(component: &str) -> Option<Svc> {
    let rest = component
        .strip_prefix("mx:service:")
        .or_else(|| component.strip_prefix("techlog:service:"))?;
    match rest.split(':').next()?.to_ascii_uppercase().as_str() {
        "WATER" => Some(Svc::Water),
        "WASTE" => Some(Svc::Waste),
        "CATERING" => Some(Svc::Catering),
        _ => None,
    }
}

/// Servicio de una línea de un recibo de GSX ("Potable water (medium
/// aircraft)", "Lavatory service…", catering). Las tasas de cancelación no
/// son el servicio.
pub fn svc_of_gsx(kind: &str, service: &str) -> Option<Svc> {
    if kind == "catering" {
        return Some(Svc::Catering);
    }
    if kind != "handling" {
        return None;
    }
    let s = service.to_ascii_lowercase();
    if s.contains("cancel") || s.contains("call-out") {
        return None;
    }
    if s.contains("lavatory") || s.contains("toilet") || s.contains("waste") {
        return Some(Svc::Waste);
    }
    s.contains("water").then_some(Svc::Water)
}

/// Un vuelo terminado, para situar las escalas.
#[derive(Debug, Clone)]
pub struct Span {
    pub id: i64,
    /// Matrícula normalizada (`airline_economy::normalize_reg`).
    pub reg: String,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
}

/// Una línea de GSX que GSX cobra a un vuelo.
#[derive(Debug, Clone)]
pub struct Cover {
    pub reg: String,
    pub svc: Svc,
    pub flight_id: i64,
    /// Lado de llegada (si no, de salida).
    pub arr: bool,
}

/// (vuelo de llegada, vuelo de salida) de la escala en la que estaba `reg`
/// en `at`: el último vuelo terminado antes y el primero que termina después.
fn turnaround(reg: &str, at: DateTime<Utc>, spans: &[Span]) -> (Option<i64>, Option<i64>) {
    let mine = spans.iter().filter(|s| s.reg == reg);
    let arrival = mine.clone().filter(|s| s.end <= at).max_by_key(|s| s.end).map(|s| s.id);
    let departure = mine.filter(|s| s.end > at).min_by_key(|s| s.start).map(|s| s.id);
    (arrival, departure)
}

/// ¿GSX cobra `svc` en la escala en la que estaba `reg` en `at`?
pub fn covered(reg: &str, svc: Svc, at: DateTime<Utc>, spans: &[Span], covers: &[Cover]) -> bool {
    let (arrival, departure) = turnaround(reg, at, spans);
    covers.iter().any(|c| {
        c.reg == reg
            && c.svc == svc
            && ((c.arr && Some(c.flight_id) == arrival) || (!c.arr && Some(c.flight_id) == departure))
    })
}

fn parse_at(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .map(|d| d.with_timezone(&Utc))
        .ok()
        .or_else(|| {
            chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S")
                .ok()
                .map(|n| n.and_utc())
        })
}

/// Qué cambió en una pasada.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Outcome {
    /// Cobros (matrícula, componente, fecha) que quedaron reembolsados ahora.
    pub refunded: Vec<(String, String, String)>,
    pub updated: usize,
    pub removed: usize,
}

/// Pone al día los reembolsos de `maintenance_log` con las escalas
/// (`spans`) y lo que GSX cobra (`covers`).
pub async fn reconcile(pool: &SqlitePool, spans: &[Span], covers: &[Cover]) -> Result<Outcome> {
    let rows = sqlx::query(
        "SELECT id, registration, component, cost, serviced_at FROM maintenance_log
          WHERE component LIKE 'mx:service:%' OR component LIKE 'techlog:service:%' OR component LIKE 'refund:gsx:%'",
    )
    .fetch_all(pool)
    .await?;
    // Reembolsos que deberían existir: (matrícula, componente original, fecha) → importe.
    let mut wanted: HashMap<(String, String, String), f64> = HashMap::new();
    // Reembolsos que existen: misma clave → (id, importe).
    let mut existing: HashMap<(String, String, String), (i64, f64)> = HashMap::new();
    for r in &rows {
        let reg: String = r.try_get("registration").unwrap_or_default();
        let component: String = r.try_get("component").unwrap_or_default();
        let cost: f64 = r.try_get("cost").unwrap_or(0.0);
        let at: String = r.try_get("serviced_at").unwrap_or_default();
        if let Some(original) = component.strip_prefix(REFUND_PREFIX) {
            existing.insert((reg, original.to_string(), at), (r.try_get("id").unwrap_or(0), cost));
            continue;
        }
        let (Some(svc), Some(when)) = (svc_of_charge(&component), parse_at(&at)) else { continue };
        if cost > 0.0 && covered(&reg, svc, when, spans, covers) {
            wanted.insert((reg, component, at), -cost);
        }
    }

    let mut out = Outcome::default();
    for (key, amount) in &wanted {
        let (reg, original, at) = key;
        match existing.get(key) {
            Some((_, have)) if (have - amount).abs() < 0.005 => {}
            Some((id, _)) => {
                sqlx::query("UPDATE maintenance_log SET cost = ?2 WHERE id = ?1")
                    .bind(id)
                    .bind(amount)
                    .execute(pool)
                    .await?;
                out.updated += 1;
            }
            None => {
                let component = format!("{REFUND_PREFIX}{original}");
                sqlx::query(
                    "INSERT OR IGNORE INTO maintenance_log (registration, component, cost, serviced_at) VALUES (?1,?2,?3,?4)")
                    .bind(reg)
                    .bind(&component)
                    .bind(amount)
                    .bind(at)
                    .execute(pool)
                    .await?;
                let mk = crate::ledger_sync::maint_key(reg, &component, at);
                crate::ledger_sync::revive(pool, crate::ledger_sync::KIND_MAINT, &mk).await?;
                tracing::info!(target: "economy", "{reg}: {original} lo cobra GSX en esa escala → reembolso de {:.0} USD", -amount);
                out.refunded.push(key.clone());
            }
        }
    }
    let stale: HashSet<_> = existing.keys().filter(|k| !wanted.contains_key(*k)).cloned().collect();
    for key in stale {
        let (id, _) = existing[&key];
        let (reg, original, at) = &key;
        sqlx::query("DELETE FROM maintenance_log WHERE id = ?1").bind(id).execute(pool).await?;
        let mk = crate::ledger_sync::maint_key(reg, &format!("{REFUND_PREFIX}{original}"), at);
        crate::ledger_sync::record(pool, crate::ledger_sync::KIND_MAINT, &mk).await?;
        tracing::info!(target: "economy", "{reg}: quitado el reembolso de {original} ({at}): GSX ya no lo cobra en esa escala");
        out.removed += 1;
    }
    Ok(out)
}

/// `reconcile` con lo guardado: los vuelos del FlightBook y las líneas de GSX
/// con el vuelo al que las repartió la última pasada de la economía.
pub async fn reconcile_from_db(pool: &SqlitePool) -> Result<Outcome> {
    let spans: Vec<Span> = crate::flight_log::list_entries(pool)
        .await?
        .iter()
        .filter_map(|e| {
            let reg = crate::airline_economy::normalize_reg(e.aircraft_registration.as_deref()?);
            let start = parse_at(&e.started_at)?;
            let end = e.ended_at.as_deref().and_then(parse_at).unwrap_or(start);
            (!reg.is_empty()).then_some(Span { id: e.id, reg, start, end })
        })
        .collect();
    let covers: Vec<Cover> = sqlx::query(
        "SELECT kind, service, side, registration, flight_id FROM gsx_charge
          WHERE flight_id IS NOT NULL AND is_cost = 1 AND amount_usd > 0",
    )
    .fetch_all(pool)
    .await?
    .iter()
    .filter_map(|r| {
        let kind: String = r.try_get("kind").ok()?;
        let service: String = r.try_get("service").ok()?;
        Some(Cover {
            reg: r.try_get("registration").ok()?,
            svc: svc_of_gsx(&kind, &service)?,
            flight_id: r.try_get("flight_id").ok()?,
            arr: r.try_get::<String, _>("side").ok()? == "arr",
        })
    })
    .collect();
    reconcile(pool, &spans, &covers).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> DateTime<Utc> {
        parse_at(s).unwrap()
    }

    /// Los vuelos reales de C-GORP del 26–28/10.
    fn cgorp() -> Vec<Span> {
        let f = |id, a, b| Span { id, reg: "CGORP".into(), start: t(a), end: t(b) };
        vec![
            f(281, "2026-10-27T03:43:46Z", "2026-10-27T06:30:36Z"), // KSFO → CYYC
            f(282, "2026-10-27T14:52:44Z", "2026-10-27T19:42:10Z"), // CYYC → MMPR
            f(283, "2026-10-27T20:32:14Z", "2026-10-28T01:17:35Z"), // MMPR → CYYC
        ]
    }

    #[test]
    fn services_are_recognised_on_both_sides() {
        assert_eq!(svc_of_charge("mx:service:WATER"), Some(Svc::Water));
        assert_eq!(svc_of_charge("techlog:service:WASTE"), Some(Svc::Waste));
        assert_eq!(svc_of_charge("mx:service:CATERING"), Some(Svc::Catering));
        assert_eq!(svc_of_charge("mx:service:ENG_OIL:1"), None);
        assert_eq!(svc_of_charge("mx:check:A"), None);
        assert_eq!(svc_of_gsx("handling", "Potable water (medium aircraft)"), Some(Svc::Water));
        assert_eq!(svc_of_gsx("handling", "Lavatory service (medium aircraft)"), Some(Svc::Waste));
        assert_eq!(svc_of_gsx("catering", "Hot meal service"), Some(Svc::Catering));
        assert_eq!(svc_of_gsx("handling", "Call-out fee (service cancelled before start)"), None);
        assert_eq!(svc_of_gsx("handling", "Passenger boarding"), None);
        assert_eq!(svc_of_gsx("fuel", "Jet-A1 fuel"), None);
    }

    /// Los casos reales: agua pedida en el Tech Log en MMPR (19:51, tras
    /// llegar el 282) y el agua de GSX cobrada a la salida del 283; los baños
    /// en CYYC (06:37, tras llegar el 281) y el "Lavatory" de GSX a la llegada
    /// del 281.
    #[test]
    fn the_same_turnaround_is_found_from_the_gsx_split() {
        let spans = cgorp();
        let covers = vec![
            Cover { reg: "CGORP".into(), svc: Svc::Water, flight_id: 283, arr: false },
            Cover { reg: "CGORP".into(), svc: Svc::Waste, flight_id: 281, arr: true },
        ];
        assert!(covered("CGORP", Svc::Water, t("2026-10-27T19:51:19Z"), &spans, &covers));
        assert!(covered("CGORP", Svc::Waste, t("2026-10-27T06:37:16Z"), &spans, &covers));
        // Otro servicio, otra escala u otro avión: no.
        assert!(!covered("CGORP", Svc::Catering, t("2026-10-27T19:51:19Z"), &spans, &covers));
        assert!(!covered("CGORP", Svc::Water, t("2026-10-27T07:00:00Z"), &spans, &covers));
        assert!(!covered("CGORP", Svc::Waste, t("2026-10-27T19:51:19Z"), &spans, &covers));
        assert!(!covered("CFAKE", Svc::Water, t("2026-10-27T19:51:19Z"), &spans, &covers));
    }

    /// Un servicio pedido con el vuelo de salida ya abierto (antes de
    /// despegar) sigue siendo de esa escala.
    #[test]
    fn a_service_before_takeoff_belongs_to_the_departing_flight() {
        let spans = cgorp();
        let covers = vec![Cover { reg: "CGORP".into(), svc: Svc::Water, flight_id: 283, arr: false }];
        assert!(covered("CGORP", Svc::Water, t("2026-10-27T20:35:00Z"), &spans, &covers));
    }

    /// Con una COPIA de la base real (no corre por defecto):
    /// `REFUND_DB=<copia .db> cargo test --lib refunds_real_copy -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn refunds_real_copy() {
        let Ok(path) = std::env::var("REFUND_DB") else { return };
        let pool = SqlitePool::connect(&format!("sqlite:{path}")).await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        let o = reconcile_from_db(&pool).await.unwrap();
        for (reg, comp, at) in &o.refunded {
            println!("reembolso: {reg} {comp} {at}");
        }
        println!("{o:?}");
        assert_eq!(reconcile_from_db(&pool).await.unwrap(), Outcome::default(), "idempotente");
    }

    async fn pool() -> SqlitePool {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool
    }

    async fn ledger(pool: &SqlitePool) -> Vec<(String, f64)> {
        sqlx::query_as("SELECT component, cost FROM maintenance_log ORDER BY component, serviced_at")
            .fetch_all(pool)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn refunds_follow_what_gsx_bills() {
        let pool = pool().await;
        for (comp, cost, at) in [
            ("mx:service:WATER", 45.0, "2026-10-27T19:51:19.934852400+00:00"),
            ("mx:service:WASTE", 53.0, "2026-10-27T06:37:16.892178900+00:00"),
            ("mx:service:CABIN_CLEAN", 188.0, "2026-10-27T06:54:21.315030600+00:00"),
        ] {
            sqlx::query("INSERT INTO maintenance_log (registration, component, cost, serviced_at) VALUES ('CGORP', ?1, ?2, ?3)")
                .bind(comp)
                .bind(cost)
                .bind(at)
                .execute(&pool)
                .await
                .unwrap();
        }
        let spans = cgorp();
        let covers = vec![
            Cover { reg: "CGORP".into(), svc: Svc::Water, flight_id: 283, arr: false },
            Cover { reg: "CGORP".into(), svc: Svc::Waste, flight_id: 281, arr: true },
        ];
        let o = reconcile(&pool, &spans, &covers).await.unwrap();
        assert_eq!(o.refunded.len(), 2);
        let total: f64 = ledger(&pool).await.iter().map(|(_, c)| c).sum();
        assert_eq!(total, 188.0, "agua y baños los cobra GSX; la limpieza no");
        // Idempotente.
        assert_eq!(reconcile(&pool, &spans, &covers).await.unwrap(), Outcome::default());

        // Se borra el vuelo del recibo del agua: el cobro del Tech Log vuelve a contar.
        let o = reconcile(&pool, &spans, &covers[1..]).await.unwrap();
        assert_eq!(o.removed, 1);
        let total: f64 = ledger(&pool).await.iter().map(|(_, c)| c).sum();
        assert_eq!(total, 188.0 + 45.0);
        let dead: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sync_tombstone WHERE kind = 'maint' AND revived_at IS NULL")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(dead, 1, "la nube no debe devolver el reembolso quitado");

        // Y si vuelve a estar cubierto, el reembolso vuelve (y el borrado se revive).
        reconcile(&pool, &spans, &covers).await.unwrap();
        let total: f64 = ledger(&pool).await.iter().map(|(_, c)| c).sum();
        assert_eq!(total, 188.0);
        let dead: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sync_tombstone WHERE kind = 'maint' AND (revived_at IS NULL OR deleted_at > revived_at)",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(dead, 0);

        // Si el cobro original cambia de importe, el reembolso lo sigue.
        sqlx::query("UPDATE maintenance_log SET cost = 50 WHERE component = 'mx:service:WATER'")
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(reconcile(&pool, &spans, &covers).await.unwrap().updated, 1);
        let total: f64 = ledger(&pool).await.iter().map(|(_, c)| c).sum();
        assert_eq!(total, 188.0);
    }
}
