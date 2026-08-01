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
            return Err(SourceError::Other(crate::tr!("skybound.cloudflareBlocked")));
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
            None => crate::tr!("skybound.download"),
        }
    }

    /// ¿La URL/label es algo que NO es el archivo instalable del addon? Skybound
    /// mezcla en la misma lista manuales PDF, visores de SharePoint, páginas de
    /// docs y pastebins. Filtrarlos evita ofrecer un botón que baja un PDF o abre
    /// un visor en lugar del addon.
    fn is_junk_download(url: &str, label: &str, mime: Option<&str>) -> bool {
        let u = url.trim().to_ascii_lowercase();
        if u.is_empty() || u == "/blank" || u == "#" {
            return true;
        }
        if u.ends_with(".pdf") {
            return true;
        }
        if mime.map(|m| m.to_ascii_lowercase().contains("pdf")).unwrap_or(false) {
            return true;
        }
        // Denylist contra el HOST (no la URL entera) para no descartar una
        // descarga real cuya RUTA contenga por casualidad "docs"/"milviz".
        let host = u
            .split("://")
            .nth(1)
            .unwrap_or(&u)
            .split('/')
            .next()
            .unwrap_or("");
        const JUNK_HOSTS: &[&str] = &[
            "sharepoint.com",
            "pastebin.com",
            "milviz.com",
            "tfdidesign.com",
            "onedrive.live.com",
        ];
        if JUNK_HOSTS.iter().any(|h| host.contains(h))
            || host.starts_with("docs.")
            || host.contains(".docs.")
        {
            return true;
        }
        // Etiquetas de documentación. Palabra FINAL (no substring crudo), así
        // "Pilot Manual" es junk pero "Manual Install" (el addon) NO lo es.
        let l = label.trim().to_ascii_lowercase();
        matches!(
            l.as_str(),
            "manual"
                | "pilot manual"
                | "pilot's manual"
                | "checklist"
                | "documentation"
                | "readme"
                | "changelog"
                | "pilot notes"
                | "pilot's notes"
        ) || l.ends_with(" manual")
            || l.ends_with(" checklist")
            || l.ends_with(" notes")
    }

    /// Parsea una etiqueta "… Part N" → (base normalizada, N). `None` si no es
    /// de una parte. Ej: "v1.0.0 - Part 2" → ("1.0.0", 2); "Liveries - Part 3" →
    /// ("liveries", 3); "2.3 - Steam - Part 1" → ("2.3 - steam", 1).
    fn part_info(label: &str) -> Option<(String, u32)> {
        let low = label.to_ascii_lowercase();
        // `to_ascii_lowercase` no cambia longitudes de bytes → `pos` sirve para
        // indexar tanto `low` como `label`.
        let pos = low.rfind("part")?;
        let after = low[pos + 4..].trim_start_matches([' ', ':', '-', '#', '.']);
        let digits: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
        if digits.is_empty() {
            return None;
        }
        let n: u32 = digits.parse().ok()?;
        let base_raw = label[..pos]
            .trim()
            .trim_end_matches(['-', '·', '—', ' ', ':'])
            .trim();
        let base = base_raw
            .strip_prefix('v')
            .or_else(|| base_raw.strip_prefix('V'))
            .unwrap_or(base_raw)
            .trim()
            .to_ascii_lowercase();
        Some((base, n))
    }

    /// Nombre legible para un grupo multi-parte a partir de su base. Toma el
    /// último segmento que NO sea un número de versión: "2.3 - steam" → "Steam";
    /// "1.0.0" → "Completo"; "liveries" → "Liveries".
    fn pretty_part_base(base: &str) -> String {
        let is_ver = |s: &str| s.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(true);
        // `str::Split` no es reversible → coleccionamos antes de mirar del final.
        let segs: Vec<&str> = base
            .split(" - ")
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect();
        let seg = segs.iter().rev().find(|s| !is_ver(s)).copied();
        match seg {
            None => "Completo".to_string(),
            Some(s) => match s.to_ascii_lowercase().as_str() {
                "ms store" | "msstore" => "MS Store".to_string(),
                "steam" => "Steam".to_string(),
                other => {
                    let mut c = other.chars();
                    match c.next() {
                        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                        None => "Completo".to_string(),
                    }
                }
            },
        }
    }

    /// Extrae los métodos de descarga de la versión MÁS NUEVA compatible con
    /// MSFS2024 (una sola, como una página de SceneryAddons), en vez del
    /// histórico entero. Clasifica cada entrada por su `label`/`versionNumber`:
    ///   · **junk** (manuales PDF, SharePoint, docs, pastebin, `/blank`) → fuera.
    ///   · **magnet** → torrent.
    ///   · **"Part N"** de una misma base → se AGRUPAN en UN método multi-parte
    ///     (`url` = parte 1, `parts` = resto) que el navegador embebido baja EN
    ///     ORDEN y une antes de instalar.
    ///   · **mirror/backup** → método suelto marcado "(mirror)".
    ///   · **archivo de Skybound** (downloadType direct) → "Skybound (directo)".
    /// Así los mirrors de Skybound se comportan como los de SceneryAddons: un
    /// botón limpio por descarga real, sin ruido ni partes sueltas que fallan.
    fn extract_downloads(doc: &serde_json::Value) -> Vec<DownloadMethod> {
        struct Raw {
            label: String,
            url: String,
            kind: DownloadKind,
            host: String,
            is_skybound_file: bool,
        }

        // 1. La versión más nueva compatible con 2024 (última del array).
        let mut version: Option<&serde_json::Value> = None;
        if let Some(comp) = doc.get("compatibility").and_then(|c| c.as_array()) {
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
                if let Some(vs) = entry.get("versions").and_then(|v| v.as_array()) {
                    if let Some(v) = vs.last() {
                        version = Some(v);
                    }
                }
            }
        }
        let Some(v) = version else {
            return Vec::new();
        };

        // 2. Normaliza cada entrada (primary + additionalDownloads) a `Raw`.
        //    `downloadType` decide si mandan la `url` externa o el `file` de
        //    Skybound (una entrada puede traer ambos).
        let mk = |label: &str,
                  dt: &str,
                  url_field: Option<&serde_json::Value>,
                  file_field: Option<&serde_json::Value>|
         -> Option<Raw> {
            let as_file = || -> Option<Raw> {
                let f = file_field?.as_object()?;
                let fu = f.get("url").and_then(|u| u.as_str())?;
                let mime = f.get("mimeType").and_then(|m| m.as_str());
                if Self::is_junk_download(fu, label, mime) {
                    return None;
                }
                let abs = if fu.starts_with("http") {
                    fu.to_string()
                } else {
                    format!("{BASE}{fu}")
                };
                Some(Raw {
                    label: label.to_string(),
                    url: abs,
                    kind: DownloadKind::Direct,
                    host: "Skybound".into(),
                    is_skybound_file: true,
                })
            };
            let as_url = || -> Option<Raw> {
                let u = url_field?.as_str()?.trim();
                if u.is_empty() {
                    return None;
                }
                let low = u.to_ascii_lowercase();
                if low.contains("skybound.cx") || low.contains("flightsim.to") {
                    return None;
                }
                if low.starts_with("magnet:") {
                    return Some(Raw {
                        label: label.to_string(),
                        url: u.to_string(),
                        kind: DownloadKind::Torrent,
                        host: "Torrent".into(),
                        is_skybound_file: false,
                    });
                }
                if !low.starts_with("http") {
                    return None;
                }
                if Self::is_junk_download(u, label, None) {
                    return None;
                }
                Some(Raw {
                    label: label.to_string(),
                    url: u.to_string(),
                    kind: DownloadKind::Mirror,
                    host: Self::host_name(u),
                    is_skybound_file: false,
                })
            };
            if matches!(dt, "direct" | "file") {
                as_file().or_else(as_url)
            } else {
                as_url().or_else(as_file)
            }
        };

        let mut raws: Vec<Raw> = Vec::new();
        {
            let dt = v.get("downloadType").and_then(|x| x.as_str()).unwrap_or("");
            let label = v.get("versionNumber").and_then(|x| x.as_str()).unwrap_or("");
            if let Some(r) = mk(label, dt, v.get("url"), v.get("file")) {
                raws.push(r);
            }
        }
        if let Some(adds) = v.get("additionalDownloads").and_then(|a| a.as_array()) {
            for a in adds {
                let dt = a.get("downloadType").and_then(|x| x.as_str()).unwrap_or("");
                let label = a.get("label").and_then(|x| x.as_str()).unwrap_or("");
                if let Some(r) = mk(label, dt, a.get("url"), a.get("file")) {
                    raws.push(r);
                }
            }
        }
        if raws.is_empty() {
            return Vec::new();
        }

        // 3. Agrupa multi-parte (misma base, >=2 partes). Dedup por URL.
        let mut groups: std::collections::BTreeMap<String, Vec<(u32, usize)>> =
            std::collections::BTreeMap::new();
        for (i, r) in raws.iter().enumerate() {
            if let Some((base, n)) = Self::part_info(&r.label) {
                groups.entry(base).or_default().push((n, i));
            }
        }

        let mut out: Vec<DownloadMethod> = Vec::new();
        let mut used: std::collections::HashSet<usize> = std::collections::HashSet::new();
        let mut seen_url: std::collections::HashSet<String> = std::collections::HashSet::new();

        // 3a. Grupos multi-parte reales (>=2 partes), ordenados por N.
        for (base, mut items) in groups {
            items.sort_by_key(|(n, _)| *n);
            // Índices de TODOS los miembros ANTES de dedup — para marcarlos usados
            // si aceptamos el grupo (así los mirrors duplicados NO reaparecen como
            // fragmentos sueltos en 3b).
            let all_idxs: Vec<usize> = items.iter().map(|(_, i)| *i).collect();
            // Dedup por número de parte: si el mismo set está espejado en dos
            // mirrors con labels iguales ("Part 1"/"Part 2" en A y en B), la base
            // colapsa y quedan [1,1,2,2] → nos quedamos con UNA URL por N (la
            // primera, que suele ser el primary/modsfire). Sin esto se
            // interleavan A1,B1,A2,B2 → archivo corrupto al unir.
            items.dedup_by_key(|(n, _)| *n);
            // Deben ser partes CONTIGUAS 1..=k; si no (faltan/saltan números),
            // NO las unimos (dato roto) — caen a métodos sueltos abajo.
            let contiguous = items
                .iter()
                .enumerate()
                .all(|(k, (n, _))| *n as usize == k + 1);
            if items.len() < 2 || !contiguous {
                continue;
            }
            // Aceptado → marca TODOS los miembros originales (incl. los mirrors
            // duplicados que dedupeamos) como usados.
            for i in &all_idxs {
                used.insert(*i);
            }
            let part_urls: Vec<String> = items.iter().map(|(_, i)| raws[*i].url.clone()).collect();
            let host = raws[items[0].1].host.clone();
            let pretty = Self::pretty_part_base(&base);
            let name = if pretty == "Completo" {
                crate::tr!("skybound.downloadParts", host = host, n = part_urls.len())
            } else {
                crate::tr!("skybound.namedParts", host = host, name = pretty, n = part_urls.len())
            };
            let (first, rest) = part_urls.split_first().unwrap();
            seen_url.insert(first.to_ascii_lowercase());
            out.push(DownloadMethod {
                kind: DownloadKind::Mirror,
                name,
                url: first.clone(),
                parts: rest.to_vec(),
                password: None,
            });
        }

        // 3b. Métodos sueltos (no parte). Nombre por rol/host.
        for (i, r) in raws.iter().enumerate() {
            if used.contains(&i) {
                continue;
            }
            if !seen_url.insert(r.url.to_ascii_lowercase()) {
                continue;
            }
            let l = r.label.to_ascii_lowercase();
            let starts_digit = l.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(true);
            let name = if r.is_skybound_file {
                crate::tr!("skybound.direct")
            } else if matches!(r.kind, DownloadKind::Torrent) {
                "Torrent".to_string()
            } else if l.contains("mirror") || l.contains("backup") {
                format!("{} (mirror)", r.host)
            } else if !r.label.trim().is_empty() && !starts_digit {
                // Etiqueta con nombre propio (Liveries, Update…) → mostrarla.
                format!("{} · {}", r.host, r.label.trim())
            } else {
                r.host.clone()
            };
            out.push(DownloadMethod {
                kind: r.kind,
                name,
                url: r.url.clone(),
                parts: Vec::new(),
                password: None,
            });
        }

        out
    }

    /// Extrae la clave del archivo de la `description` (rich-text de Payload/
    /// Lexical). Skybound la escribe como "File password: https://skybound.cx".
    /// Junta todo el texto, busca "password" y toma el primer token siguiente.
    fn extract_password(doc: &serde_json::Value) -> Option<String> {
        fn collect_text(node: &serde_json::Value, out: &mut String) {
            if let Some(t) = node.get("text").and_then(|t| t.as_str()) {
                out.push_str(t);
                out.push(' ');
            }
            if let Some(children) = node.get("children").and_then(|c| c.as_array()) {
                for c in children {
                    collect_text(c, out);
                }
            }
            if let Some(root) = node.get("root") {
                collect_text(root, out);
            }
        }
        let desc = doc.get("description")?;
        let mut text = String::new();
        collect_text(desc, &mut text);
        // `to_ascii_lowercase` conserva las posiciones de bytes → `pos` sirve
        // para indexar el texto original.
        let low = text.to_ascii_lowercase();
        let pos = low.find("password")?;
        let after = &text[pos + "password".len()..];
        // Salta separadores (":", "-", "="), espacios y un "is"/"es" opcional.
        let sep = |c: char| c.is_whitespace() || matches!(c, ':' | '-' | '=' | '\u{a0}');
        let after = after.trim_start_matches(sep);
        // Salta un "is"/"es" SÓLO si es palabra suelta (fin de cadena o seguido de
        // separador) — si no, una clave que empiece por "is"/"es" (p.ej. "island")
        // se mutilaría a "land".
        let after = ["is", "es"]
            .iter()
            .find_map(|w| {
                after
                    .strip_prefix(*w)
                    .filter(|rest| rest.is_empty() || rest.starts_with(sep))
            })
            .unwrap_or(after);
        let after = after.trim_start_matches(sep);
        let token: String = after.chars().take_while(|c| !c.is_whitespace()).collect();
        let token = token
            .trim_end_matches(|c: char| matches!(c, '.' | ',' | ';' | ')' | '(' | '"' | '\''))
            .to_string();
        if token.is_empty() || token.len() > 128 {
            return None;
        }
        Some(token)
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
                    name: crate::tr!("skybound.openInSkybound"),
                    url: page_url,
                    parts: Vec::new(),
                    password: None,
                });
                // (v7) Clave del archivo indicada en la ficha ("File password:
                // https://skybound.cx") → la adjuntamos a cada método para que la
                // extracción NO pida contraseña.
                let pw = Self::extract_password(doc);
                if pw.is_some() {
                    for method in &mut m {
                        method.password = pw.clone();
                    }
                }
                // (v7) Prioriza modsfire/mixdrop y esconde rapidgator.
                super::prioritize_methods(m)
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
