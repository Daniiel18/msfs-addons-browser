//! (06/11) Optimizador de MSFS: revisa lo que afecta al simulador y vacía solo
//! las cachés que marque el usuario. Lo de MSFS y Windows únicamente se lee.
//!
//! Las rutas salen siempre de `targets()`: el front elige por id y nunca manda una
//! ruta. Por eso ni los perfiles de Adrenalin (`AMD\CN`) ni nada fuera de esa
//! lista se pueden tocar desde aquí.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Result};
use serde::Serialize;
use serde_json::Value;

const ROLLING_ID: &str = "rolling";
const ROLLING_FILE: &str = "ROLLINGCACHE.CCC";

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GpuInfo {
    pub name: String,
    pub driver_version: Option<String>,
    pub driver_date: Option<String>,
}

/// Lo que dice `UserCfg.opt`. Solo se lee.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SimSettings {
    pub adapter: Option<String>,
    /// Nombra una tarjeta que este PC no tiene: manda el sistema, no el archivo.
    pub other_adapter: bool,
    pub preset: Option<String>,
    pub anti_aliasing: Option<String>,
    pub upscaling_mode: Option<String>,
    pub frame_generation: Option<String>,
    pub frame_limiter: Option<String>,
    pub vsync: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheItem {
    pub id: String,
    pub path: String,
    pub size_bytes: u64,
    pub files: u64,
}

/// Un ajuste de Windows, tal como está. Consejo, nunca una acción.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowsTip {
    pub id: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OptimizerReport {
    pub gpus: Vec<GpuInfo>,
    pub sim: Option<SimSettings>,
    pub caches: Vec<CacheItem>,
    pub windows: Vec<WindowsTip>,
    pub sim_running: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanedCache {
    pub id: String,
    pub before_bytes: u64,
    pub after_bytes: u64,
    pub removed_files: u64,
    /// En uso o sin permiso: se quedan donde estaban.
    pub skipped_files: u64,
}

// ---------------------------------------------------------------------------
// Tarjeta
// ---------------------------------------------------------------------------

/// PowerShell devuelve un objeto con una tarjeta y una lista con varias.
fn parse_gpus(json: &str) -> Vec<GpuInfo> {
    let Ok(value) = serde_json::from_str::<Value>(json.trim()) else {
        return Vec::new();
    };
    let list = match value {
        Value::Array(list) => list,
        other => vec![other],
    };
    let text = |item: &Value, key: &str| item.get(key).and_then(Value::as_str).map(str::to_string);
    list.iter()
        .filter_map(|item| {
            // Windows trae nombres con espacios dobles: «AMD  Radeon RX 7800 XT».
            let name = text(item, "name")?.split_whitespace().collect::<Vec<_>>().join(" ");
            // El adaptador básico de Windows no es una tarjeta.
            (!name.is_empty() && !name.starts_with("Microsoft ")).then(|| GpuInfo {
                name,
                driver_version: text(item, "driverVersion"),
                driver_date: text(item, "driverDate"),
            })
        })
        .collect()
}

#[cfg(windows)]
fn read_gpus() -> Vec<GpuInfo> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const SCRIPT: &str = "Get-CimInstance Win32_VideoController | Sort-Object AdapterRAM -Descending | \
        ForEach-Object { [pscustomobject]@{ name = $_.Name; driverVersion = $_.DriverVersion; \
        driverDate = if ($_.DriverDate) { $_.DriverDate.ToString('yyyy-MM-dd') } else { $null } } } | \
        ConvertTo-Json -Compress";
    std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", SCRIPT])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map(|out| parse_gpus(&String::from_utf8_lossy(&out.stdout)))
        .unwrap_or_default()
}

#[cfg(not(windows))]
fn read_gpus() -> Vec<GpuInfo> {
    Vec::new()
}

fn compact(name: &str) -> String {
    name.chars().filter(char::is_ascii_alphanumeric).collect::<String>().to_ascii_lowercase()
}

fn knows_adapter(gpus: &[GpuInfo], adapter: &str) -> bool {
    let wanted = compact(adapter);
    gpus.iter().map(|g| compact(&g.name)).any(|name| name.contains(&wanted) || wanted.contains(&name))
}

// ---------------------------------------------------------------------------
// UserCfg.opt
// ---------------------------------------------------------------------------

