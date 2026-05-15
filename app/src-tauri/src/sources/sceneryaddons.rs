use async_trait::async_trait;
use futures_util::{stream, StreamExt};
use scraper::{Html, Selector};

use super::{
    extract_article_date, extract_article_image, stable_id, Addon, BrowsePage, DownloadKind,
    DownloadMethod, Source, SourceError,
};

const BASE: &str = "https://sceneryaddons.org";

/// Max parallel detail-page fetches. Keeps scrape latency bounded when
/// query volume grows without being rude to the origin server.
const DETAIL_CONCURRENCY: usize = 6;

/// Hard cap on detail fetches per search. The site's results page already
/// paginates at ~10; this only guards against misbehaving themes that
/// spill the entire archive into a single response.
const MAX_DETAIL_FETCHES: usize = 40;

pub struct SceneryAddonsSource {
    client: reqwest::Client,
}

impl SceneryAddonsSource {
    pub fn new(client: reqwest::Client) -> Self {
        Self { client }
    }

    async fn fetch_html(&self, url: &str) -> Result<String, SourceError> {
        let resp = self.client.get(url).send().await?.error_for_status()?;
        Ok(resp.text().await?)
    }

    async fn download_methods(&self, page_url: &str) -> Vec<DownloadMethod> {
        let html = match self.fetch_html(page_url).await {
            Ok(h) => h,
            Err(e) => {
                tracing::warn!("sceneryaddons: could not fetch detail page {}: {}", page_url, e);
                return vec![];
            }
        };
        let doc = Html::parse_document(&html);
        // The original .NET project targets <a href*='get.php'>.
        let sel = Selector::parse("a[href*='get.php']").unwrap();
        let mut out = Vec::new();

        for a in doc.select(&sel) {
            let href = a.value().attr("href").unwrap_or("").trim().to_string();
            if href.is_empty() {
                continue;
            }

            let raw_text: String = a.text().collect::<String>().trim().to_string();
            let lower = raw_text.to_lowercase();
            let kind = if lower.contains("torrent") {
                DownloadKind::Torrent
            } else {
                DownloadKind::Mirror
            };

            let name = match kind {
                DownloadKind::Torrent => "Torrent Download".to_string(),
                _ => raw_text,
            };

            let url = if let Some(rest) = href.strip_prefix('/') {
                format!("{}/{}", BASE, rest)
            } else {
                href
            };

            out.push(DownloadMethod { kind, name, url });
        }
        out
    }
}

#[async_trait]
impl Source for SceneryAddonsSource {
    fn id(&self) -> &'static str { "sceneryaddons" }
    fn name(&self) -> &'static str { "SceneryAddons" }
    fn home_url(&self) -> &'static str { BASE }

    async fn search(&self, query: &str) -> Result<Vec<Addon>, SourceError> {
        let q = query.trim();
        if q.is_empty() {
            return Ok(vec![]);
        }

        let search_url = format!("{}/?s={}", BASE, urlencoding::encode(q));
        tracing::info!("sceneryaddons: searching {}", search_url);
        let html = self.fetch_html(&search_url).await?;

        // Scraper types (`Html`, `ElementRef`, …) are backed by `Rc` and therefore
        // are not `Send`. We must pull out every piece of data we need into owned
        // `Send` types before crossing any `await`.
        let entries = parse_search_page(&html);

        // WordPress's `?s=` matches on body content, not just title — so a search
        // for "EHAM" returns posts that merely mention EHAM in passing. When the
        // query looks like an ICAO, keep only results whose title actually
        // contains that ICAO (matches the original .NET behaviour).
        let icao_filter = detect_icao_query(q);

        let q_lower = q.to_lowercase();

        // First pass: pure filtering, no IO — cheap enough to run sync.
        let relevant: Vec<_> = entries
            .into_iter()
            .filter_map(
                |SearchEntry {
                     title,
                     page_url,
                     simulator,
                     image_url,
                     released_at,
                 }| {
                    let parsed = crate::parser::parse(&title);

                    if let Some(wanted) = &icao_filter {
                        if parsed.icao.as_deref() != Some(wanted.as_str()) {
                            return None;
                        }
                    } else {
                        let in_title = title.to_lowercase().contains(&q_lower);
                        let in_dev = parsed
                            .developer
                            .as_deref()
                            .map_or(false, |d| d.to_lowercase().contains(&q_lower));
                        if !in_title && !in_dev {
                            return None;
                        }
                    }

                    Some((title, page_url, simulator, image_url, released_at, parsed))
                },
            )
            .take(MAX_DETAIL_FETCHES)
            .collect();

        // Second pass: fan out detail-page fetches with bounded concurrency
        // while preserving input order so ranking by the site is kept.
        let total = relevant.len();
        let addons: Vec<Addon> = stream::iter(relevant.into_iter().map(
            |(title, page_url, simulator, image_url, released_at, parsed)| async move {
                let download_methods = self.download_methods(&page_url).await;
                Addon {
                    id: stable_id("sceneryaddons", &page_url),
                    source: "sceneryaddons".into(),
                    title,
                    developer: parsed.developer,
                    name: parsed.name,
                    version: parsed.version,
                    icao: parsed.icao,
                    simulator,
                    page_url,
                    download_methods,
                    image_url,
                    released_at,
                }
            },
        ))
        .buffered(DETAIL_CONCURRENCY)
        .collect()
        .await;

        tracing::info!(
            "sceneryaddons: {} results (from {} candidates, concurrency={})",
            addons.len(),
            total,
            DETAIL_CONCURRENCY,
        );
        Ok(addons)
    }

