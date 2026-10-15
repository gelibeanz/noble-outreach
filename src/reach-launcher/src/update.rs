//! Client install/update. Two sources (chosen at build time, see `source`): the latest GitHub release of a repo
//! (assets + their SHA256SUMS), or reach-master (`GET /api/client/manifest`, then `GET /api/client/files/<name>`).
//! Both become one `Manifest`; each file whose local sha256 differs is downloaded to `<name>.part`, verified,
//! then renamed.

use std::io::{Read, Write};
use std::path::Path;
use std::time::Duration;

use serde::Deserialize;
use sha2::{Digest, Sha256};

/// Update source built in with `NOBLE_UPDATE_SOURCE` (build.rs reruns when it changes):
/// `github:<owner>/<repo>` = the repo's latest release (default), or `master:<url>[,<url>...]` = reach-master's
/// client manifest, first master that answers. At run time `NOBLE_UPDATE_SOURCE` replaces it and
/// `REACH_LAUNCHER_MASTERS=a,b` forces master mode (testing).
const BUILT_SOURCE: &str = match option_env!("NOBLE_UPDATE_SOURCE") {
    Some(v) => v,
    None => "github:gelibeanz/noble-outreach",
};

/// Release asset names (GitHub source); the master serves the same names.
const CLIENT_FILES: &[&str] = &["reach-host.exe"];
const LAUNCHER_WINDOWS: &str = "NobleOutreach-Launcher.exe";
const LAUNCHER_LINUX: &str = "NobleOutreach-Launcher-linux";

pub enum Source {
    GitHub(String),
    Masters(Vec<String>),
}

fn split_list(s: &str) -> Vec<String> {
    s.split(',').map(|s| s.trim().trim_end_matches('/').to_string()).filter(|s| !s.is_empty()).collect()
}

pub fn parse_source(spec: &str) -> Result<Source, String> {
    let spec = spec.trim();
    if let Some(repo) = spec.strip_prefix("github:") {
        let ok = repo.split('/').count() == 2
            && repo.split('/').all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')));
        return if ok { Ok(Source::GitHub(repo.to_string())) } else { Err(format!("bad update source {spec:?} (github:<owner>/<repo>)")) };
    }
    if let Some(list) = spec.strip_prefix("master:") {
        let v = split_list(list);
        return if v.is_empty() { Err("update source master: has no URL".into()) } else { Ok(Source::Masters(v)) };
    }
    Err(format!("bad update source {spec:?} (github:<owner>/<repo> or master:<url>)"))
}

pub fn source() -> Result<Source, String> {
    if let Some(v) = std::env::var("REACH_LAUNCHER_MASTERS").ok().map(|v| split_list(&v)).filter(|v| !v.is_empty()) {
        return Ok(Source::Masters(v));
    }
    let rt = std::env::var("NOBLE_UPDATE_SOURCE").ok().filter(|v| !v.trim().is_empty());
    parse_source(rt.as_deref().unwrap_or(BUILT_SOURCE))
}

/// What a check found: text for the UI, the manifest (every entry carries its download URL), and the master to
/// give reach-host (empty with the GitHub source: the player's own setting is kept).
pub struct Update {
    pub label: String,
    pub manifest: Manifest,
    pub master: String,
}

pub fn resolve() -> Result<Update, String> {
    match source()? {
        Source::Masters(ms) => {
            let m = pick_master(&ms)?;
            let manifest = fetch_manifest(&m)?;
            Ok(Update { label: format!("server {m}"), manifest, master: m })
        }
        Source::GitHub(repo) => {
            let manifest = github_manifest(&repo)?;
            Ok(Update { label: format!("GitHub {repo}, release {}", manifest.version), manifest, master: String::new() })
        }
    }
}

#[derive(Deserialize, Clone, Debug)]
pub struct FileEntry {
    pub name: String,
    pub sha256: String,
    pub size: u64,
    /// Where to download it (filled in after fetching: release asset URL or the master's file route).
    #[serde(default)]
    pub url: String,
}

