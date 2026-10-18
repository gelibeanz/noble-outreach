//! Dedicated server playlists (`--playlist <file.json>`): a weighted list of games the server rotates through by
//! player vote (vote.rs). The server process plays one entry; a vote for another entry restarts the process, which
//! reads the winner from `vote-next.json` next to the exe. Format and behaviour: notes/dedicated.md "Playlists".

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::log;

/// Written by the server before a vote restart; read (and deleted) by the next process.
pub const STATE_FILE: &str = "vote-next.json";

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub name: String,
    /// `slayer` (any multiplayer game variant; `multiplayer`/`mp` also accepted), `firefight` or `campaign`.
    pub mode: String,
    pub map: String,
    /// File in `haloreach/game_variants` (not needed for campaign).
    #[serde(default)]
    pub game_variant: Option<String>,
    /// Map variant file (path or file name in `haloreach/map_variants`), multiplayer only.
    #[serde(default)]
    pub map_variant: Option<String>,
    /// easy | normal | heroic | legendary (Firefight / campaign; default normal).
    #[serde(default)]
    pub difficulty: Option<String>,
    #[serde(default = "one")]
    pub weight: u32,
}

fn one() -> u32 {
    1
}

#[derive(Deserialize)]
#[serde(untagged)]
enum FileForm {
    Full { name: Option<String>, entries: Vec<Entry> },
    Bare(Vec<Entry>),
}

pub struct Playlist {
    pub name: String,
    pub path: String,
    pub entries: Vec<Entry>,
    /// Entry this process plays.
    pub current: usize,
    /// This process was started by a vote (no pre-game vote then).
    pub voted: bool,
}

static PLAYLIST: OnceLock<Playlist> = OnceLock::new();

pub fn get() -> Option<&'static Playlist> {
    PLAYLIST.get()
}

/// Query mode byte (query.rs) of an entry.
pub fn mode_byte(e: &Entry) -> u8 {
    match e.mode.as_str() {
        "firefight" => crate::query::MODE_FIREFIGHT,
        "campaign" => crate::query::MODE_CAMPAIGN,
        _ => crate::query::MODE_MULTIPLAYER,
    }
}

fn is_mission(map: &str) -> bool {
    let b = map.as_bytes();
    b.len() > 1 && b[0] == b'm' && b[1].is_ascii_digit()
}

/// Load and check a playlist against the MCC files. Errors name the file and entry.
pub fn load(path: &Path, mcc: &Path) -> Result<(String, Vec<Entry>), String> {
    let file = path.display();
    let text = std::fs::read_to_string(path).map_err(|e| format!("playlist {file}: {e}"))?;
    let (name, mut entries) = match serde_json::from_str::<FileForm>(&text) {
        Ok(FileForm::Full { name, entries }) => (name, entries),
        Ok(FileForm::Bare(v)) => (None, v),
        // The untagged error is vague; re-parse as the full form for a line/column message.
        Err(_) => {
            #[derive(Deserialize)]
            #[allow(dead_code)]
            struct Strict {
                name: Option<String>,
                entries: Vec<Entry>,
            }
            let e = serde_json::from_str::<Strict>(&text).err().map_or_else(|| "not a playlist".to_string(), |e| e.to_string());
            return Err(format!("playlist {file}: {e} (expected {{\"name\": ..., \"entries\": [{{name, mode, map, game_variant, map_variant?, difficulty?, weight}}, ...]}})"));
        }
    };
    let name = name.unwrap_or_else(|| path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default());
    if entries.len() < 3 {
        return Err(format!("playlist {file}: {} entries; a playlist needs at least 3 (the vote offers 3)", entries.len()));
    }
    let hr = mcc.join("haloreach");
    for (i, e) in entries.iter_mut().enumerate() {
        let at = format!("playlist {file}: entry {} (\"{}\")", i + 1, e.name);
        e.mode = match e.mode.to_ascii_lowercase().as_str() {
            "slayer" | "multiplayer" | "mp" => "slayer".into(),
            "firefight" | "ff" => "firefight".into(),
            "campaign" | "coop" => "campaign".into(),
            m => return Err(format!("{at}: mode \"{m}\": expected slayer, firefight or campaign")),
        };
        if e.weight == 0 {
            return Err(format!("{at}: weight must be 1 or more"));
        }
        let stem = if e.map == "forge_world" { "forge_halo" } else { e.map.as_str() };
        if !hr.join("maps").join(format!("{stem}.map")).is_file() {
            return Err(format!("{at}: map \"{}\" not found ({})", e.map, hr.join("maps").join(format!("{stem}.map")).display()));
        }
        match e.mode.as_str() {
            "firefight" if !e.map.starts_with("ff") => return Err(format!("{at}: \"{}\" is not a Firefight map (ff...)", e.map)),
            "campaign" if !is_mission(&e.map) => return Err(format!("{at}: \"{}\" is not a campaign mission (m10, m20, ...)", e.map)),
            "slayer" if e.map.starts_with("ff") || is_mission(&e.map) => {
                return Err(format!("{at}: \"{}\" is a Firefight/campaign map; set mode accordingly", e.map))
            }
            _ => {}
        }
        if e.mode != "campaign" {
            let gv = e.game_variant.as_deref().ok_or(format!("{at}: game_variant missing (a file in {})", hr.join("game_variants").display()))?;
            if !crate::catalog::game_variant_path(mcc, gv).is_file() {
                return Err(format!("{at}: game variant {gv} not found in {} or hopper_game_variants", hr.join("game_variants").display()));
            }
            let ff_gv = gv.starts_with("hr_ff_") || gv.starts_with("ff_");
            if ff_gv != (e.mode == "firefight") {
                return Err(format!("{at}: game variant {gv} does not fit mode {}", e.mode));
            }
        }
        if let Some(mv) = &e.map_variant {
            if e.mode != "slayer" {
                return Err(format!("{at}: map_variant is for multiplayer entries only"));
            }
            if !crate::catalog::map_variant_path(mcc, mv).is_file() {
                return Err(format!("{at}: map variant {mv} not found"));
            }
        }
        if let Some(d) = &e.difficulty {
            if !matches!(d.as_str(), "easy" | "normal" | "heroic" | "legendary") {
                return Err(format!("{at}: difficulty \"{d}\": expected easy, normal, heroic or legendary"));
            }
        }
    }
    Ok((name, entries))
}

