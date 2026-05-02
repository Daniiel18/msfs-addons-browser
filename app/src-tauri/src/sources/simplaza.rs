use async_trait::async_trait;
use futures_util::{stream, StreamExt};
use scraper::{Html, Selector};

use super::{
    extract_article_image, stable_id, Addon, BrowsePage, DownloadKind, DownloadMethod, Source,
    SourceError,
};

const BASE: &str = "https://simplaza.org";

/// Max parallel detail-page fetches. The bottleneck is Simplaza's origin
/// (WordPress on shared hosting), not the client — 6 keeps wall-clock
/// latency ~1/6 of the sequential version while still being polite.
const DETAIL_CONCURRENCY: usize = 6;

/// Hard cap on how many detail pages we fetch per search. A common-word
/// query like "777" can match dozens of posts on WordPress body search;
/// fetching them all is pointless UI lag — the user can always refine.
const MAX_DETAIL_FETCHES: usize = 40;

/// Simplaza hosts aircraft / addons (PMDG, Fenix, liveries…), not airports.
/// Their search page at `https://simplaza.org/?s={query}` uses the default
/// WordPress layout — each result is an `<article>` with the title inside
/// `h2 a`. Detail pages expose download links through the same `get.php`
/// pattern as SceneryAddons (served from `download.simplaza.org`), with the
/// hoster name carried in the `hoster` query param.
pub struct SimplazaSource {
    client: reqwest::Client,
}

impl SimplazaSource {
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
                tracing::warn!("simplaza: could not fetch detail page {}: {}", page_url, e);
                return vec![];
            }
        };
        parse_download_methods(&html)
    }
}

/// Simplaza's download links all go through `https://download.simplaza.org/get.php?hoster=X&file=Y&hash=Z`.
/// The hoster query param is authoritative, so we parse that to derive both
/// the kind (torrent vs mirror) and a pretty label — the raw link text is
/// just "Download" for every button, which is useless as a UI label.
fn parse_download_methods(html: &str) -> Vec<DownloadMethod> {
    let doc = Html::parse_document(html);
    let sel = Selector::parse("a[href*='download.simplaza.org']").unwrap();
    let mut out = Vec::new();

    for a in doc.select(&sel) {
        let raw = a.value().attr("href").unwrap_or("").trim();
        if raw.is_empty() {
            continue;
        }
        // WordPress emits `&#038;` for `&` — decode before URL-parsing.
        let href = decode_html_entities(raw);

        let Some(hoster) = extract_hoster(&href) else { continue };
        let lower = hoster.to_lowercase();
        let kind = if lower.contains("torrent") {
            DownloadKind::Torrent
        } else {
            DownloadKind::Mirror
        };
        let name = match kind {
            DownloadKind::Torrent => "Torrent Download".to_string(),
            _ => format!("{} Download", prettify_hoster(&hoster)),
        };

        out.push(DownloadMethod { kind, name, url: href });
    }

    // Dedupe identical URLs (Simplaza sometimes duplicates a link in nearby
    // containers) while preserving order.
    let mut seen = std::collections::HashSet::new();
    out.retain(|m| seen.insert(m.url.clone()));
    out
}

fn extract_hoster(href: &str) -> Option<String> {
    let parsed = url::Url::parse(href).ok()?;
    parsed
        .query_pairs()
        .find(|(k, _)| k == "hoster")
        .map(|(_, v)| v.into_owned())
}

