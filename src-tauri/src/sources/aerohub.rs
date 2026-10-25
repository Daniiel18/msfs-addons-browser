//! (v8.3) aerohub.st — catálogo de addons (MSFS 2024/2020 y X-Plane).
//!
//! HTML servido por nginx, sin Cloudflare: listado `/browse?p=N` (24 por
//! página, `rel="next"` en el paginador), búsqueda `/browse?q=…` y ficha
//! `/addon/<slug>` con los botones de descarga (`.detail__actions`) y las notas
//! de instalación (`.detail__notes-text`). Las descargas viven en VaultDrop y
//! ModsFire; el resto de enlaces de la ficha (Telegram, otra ficha de aerohub,
//! flightsim.to…) no son descargas y se omiten. X-Plane se oculta: SimFleet es
//! sólo MSFS.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use futures_util::{stream, StreamExt};
use once_cell::sync::Lazy;
use regex::Regex;
use scraper::{ElementRef, Html, Selector};

use super::{stable_id, Addon, BrowsePage, DownloadKind, DownloadMethod, Source, SourceError};

const BASE: &str = "https://aerohub.st";

/// Fichas pedidas en paralelo (una por addon de la página, para sus enlaces).
/// aerohub limita las ráfagas: con 6 a la vez respondía 503 a parte de ellas.
const DETAIL_CONCURRENCY: usize = 3;

/// Intentos por petición cuando el sitio responde 503/429 (límite de ráfaga).
const FETCH_ATTEMPTS: u32 = 4;

/// Una ficha ya leída se reutiliza este tiempo (volver de página, buscar…).
const DETAIL_TTL: Duration = Duration::from_secs(30 * 60);

/// Páginas de resultados (24 cada una) que recorre una búsqueda.
const SEARCH_MAX_PAGES: usize = 3;

pub struct AerohubSource {
    client: reqwest::Client,
    details: Mutex<HashMap<String, (Instant, Detail)>>,
}

impl AerohubSource {
    pub fn new(client: reqwest::Client) -> Self {
        Self { client, details: Mutex::new(HashMap::new()) }
    }

    /// GET con reintentos: el nginx de aerohub contesta 503 (o 429) cuando
    /// llegan varias peticiones juntas; esperar un poco basta.
    async fn fetch_html(&self, url: &str) -> Result<String, SourceError> {
        let mut attempt = 0;
        loop {
            attempt += 1;
            let resp = self.client.get(url).send().await?;
            let code = resp.status().as_u16();
            if (code == 503 || code == 429) && attempt < FETCH_ATTEMPTS {
                tokio::time::sleep(Duration::from_millis(700 * u64::from(attempt))).await;
                continue;
            }
            return Ok(resp.error_for_status()?.text().await?);
        }
    }

    async fn detail(&self, page_url: &str) -> Detail {
        if let Some((at, d)) = self.details.lock().ok().and_then(|c| c.get(page_url).cloned()) {
            if at.elapsed() < DETAIL_TTL {
                return d;
            }
        }
        match self.fetch_html(page_url).await {
            Ok(html) => {
                let d = parse_detail(&html);
                if let Ok(mut c) = self.details.lock() {
                    c.insert(page_url.to_string(), (Instant::now(), d.clone()));
                }
                d
            }
            Err(e) => {
                tracing::warn!("aerohub: could not fetch detail page {}: {}", page_url, e);
                Detail::default()
            }
        }
    }

    /// Completa cada tarjeta con su ficha (enlaces + notas), conservando el
    /// orden del sitio.
    async fn with_details(&self, tiles: Vec<Tile>) -> Vec<Addon> {
        stream::iter(tiles.into_iter().map(|t| async move {
            let d = self.detail(&t.page_url).await;
            to_addon(t, d)
        }))
        .buffered(DETAIL_CONCURRENCY)
        .collect()
        .await
    }
}

#[async_trait]
impl Source for AerohubSource {
    fn id(&self) -> &'static str { "aerohub" }
    fn name(&self) -> &'static str { "AeroHub" }
    fn home_url(&self) -> &'static str { BASE }

