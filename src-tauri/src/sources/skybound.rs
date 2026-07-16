//! (v6.2.40) Fuente **Skybound** (https://skybound.cx) para MSFS2024.
//!
//! Simplaza no publica addons de MSFS2024, así que en esa versión del sim
//! ofrecemos Skybound. La web es un Next.js/PayloadCMS con una **API JSON
//! PÚBLICA** en `/api/addons` — NO hace falta login para explorar/buscar
//! (la descarga se hace en la web). Filtramos a lo compatible con MSFS2024
//! y excluimos la categoría "scenery" (ya la cubre SceneryAddons).
//!
//! Cloudflare bloquea clientes-bot obvios (datacenter/UA raro) con 403,
//! así que mandamos cabeceras de navegador. Desde la IP residencial del
//! usuario suele pasar; si algún día exige challenge JS, habría que
//! recurrir al WebView.

use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::RwLock;

use super::{stable_id, Addon, BrowsePage, DownloadKind, DownloadMethod, Source, SourceError};

const BASE: &str = "https://skybound.cx";
const CATALOG_SLUG: &str = "msfs-2024";
const SIM_SLUG_2024: &str = "msfs-2024";
const PER_PAGE: usize = 30;

/// Credenciales (opcionales, para automatizar descargas en el futuro).
/// Explorar/buscar NO las necesita.
#[derive(Default)]
pub struct SkyboundAuth {
    pub username: Option<String>,
    pub password: Option<String>,
    pub session_cookie: Option<String>,
}

pub struct SkyboundSource {
    client: reqwest::Client,
    #[allow(dead_code)]
    auth: Arc<RwLock<SkyboundAuth>>,
}

impl SkyboundSource {
    pub fn new(client: reqwest::Client, auth: Arc<RwLock<SkyboundAuth>>) -> Self {
        Self { client, auth }
    }

    /// GET a la API con cabeceras de navegador (para pasar Cloudflare).
    async fn api_get(&self, path_and_query: &str) -> Result<serde_json::Value, SourceError> {
        let url = format!("{BASE}{path_and_query}");
        let resp = self
            .client
            .get(&url)
            .header(reqwest::header::ACCEPT, "application/json, text/plain, */*")
            .header(reqwest::header::ACCEPT_LANGUAGE, "en-US,en;q=0.9")
            .header(reqwest::header::REFERER, format!("{BASE}/{CATALOG_SLUG}"))
            .header(
                reqwest::header::USER_AGENT,
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36",
            )
            .send()
            .await
            .map_err(SourceError::Http)?;
        let status = resp.status();
        if status.as_u16() == 403 {
            return Err(SourceError::Other(
                "Skybound bloqueó la petición (Cloudflare). Prueba de nuevo; si persiste, ábrelo en el navegador.".into(),
            ));
        }
        if !status.is_success() {
            return Err(SourceError::Other(format!("Skybound HTTP {status}")));
        }
        resp.json::<serde_json::Value>().await.map_err(SourceError::Http)
    }

    /// ¿El addon es compatible con MSFS2024?
    fn is_2024(doc: &serde_json::Value) -> bool {
        doc.get("compatibility")
            .and_then(|c| c.as_array())
            .map(|arr| {
                arr.iter().any(|e| {
                    e.get("simulator")
                        .and_then(|s| s.get("slug"))
                        .and_then(|s| s.as_str())
                        .map(|s| s.eq_ignore_ascii_case(SIM_SLUG_2024))
                        .unwrap_or(false)
                })
            })
            .unwrap_or(false)
    }

    /// ¿Es escenario? (lo excluimos: ya lo cubre SceneryAddons.)
    fn is_scenery(doc: &serde_json::Value) -> bool {
        doc.get("category")
            .and_then(|c| c.get("slug"))
            .and_then(|s| s.as_str())
            .map(|s| s.eq_ignore_ascii_case("scenery"))
            .unwrap_or(false)
    }