fn prettify_hoster(h: &str) -> String {
    // Common hosters with canonical capitalization; fall back to Title-case
    // of whatever came in the query string.
    match h.to_lowercase().as_str() {
        "rapidgator" => "Rapidgator".into(),
        "modsfire"   => "ModsFire".into(),
        "mixdrop"    => "MixDrop".into(),
        "mediafire"  => "MediaFire".into(),
        "mega"       => "Mega".into(),
        "gofile"     => "GoFile".into(),
        "1fichier"   => "1fichier".into(),
        "uptobox"    => "Uptobox".into(),
        "pixeldrain" => "Pixeldrain".into(),
        _ => {
            let mut c = h.chars();
            match c.next() {
                Some(first) => first.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        }
    }
}

fn decode_html_entities(s: &str) -> String {
    s.replace("&#038;", "&")
        .replace("&amp;", "&")
        .replace("&#038;", "&")
}

struct SearchEntry {
    title: String,
    page_url: String,
    image_url: Option<String>,
}

/// Pure sync parser — keeps non-`Send` scraper types out of the async fn.
///
/// Two-phase strategy:
///   1. Preferred path — walk each `<article>` and pull the title anchor +
///      featured image from the same scope. This is what the Neve theme
///      ships today (matches the SceneryAddons layout).
///   2. Fallback path — if no articles were found (old theme, custom
///      landing page, etc.), try a list of bare title selectors and skip
///      the image entirely. Losing the thumbnail is better than losing
///      the result.
fn parse_search_page(html: &str) -> Vec<SearchEntry> {
    let doc = Html::parse_document(html);

    let article_sel = Selector::parse("article").unwrap();
    // Ordered from most-specific to least — first match wins per article.
    let title_selectors = [
        "h2.entry-title a",
        ".entry-title a",
        "h2 a",
        ".post-title a",
    ];

    let mut out: Vec<SearchEntry> = Vec::new();

    for article in doc.select(&article_sel) {
        let anchor = title_selectors.iter().find_map(|sel_str| {
            let sel = Selector::parse(sel_str).ok()?;
            article.select(&sel).next()
        });
        let Some(a) = anchor else { continue };
        let title: String = a.text().collect::<String>().trim().to_string();
        let page_url = a.value().attr("href").unwrap_or("").trim().to_string();
        if title.is_empty() || page_url.is_empty() || is_non_result(&page_url) {
            continue;
        }
        let image_url = extract_article_image(&article);
        out.push(SearchEntry { title, page_url, image_url });
    }

    if out.is_empty() {
        // Fallback for stripped-down themes that don't wrap results in
        // `<article>` tags. We don't get images here, but the user still
        // gets results.
        let flat_selectors = [
            "h2.entry-title a",
            ".post-title a",
            ".search-results a[href]",
        ];
        for sel_str in flat_selectors {
            let Ok(sel) = Selector::parse(sel_str) else { continue };
            for a in doc.select(&sel) {
                let title: String = a.text().collect::<String>().trim().to_string();
                let page_url = a.value().attr("href").unwrap_or("").trim().to_string();
                if title.is_empty() || page_url.is_empty() || is_non_result(&page_url) {
                    continue;
                }
                out.push(SearchEntry { title, page_url, image_url: None });
            }
            if !out.is_empty() {
                break;
            }
        }
    }

    // Dedupe by page_url while preserving order.
    let mut seen = std::collections::HashSet::new();
    out.retain(|e| seen.insert(e.page_url.clone()));
    out
}

/// Skip obvious non-result links (nav, promos, category pages).
fn is_non_result(page_url: &str) -> bool {
    page_url.contains("/category/")
        || page_url.contains("/tag/")
        || page_url.contains("/page/")
        || page_url.ends_with("/simplaza.org/")
        || page_url == BASE
}

#[async_trait]
impl Source for SimplazaSource {
    fn id(&self) -> &'static str { "simplaza" }
    fn name(&self) -> &'static str { "Simplaza" }
    fn home_url(&self) -> &'static str { BASE }

    async fn search(&self, query: &str) -> Result<Vec<Addon>, SourceError> {
        let q = query.trim();
        if q.is_empty() {
            return Ok(vec![]);
        }

        let search_url = format!("{}/?s={}", BASE, urlencoding::encode(q));
        tracing::info!("simplaza: searching {}", search_url);
        let html = self.fetch_html(&search_url).await?;

        let entries = parse_search_page(&html);
        tracing::info!("simplaza: parsed {} raw entries", entries.len());

        let q_lower = q.to_lowercase();

        // First pass: filter by relevance (title or developer must contain query).
        let relevant: Vec<_> = entries
            .into_iter()
            .filter_map(|SearchEntry { title, page_url, image_url }| {
                let parsed = crate::parser::parse(&title);
                let in_title = title.to_lowercase().contains(&q_lower);
                let in_dev = parsed
                    .developer
                    .as_deref()
                    .map_or(false, |d| d.to_lowercase().contains(&q_lower));
                if !in_title && !in_dev {
                    return None;
                }
                Some((title, page_url, image_url, parsed))
            })
            .take(MAX_DETAIL_FETCHES)
            .collect();

        // Second pass: fetch each detail page to extract download methods.
        // `buffered` drives up to DETAIL_CONCURRENCY requests in flight while
        // preserving input order — so search relevance ranking is not lost.
        // This was the main cause of 15+ s latency on broad queries like
        // "777" when done sequentially.
        let total = relevant.len();
        let addons: Vec<Addon> = stream::iter(relevant.into_iter().map(
            |(title, page_url, image_url, parsed)| async move {
                let mut methods = self.download_methods(&page_url).await;
                if methods.is_empty() {
                    // Fallback so the user at least has a way to reach the
                    // product page when the detail scraper can't find links.
                    methods.push(DownloadMethod {
                        kind: DownloadKind::Direct,
                        name: "Abrir en Simplaza".to_string(),
                        url: page_url.clone(),
                    });
                }
                Addon {
                    id: stable_id("simplaza", &page_url),
                    source: "simplaza".into(),
                    title,
                    developer: parsed.developer,
                    name: parsed.name,
                    version: parsed.version,
                    icao: parsed.icao,
                    simulator: "MSFS 2020".to_string(),
                    download_methods: methods,
                    page_url,
                    image_url,
                }
            },
        ))
        .buffered(DETAIL_CONCURRENCY)
        .collect()
        .await;

        tracing::info!(
            "simplaza: {} results (from {} candidates, concurrency={})",
            addons.len(),
            total,
            DETAIL_CONCURRENCY,
        );
        Ok(addons)
    }

    async fn browse(&self, page: usize) -> Result<BrowsePage, SourceError> {
        // Simplaza también es WordPress: `/page/N/` para páginas
        // siguientes, `/` para la home. Misma estrategia que
        // SceneryAddons.
        let url = if page <= 1 {
            format!("{}/", BASE)
        } else {
            format!("{}/page/{}/", BASE, page)
        };
        tracing::info!("simplaza: browsing {}", url);

        let html = self.fetch_html(&url).await?;
        let entries = parse_search_page(&html);
        let has_more = doc_has_next_page(&html);

        let prepped: Vec<_> = entries
            .into_iter()
            .take(MAX_DETAIL_FETCHES)
            .map(|e| {
                let parsed = crate::parser::parse(&e.title);
                (e.title, e.page_url, e.image_url, parsed)
            })
            .collect();

        let addons: Vec<Addon> = stream::iter(prepped.into_iter().map(
            |(title, page_url, image_url, parsed)| async move {
                let mut methods = self.download_methods(&page_url).await;
                if methods.is_empty() {
                    methods.push(DownloadMethod {
                        kind: DownloadKind::Direct,
                        name: "Abrir en Simplaza".to_string(),
                        url: page_url.clone(),
                    });
                }
                Addon {
                    id: stable_id("simplaza", &page_url),
                    source: "simplaza".into(),
                    title,
                    developer: parsed.developer,
                    name: parsed.name,
                    version: parsed.version,
                    icao: parsed.icao,
                    simulator: "MSFS 2020".to_string(),
                    download_methods: methods,
                    page_url,
                    image_url,
                }
            },
        ))
        .buffered(DETAIL_CONCURRENCY)
        .collect()
        .await;

        Ok(BrowsePage { addons, page, has_more })
    }
}

fn doc_has_next_page(html: &str) -> bool {
    let doc = Html::parse_document(html);
    let sel = match Selector::parse("a.next.page-numbers, a.next") {
        Ok(s) => s,
        Err(_) => return false,
    };
    doc.select(&sel).next().is_some()
}