    async fn search(&self, query: &str) -> Result<Vec<Addon>, SourceError> {
        let q = query.trim();
        if q.is_empty() {
            return Ok(vec![]);
        }
        let mut tiles = Vec::new();
        for page in 1..=SEARCH_MAX_PAGES {
            let mut url = format!("{}/browse?q={}", BASE, urlencoding::encode(q));
            if page > 1 {
                url.push_str(&format!("&p={page}"));
            }
            tracing::info!("aerohub: searching {}", url);
            let html = self.fetch_html(&url).await?;
            let (found, has_more) = parse_tiles(&html);
            tiles.extend(found);
            if !has_more {
                break;
            }
        }
        let tiles: Vec<Tile> = tiles.into_iter().filter(|t| sim_matches(&t.sim)).collect();
        let addons = self.with_details(tiles).await;
        tracing::info!("aerohub: {} results for {:?}", addons.len(), q);
        Ok(addons)
    }

    async fn browse(&self, page: usize) -> Result<BrowsePage, SourceError> {
        let url = browse_url(page);
        tracing::info!("aerohub: browsing {}", url);
        let html = self.fetch_html(&url).await?;
        let (tiles, has_more) = parse_tiles(&html);
        let tiles: Vec<Tile> = tiles.into_iter().filter(|t| sim_matches(&t.sim)).collect();
        let addons = self.with_details(tiles).await;
        Ok(BrowsePage { addons, page, has_more })
    }

    /// (25/10) Para las updates basta la tarjeta (trae la versión): una
    /// petición por página en vez de 24 fichas.
    async fn listing(&self, page: usize) -> Result<BrowsePage, SourceError> {
        let html = self.fetch_html(&browse_url(page)).await?;
        let (tiles, has_more) = parse_tiles(&html);
        let addons = tiles
            .into_iter()
            .filter(|t| sim_matches(&t.sim))
            .map(|t| to_addon(t, Detail::default()))
            .collect();
        Ok(BrowsePage { addons, page, has_more })
    }
}

fn browse_url(page: usize) -> String {
    if page <= 1 {
        format!("{}/browse", BASE)
    } else {
        format!("{}/browse?p={}", BASE, page)
    }
}

/// Tarjeta del listado (`a.tile`).
struct Tile {
    page_url: String,
    name: String,
    developer: Option<String>,
    version: Option<String>,
    /// `data-sim`: `msfs2024` | `msfs2020` | `msfs-20-24` | `xplane…`.
    sim: String,
    /// `Plane` | `Helicopter` | `Scenery` | `Utility`.
    kind: Option<String>,
    image_url: Option<String>,
}

/// Lo que aporta la ficha del addon.
#[derive(Default, Clone)]
struct Detail {
    methods: Vec<DownloadMethod>,
    notes: Option<String>,
    added_at: Option<String>,
}

/// ¿El addon es del sim activo? `msfs-20-24` vale para ambos; X-Plane nunca.
fn sim_matches(sim: &str) -> bool {
    match sim {
        "msfs-20-24" => true,
        "msfs2024" => crate::sim::is_2024(),
        "msfs2020" => !crate::sim::is_2024(),
        _ => false,
    }
}

/// Misma etiqueta que usan las demás fuentes (la lee el ruteo 2020/2024).
fn sim_label(sim: &str) -> &'static str {
    match sim {
        "msfs2024" => "MSFS 2024",
        "msfs2020" => "MSFS 2020",
        _ => "MSFS 2020/2024",
    }
}

fn sel(s: &str) -> Selector {
    Selector::parse(s).expect("selector literal")
}

fn first_text(el: &ElementRef<'_>, s: &Selector) -> Option<String> {
    el.select(s)
        .map(|e| e.text().collect::<String>().trim().to_string())
        .find(|t| !t.is_empty())
}