    async fn browse(&self, page: usize) -> Result<BrowsePage, SourceError> {
        // SceneryAddons usa el patrón de WordPress: página 1 es la
        // home (`/`), página N es `/page/N/`. Los listados ya vienen
        // ordenados por fecha más recientes primero.
        let url = if page <= 1 {
            format!("{}/", BASE)
        } else {
            format!("{}/page/{}/", BASE, page)
        };
        tracing::info!("sceneryaddons: browsing {}", url);

        let html = self.fetch_html(&url).await?;
        let entries = parse_search_page(&html);

        // ¿Hay siguiente página? Heurística simple: si la página
        // trae al menos algunos resultados, asumimos que la siguiente
        // existe — el primer 404 lo detectará el cliente al pulsar
        // siguiente. Más robusto: buscar `<a class="next page-numbers">`.
        let has_more = doc_has_next_page(&html);

        // Parse antes del stream para evitar el non-Send `Html`
        // dentro de un futuro async — replica el patrón de `search`.
        let entries: Vec<_> = entries
            .into_iter()
            .take(MAX_DETAIL_FETCHES)
            .map(|e| {
                let parsed = crate::parser::parse(&e.title);
                (
                    e.title,
                    e.page_url,
                    e.simulator,
                    e.image_url,
                    e.released_at,
                    parsed,
                )
            })
            .collect();

        let addons: Vec<Addon> = stream::iter(entries.into_iter().map(
            |(title, page_url, simulator, image_url, released_at, parsed)| async move {
                let download_methods = self.download_methods(&page_url).await;
                Addon {
                    id: stable_id("sceneryaddons", &page_url),
                    source: "sceneryaddons".into(),
                    title,
                    developer: parsed.developer,
                    name: parsed.name,
                    version: parsed.version,
                    icao: parsed.icao,
                    simulator,
                    page_url,
                    download_methods,
                    image_url,
                    released_at,
                }
            },
        ))
        .buffered(DETAIL_CONCURRENCY)
        .collect()
        .await;

        Ok(BrowsePage { addons, page, has_more })
    }
}

/// `true` si el HTML de la home/page-N tiene un enlace "siguiente"
/// que el tema Neve renderiza como `<a class="next page-numbers">`.
fn doc_has_next_page(html: &str) -> bool {
    let doc = Html::parse_document(html);
    let sel = match Selector::parse("a.next.page-numbers, a.next") {
        Ok(s) => s,
        Err(_) => return false,
    };
    doc.select(&sel).next().is_some()
}

/// Returns the uppercased query iff it looks like a 4-letter ICAO code.
fn detect_icao_query(q: &str) -> Option<String> {
    let up = q.trim().to_uppercase();
    (up.len() == 4 && up.chars().all(|c| c.is_ascii_alphabetic())).then_some(up)
}

struct SearchEntry {
    title: String,
    page_url: String,
    simulator: String,
    image_url: Option<String>,
    released_at: Option<String>,
}

/// Pure, synchronous parser — isolates non-`Send` scraper types from the async
/// fn so the returned future stays `Send`.
fn parse_search_page(html: &str) -> Vec<SearchEntry> {
    let doc = Html::parse_document(html);
    let article_sel = Selector::parse("article").unwrap();
    let title_sel = Selector::parse("h2 a").unwrap();

    doc.select(&article_sel)
        .filter_map(|article| {
            let anchor = article.select(&title_sel).next()?;
            let title: String = anchor.text().collect::<String>().trim().to_string();
            let page_url = anchor.value().attr("href").unwrap_or("").to_string();
            if title.is_empty() || page_url.is_empty() {
                return None;
            }

            let title_lower = title.to_lowercase();
            // MSFS 2020 filter — matches the behaviour in the original SearchService.cs
            if !title_lower.contains("msfs 2020") {
                return None;
            }
            let simulator = if title_lower.contains("2024") {
                "MSFS 2020/2024".to_string()
            } else {
                "MSFS 2020".to_string()
            };

            // Grab the featured image straight from the list page so we don't
            // need a second HTTP request per card.
            let image_url = extract_article_image(&article);
            let released_at = extract_article_date(&article);

            Some(SearchEntry {
                title,
                page_url,
                simulator,
                image_url,
                released_at,
            })
        })
        .collect()
}
