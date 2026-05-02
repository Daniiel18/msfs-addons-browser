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
    let html = state
        .http
        .get(&page_url)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .text()
        .await
        .map_err(|e| e.to_string())?;

    let lines = extract_changelog_lines(&html);
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
        if lower.contains("changelog")
            || lower.contains("change log")
            || lower.contains("what's new")
            || lower.contains("release notes")
            || lower.contains("updates")
            || lower.contains("history")
        {
            start_idx = Some(i);
            break;
        }
    }
    let Some(idx) = start_idx else {
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