/// Tarjetas de una página del listado + si hay página siguiente.
fn parse_tiles(html: &str) -> (Vec<Tile>, bool) {
    let doc = Html::parse_document(html);
    let tile_sel = sel("a.tile[href^='/addon/']");
    let name_sel = sel(".tile__name");
    let dev_sel = sel(".tile__dev");
    let ver_sel = sel(".tile__ver");
    let sim_sel = sel(".tile__sim[data-sim]");
    let kind_sel = sel(".tile__kind .sr-only");
    let img_sel = sel("img.tile__img");

    let tiles = doc
        .select(&tile_sel)
        .filter_map(|a| {
            let href = a.value().attr("href")?.trim();
            let name = first_text(&a, &name_sel)?;
            let sim = a
                .select(&sim_sel)
                .next()
                .and_then(|s| s.value().attr("data-sim"))
                .unwrap_or("")
                .to_ascii_lowercase();
            let version = first_text(&a, &ver_sel).map(|v| {
                let v = v.trim();
                match v.strip_prefix(['v', 'V']) {
                    Some(rest) if rest.starts_with(|c: char| c.is_ascii_digit()) => rest.to_string(),
                    _ => v.to_string(),
                }
            });
            Some(Tile {
                page_url: format!("{BASE}{href}"),
                name,
                developer: first_text(&a, &dev_sel),
                version,
                sim,
                kind: first_text(&a, &kind_sel),
                image_url: a
                    .select(&img_sel)
                    .next()
                    .and_then(|i| i.value().attr("src"))
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty()),
            })
        })
        .collect();
    let has_more = doc.select(&sel("nav.pager a[rel='next']")).next().is_some();
    (tiles, has_more)
}

/// Enlaces de descarga, notas y fecha de alta de la ficha.
fn parse_detail(html: &str) -> Detail {
    let doc = Html::parse_document(html);
    let span_sel = sel("span");
    let links: Vec<(String, String)> = doc
        .select(&sel(".detail__actions a[href]"))
        .map(|a| {
            let href = a.value().attr("href").unwrap_or("").trim().to_string();
            let label = first_text(&a, &span_sel).unwrap_or_default();
            (label, href)
        })
        .collect();
    let notes = doc
        .select(&sel(".detail__notes-text"))
        .next()
        .map(|n| clean_notes(&n.text().collect::<String>()))
        .filter(|n| !n.is_empty());
    let added_at = doc
        .select(&sel(".detail__meta"))
        .next()
        .and_then(|m| parse_added_date(&m.text().collect::<String>()));
    Detail { methods: build_methods(links), notes, added_at }
}

/// Recorta cada línea y deja como mucho una línea en blanco seguida.
fn clean_notes(raw: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    for line in raw.lines().map(str::trim) {
        if line.is_empty() && out.last().map_or(true, |l| l.is_empty()) {
            continue;
        }
        out.push(line);
    }
    while out.last().is_some_and(|l| l.is_empty()) {
        out.pop();
    }
    out.join("\n")
}

/// "Added Oct 21, 2026 for MSFS 2024" → "2026-10-21T12:00:00Z". Mediodía UTC
/// para que el día no cambie al mostrarlo en la zona horaria local.
fn parse_added_date(meta: &str) -> Option<String> {
    static RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r"(?i)\b(jan|feb|mar|apr|may|jun|jul|aug|sep|oct|nov|dec)[a-z]*\.?\s+(\d{1,2}),\s*(\d{4})")
            .unwrap()
    });
    const MONTHS: [&str; 12] = [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ];
    let c = RE.captures(meta)?;
    let month = MONTHS.iter().position(|m| c[1].eq_ignore_ascii_case(m))? + 1;
    let day: u32 = c[2].parse().ok()?;
    Some(format!("{}-{month:02}-{day:02}T12:00:00Z", &c[3]))
}

/// Host de descarga soportado por el navegador de descargas de SimFleet.
fn download_host(url: &str) -> Option<&'static str> {
    let u = url::Url::parse(url).ok()?;
    let host = u.host_str()?.to_ascii_lowercase();
    match host.trim_start_matches("www.") {
        "vaultdrop.st" => Some("VaultDrop"),
        "modsfire.com" => Some("ModsFire"),
        _ => None,
    }
}

