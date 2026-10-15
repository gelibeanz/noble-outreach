//! Finding MCC: Steam roots per OS, every library in `libraryfolders.vdf`, then
//! `steamapps/common/Halo The Master Chief Collection` (app 976730).

use std::path::{Path, PathBuf};

pub const MCC_FOLDER: &str = "Halo The Master Chief Collection";

/// Steam install folders that exist on this machine, most likely first.
pub fn steam_roots() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = Vec::new();
    #[cfg(windows)]
    {
        if let Some(p) = registry_steam_path() {
            v.push(p);
        }
        for d in [r"C:\Program Files (x86)\Steam", r"C:\Program Files\Steam"] {
            v.push(PathBuf::from(d));
        }
    }
    #[cfg(not(windows))]
    if let Some(h) = dirs::home_dir() {
        for d in [
            ".steam/steam",
            ".local/share/Steam",
            ".steam/root",
            ".var/app/com.valvesoftware.Steam/.local/share/Steam",
            ".var/app/com.valvesoftware.Steam/data/Steam",
            "snap/steam/common/.local/share/Steam",
        ] {
            v.push(h.join(d));
        }
    }
    let mut out: Vec<PathBuf> = Vec::new();
    for p in v {
        if p.join("steamapps").is_dir() {
            let c = p.canonicalize().unwrap_or(p);
            if !out.contains(&c) {
                out.push(c);
            }
        }
    }
    out
}

/// `HKCU\Software\Valve\Steam` `SteamPath`, read with `reg query` (no registry crate needed).
#[cfg(windows)]
fn registry_steam_path() -> Option<PathBuf> {
    use std::os::windows::process::CommandExt;
    let out = std::process::Command::new("reg")
        .args(["query", r"HKCU\Software\Valve\Steam", "/v", "SteamPath"])
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let line = text.lines().find(|l| l.contains("SteamPath"))?;
    let v = line.split("REG_SZ").nth(1)?.trim();
    (!v.is_empty()).then(|| PathBuf::from(v.replace('/', "\\")))
}

/// All `"path"` values in a libraryfolders.vdf (VDF escapes backslashes as `\\`).
pub fn parse_library_paths(vdf: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for line in vdf.lines() {
        let parts: Vec<&str> = line.split('"').collect();
        // `"path"		"C:\\Games\\Steam"` splits into ["\t\t", "path", "\t\t", "C:\\Games\\Steam", ""]
        if parts.len() >= 4 && parts[1] == "path" {
            out.push(PathBuf::from(parts[3].replace("\\\\", "\\")));
        }
    }
    out
}

/// Every Steam library folder (roots plus libraryfolders.vdf entries).
pub fn libraries() -> Vec<PathBuf> {
    let mut libs = Vec::new();
    for root in steam_roots() {
        if !libs.contains(&root) {
            libs.push(root.clone());
        }
        if let Ok(t) = std::fs::read_to_string(root.join("steamapps").join("libraryfolders.vdf")) {
            for p in parse_library_paths(&t) {
                if !libs.contains(&p) {
                    libs.push(p);
                }
            }
        }
    }
    libs
}

pub fn looks_like_mcc(dir: &Path) -> bool {
    dir.join("haloreach").join("haloreach.dll").is_file()
}

/// The MCC install folder, if Steam has one with Reach in it.
pub fn find_mcc() -> Option<PathBuf> {
    libraries()
        .into_iter()
        .map(|l| l.join("steamapps").join("common").join(MCC_FOLDER))
        .find(|d| looks_like_mcc(d))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vdf_paths() {
        let vdf = "\"libraryfolders\"\n{\n\t\"0\"\n\t{\n\t\t\"path\"\t\t\"C:\\\\Program Files (x86)\\\\Steam\"\n\t\t\"apps\"\n\t\t{\n\t\t\t\"976730\"\t\t\"1\"\n\t\t}\n\t}\n\t\"1\"\n\t{\n\t\t\"path\"\t\t\"/run/media/x/SteamLibrary\"\n\t}\n}\n";
        let p = parse_library_paths(vdf);
        assert_eq!(p, vec![PathBuf::from("C:\\Program Files (x86)\\Steam"), PathBuf::from("/run/media/x/SteamLibrary")]);
    }
}
