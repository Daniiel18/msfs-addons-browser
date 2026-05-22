//! Comprobación de actualizaciones contra GitHub Releases.
//!
//! Decisión deliberada: **no** usamos `tauri-plugin-updater`. Ese plugin
//! exige firmar los binarios con una clave privada y publicar un manifest
//! signado al lado de cada release. Para un proyecto que aún no tiene
//! pipeline de release (este se distribuye como NSIS/MSIX hecho a mano)
//! es overkill.
//!
//! En cambio leemos `https://api.github.com/repos/{owner}/{repo}/releases/latest`,
//! comparamos `tag_name` con `CARGO_PKG_VERSION`, y si hay nueva
//! versión devolvemos al frontend el `tag_name`, las notas (`body`,
//! que GitHub guarda como Markdown) y la URL del primer asset de
//! Windows. La UI muestra un banner; el usuario decide abrir la
//! página de release en el navegador.
//!
//! Esto puede migrar al plugin oficial cuando exista pipeline; de
//! momento, comparativa de versiones + URL es suficiente.

use semver::Version;
use serde::{Deserialize, Serialize};

/// Owner/repo del proyecto. Se mantiene en código y no en un setting
/// porque la URL de actualización no es algo que el usuario deba
/// poder cambiar — y porque es leído antes de que exista la UI.
///
/// Si el repo se mueve, esta constante se actualiza y se compila.
const OWNER_REPO: &str = "Daniiel18/msfs-addons-browser";

const UA: &str = "SimFleet/3.0 (+https://github.com/n0xful)";

/// Tiempo máximo aceptable para que la consulta de versión bloquee la
/// pantalla de inicio. Subido de 8s → 20s en v0.1.12 porque algunos
/// usuarios reportaban que GitHub tardaba ~12s en responder desde su
/// red y la app daba "no hay update" silenciosamente.
const TIMEOUT_SECS: u64 = 20;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    /// Versión actual de la app (de `Cargo.toml`).
    pub current_version: String,
    /// Última versión publicada en GitHub.
    pub latest_version: String,
    /// URL para que el usuario abra la página de la release.
    pub release_url: String,
    /// URL del primer asset Windows (.exe / .msi / .msix). Puede ser
    /// `None` si la release todavía no tiene binarios subidos.
    pub asset_url: Option<String>,
    /// Notas de la release tal como vienen de GitHub (Markdown).
    pub notes_markdown: String,
    /// Fecha de publicación (ISO-8601).
    pub published_at: Option<String>,
}

// Bug fix v0.1.13: NO usar `rename_all = "camelCase"` — la API REST
// de GitHub devuelve sus campos en **snake_case** (tag_name,
// html_url, browser_download_url, etc). El rename forzaba a serde a
// buscar `browserDownloadUrl` (camelCase) que NUNCA aparece en la
// respuesta, y el parse fallaba con "missing field
// `browserDownloadUrl` at line 1 column 3244". Sin rename, los
// nombres de campos Rust (ya snake_case) matchean directo el JSON.
#[derive(Debug, Deserialize)]
struct GhRelease {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    published_at: Option<String>,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    assets: Vec<GhAsset>,
}

#[derive(Debug, Deserialize)]
struct GhAsset {
    name: String,
    browser_download_url: String,
}

/// Consulta la API de GitHub. Devuelve `Some(info)` sólo si la última
/// release **publicada** (no draft ni pre-release) tiene una versión
/// estrictamente mayor que la actual; cualquier otro caso (mismo tag,
/// tag inferior, tag malformado, error HTTP) → `None`.
///
/// Errores de red se loguean pero no se propagan al frontend para que
/// arrancar offline no muestre un mensaje rojo confuso al usuario.
pub async fn check_latest(http: &reqwest::Client) -> anyhow::Result<Option<UpdateInfo>> {
    let url = format!("https://api.github.com/repos/{}/releases/latest", OWNER_REPO);
    tracing::info!("updater: querying {}", url);

    let resp = http
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", UA)
        .header("X-GitHub-Api-Version", "2022-11-28")
        .timeout(std::time::Duration::from_secs(TIMEOUT_SECS))
        .send()
        .await?;

    if !resp.status().is_success() {
        // 404 cuando aún no hay releases publicadas — caso normal en
        // un proyecto recién subido. No es un error real, sólo "no
        // hay update".
        tracing::debug!("updater: github returned {}", resp.status());
        return Ok(None);
    }

    let release: GhRelease = resp.json().await?;
    if release.draft || release.prerelease {
        tracing::debug!("updater: latest release is draft/pre-release, ignoring");
        return Ok(None);
    }

    let current = env!("CARGO_PKG_VERSION");
    let latest_str = release.tag_name.trim_start_matches('v');

    let current_v = match Version::parse(current) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!("updater: cannot parse current version '{}': {}", current, e);
            return Ok(None);
        }
    };
    let latest_v = match Version::parse(latest_str) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(
                "updater: cannot parse remote tag '{}': {}",
                release.tag_name,
                e
            );
            return Ok(None);
        }
    };

    tracing::info!(
        target: "updater",
        "updater: current={} latest={} → update={}",
        current,
        latest_str,
        latest_v > current_v
    );
    if latest_v <= current_v {
        tracing::info!(
            target: "updater",
            "updater: ya estás en la última versión, no se ofrece update"
        );
        return Ok(None);
    }

    // Heurística simple para el asset Windows: priorizamos `.msix`
    // (preferido en Windows 11), luego `.msi`, luego `.exe`. Si la
    // release no trae ninguno, asset_url queda en None y la UI
    // ofrece sólo "abrir página".
    let asset_url = pick_windows_asset(&release.assets);

    Ok(Some(UpdateInfo {
        current_version: current.to_string(),
        latest_version: latest_str.to_string(),
        release_url: release.html_url,
        asset_url,
        notes_markdown: release.body.unwrap_or_default(),
        published_at: release.published_at,
    }))
}

/// Elige el asset Windows preferido. **Orden importa**:
///
///   1. `.exe` (NSIS) — preferido porque:
///      · Soporta install per-user sin admin (Tauri default).
///      · Reemplaza el `.exe` corriendo usando Restart Manager.
///      · Tauri lo configura para relanzar la app post-install.
///   2. `.msix` — usado raramente, MS Store apps.
///   3. `.msi` — último recurso. **NO recomendado** porque
///      `msiexec /quiet` instala bien pero NO relanza la app, y
///      el usuario reportó (log v0.1.13→v0.1.14) que tras
///      `exit(0)` la app desaparecía y no volvía a abrir.
fn pick_windows_asset(assets: &[GhAsset]) -> Option<String> {
    let by_ext = |ext: &str| {
        assets
            .iter()
            .find(|a| a.name.to_lowercase().ends_with(ext))
            .map(|a| a.browser_download_url.clone())
    };
    by_ext(".exe")
        .or_else(|| by_ext(".msix"))
        .or_else(|| by_ext(".msi"))
}