/// Líneas directas de un bloque `{Nombre … }`, sin sus sub-bloques.
fn block<'a>(text: &'a str, name: &str) -> Vec<&'a str> {
    let open = format!("{{{name}");
    let mut lines = text.lines().map(str::trim).skip_while(|line| *line != open);
    lines.next();
    let mut depth = 0;
    let mut out = Vec::new();
    for line in lines {
        if line.starts_with('{') {
            depth += 1;
        } else if line.starts_with('}') {
            if depth == 0 {
                break;
            }
            depth -= 1;
        } else if depth == 0 {
            out.push(line);
        }
    }
    out
}

fn value(lines: &[&str], key: &str) -> Option<String> {
    lines.iter().find_map(|line| {
        let (name, rest) = line.split_once(char::is_whitespace)?;
        (name == key).then(|| rest.trim().trim_matches('"').to_string())
    })
}

fn parse_user_cfg(text: &str, gpus: &[GpuInfo]) -> SimSettings {
    let video = block(text, "Video");
    let anti_aliasing = value(&video, "AntiAliasing");
    let upscaling_mode = match anti_aliasing.as_deref() {
        Some("FSR") => value(&video, "FSRMode"),
        Some("DLSS") => value(&video, "DLSSMode"),
        _ => None,
    };
    let adapter = value(&video, "Adapter").filter(|a| !a.is_empty());
    SimSettings {
        other_adapter: !gpus.is_empty() && adapter.as_deref().is_some_and(|a| !knows_adapter(gpus, a)),
        adapter,
        preset: value(&block(text, "Graphics"), "Preset"),
        anti_aliasing,
        upscaling_mode,
        frame_generation: value(&video, "FrameGeneration"),
        frame_limiter: value(&video, "FrameLimiter"),
        vsync: value(&video, "VSync").map(|v| v != "0"),
    }
}

// ---------------------------------------------------------------------------
// Cachés
// ---------------------------------------------------------------------------

struct Target {
    id: &'static str,
    path: PathBuf,
}

fn env_dir(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).map(PathBuf::from)
}

/// Lo único que se puede vaciar. Con carpeta de pruebas, todo cuelga de ella.
fn targets() -> Vec<Target> {
    let fake = crate::test_env::optimizer_root();
    let local = fake.as_ref().map(|r| r.join("Local")).or_else(|| env_dir("LOCALAPPDATA"));
    let low = fake
        .as_ref()
        .map(|r| r.join("LocalLow"))
        .or_else(|| env_dir("USERPROFILE").map(|h| h.join("AppData").join("LocalLow")));
    let rolling = match &fake {
        Some(root) => Some(root.join(ROLLING_FILE)),
        None => crate::community::active_user_cfg().and_then(|cfg| Some(cfg.parent()?.join(ROLLING_FILE))),
    };
    target_list(local, low, rolling)
}

fn target_list(local: Option<PathBuf>, low: Option<PathBuf>, rolling: Option<PathBuf>) -> Vec<Target> {
    let mut out = Vec::new();
    let mut add = |id: &'static str, base: &Option<PathBuf>, parts: &[&str]| {
        if let Some(base) = base {
            out.push(Target { id, path: parts.iter().fold(base.clone(), |p, part| p.join(part)) });
        }
    };
    add("d3dscache", &local, &["D3DSCache"]);
    add("amd_dx", &local, &["AMD", "DxCache"]);
    add("amd_dxc", &local, &["AMD", "DxcCache"]);
    add("nvidia_dx", &local, &["NVIDIA", "DXCache"]);
    add("nvidia_nv", &local, &["NVIDIA Corporation", "NV_Cache"]);
    add("nvidia_low", &low, &["NVIDIA", "PerDriverVersion", "DXCache"]);
    add("intel", &local, &["Intel", "ShaderCache"]);
    add("intel_low", &low, &["Intel", "ShaderCache"]);
    if let Some(path) = rolling {
        out.push(Target { id: ROLLING_ID, path });
    }
    out
}

fn measure(path: &Path) -> (u64, u64) {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return (0, 0);
    };
    if meta.is_file() {
        return (meta.len(), 1);
    }
    if !meta.is_dir() {
        return (0, 0);
    }
    let Ok(entries) = fs::read_dir(path) else {
        return (0, 0);
    };
    entries.flatten().fold((0, 0), |(size, files), entry| {
        let (s, f) = measure(&entry.path());
        (size + s, files + f)
    })
}

fn cache_items(targets: &[Target]) -> Vec<CacheItem> {
    targets
        .iter()
        .filter(|t| t.path.exists())
        .map(|t| {
            let (size_bytes, files) = measure(&t.path);
            CacheItem { id: t.id.to_string(), path: t.path.display().to_string(), size_bytes, files }
        })
        .collect()
}

#[derive(Default)]
struct Tally {
    removed: u64,
    skipped: u64,
}

