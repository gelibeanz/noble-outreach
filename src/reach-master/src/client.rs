//! Client files for reach-launcher: `GET /api/client/manifest` lists the files in `--client-dir` (name, sha256,
//! size), `GET /api/client/files/<name>` serves one. Hashes are cached per (size, mtime), so replacing a file
//! in the directory is picked up on the next request without a restart. Notes: notes/launcher.md.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::Serialize;
use sha2::{Digest, Sha256};

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct FileEntry {
    pub name: String,
    pub sha256: String,
    pub size: u64,
}

#[derive(Serialize, Clone, Debug)]
pub struct Manifest {
    /// `version.txt` in the client dir (first line), else the first 12 hex digits of a hash over all entries.
    pub version: String,
    pub files: Vec<FileEntry>,
    /// Newest launcher builds from `<dir>/launcher/` (None when `launcher-version.txt` is missing).
    pub launcher: Option<Launcher>,
}

#[derive(Serialize, Clone, Debug)]
pub struct Launcher {
    pub version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub windows: Option<FileEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub linux: Option<FileEntry>,
}

const LAUNCHER_WINDOWS: &str = "NobleOutreach-Launcher.exe";
const LAUNCHER_LINUX: &str = "NobleOutreach-Launcher-linux";
/// Names from before the rename, served when the new one is missing from `<dir>/launcher/`. Launchers replace
/// their own exe whatever the manifest calls the file, so old ones update to new builds either way.
const OLD_LAUNCHER_WINDOWS: &str = "ReachReclaimer-Launcher.exe";
const OLD_LAUNCHER_LINUX: &str = "ReachReclaimer-Launcher-linux";

pub struct ClientDir {
    dir: PathBuf,
    cache: HashMap<String, (u64, SystemTime, String)>,
}