#[derive(Deserialize, Clone, Debug)]
pub struct Manifest {
    pub version: String,
    pub files: Vec<FileEntry>,
    #[serde(default)]
    pub launcher: Option<LauncherInfo>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct LauncherInfo {
    pub version: String,
    pub windows: Option<FileEntry>,
    pub linux: Option<FileEntry>,
}

/// This launcher's build version (date + commit, embedded by build.rs).
pub const VERSION: &str = env!("REACH_LAUNCHER_BUILD");

fn agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder().timeout_global(Some(timeout)).build().into()
}

/// First master whose /healthz answers "ok".
pub fn pick_master(masters: &[String]) -> Result<String, String> {
    let a = agent(Duration::from_secs(5));
    let mut errs = Vec::new();
    for m in masters.iter().cloned() {
        match a.get(format!("{m}/healthz")).call().and_then(|mut r| r.body_mut().read_to_string()) {
            Ok(b) if b.trim() == "ok" => return Ok(m),
            Ok(b) => errs.push(format!("{m}: unexpected answer {:?}", b.chars().take(40).collect::<String>())),
            Err(e) => errs.push(format!("{m}: {e}")),
        }
    }
    Err(format!("no master server answered ({})", errs.join("; ")))
}

fn valid_name(n: &str) -> bool {
    !n.is_empty() && !n.starts_with('.') && n.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

pub fn fetch_manifest(master: &str) -> Result<Manifest, String> {
    let text = agent(Duration::from_secs(15))
        .get(format!("{master}/api/client/manifest"))
        .call()
        .and_then(|mut r| r.body_mut().read_to_string())
        .map_err(|e| format!("manifest from {master}: {e}"))?;
    let mut m: Manifest = serde_json::from_str(&text).map_err(|e| format!("manifest from {master}: {e}"))?;
    // ?sha= makes every build its own URL, so a cache in front of the master (Cloudflare caches .exe) can't serve
    // an older file.
    let fill = |f: &mut FileEntry| f.url = format!("{master}/api/client/files/{}?sha={}", f.name, f.sha256);
    m.files.iter_mut().for_each(fill);
    if let Some(l) = &mut m.launcher {
        l.windows.iter_mut().chain(l.linux.iter_mut()).for_each(fill);
    }
    Ok(m)
}

#[derive(Deserialize)]
struct GhRelease {
    tag_name: String,
    #[serde(default)]
    assets: Vec<GhAsset>,
}

#[derive(Deserialize)]
struct GhAsset {
    name: String,
    size: u64,
    browser_download_url: String,
}

/// GET from GitHub (API or asset download) with the User-Agent it requires; readable errors for the cases players hit.
fn gh_get(url: &str, what: &str) -> Result<String, String> {
    agent(Duration::from_secs(15))
        .get(url)
        .header("User-Agent", concat!("noble-outreach-launcher/", env!("CARGO_PKG_VERSION")))
        .header("Accept", "application/vnd.github+json")
        .call()
        .and_then(|mut r| r.body_mut().read_to_string())
        .map_err(|e| match e {
            ureq::Error::StatusCode(404) => format!("{what}: no release published yet"),
            ureq::Error::StatusCode(403 | 429) => format!("{what}: GitHub rate limit reached (60 checks per hour), try again later"),
            e => format!("{what}: {e}"),
        })
}

/// `<sha256>  <name>` lines (sha256sum format; a leading `*` on the name is binary mode).
fn parse_sums(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|l| {
            let (h, n) = l.trim().split_once(char::is_whitespace)?;
            let n = n.trim().trim_start_matches('*');
            (h.len() == 64 && h.chars().all(|c| c.is_ascii_hexdigit()) && !n.is_empty()).then(|| (h.to_ascii_lowercase(), n.to_string()))
        })
        .collect()
}