/// Borra lo que hay dentro y deja la carpeta. Lo que esté en uso se salta. Un
/// enlace no se sigue: `symlink_metadata` lo ve como enlace, no como carpeta.
fn empty_dir(dir: &Path, tally: &mut Tally) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if fs::symlink_metadata(&path).is_ok_and(|m| m.is_dir()) {
            empty_dir(&path, tally);
            let _ = fs::remove_dir(&path);
        } else if fs::remove_file(&path).is_ok() {
            tally.removed += 1;
        } else {
            tally.skipped += 1;
        }
    }
}

fn clean_targets(targets: &[Target], ids: &[String]) -> Vec<CleanedCache> {
    targets
        .iter()
        .filter(|t| ids.iter().any(|id| id == t.id) && t.path.exists())
        .map(|t| {
            let (before_bytes, _) = measure(&t.path);
            let mut tally = Tally::default();
            if t.path.is_dir() {
                empty_dir(&t.path, &mut tally);
            } else if fs::remove_file(&t.path).is_ok() {
                tally.removed += 1;
            } else {
                tally.skipped += 1;
            }
            CleanedCache {
                id: t.id.to_string(),
                before_bytes,
                after_bytes: measure(&t.path).0,
                removed_files: tally.removed,
                skipped_files: tally.skipped,
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Windows: solo lectura
// ---------------------------------------------------------------------------

/// Modo de juego: sin la clave, Windows lo tiene activado.
fn game_mode(value: Option<u32>) -> &'static str {
    if value == Some(0) {
        "off"
    } else {
        "on"
    }
}

fn on_off(value: Option<u32>) -> &'static str {
    match value {
        Some(0) => "off",
        Some(_) => "on",
        None => "unknown",
    }
}

/// `HwSchMode`: 2 activada, 1 apagada; sin la clave decide Windows.
fn gpu_scheduling(value: Option<u32>) -> &'static str {
    match value {
        Some(2) => "on",
        Some(_) => "off",
        None => "unset",
    }
}

fn power_plan(guid: Option<&str>) -> &'static str {
    match guid.map(str::to_ascii_lowercase).as_deref() {
        Some("8c5e7fda-e8bf-4a96-9a85-a6e23a8c635c") => "high",
        Some("e9a42b02-d5df-448d-aa00-03f14749eb61") => "ultimate",
        Some("381b4222-f694-41f0-9685-ff5bb260df2e") => "balanced",
        Some("a1841308-3541-4fab-bc81-f71556f20b4a") => "saver",
        Some(_) => "other",
        None => "unknown",
    }
}

#[cfg(windows)]
fn windows_tips() -> Vec<WindowsTip> {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    use winreg::RegKey;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let dword = |root: &RegKey, path: &str, name: &str| -> Option<u32> { root.open_subkey(path).ok()?.get_value(name).ok() };
    let plan: Option<String> = hklm
        .open_subkey(r"SYSTEM\CurrentControlSet\Control\Power\User\PowerSchemes")
        .ok()
        .and_then(|key| key.get_value("ActivePowerScheme").ok());
    let tip = |id: &str, value: &str| WindowsTip { id: id.to_string(), value: value.to_string() };
    vec![
        tip("game_mode", game_mode(dword(&hkcu, r"Software\Microsoft\GameBar", "AutoGameModeEnabled"))),
        tip(
            "background_recording",
            on_off(dword(&hkcu, r"Software\Microsoft\Windows\CurrentVersion\GameDVR", "HistoricalCaptureEnabled")),
        ),
        tip(
            "gpu_scheduling",
            gpu_scheduling(dword(&hklm, r"SYSTEM\CurrentControlSet\Control\GraphicsDrivers", "HwSchMode")),
        ),
        tip("power_plan", power_plan(plan.as_deref())),
        tip(
            "memory_integrity",
            on_off(dword(
                &hklm,
                r"SYSTEM\CurrentControlSet\Control\DeviceGuard\Scenarios\HypervisorEnforcedCodeIntegrity",
                "Enabled",
            )),
        ),
    ]
}

#[cfg(not(windows))]
fn windows_tips() -> Vec<WindowsTip> {
    Vec::new()
}

// ---------------------------------------------------------------------------
// Lo que usan los comandos
// ---------------------------------------------------------------------------

