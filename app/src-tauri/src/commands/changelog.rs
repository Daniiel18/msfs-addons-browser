use scraper::{Html, Selector};
use serde::Serialize;

use crate::AppState;

/// Resultado de scrappear la página de detalle de un addon en busca
/// de un changelog. Devolvemos texto plano dividido en líneas para
/// que el frontend lo pinte como una lista — más legible que volcar
/// HTML que tendría que sanitizar.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Changelog {
    pub source_url: String,
    pub lines: Vec<String>,
}

/// Scrape el changelog de una página de SceneryAddons o Simplaza.
///
/// Heurística — los temas WordPress que usan tienden a tener:
///   · Un heading `<h2>` o `<h3>` con texto "Changelog", "Change Log",
///     "What's New", "Release Notes", "Updates"…
///   · El contenido posterior en `<ul><li>…</li></ul>`, `<ol>`,
///     `<p>` con `<br>` o `<pre>`. Recogemos los siguientes hijos
///     hasta el siguiente heading o final.
///
/// Si nada matchea, devolvemos lista vacía — el frontend muestra
/// "Sin changelog disponible".
#[tauri::command]
pub async fn fetch_changelog(
    page_url: String,
    state: tauri::State<'_, AppState>,
) -> Result<Changelog, String> {
    tracing::info!(target: "changelog", "fetch: {}", page_url);
    // (v2.2.0) User-Agent realista — sin esto algunos sitios devuelven
    // 403 o un challenge de Cloudflare. Replica del UA que usamos en
    // los scrapers de Simplaza/SceneryAddons.
    let html = state
        .http
        .get(&page_url)
        .header(
            "User-Agent",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
             (KHTML, like Gecko) Chrome/120.0 Safari/537.36",
        )
        .header("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8")
        .header("Accept-Language", "en-US,en;q=0.5")
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .text()
        .await
        .map_err(|e| e.to_string())?;
    tracing::debug!(target: "changelog", "HTML recibido {} bytes", html.len());

    let lines = extract_changelog_lines(&html);
    tracing::info!(
        target: "changelog",
        "extraidas {} líneas de {}",
        lines.len(), page_url
    );
    Ok(Changelog {
        source_url: page_url,
        lines,
    })
}

fn extract_changelog_lines(html: &str) -> Vec<String> {
    let doc = Html::parse_document(html);

    // Primera estrategia: localizar un heading que mencione changelog
    // y recoger los siguientes <li>/<p> hasta el próximo heading. El
    // resultado es una lista bastante limpia para los temas Neve
    // (WordPress) que usan SceneryAddons y Simplaza.
    let head_selector = Selector::parse("h1, h2, h3, h4").ok();
    let Some(head_selector) = head_selector else {
        return Vec::new();
    };

    let mut start_idx: Option<usize> = None;
    let headings: Vec<_> = doc.select(&head_selector).collect();
    for (i, h) in headings.iter().enumerate() {
        let text = h.text().collect::<String>();
        let lower = text.to_lowercase();
        // (v2.2.0) Más keywords + términos en español (Simplaza tiene
        // contenido bilingüe ocasional).
        if lower.contains("changelog")
            || lower.contains("change log")
            || lower.contains("what's new")
            || lower.contains("whats new")
            || lower.contains("release notes")
            || lower.contains("updates")
            || lower.contains("update history")
            || lower.contains("history")
            || lower.contains("version history")
            || lower.contains("versions")
            || lower.contains("cambios")
            || lower.contains("historial")
            || lower.contains("notas de versión")
            || lower.contains("registro de cambios")
        {
            tracing::debug!(target: "changelog", "match heading {:?}", text);
            start_idx = Some(i);
            break;
        }
    }
    let Some(idx) = start_idx else {
        // (v2.2.0) Fallback: buscar contenedores `class=*changelog*` o
        // `id=*changelog*`. Algunos temas WordPress meten el changelog
        // dentro de un <div> con clase específica sin heading propio.
        tracing::debug!(target: "changelog", "sin heading — probando selectores por clase/id");
        let class_selectors = [
            "[id*='changelog' i]",
            "[class*='changelog' i]",
            "[id*='cambios' i]",
            "[class*='cambios' i]",
            "[id*='history' i]",
        ];
        for sel_str in &class_selectors {
            if let Ok(sel) = Selector::parse(sel_str) {
                for elem in doc.select(&sel) {
                    let mut lines = Vec::new();
                    collect_lines(&elem, &mut lines);
                    if !lines.is_empty() {
                        tracing::info!(
                            target: "changelog",
                            "fallback selector {} → {} líneas",
                            sel_str, lines.len()
                        );
                        let mut out: Vec<String> = Vec::new();
                        let mut prev = String::new();
                        for raw in lines {
                            let cleaned = raw.trim().to_string();
                            if cleaned.is_empty() || cleaned == prev {
                                continue;
                            }
                            prev = cleaned.clone();
                            out.push(cleaned);
                        }
                        return out;
                    }
                }
            }
        }
        tracing::info!(target: "changelog", "no se encontró ningún changelog en la página");
        return Vec::new();
    };

    // Tomamos todos los siguientes elementos del DOM hasta el próximo
    // heading. Cada `<li>`, `<p>`, `<pre>`, `<br>` separado por
    // newline genera una línea distinta.
    let start = headings[idx];
    let stop_id = headings.get(idx + 1).map(|h| h.id());
    let mut current = start.next_sibling();
    let mut lines: Vec<String> = Vec::new();

    while let Some(node) = current {
        if let Some(stop) = stop_id {
            if node.id() == stop {
                break;
            }
        }

        if let Some(elem) = node.value().as_element() {
            // Si encontramos otro heading antes del esperado (raro,
            // pero por seguridad), paramos.
            let tag = elem.name();
            if matches!(tag, "h1" | "h2" | "h3" | "h4") {
                break;
            }
            if let Some(elref) = scraper::ElementRef::wrap(node) {
                collect_lines(&elref, &mut lines);
            }
        }
        current = node.next_sibling();
    }

    // Limpieza final: trim, dedupe consecutivos, filtra vacías.
    let mut out: Vec<String> = Vec::new();
    let mut prev = String::new();
    for raw in lines {
        let cleaned = raw.trim().to_string();
        if cleaned.is_empty() || cleaned == prev {
            continue;
        }
        prev = cleaned.clone();
        out.push(cleaned);
    }
    out
}

fn collect_lines(elem: &scraper::ElementRef<'_>, out: &mut Vec<String>) {
    let tag = elem.value().name();
    match tag {
        "ul" | "ol" => {
            let li_sel = Selector::parse("li").unwrap();
            for li in elem.select(&li_sel) {
                let text: String = li.text().collect::<String>();
                push_split(&text, out);
            }
        }
        "p" | "div" | "pre" => {
            // En `<p>`, los `<br>` insertan saltos que `text()` colapsa
            // — split manual.
            let text: String = elem.text().collect::<String>();
            push_split(&text, out);
        }
        _ => {
            let text: String = elem.text().collect::<String>();
            push_split(&text, out);
        }
    }
}

fn push_split(s: &str, out: &mut Vec<String>) {
    for line in s.split(['\n', '\r']) {
        let t = line.trim();
        if !t.is_empty() {
            out.push(t.to_string());
        }
    }
}