/// Names we serve: plain file names, no paths, no dot files.
pub fn valid_name(n: &str) -> bool {
    !n.is_empty()
        && n.len() <= 128
        && !n.starts_with('.')
        && n.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

fn sha256_file(p: &Path) -> std::io::Result<String> {
    let mut f = std::fs::File::open(p)?;
    let mut h = Sha256::new();
    std::io::copy(&mut f, &mut h)?;
    Ok(format!("{:x}", h.finalize()))
}

impl ClientDir {
    pub fn new(dir: PathBuf) -> Self {
        ClientDir { dir, cache: HashMap::new() }
    }

    fn hashed(&mut self, path: &Path, key: String) -> std::io::Result<Option<FileEntry>> {
        let Ok(md) = path.metadata() else { return Ok(None) };
        if !md.is_file() {
            return Ok(None);
        }
        let mtime = md.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        let sha256 = match self.cache.get(&key) {
            Some((s, t, h)) if *s == md.len() && *t == mtime => h.clone(),
            _ => {
                let h = sha256_file(path)?;
                self.cache.insert(key, (md.len(), mtime, h.clone()));
                h
            }
        };
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        Ok(Some(FileEntry { name, sha256, size: md.len() }))
    }

    fn launcher(&mut self) -> std::io::Result<Option<Launcher>> {
        let ld = self.dir.join("launcher");
        let Some(version) = std::fs::read_to_string(ld.join("launcher-version.txt"))
            .ok()
            .and_then(|t| t.lines().next().map(|l| l.trim().to_string()))
            .filter(|v| !v.is_empty())
        else {
            return Ok(None);
        };
        let mut pick = |new: &str, old: &str| -> std::io::Result<Option<FileEntry>> {
            match self.hashed(&ld.join(new), format!("launcher/{new}"))? {
                Some(f) => Ok(Some(f)),
                None => self.hashed(&ld.join(old), format!("launcher/{old}")),
            }
        };
        let windows = pick(LAUNCHER_WINDOWS, OLD_LAUNCHER_WINDOWS)?;
        let linux = pick(LAUNCHER_LINUX, OLD_LAUNCHER_LINUX)?;
        Ok(Some(Launcher { version, windows, linux }))
    }

    pub fn manifest(&mut self) -> std::io::Result<Manifest> {
        let mut files = Vec::new();
        for e in std::fs::read_dir(&self.dir)? {
            let e = e?;
            let name = e.file_name().to_string_lossy().into_owned();
            let md = e.metadata()?;
            if !md.is_file() || !valid_name(&name) || name == "version.txt" {
                continue;
            }
            let mtime = md.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            let sha = match self.cache.get(&name) {
                Some((s, t, h)) if *s == md.len() && *t == mtime => h.clone(),
                _ => {
                    let h = sha256_file(&e.path())?;
                    self.cache.insert(name.clone(), (md.len(), mtime, h.clone()));
                    h
                }
            };
            files.push(FileEntry { name, sha256: sha, size: md.len() });
        }
        files.sort_by(|a, b| a.name.cmp(&b.name));
        let version = std::fs::read_to_string(self.dir.join("version.txt"))
            .ok()
            .and_then(|t| t.lines().next().map(|l| l.trim().to_string()))
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| {
                let mut h = Sha256::new();
                for f in &files {
                    h.update(format!("{} {}\n", f.name, f.sha256));
                }
                format!("{:x}", h.finalize())[..12].to_string()
            });
        let launcher = self.launcher()?;
        Ok(Manifest { version, files, launcher })
    }

    /// Path of a served file, only for names the manifest would list.
    pub fn file(&self, name: &str) -> Option<PathBuf> {
        if !valid_name(name) || name == "version.txt" {
            return None;
        }
        let p = self.dir.join(name);
        if p.is_file() {
            return Some(p);
        }
        // Launcher builds live in <dir>/launcher/ and are listed under the manifest's `launcher`.
        let p = self.dir.join("launcher").join(name);
        (matches!(name, LAUNCHER_WINDOWS | LAUNCHER_LINUX | OLD_LAUNCHER_WINDOWS | OLD_LAUNCHER_LINUX) && p.is_file()).then_some(p)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert!(valid_name("reach-host.exe"));
        assert!(!valid_name("../etc/passwd"));
        assert!(!valid_name("a/b"));
        assert!(!valid_name(".env"));
        assert!(!valid_name(""));
    }

    #[test]
    fn manifest_and_cache() {
        let d = std::env::temp_dir().join(format!("reach-master-client-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("a.exe"), b"abc").unwrap();
        std::fs::write(d.join(".hidden"), b"x").unwrap();
        let mut c = ClientDir::new(d.clone());
        let m = c.manifest().unwrap();
        assert_eq!(m.files.len(), 1);
        assert_eq!(m.files[0].sha256, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(m.files[0].size, 3);
        assert_eq!(m.version.len(), 12);
        std::fs::write(d.join("version.txt"), "2026.10.15\n").unwrap();
        assert_eq!(c.manifest().unwrap().version, "2026.10.15");
        assert!(m.launcher.is_none());
        std::fs::create_dir_all(d.join("launcher")).unwrap();
        std::fs::write(d.join("launcher").join(LAUNCHER_LINUX), b"abc").unwrap();
        std::fs::write(d.join("launcher").join("launcher-version.txt"), "L1\n").unwrap();
        let l = c.manifest().unwrap().launcher.unwrap();
        assert_eq!(l.version, "L1");
        assert!(l.windows.is_none());
        assert_eq!(l.linux.unwrap().size, 3);
        assert!(c.file(LAUNCHER_LINUX).is_some());
        // Only the old name present: listed and served under it.
        std::fs::write(d.join("launcher").join(OLD_LAUNCHER_WINDOWS), b"abcd").unwrap();
        let l = c.manifest().unwrap().launcher.unwrap();
        assert_eq!(l.windows.unwrap().name, OLD_LAUNCHER_WINDOWS);
        assert!(c.file(OLD_LAUNCHER_WINDOWS).is_some());
        assert!(c.file("launcher-version.txt").is_none());
        assert!(c.file("a.exe").is_some());
        assert!(c.file("version.txt").is_none());
        assert!(c.file("../a.exe").is_none());
        let _ = std::fs::remove_dir_all(&d);
    }
}
