//! Starting reach-host: directly on Windows, through umu-run + Proton on Linux (same env as scripts/run-ui.sh).

use std::path::Path;
#[cfg(not(windows))]
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

/// Proton builds we can hand to umu as PROTONPATH: (label, folder).
#[cfg(not(windows))]
pub fn protons() -> Vec<(String, PathBuf)> {
    let mut v = Vec::new();
    for root in crate::steam::steam_roots() {
        if let Ok(rd) = std::fs::read_dir(root.join("compatibilitytools.d")) {
            for e in rd.flatten() {
                if e.path().join("proton").is_file() {
                    v.push((e.file_name().to_string_lossy().into_owned(), e.path()));
                }
            }
        }
    }
    for lib in crate::steam::libraries() {
        if let Ok(rd) = std::fs::read_dir(lib.join("steamapps").join("common")) {
            for e in rd.flatten() {
                let n = e.file_name().to_string_lossy().into_owned();
                if n.starts_with("Proton") && !n.contains("EasyAntiCheat") && e.path().join("proton").is_file() {
                    v.push((n, e.path()));
                }
            }
        }
    }
    v.sort_by(|a, b| b.0.cmp(&a.0));
    v.dedup_by(|a, b| a.0 == b.0);
    // GE-Proton first (what we test with), newest first.
    v.sort_by_key(|(n, _)| !n.starts_with("GE-Proton"));
    v
}

#[cfg(not(windows))]
pub fn find_umu() -> Option<PathBuf> {
    let mut cands: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).map(|d| d.join("umu-run")).collect())
        .unwrap_or_default();
    if let Some(h) = dirs::home_dir() {
        cands.push(h.join(".local/bin/umu-run"));
    }
    cands.into_iter().find(|p| p.is_file())
}

/// Linux path -> Wine Z: path.
#[cfg(not(windows))]
pub fn wine_path(p: &Path) -> String {
    format!("Z:{}", p.to_string_lossy().replace('/', "\\"))
}

/// Merge player name + master URL (when the update source is a master) into reach-host's `reach-data/settings.json`
/// (the menu reads both from there; other fields the player set in the game's own settings screen are kept).
pub fn write_host_settings(client: &Path, name: &str, master: &str) -> Result<(), String> {
    let dir = client.join("reach-data");
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let p = dir.join("settings.json");
    let mut v: serde_json::Value = std::fs::read_to_string(&p)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .filter(|v: &serde_json::Value| v.is_object())
        .unwrap_or_else(|| serde_json::json!({}));
    v["player_name"] = name.into();
    if !master.is_empty() {
        v["master_url"] = master.into();
    }
    std::fs::write(&p, serde_json::to_string_pretty(&v).unwrap()).map_err(|e| format!("{}: {e}", p.display()))
}

pub struct Launch<'a> {
    pub client: &'a Path,
    pub mcc: &'a Path,
    pub name: &'a str,
    pub master: &'a str,
    pub prefix: &'a Path,
    pub proton: Option<&'a Path>,
    pub log: &'a Path,
}

pub fn spawn(l: &Launch) -> Result<Child, String> {
    let exe = l.client.join("reach-host.exe");
    if !exe.is_file() {
        return Err("reach-host.exe is not installed yet (update first)".into());
    }
    write_host_settings(l.client, l.name, l.master)?;
    let log = std::fs::File::create(l.log).map_err(|e| format!("{}: {e}", l.log.display()))?;
    let log2 = log.try_clone().map_err(|e| e.to_string())?;
    #[cfg(windows)]
    let mut cmd = {
        use std::os::windows::process::CommandExt;
        let _ = (l.prefix, l.proton);
        let mut c = Command::new(&exe);
        c.arg("--mcc").arg(l.mcc).creation_flags(0x0800_0000); // CREATE_NO_WINDOW: no console box
        c
    };
    #[cfg(not(windows))]
    let mut cmd = {
        let umu = find_umu().ok_or("umu-run is not installed (see the note under Proton)")?;
        std::fs::create_dir_all(l.prefix).map_err(|e| format!("{}: {e}", l.prefix.display()))?;
        let mut c = Command::new(umu);
        c.arg(&exe).arg("--mcc").arg(wine_path(l.mcc));
        c.env("WINEPREFIX", l.prefix).env("GAMEID", "umu-976730");
        match l.proton {
            Some(p) => c.env("PROTONPATH", p),
            None => c.env("PROTONPATH", "GE-Proton"), // umu downloads the latest GE-Proton itself
        };
        c
    };
    cmd.current_dir(l.client).env("REACH_NAME", l.name).stdin(Stdio::null()).stdout(log).stderr(log2);
    cmd.spawn().map_err(|e| format!("start {}: {e}", exe.display()))
}

pub fn open_folder(p: &Path) {
    #[cfg(windows)]
    let _ = Command::new("explorer").arg(p).spawn();
    #[cfg(not(windows))]
    let _ = Command::new("xdg-open").arg(p).spawn();
}

/// Last `n` lines of a text file (lossy), for the status panel.
pub fn tail(p: &Path, n: usize) -> Vec<String> {
    let Ok(b) = std::fs::read(p) else { return Vec::new() };
    let start = b.len().saturating_sub(16 * 1024);
    let t = String::from_utf8_lossy(&b[start..]);
    let lines: Vec<&str> = t.lines().collect();
    lines[lines.len().saturating_sub(n)..].iter().map(|s| s.to_string()).collect()
}