    /// Versión declarada para MSFS2024 (última del array), si hay.
    fn version_2024(doc: &serde_json::Value) -> Option<String> {
        let arr = doc.get("compatibility")?.as_array()?;
        let entry = arr.iter().find(|e| {
            e.get("simulator")
                .and_then(|s| s.get("slug"))
                .and_then(|s| s.as_str())
                .map(|s| s.eq_ignore_ascii_case(SIM_SLUG_2024))
                .unwrap_or(false)
        })?;
        let versions = entry.get("versions")?.as_array()?;
        versions
            .last()
            .and_then(|v| v.get("versionNumber"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }

    /// Nombre bonito de un host de descarga. Genérico: dominio limpio con
    /// mayúscula inicial. Casos comunes con nombre propio.
    fn host_name(url: &str) -> String {
        if url.to_lowercase().starts_with("magnet:") {
            return "Torrent".into();
        }
        let host = url
            .split("://")
            .nth(1)
            .unwrap_or(url)
            .split('/')
            .next()
            .unwrap_or("")
            .trim_start_matches("www.");
        let low = host.to_lowercase();
        for (needle, label) in [
            ("modsfire", "Modsfire"),
            ("buzzheavier", "Buzzheavier"),
            ("drive.google", "Google Drive"),
            ("mega.nz", "MEGA"),
            ("mega.io", "MEGA"),
            ("mediafire", "MediaFire"),
            ("gofile", "Gofile"),
            ("pixeldrain", "Pixeldrain"),
            ("1fichier", "1fichier"),
            ("sharepoint", "SharePoint"),
            ("dropbox", "Dropbox"),
        ] {
            if low.contains(needle) {
                return label.into();
            }
        }
        // Genérico: primer segmento del dominio con mayúscula.
        let base = low.split('.').next().unwrap_or(&low);
        let mut c = base.chars();
        match c.next() {
            Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
            None => "Descargar".into(),
        }
    }

    /// Añade un método de descarga a partir de una URL suelta.
    fn push_url(url: &str, out: &mut Vec<DownloadMethod>, seen: &mut std::collections::HashSet<String>) {
        let url = url.trim().trim_end_matches(['\\', '"', ')', ',', ' ']);
        if url.is_empty() {
            return;
        }
        let low = url.to_lowercase();
        // Descartar lo que NO es una descarga (página de contraseña / origen).
        if low.contains("skybound.cx") || low.contains("flightsim.to") {
            return;
        }
        let is_magnet = low.starts_with("magnet:");
        if !is_magnet && !low.starts_with("http") {
            return;
        }
        let dedup_key = if is_magnet {
            low.split('&').next().unwrap_or(&low).to_string()
        } else {
            low.clone()
        };
        if !seen.insert(dedup_key) {
            return;
        }
        out.push(DownloadMethod {
            kind: if is_magnet {
                DownloadKind::Torrent
            } else {
                DownloadKind::Mirror
            },
            name: Self::host_name(url),
            url: url.to_string(),
        });
    }

    /// Extrae los métodos de descarga de las VERSIONES compatibles con
    /// MSFS2024. TODOS los addons publican el enlace en la API (con
    /// depth=2): `versions[].url` (cualquier host: modsfire, buzzheavier,
    /// magnet…), `additionalDownloads[].url`, o un `file` alojado en
    /// Skybound. Aceptamos CUALQUIER host — NO hace falta login.
    fn extract_downloads(doc: &serde_json::Value) -> Vec<DownloadMethod> {
        let mut out: Vec<DownloadMethod> = Vec::new();
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        let Some(comp) = doc.get("compatibility").and_then(|c| c.as_array()) else {
            return out;
        };
        for entry in comp {
            let is_2024 = entry
                .get("simulator")
                .and_then(|s| s.get("slug"))
                .and_then(|s| s.as_str())
                .map(|s| s.eq_ignore_ascii_case(SIM_SLUG_2024))
                .unwrap_or(false);
            if !is_2024 {
                continue;
            }
            let Some(versions) = entry.get("versions").and_then(|v| v.as_array()) else {
                continue;
            };
            for v in versions {
                if let Some(u) = v.get("url").and_then(|u| u.as_str()) {
                    Self::push_url(u, &mut out, &mut seen);
                }
                if let Some(add) = v.get("additionalDownloads").and_then(|a| a.as_array()) {
                    for a in add {
                        if let Some(u) = a.get("url").and_then(|u| u.as_str()) {
                            Self::push_url(u, &mut out, &mut seen);
                        }
                    }
                }
                // Archivo alojado en Skybound (downloadType "file").
                if let Some(fu) = v.get("file").and_then(|f| f.get("url")).and_then(|u| u.as_str()) {
                    let abs = if fu.starts_with("http") {
                        fu.to_string()
                    } else {
                        format!("{BASE}{fu}")
                    };
                    if seen.insert(abs.to_lowercase()) {
                        out.push(DownloadMethod {
                            kind: DownloadKind::Direct,
                            name: "Skybound".into(),
                            url: abs,
                        });
                    }
                }
            }
        }
        out
    }

    fn map_addon(&self, doc: &serde_json::Value) -> Option<Addon> {
        let title = doc.get("title")?.as_str()?.to_string();
        let slug = doc.get("slug")?.as_str()?.to_string();
        let page_url = format!("{BASE}/{CATALOG_SLUG}/{slug}");
        // Imagen: thumbnail.url (o el tamaño small) → absoluta.
        let image_url = doc
            .get("thumbnail")
            .map(|t| {
                t.get("sizes")
                    .and_then(|s| s.get("small"))
                    .and_then(|s| s.get("url"))
                    .and_then(|u| u.as_str())
                    .or_else(|| t.get("url").and_then(|u| u.as_str()))
            })
            .flatten()
            .map(|rel| {
                if rel.starts_with("http") {
                    rel.to_string()
                } else {
                    format!("{BASE}{rel}")
                }
            });
        let released_at = doc
            .get("createdAt")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        Some(Addon {
            id: stable_id("skybound", &page_url),
            source: "skybound".into(),
            title,
            developer: None,
            name: slug,
            version: Self::version_2024(doc),
            icao: None,
            simulator: "MSFS 2024".into(),
            page_url: page_url.clone(),
            // Links reales de la descripción (buzzheavier / GDrive / magnet)
            // + fallback a la ficha de Skybound (donde el resto se descarga
            // tras iniciar sesión con Clerk en la web).
            download_methods: {
                let mut m = Self::extract_downloads(doc);
                m.push(DownloadMethod {
                    kind: DownloadKind::Direct,
                    name: "Abrir en Skybound".into(),
                    url: page_url,
                });
                m
            },
            image_url,
            released_at,
        })
    }

    /// Trae TODOS los addons (son ~90) y filtra a MSFS2024 sin scenery.
    async fn all_2024(&self) -> Result<Vec<Addon>, SourceError> {
        let v = self
            .api_get("/api/addons?depth=2&limit=200&sort=-createdAt")
            .await?;
        let docs = v
            .get("docs")
            .and_then(|d| d.as_array())
            .cloned()
            .unwrap_or_default();
        Ok(docs
            .iter()
            .filter(|d| Self::is_2024(d) && !Self::is_scenery(d))
            .filter_map(|d| self.map_addon(d))
            .collect())
    }
}

#[async_trait]
impl Source for SkyboundSource {
    fn id(&self) -> &'static str {
        "skybound"
    }
    fn name(&self) -> &'static str {
        "Skybound"
    }
    fn home_url(&self) -> &'static str {
        BASE
    }

    async fn search(&self, query: &str) -> Result<Vec<Addon>, SourceError> {
        let q = query.trim();
        if q.is_empty() {
            return Ok(Vec::new());
        }
        // API PayloadCMS: where[title][like] (case-insensitive).
        let path = format!(
            "/api/addons?where[title][like]={}&depth=2&limit=100&sort=-createdAt",
            urlencoding::encode(q)
        );
        let v = self.api_get(&path).await?;
        let docs = v
            .get("docs")
            .and_then(|d| d.as_array())
            .cloned()
            .unwrap_or_default();
        Ok(docs
            .iter()
            .filter(|d| Self::is_2024(d) && !Self::is_scenery(d))
            .filter_map(|d| self.map_addon(d))
            .collect())
    }

    async fn browse(&self, page: usize) -> Result<BrowsePage, SourceError> {
        let all = self.all_2024().await?;
        let page = page.max(1);
        let start = (page - 1) * PER_PAGE;
        let slice: Vec<Addon> = all.iter().skip(start).take(PER_PAGE).cloned().collect();
        let has_more = start + slice.len() < all.len();
        Ok(BrowsePage {
            addons: slice,
            page,
            has_more,
        })
    }
}