/// Weighted random pick of up to `n` distinct entries, skipping `exclude` while enough others remain.
pub fn pick(entries: &[Entry], n: usize, exclude: &[usize]) -> Vec<usize> {
    let mut pool: Vec<usize> = (0..entries.len()).filter(|i| !exclude.contains(i)).collect();
    if pool.len() < n {
        pool = (0..entries.len()).collect();
    }
    let mut out = Vec::new();
    while out.len() < n && !pool.is_empty() {
        let total: u64 = pool.iter().map(|&i| entries[i].weight as u64).sum();
        let mut r = rand_u64() % total.max(1);
        let k = pool.iter().position(|&i| {
            let w = entries[i].weight as u64;
            if r < w {
                true
            } else {
                r -= w;
                false
            }
        });
        out.push(pool.remove(k.unwrap_or(0)));
    }
    out
}

/// splitmix64 over a clock/pid seed (no rand crate; xorshift's low bits were poor for small moduli).
pub fn rand_u64() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static S: AtomicU64 = AtomicU64::new(0);
    if S.load(Ordering::Relaxed) == 0 {
        let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1)
            ^ ((std::process::id() as u64) << 32);
        let _ = S.compare_exchange(0, seed | 1, Ordering::Relaxed, Ordering::Relaxed);
    }
    let mut z = S.fetch_add(0x9E37_79B9_7F4A_7C15, Ordering::Relaxed).wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

#[derive(Serialize, Deserialize)]
struct NextState {
    playlist: String,
    entry: usize,
    name: String,
}

fn state_path() -> PathBuf {
    std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.join(STATE_FILE))).unwrap_or_else(|| STATE_FILE.into())
}

/// Server, before a vote restart: the next process plays `entry`.
pub fn write_next(entry: usize) -> Result<(), String> {
    let p = get().ok_or("no playlist")?;
    let s = NextState { playlist: p.path.clone(), entry, name: p.entries[entry].name.clone() };
    std::fs::write(state_path(), serde_json::to_vec(&s).unwrap()).map_err(|e| format!("write {}: {e}", state_path().display()))
}

/// Mode/map flags dropped from the command line when a playlist entry replaces them.
const MODE_FLAGS: &[&str] = &["--map", "--firefight", "--campaign", "--difficulty", "--game-variant", "--map-variant", "--insertion", "--checkpoint"];

/// Command-line flags that play `e` (game variant as a full path under `mcc`).
/// Co-op player cap from the raw arguments (`--coop-max N`, read before main parses them), 4 without it.
fn coop_cap(argv: &[String]) -> u32 {
    let i = argv.iter().position(|a| a == "--coop-max");
    i.and_then(|i| argv.get(i + 1)).and_then(|v| v.parse::<u32>().ok()).map_or(4, |n| n.clamp(4, crate::coop::MAX))
}

pub fn entry_args(e: &Entry, mcc: &Path) -> Vec<String> {
    let mut a: Vec<String> = match e.mode.as_str() {
        "firefight" => vec!["--firefight".into(), e.map.clone()],
        "campaign" => vec!["--campaign".into(), e.map.clone()],
        _ => vec!["--map".into(), e.map.clone()],
    };
    if e.mode != "campaign" {
        if let Some(gv) = &e.game_variant {
            a.extend(["--game-variant".into(), crate::catalog::game_variant_path(mcc, gv).to_string_lossy().into_owned()]);
        }
    }
    if let Some(mv) = &e.map_variant {
        let p = crate::catalog::map_variant_path(mcc, mv);
        a.extend(["--map-variant".into(), p.to_string_lossy().into_owned()]);
    }
    if e.mode != "slayer" {
        a.extend(["--difficulty".into(), e.difficulty.clone().unwrap_or_else(|| "normal".into())]);
    }
    a
}