/// "Part 1" / "Liveries Part 2" → (prefijo, número).
static PART_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)^(.*?)\s*\bpart\s*(\d+)\s*$").unwrap());

fn part_of(label: &str) -> Option<(String, u32)> {
    let c = PART_RE.captures(label)?;
    Some((c[1].trim().to_string(), c[2].parse().ok()?))
}

/// Convierte los botones de la ficha en métodos de descarga:
///   · sólo VaultDrop y ModsFire (el resto no son descargas);
///   · los botones "Part 1", "Part 2"… de un mismo prefijo se unen en UN método
///     multi-parte (`parts`), que el navegador baja en orden y une;
///   · la variante del sim activo ("Download for 2024", "2024"…) va primero.
fn build_methods(links: Vec<(String, String)>) -> Vec<DownloadMethod> {
    let links: Vec<(String, String, &'static str)> = links
        .into_iter()
        .filter_map(|(label, url)| download_host(&url).map(|h| (label, url, h)))
        .collect();

    let mut used = vec![false; links.len()];
    let mut out = Vec::new();
    for i in 0..links.len() {
        if used[i] {
            continue;
        }
        if let Some((prefix, _)) = part_of(&links[i].0) {
            let mut group: Vec<(u32, usize)> = links
                .iter()
                .enumerate()
                .filter(|(j, _)| !used[*j])
                .filter_map(|(j, (label, _, _))| {
                    let (p, n) = part_of(label)?;
                    p.eq_ignore_ascii_case(&prefix).then_some((n, j))
                })
                .collect();
            group.sort();
            let complete = group.len() >= 2
                && group.iter().enumerate().all(|(k, (n, _))| *n as usize == k + 1);
            if complete {
                for (_, j) in &group {
                    used[*j] = true;
                }
                let (_, first_url, host) = &links[group[0].1];
                let label = if prefix.is_empty() { "Download" } else { prefix.as_str() };
                out.push(DownloadMethod {
                    kind: DownloadKind::Mirror,
                    name: format!("{label} ({} parts) · {host}", group.len()),
                    url: first_url.clone(),
                    parts: group[1..].iter().map(|(_, j)| links[*j].1.clone()).collect(),
                    password: None,
                });
                continue;
            }
        }
        used[i] = true;
        let (label, url, host) = &links[i];
        let label = if label.is_empty() { "Download" } else { label.as_str() };
        out.push(DownloadMethod {
            kind: DownloadKind::Mirror,
            name: format!("{label} · {host}"),
            url: url.clone(),
            parts: Vec::new(),
            password: None,
        });
    }
    out.sort_by_key(|m| sim_rank(&m.name, crate::sim::is_2024()));
    out
}

/// 0 = variante del sim activo, 1 = neutra, 2 = la del otro sim.
fn sim_rank(name: &str, is_2024: bool) -> u8 {
    let l = name.to_ascii_lowercase();
    let (mine, other) = if is_2024 { ("2024", "2020") } else { ("2020", "2024") };
    match (l.contains(mine), l.contains(other)) {
        (true, false) => 0,
        (false, true) => 2,
        _ => 1,
    }
}

fn to_addon(t: Tile, d: Detail) -> Addon {
    // El ICAO sólo tiene sentido en escenarios ("PMDG" no es un aeropuerto).
    let icao = if t.kind.as_deref() == Some("Scenery") {
        crate::parser::parse(&t.name).icao
    } else {
        None
    };
    Addon {
        id: stable_id("aerohub", &t.page_url),
        source: "aerohub".into(),
        title: t.name.clone(),
        developer: t.developer,
        name: t.name,
        version: t.version,
        icao,
        simulator: sim_label(&t.sim).to_string(),
        page_url: t.page_url,
        download_methods: d.methods,
        image_url: t.image_url,
        released_at: d.added_at,
        notes: d.notes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(label: &str, url: &str) -> (String, String) {
        (label.to_string(), url.to_string())
    }

    #[test]
    fn parts_are_joined_and_other_hosts_dropped() {
        let m = build_methods(vec![
            link("Part 1", "https://vaultdrop.st/s/AAA"),
            link("Part 2", "https://vaultdrop.st/s/BBB"),
            link("Go here to buy a cheap account", "https://t.me/+xyz"),
            link("Download Main", "https://aerohub.st/addon/other-123"),
        ]);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].url, "https://vaultdrop.st/s/AAA");
        assert_eq!(m[0].parts, vec!["https://vaultdrop.st/s/BBB".to_string()]);
        assert_eq!(m[0].name, "Download (2 parts) · VaultDrop");
    }

    #[test]
    fn liveries_parts_group_apart_from_main_download() {
        let m = build_methods(vec![
            link("Download", "https://vaultdrop.st/s/MAIN"),
            link("Liveries Part 1", "https://modsfire.com/L1"),
            link("Liveries Part 2", "https://modsfire.com/L2"),
        ]);
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].name, "Download · VaultDrop");
        assert_eq!(m[1].name, "Liveries (2 parts) · ModsFire");
        assert_eq!(m[1].parts, vec!["https://modsfire.com/L2".to_string()]);
    }

    #[test]
    fn active_sim_variant_ranks_first() {
        assert_eq!(sim_rank("Download for 2024 (v0.3.7) · VaultDrop", true), 0);
        assert_eq!(sim_rank("Download for 2020 (v1.8.8) · VaultDrop", true), 2);
        assert_eq!(sim_rank("Download · VaultDrop", true), 1);
        assert_eq!(sim_rank("2020 · VaultDrop", false), 0);
    }

    #[test]
    fn tiles_and_detail_parse() {
        let list = r#"<div class="grid"><a class="tile" href="/addon/fly2high-khou-2bbfef" data-reveal>
          <span class="tile__media"><img class="tile__img" src="https://i.ibb.co/x/cover.webp" />
          <span class="tile__sim" data-sim="msfs-20-24"><span class="tile__sim-text">MSFS 20/24</span></span>
          <span class="tile__kind"><svg></svg><span class="sr-only">Scenery</span></span></span>
          <span class="tile__body"><span class="tile__name">Fly2High William P. Hobby Airport KHOU</span>
          <span class="tile__by"><span class="tile__dev">Fly2High</span><span class="tile__ver">v0.1.0</span></span></span>
        </a></div><nav class="pager"><a class="pager__btn" href="/browse?p=2" rel="next"></a></nav>"#;
        let (tiles, more) = parse_tiles(list);
        assert!(more);
        assert_eq!(tiles.len(), 1);
        let t = &tiles[0];
        assert_eq!(t.page_url, "https://aerohub.st/addon/fly2high-khou-2bbfef");
        assert_eq!(t.version.as_deref(), Some("0.1.0"));
        assert_eq!(t.sim, "msfs-20-24");
        assert_eq!(t.kind.as_deref(), Some("Scenery"));

        let detail = r#"<div class="detail__buy"><p class="detail__meta">Added Oct 21, 2026 for MSFS 2024</p>
          <div class="detail__notes"><p class="detail__notes-text">Install:
          Extract to anywhere &amp; run Setup.exe


          Done!</p></div>
          <div class="detail__actions"><a class="btn" href="https://vaultdrop.st/s/EisP8vMvDc"><svg></svg><span>Download Navdata</span><svg></svg></a></div></div>"#;
        let d = parse_detail(detail);
        assert_eq!(d.added_at.as_deref(), Some("2026-10-21T12:00:00Z"));
        assert_eq!(
            d.notes.as_deref(),
            Some("Install:\nExtract to anywhere & run Setup.exe\n\nDone!")
        );
        assert_eq!(d.methods.len(), 1);
        assert_eq!(d.methods[0].name, "Download Navdata · VaultDrop");
    }
}
