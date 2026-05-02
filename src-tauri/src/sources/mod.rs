pub mod sceneryaddons;
pub mod simplaza;

use async_trait::async_trait;
use scraper::{ElementRef, Selector};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Addon {
    pub id: String,
    pub source: String,
    pub title: String,
    pub developer: Option<String>,
    pub name: String,
    pub version: Option<String>,
    pub icao: Option<String>,
    pub simulator: String,
    pub page_url: String,
    pub download_methods: Vec<DownloadMethod>,
    /// Featured-image URL scraped from the source's search page. `None`
    /// when the theme doesn't expose a thumbnail for this post (or the
    /// selector missed it); the UI falls back to a placeholder icon.
    pub image_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadMethod {
    pub kind: DownloadKind,
    pub name: String,
    pub url: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DownloadKind {
    Torrent,
    Mirror,
    Direct,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceDescriptor {
    pub id: String,
    pub name: String,
    pub home_url: String,
}

/// Resultado de un browse paginado del catálogo de una fuente.
/// Incluye los addons de la página solicitada + flag `has_more` para
/// que el frontend sepa si vale la pena pintar el botón "Siguiente".
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowsePage {
    pub addons: Vec<Addon>,
    pub page: usize,
    pub has_more: bool,
}

#[async_trait]
pub trait Source: Send + Sync {
    fn id(&self) -> &'static str;
    fn name(&self) -> &'static str;
    fn home_url(&self) -> &'static str;

    async fn search(&self, query: &str) -> Result<Vec<Addon>, SourceError>;

    /// Recorre el catálogo paginado del source. `page` es 1-based —
    /// `1` corresponde a la home. Las fuentes que no soporten browse
    /// pueden devolver `NotImplemented`; el frontend lo interpreta
    /// como "no hay catálogo y muestra la pista de buscar".
    async fn browse(&self, _page: usize) -> Result<BrowsePage, SourceError> {
        Err(SourceError::NotImplemented)
    }

    fn descriptor(&self) -> SourceDescriptor {
        SourceDescriptor {
            id: self.id().to_string(),
            name: self.name().to_string(),
            home_url: self.home_url().to_string(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SourceError {
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("parse error: {0}")]
    Parse(String),
    #[error("not implemented")]
    NotImplemented,
    #[error("other: {0}")]
    Other(String),
}

pub(crate) fn stable_id(source: &str, page_url: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    source.hash(&mut h);
    page_url.hash(&mut h);
    format!("{}-{:016x}", source, h.finish())
}

/// Pull the featured-image URL out of an `<article>` on a WordPress search
/// page. Works for both SceneryAddons and Simplaza — they ship the same
/// Neve theme markup:
///
/// ```html
/// <div class="nv-post-thumbnail-wrap img-wrap">
///   <a …><img class="wp-post-image"
///              src="…/foo.jpg"
///              srcset="…/foo.jpg 1280w, …/foo-300x169.jpg 300w, …"></a>
/// </div>
/// ```
///
/// We prefer a mid-size variant from `srcset` (~768w when present, else
/// 300w) because the card thumbnail is ~96px on screen — pulling the 1280w
/// master would waste a lot of bandwidth. If parsing `srcset` fails for
/// any reason we fall back to the `src` attribute untouched.
pub(crate) fn extract_article_image(article: &ElementRef<'_>) -> Option<String> {
    // `unwrap` is fine — the selector string is a compile-time literal that
    // the test suite would catch. Wrapping it in a `OnceLock` just bloats
    // the call site; parsing a two-selector string is microseconds.
    let img_sel = Selector::parse(".nv-post-thumbnail-wrap img, img.wp-post-image").ok()?;
    let img = article.select(&img_sel).next()?;

    if let Some(srcset) = img.value().attr("srcset") {
        if let Some(picked) = pick_from_srcset(srcset) {
            return Some(picked);
        }
    }
    img.value().attr("src").map(|s| s.trim().to_string())
}

/// Pick a reasonably-sized image from a `srcset` string. Returns the URL
/// with width nearest to (but not below) 600 — prefers 768w if available,
/// otherwise 300w, otherwise the largest.
///
/// A `srcset` looks like: `"url1 300w, url2 768w, url3 1024w, url4 1280w"`.
/// Invalid chunks are silently skipped so one malformed entry doesn't
/// throw the whole value away.
fn pick_from_srcset(srcset: &str) -> Option<String> {
    let mut candidates: Vec<(u32, String)> = srcset
        .split(',')
        .filter_map(|chunk| {
            let mut parts = chunk.trim().rsplitn(2, char::is_whitespace);
            let descriptor = parts.next()?;
            let url = parts.next()?.trim().to_string();
            // Only width descriptors — ignore pixel-density ones (`2x`).
            let w: u32 = descriptor.trim_end_matches('w').parse().ok()?;
            Some((w, url))
        })
        .collect();
    if candidates.is_empty() {
        return None;
    }
    candidates.sort_by_key(|(w, _)| *w);

    // Prefer 768w, else 300w, else the largest available.
    if let Some((_, u)) = candidates.iter().find(|(w, _)| *w == 768) {
        return Some(u.clone());
    }
    if let Some((_, u)) = candidates.iter().find(|(w, _)| *w == 300) {
        return Some(u.clone());
    }
    candidates.pop().map(|(_, u)| u)
}