/// `argv` with the mode flags (and their values) replaced by `new`.
pub fn replace_mode_args(argv: &[String], new: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut it = argv.iter();
    while let Some(a) = it.next() {
        if MODE_FLAGS.contains(&a.as_str()) {
            it.next();
            continue;
        }
        out.push(a.clone());
    }
    out.extend(new.iter().cloned());
    out
}

fn flag_value<'a>(argv: &'a [String], flag: &str) -> Option<&'a String> {
    argv.iter().position(|a| a == flag).and_then(|i| argv.get(i + 1))
}

/// `--playlist <file>` on a dedicated server: load and check it, choose this process's entry (the vote winner from
/// `vote-next.json`, else a weighted random one) and rewrite the mode flags in `argv` to play it.
pub fn apply(argv: &mut Vec<String>) -> Result<(), String> {
    let Some(path) = flag_value(argv, "--playlist").cloned() else { return Ok(()) };
    if !argv.iter().any(|a| a == "--dedicated") {
        return Err("--playlist is for --dedicated servers".into());
    }
    let mcc = flag_value(argv, "--mcc").map(PathBuf::from).or_else(|| std::env::var_os("MCC_PATH").map(PathBuf::from)).ok_or("no --mcc <dir> and MCC_PATH unset")?;
    let (name, entries) = load(Path::new(&path), &mcc)?;
    let sp = state_path();
    let next = std::fs::read(&sp).ok().and_then(|b| serde_json::from_slice::<NextState>(&b).ok());
    let _ = std::fs::remove_file(&sp); // one-shot: a crash or empty-server restart picks afresh
    let (current, voted) = match next {
        Some(s) if s.playlist == path && s.entry < entries.len() && entries[s.entry].name == s.name => (s.entry, true),
        Some(s) => {
            log!("playlist: ignoring {} (entry {} \"{}\" of {} does not match this playlist)", sp.display(), s.entry, s.name, s.playlist);
            (pick(&entries, 1, &[])[0], false)
        }
        None => (pick(&entries, 1, &[])[0], false),
    };
    let e = &entries[current];
    log!("playlist: \"{name}\" ({} entries) from {path}; playing entry {} \"{}\" ({} on {}){}",
        entries.len(), current + 1, e.name, e.mode, e.map, if voted { ", chosen by vote" } else { "" });
    let mut new = entry_args(e, &mcc);
    // Firefight and campaign are 4-player co-op modes (same cap as the Docker entrypoint), or up to `--coop-max N`.
    if e.mode != "slayer" {
        let cap = coop_cap(argv);
        if let Some(i) = argv.iter().position(|a| a == "--max-players") {
            if argv.get(i + 1).and_then(|v| v.parse::<u32>().ok()).is_some_and(|n| n > cap) {
                log!("playlist: {} entry, max players capped at {cap}", e.mode);
                argv[i + 1] = cap.to_string();
            }
        }
    }
    new.extend(["--playlist".into(), path.clone()]);
    let rest: Vec<String> = {
        // drop the old --playlist pair; `new` re-adds it at the end
        let mut v = Vec::new();
        let mut it = argv.iter();
        while let Some(a) = it.next() {
            if a == "--playlist" {
                it.next();
                continue;
            }
            v.push(a.clone());
        }
        v
    };
    *argv = replace_mode_args(&rest, &new);
    let _ = PLAYLIST.set(Playlist { name, path, entries, current, voted });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(name: &str, w: u32) -> Entry {
        Entry { name: name.into(), mode: "slayer".into(), map: "forge_world".into(), game_variant: Some("slayer_054.bin".into()), map_variant: None, difficulty: None, weight: w }
    }

    #[test]
    fn pick_distinct() {
        let v = vec![e("a", 1), e("b", 5), e("c", 1), e("d", 1)];
        for _ in 0..50 {
            let p = pick(&v, 3, &[]);
            assert_eq!(p.len(), 3);
            assert!(p[0] != p[1] && p[1] != p[2] && p[0] != p[2]);
            let q = pick(&v, 3, &[0]);
            assert!(!q.contains(&0));
        }
    }

    #[test]
    fn coop_cap_from_args() {
        let a = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(coop_cap(&a(&["--dedicated"])), 4);
        assert_eq!(coop_cap(&a(&["--coop-max", "6"])), 6);
        assert_eq!(coop_cap(&a(&["--coop-max", "12"])), 8);
    }

    #[test]
    fn replace() {
        let a: Vec<String> = ["--dedicated", "--map", "x", "--max-players", "4", "--game-variant", "g"].iter().map(|s| s.to_string()).collect();
        let r = replace_mode_args(&a, &["--firefight".into(), "ff10_prototype".into()]);
        assert_eq!(r, ["--dedicated", "--max-players", "4", "--firefight", "ff10_prototype"]);
    }
}