/// Latest release of `repo` as a manifest: version = the tag, files and launcher builds = assets listed in the
/// release's own SHA256SUMS (an asset without a sum is ignored).
pub fn github_manifest(repo: &str) -> Result<Manifest, String> {
    let what = format!("GitHub {repo}");
    let rel: GhRelease = serde_json::from_str(&gh_get(&format!("https://api.github.com/repos/{repo}/releases/latest"), &what)?)
        .map_err(|e| format!("{what}: {e}"))?;
    let tag = rel.tag_name;
    let sums_asset = rel.assets.iter().find(|a| a.name == "SHA256SUMS").ok_or_else(|| format!("{what}: release {tag} has no SHA256SUMS"))?;
    let sums = parse_sums(&gh_get(&sums_asset.browser_download_url, &what)?);
    let entry = |name: &str| -> Option<FileEntry> {
        let a = rel.assets.iter().find(|a| a.name == name)?;
        let (sha, _) = sums.iter().find(|(_, n)| n == name)?;
        Some(FileEntry { name: name.into(), sha256: sha.clone(), size: a.size, url: a.browser_download_url.clone() })
    };
    let files = CLIENT_FILES
        .iter()
        .map(|n| entry(n).ok_or_else(|| format!("{what}: release {tag} has no {n} (or it is missing from SHA256SUMS)")))
        .collect::<Result<Vec<_>, _>>()?;
    let launcher = Some(LauncherInfo { version: tag.clone(), windows: entry(LAUNCHER_WINDOWS), linux: entry(LAUNCHER_LINUX) });
    Ok(Manifest { version: tag, files, launcher })
}

/// Download `f` to `part`, verifying sha256 + size (the part file is removed when corrupt).
fn download(f: &FileEntry, part: &Path, progress: &dyn Fn(String)) -> Result<(), String> {
    let big = agent(Duration::from_secs(600));
    progress(format!("downloading {} ({:.1} MB)", f.name, f.size as f64 / 1e6));
    if f.url.is_empty() {
        return Err(format!("{}: no download URL", f.name));
    }
    let resp = big.get(&f.url).header("User-Agent", concat!("noble-outreach-launcher/", env!("CARGO_PKG_VERSION"))).call().map_err(|e| format!("{}: {e}", f.name))?;
    let mut rd = resp.into_body().into_reader();
    let mut out = std::fs::File::create(part).map_err(|e| format!("{}: {e}", part.display()))?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    let mut n_total = 0u64;
    let mut last_report = 0u64;
    loop {
        let n = rd.read(&mut buf).map_err(|e| format!("{}: {e}", f.name))?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
        out.write_all(&buf[..n]).map_err(|e| format!("{}: {e}", part.display()))?;
        n_total += n as u64;
        if n_total - last_report > 1_000_000 {
            last_report = n_total;
            progress(format!("downloading {} {:.0} %", f.name, 100.0 * n_total as f64 / f.size.max(1) as f64));
        }
    }
    drop(out);
    let sha = format!("{:x}", h.finalize());
    if sha != f.sha256 || n_total != f.size {
        let _ = std::fs::remove_file(part);
        return Err(format!("{}: download corrupt (sha256 {sha}, {n_total} bytes; expected {}, {})", f.name, f.sha256, f.size));
    }
    Ok(())
}

/// Bring `dir` up to the manifest. `progress(text)` is called with human-readable steps.
/// Returns (version, files downloaded).
pub fn sync(m: &Manifest, dir: &Path, progress: &dyn Fn(String)) -> Result<(String, usize), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    if m.files.is_empty() {
        return Err("the update source offers no client files yet".into());
    }
    let mut got = 0;
    for f in &m.files {
        if !valid_name(&f.name) {
            return Err(format!("manifest has a bad file name {:?}", f.name));
        }
        let dest = dir.join(&f.name);
        if dest.metadata().map(|md| md.len() == f.size).unwrap_or(false)
            && crate::check::sha256_file(&dest).map(|s| s == f.sha256).unwrap_or(false)
        {
            continue;
        }
        let part = dir.join(format!("{}.part", f.name));
        download(f, &part, progress)?;
        let _ = std::fs::remove_file(&dest); // Windows rename does not replace
        std::fs::rename(&part, &dest).map_err(|e| format!("{}: {e} (is reach-host still running?)", dest.display()))?;
        got += 1;
    }
    let _ = std::fs::write(dir.join("client-version.txt"), &m.version);
    Ok((m.version.clone(), got))
}

/// Set when we were just relaunched by a self-update: never update twice in one run (no loops).
const NO_UPDATE_ENV: &str = "REACH_LAUNCHER_NO_SELFUPDATE";

