use std::path::{Path, PathBuf};

use once_cell::sync::Lazy;
use regex::Regex;
use serde::Serialize;

/// Description of a detected MSFS 2020 Community folder, ready for the UI.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommunityInfo {
    /// Absolute path to the Community directory.
    pub path: String,
    /// "msstore" | "steam" — which install UserCfg.opt we parsed.
    pub variant: &'static str,
    /// True if the directory actually exists and is writable.
    pub exists: bool,
}

/// Attempt to auto-detect the MSFS 2020 Community folder by parsing
/// `UserCfg.opt` — exactly the same approach as the legacy .NET project.
///
/// Returns `Ok(None)` if neither install path is present (user may not
/// have the sim installed, or has a custom location we don't yet handle).
pub fn detect_community_folder() -> anyhow::Result<Option<CommunityInfo>> {
    if let Some((cfg, variant)) = find_user_cfg() {
        if let Some(path) = read_packages_path(&cfg)? {
            let community = path.join("Community");
            let exists = community.is_dir();
            return Ok(Some(CommunityInfo {
                path: community.to_string_lossy().into_owned(),
                variant,
                exists,
            }));
        }
    }
    Ok(None)
}

/// Returns (UserCfg.opt path, variant) for the first MSFS 2020 install we find.
fn find_user_cfg() -> Option<(PathBuf, &'static str)> {
    // MS Store (WinUI/UWP package) layout.
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        let p = Path::new(&local)
            .join("Packages")
            .join("Microsoft.FlightSimulator_8wekyb3d8bbwe")
            .join("LocalCache")
            .join("UserCfg.opt");
        if p.is_file() {
            return Some((p, "msstore"));
        }
    }

    // Steam layout.
    if let Some(roaming) = std::env::var_os("APPDATA") {
        let p = Path::new(&roaming)
            .join("Microsoft Flight Simulator")
            .join("UserCfg.opt");
        if p.is_file() {
            return Some((p, "steam"));
        }
    }

    None
}

static PACKAGES_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"(?i)InstalledPackagesPath\s+"(.+?)""#).unwrap());

fn read_packages_path(cfg: &Path) -> anyhow::Result<Option<PathBuf>> {
    let content = std::fs::read_to_string(cfg)?;
    Ok(PACKAGES_RE
        .captures(&content)
        .and_then(|c| c.get(1))
        .map(|m| PathBuf::from(m.as_str().trim())))
}