pub async fn report() -> Result<OptimizerReport> {
    let sim_running = crate::launch_flow::sim_running_now().await;
    let found = tokio::task::spawn_blocking(|| {
        let gpus = read_gpus();
        let sim = crate::community::active_user_cfg()
            .and_then(|cfg| fs::read(cfg).ok())
            .map(|bytes| parse_user_cfg(&String::from_utf8_lossy(&bytes), &gpus));
        (gpus, sim, cache_items(&targets()), windows_tips())
    })
    .await?;
    let (gpus, sim, caches, windows) = found;
    Ok(OptimizerReport { gpus, sim, caches, windows, sim_running })
}

/// Vacía las cachés marcadas. Con MSFS abierto no se toca nada, y en el entorno
/// de pruebas solo la carpeta falsa, salvo permiso expreso.
pub async fn clean(ids: Vec<String>) -> Result<Vec<CleanedCache>> {
    if crate::launch_flow::sim_running_now().await {
        bail!("{}", crate::tr!("optimizer.simRunning"));
    }
    if crate::test_env::is_active() && crate::test_env::optimizer_root().is_none() && !crate::test_env::optimizer_real() {
        bail!("{}", crate::tr!("optimizer.testOnly"));
    }
    Ok(tokio::task::spawn_blocking(move || clean_targets(&targets(), &ids)).await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    const USER_CFG: &str = "Version 66\n{Video\n\tAdapter \"NVIDIA GeForce RTX 4090\"\n\tAntiAliasing FSR\n\tDLSSMode AUTO\n\tFSRMode QUALITY\n\tFrameGeneration NONE\n\tVSync 0\n\tFrameLimiter 0\n}\n{Graphics\n\tVersion 2.1.0\n\tPreset Custom\n\t{Texture\n\t\tMaxAnisotropy 16\n\t}\n}\n{GraphicsVR\n\tPreset VRMedium\n}\n";

    fn gpu(name: &str) -> GpuInfo {
        GpuInfo { name: name.to_string(), driver_version: None, driver_date: None }
    }

    #[test]
    fn one_card_or_several_are_read() {
        let one = parse_gpus(r#"{"name":"AMD Radeon RX 7800 XT","driverVersion":"32.0","driverDate":"2026-10-17"}"#);
        assert_eq!(one[0].driver_date.as_deref(), Some("2026-10-17"));
        assert_eq!(parse_gpus(r#"{"name":"AMD  Radeon RX 7800 XT"}"#)[0].name, "AMD Radeon RX 7800 XT");
        let many = parse_gpus(
            r#"[{"name":"AMD Radeon RX 7800 XT"},{"name":"AMD Radeon(TM) Graphics"},{"name":"Microsoft Basic Display Adapter"}]"#,
        );
        assert_eq!(many.len(), 2, "el adaptador básico no cuenta");
        assert!(parse_gpus("").is_empty());
    }

    #[test]
    fn the_system_wins_over_user_cfg() {
        let cfg = parse_user_cfg(USER_CFG, &[gpu("AMD Radeon RX 7800 XT")]);
        assert_eq!(cfg.adapter.as_deref(), Some("NVIDIA GeForce RTX 4090"));
        assert!(cfg.other_adapter, "tu caso: el archivo nombra otra tarjeta");
        assert!(!parse_user_cfg(USER_CFG, &[gpu("NVIDIA GeForce RTX 4090")]).other_adapter);
        assert!(!parse_user_cfg(USER_CFG, &[]).other_adapter, "sin datos del sistema no se acusa a nadie");
    }

    #[test]
    fn the_key_settings_are_read() {
        let cfg = parse_user_cfg(USER_CFG, &[]);
        assert_eq!(cfg.preset.as_deref(), Some("Custom"), "de {{Graphics, no de {{GraphicsVR");
        assert_eq!(cfg.anti_aliasing.as_deref(), Some("FSR"));
        assert_eq!(cfg.upscaling_mode.as_deref(), Some("QUALITY"));
        assert_eq!(cfg.frame_generation.as_deref(), Some("NONE"));
        assert_eq!(cfg.frame_limiter.as_deref(), Some("0"));
        assert_eq!(cfg.vsync, Some(false));
        let taa = parse_user_cfg("{Video\n\tAntiAliasing TAA\n\tFSRMode QUALITY\n}\n", &[]);
        assert_eq!(taa.upscaling_mode, None, "sin reescalado no hay modo");
        assert_eq!(parse_user_cfg("basura", &[]), SimSettings::default());
    }

    fn fake(root: &Path) -> Vec<Target> {
        for dir in ["Local/D3DSCache/sub", "Local/AMD/DxcCache", "Local/AMD/CN", "Documents/Arreglo-Adrenalin"] {
            fs::create_dir_all(root.join(dir)).unwrap();
        }
        fs::write(root.join("Local/D3DSCache/a.bin"), [0u8; 100]).unwrap();
        fs::write(root.join("Local/D3DSCache/sub/b.bin"), [0u8; 50]).unwrap();
        fs::write(root.join("Local/AMD/DxcCache/c.bin"), [0u8; 10]).unwrap();
        fs::write(root.join("Local/AMD/CN/gmdb.blb"), [0u8; 10]).unwrap();
        fs::write(root.join("Documents/Arreglo-Adrenalin/User.blb"), [0u8; 10]).unwrap();
        fs::write(root.join(ROLLING_FILE), [0u8; 1000]).unwrap();
        target_list(Some(root.join("Local")), Some(root.join("LocalLow")), Some(root.join(ROLLING_FILE)))
    }

    #[test]
    fn only_what_exists_is_listed_with_its_size() {
        let tmp = tempfile::tempdir().unwrap();
        let items = cache_items(&fake(tmp.path()));
        let ids: Vec<&str> = items.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(ids, ["d3dscache", "amd_dxc", "rolling"], "sin NVIDIA ni Intel en este PC");
        assert_eq!((items[0].size_bytes, items[0].files), (150, 2));
        assert_eq!(items[2].size_bytes, 1000);
    }

    #[test]
    fn only_the_marked_ones_are_emptied_and_adrenalin_is_left_alone() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let targets = fake(root);

        let done = clean_targets(&targets, &["d3dscache".to_string(), "no-existe".to_string()]);

        assert_eq!(done.len(), 1);
        assert_eq!((done[0].before_bytes, done[0].after_bytes, done[0].removed_files), (150, 0, 2));
        assert!(root.join("Local/D3DSCache").is_dir(), "la carpeta se queda");
        assert!(!root.join("Local/D3DSCache/sub").exists());
        assert!(root.join("Local/AMD/DxcCache/c.bin").is_file(), "no estaba marcada");
        assert!(root.join(ROLLING_FILE).is_file());
        assert!(root.join("Local/AMD/CN/gmdb.blb").is_file());
        assert!(root.join("Documents/Arreglo-Adrenalin/User.blb").is_file());
    }

    #[test]
    fn the_rolling_cache_file_goes_only_when_marked() {
        let tmp = tempfile::tempdir().unwrap();
        let targets = fake(tmp.path());
        let done = clean_targets(&targets, &[ROLLING_ID.to_string()]);
        assert_eq!((done[0].before_bytes, done[0].after_bytes), (1000, 0));
        assert!(!tmp.path().join(ROLLING_FILE).exists());
    }

    #[cfg(windows)]
    #[test]
    fn a_file_in_use_is_skipped_and_counted() {
        use std::os::windows::fs::OpenOptionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let targets = fake(tmp.path());
        let held = tmp.path().join("Local/D3DSCache/a.bin");
        let _lock = fs::OpenOptions::new().read(true).share_mode(0).open(&held).unwrap();

        let done = clean_targets(&targets, &["d3dscache".to_string()]);

        assert_eq!((done[0].removed_files, done[0].skipped_files), (1, 1));
        assert!(held.exists());
    }

    #[cfg(windows)]
    #[test]
    fn a_link_inside_a_cache_is_not_followed() {
        let tmp = tempfile::tempdir().unwrap();
        let targets = fake(tmp.path());
        // `mklink` no admite barras mezcladas.
        let keep = tmp.path().join("Local").join("AMD").join("CN");
        crate::folder_link::tests::junction(&tmp.path().join("Local").join("D3DSCache").join("enlace"), &keep);

        clean_targets(&targets, &["d3dscache".to_string()]);

        assert!(keep.join("gmdb.blb").is_file());
    }

    #[test]
    fn windows_values_are_named() {
        assert_eq!(game_mode(None), "on", "sin la clave, Windows lo tiene activado");
        assert_eq!(game_mode(Some(0)), "off");
        assert_eq!(on_off(Some(0)), "off");
        assert_eq!(on_off(Some(1)), "on");
        assert_eq!(on_off(None), "unknown");
        assert_eq!(gpu_scheduling(Some(2)), "on");
        assert_eq!(gpu_scheduling(Some(1)), "off");
        assert_eq!(gpu_scheduling(None), "unset");
        assert_eq!(power_plan(Some("8C5E7FDA-E8BF-4A96-9A85-A6E23A8C635C")), "high");
        assert_eq!(power_plan(Some("381b4222-f694-41f0-9685-ff5bb260df2e")), "balanced");
        assert_eq!(power_plan(Some("otra")), "other");
        assert_eq!(power_plan(None), "unknown");
    }
}