/// `<exe>.old` left by a Windows self-update (the running exe is renamed, not overwritten); delete it on start.
pub fn cleanup_old() {
    if let Ok(exe) = std::env::current_exe() {
        let mut old = exe.into_os_string();
        old.push(".old");
        let _ = std::fs::remove_file(old);
    }
}

/// If the manifest offers a different launcher build for this OS, download it, replace this executable and
/// relaunch with the same args (does not return on success). Ok(()) when nothing to do; Err = update failed and
/// the current launcher keeps running.
pub fn self_update(m: &Manifest, progress: &dyn Fn(String)) -> Result<(), String> {
    if std::env::var_os(NO_UPDATE_ENV).is_some() {
        return Ok(());
    }
    let Some(l) = &m.launcher else { return Ok(()) };
    if l.version == VERSION {
        return Ok(());
    }
    let Some(f) = (if cfg!(windows) { &l.windows } else { &l.linux }) else { return Ok(()) };
    let exe = std::env::current_exe().map_err(|e| format!("own path: {e}"))?;
    progress(format!("Updating launcher to {}...", l.version));
    let mut new = exe.clone().into_os_string();
    new.push(".new");
    let new = std::path::PathBuf::from(new);
    if let Err(e) = download(f, &new, &|t| progress(format!("Updating launcher: {t}"))) {
        let _ = std::fs::remove_file(&new);
        return Err(e);
    }
    replace_and_relaunch(&exe, &new).map_err(|e| {
        let _ = std::fs::remove_file(&new);
        e
    })
}

#[cfg(windows)]
fn replace_and_relaunch(exe: &Path, new: &Path) -> Result<(), String> {
    let mut old = exe.as_os_str().to_owned();
    old.push(".old");
    let old = std::path::PathBuf::from(old);
    let _ = std::fs::remove_file(&old);
    // A running exe can be renamed but not overwritten.
    std::fs::rename(exe, &old).map_err(|e| format!("rename {}: {e}", exe.display()))?;
    if let Err(e) = std::fs::rename(new, exe) {
        let _ = std::fs::rename(&old, exe);
        return Err(format!("move new launcher into place: {e}"));
    }
    match std::process::Command::new(exe).args(std::env::args_os().skip(1)).env(NO_UPDATE_ENV, "1").spawn() {
        Ok(_) => std::process::exit(0),
        Err(e) => {
            // Put the old one back so the next start is not a surprise either way; keep running.
            Err(format!("relaunch: {e} (new launcher is installed; start it again)"))
        }
    }
}

#[cfg(not(windows))]
fn replace_and_relaunch(exe: &Path, new: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::process::CommandExt;
    std::fs::set_permissions(new, std::fs::Permissions::from_mode(0o755)).map_err(|e| format!("chmod: {e}"))?;
    std::fs::rename(new, exe).map_err(|e| format!("replace {}: {e}", exe.display()))?;
    let e = std::process::Command::new(exe).args(std::env::args_os().skip(1)).env(NO_UPDATE_ENV, "1").exec();
    Err(format!("relaunch: {e} (new launcher is installed; start it again)"))
}

/// Version of the last complete sync, if any.
pub fn local_version(dir: &Path) -> Option<String> {
    std::fs::read_to_string(dir.join("client-version.txt")).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sources() {
        assert!(matches!(parse_source("github:gelibeanz/noble-outreach"), Ok(Source::GitHub(r)) if r == "gelibeanz/noble-outreach"));
        assert!(matches!(parse_source("master:http://a:1/, http://b:2"), Ok(Source::Masters(v)) if v == ["http://a:1", "http://b:2"]));
        assert!(parse_source("github:nope").is_err());
        assert!(parse_source("master:").is_err());
        assert!(parse_source("https://x").is_err());
        assert!(parse_source(BUILT_SOURCE).is_ok());
    }

    #[test]
    fn sums() {
        let h = "a".repeat(64);
        let v = parse_sums(&format!("{h}  reach-host.exe\n{h} *NobleOutreach-Launcher.exe\nbad line\n"));
        assert_eq!(v, vec![(h.clone(), "reach-host.exe".into()), (h, "NobleOutreach-Launcher.exe".into())]);
    }
}
