//! What the player's MCC files offer for multiplayer: game types (`haloreach/game_variants` and
//! `hopper_game_variants`, `.bin`) and map variants (`map_variants`, `hopper_map_variants`, our saved Forge maps,
//! `.mvar`), grouped for the PLAY / HOST screens (ui/play_screen.rs). Scanned once per MCC folder and cached.
//!
//! Both file kinds are BLF: `_blf` (0x30 bytes), then `chdr` at +0x30 whose content (file +0x40) is the plain
//! content header: +0x2A u8 game engine (1 Forge, 2 Megalo = Slayer etc., 3 campaign, 4 Firefight), +0x2C i32
//! mapinfo id (map variants; -1 in game variants), +0x80 name and +0x180 description (UTF-16LE, 128 chars each).
//! The next chunk (`mpvr` / `mvar`) is a bit stream; in `mpvr` the same name and description follow as
//! null-terminated UTF-16BE, and right after them come the game type icon (8 bits: 0 CTF, 1 Slayer, 2 Oddball,
//! 3 King of the Hill, 4 Juggernaut, 5 Territories, 6 Assault, 7 Infection, 9 Invasion, 10 Invasion Slayer,
//! 11 Stockpile, 13 Race, 14 Rocket Race, 15 Grifball, 17 Headhunter, 20 other) and, one bit later, "teams on".
//! Layout facts: notes/frontend.md "Multiplayer screens".
//!
//! Community content (notes/community.md): `<reach-data>/custom/` (made on first run, README inside) and the
//! player's MCC Steam Workshop folders (`steamapps/workshop/content/976730/<item>/`) are walked for `.mvar` and
//! `.bin` files; only Reach files are listed (content header: BLF with `chdr` + `mvar` / `mpvr`, a known engine,
//! a known base map), everything else is skipped with one log line.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::log;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Category {
    Slayer,
    Objective,
    Infection,
    Party,
    Invasion,
    Firefight,
    Forge,
}

impl Category {
    pub const ALL: [Category; 7] = [Category::Slayer, Category::Objective, Category::Infection, Category::Party, Category::Invasion, Category::Firefight, Category::Forge];

    pub fn name(self) -> &'static str {
        match self {
            Category::Slayer => "Slayer",
            Category::Objective => "Objective",
            Category::Infection => "Infection",
            Category::Party => "Party",
            Category::Invasion => "Invasion",
            Category::Firefight => "Firefight",
            Category::Forge => "Forge",
        }
    }
}

/// A kind of game: (name, category, texture-pack banner, texture-pack icon, our own one-line description).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Kind {
    pub name: &'static str,
    pub category: Category,
    pub banner: &'static str,
    pub icon: &'static str,
    pub about: &'static str,
}

const fn k(name: &'static str, category: Category, banner: &'static str, icon: &'static str, about: &'static str) -> Kind {
    Kind { name, category, banner, icon, about }
}

const SLAYER: Kind = k("Slayer", Category::Slayer, "CG_SLAYER", "PLAYERHUB_SLAYER", "Kill the other players. Most kills when time runs out wins.");
const ESCALATION: Kind = k("Escalation Slayer", Category::Slayer, "CG_ESCALATION", "PLAYERHUB_ESCALATION", "Each kill moves you up to the next weapon. First through the list wins.");
const SWAT: Kind = k("SWAT", Category::Party, "CG_SLAYER", "PLAYERHUB_SLAYER", "No shields, no radar, precision weapons. One headshot is all it takes.");
const CTF: Kind = k("Capture the Flag", Category::Objective, "CG_CAPTURETHEFLAG", "PLAYERHUB_CTF", "Take the enemy flag back to your base while you defend your own.");
const ODDBALL: Kind = k("Oddball", Category::Objective, "CG_ODDBALL", "PLAYERHUB_ODDBALL", "Grab the skull and keep it. The longer you hold it, the more you score.");
const KOTH: Kind = k("King of the Hill", Category::Objective, "CG_KINGOFTHEHILL", "PLAYERHUB_KINGOFTHEHILL", "Stand in the hill to score. The hill moves, so keep moving with it.");
const TERRITORIES: Kind = k("Territories", Category::Objective, "CG_TERRITORIES", "PLAYERHUB_TERRITORIES", "Capture and hold territories around the map.");
const ASSAULT: Kind = k("Assault", Category::Objective, "CG_ASSAULT", "PLAYERHUB_ASSAULT", "Carry the bomb into the enemy base and arm it.");
const STOCKPILE: Kind = k("Stockpile", Category::Objective, "CG_STOCKPILE", "PLAYERHUB_STOCKPILE", "Collect flags from the field and hold them in your base until the count.");
const HEADHUNTER: Kind = k("Headhunter", Category::Objective, "CG_HEADHUNTER", "PLAYERHUB_HEADHUNTER", "Kills drop skulls. Carry them to a scoring zone before someone takes them.");
const INFECTION: Kind = k("Infection", Category::Infection, "CG_INFECTION", "PLAYERHUB_INFECTION", "Survivors against zombies. Every survivor killed joins the horde.");
const JUGGERNAUT: Kind = k("Juggernaut", Category::Party, "CG_JUGGERNAUT", "PLAYERHUB_JUGGERNAUT", "One player is the Juggernaut. Kill the Juggernaut to become it.");
const RACE: Kind = k("Race", Category::Party, "CG_RACE", "PLAYERHUB_RACE", "Drive through the checkpoints. Fastest lap wins.");
const GRIFBALL: Kind = k("Grifball", Category::Party, "CG_GRIFBALL", "PLAYERHUB_GRIFBALL", "Hammers, swords and a bomb on a court. Score by planting it in the enemy goal.");
const MINIGAME: Kind = k("Mini game", Category::Party, "CG_INSANE", "PLAYERHUB_DEFAULT", "A community party game made with the Megalo game type editor.");
const CUSTOM: Kind = k("Custom game", Category::Party, "CG_INSANE", "PLAYERHUB_DEFAULT", "A custom game type.");
const INVASION: Kind = k("Invasion", Category::Invasion, "CG_INVASION", "PLAYERHUB_INVASION", "Spartans against Elites in phases: attack or defend objectives across the map.");
const INVASION_SLAYER: Kind = k("Invasion Slayer", Category::Invasion, "CG_INVASION", "PLAYERHUB_INVASION", "Spartans against Elites with weapon and vehicle drops that grow with the score.");
const FIREFIGHT: Kind = k("Firefight", Category::Firefight, "", "PLAYERHUB_FIREFIGHT", "Co-op survival against waves of Covenant.");
const FORGE: Kind = k("Forge", Category::Forge, "CG_BASICEDITING", "PLAYERHUB_BASICEDITING", "Edit the map: place, move and delete objects with your friends.");

/// One game type file.
#[derive(Clone, Debug)]
pub struct GameType {
    /// File name (`slayer_054.bin`), what launches and the server browser carry.
    pub file: String,
    pub path: PathBuf,
    pub title: String,
    pub desc: String,
    pub kind: Kind,
    /// Teams on (None: Firefight / Forge, where it does not apply).
    pub teams: Option<bool>,
    /// From `hopper_game_variants` (matchmaking playlists), not the shipped custom-game set.
    pub hopper: bool,
    /// Builtin (shipped custom games), Hopper, Custom or Workshop.
    pub source: Source,
}

impl GameType {
    /// From the player's custom folder or the Steam Workshop.
    pub fn community(&self) -> bool {
        self.source.community()
    }

    /// Same mode, different file: Reach shipped many modes several times (custom games, 2010 matchmaking, the
    /// Title Update, Firefight Arcade, MCC's `hr_*` remakes), all with one display name.
    pub fn same_mode(&self, o: &GameType) -> bool {
        self.kind.name == o.kind.name && self.title.eq_ignore_ascii_case(&o.title)
    }

    /// Which release of the mode this file is, plus the settings its file name spells out: "MCC · 4v4 · 50 points",
    /// "Arcade", "Original matchmaking", "Title Update", "Custom games", "Community".
    pub fn edition(&self) -> String {
        let f = self.file.to_ascii_lowercase();
        let stem = f.trim_end_matches(".bin");
        let base = if self.community() {
            "Community"
        } else if stem.starts_with("hr_") {
            "MCC"
        } else if stem.starts_with("ff_arcade_") {
            "Arcade"
        } else if stem.starts_with("tu1_") || stem.ends_with("_tu") || stem.contains("_tu_") {
            "Title Update"
        } else if self.hopper {
            "Original matchmaking"
        } else {
            "Custom games"
        };
        let mut parts = vec![base.to_string()];
        for t in stem.split('_') {
            let n: String = t.chars().take_while(|c| c.is_ascii_digit()).collect();
            let rest = &t[n.len()..];
            if n.is_empty() {
                continue;
            }
            let unit = match rest {
                "points" => "points",
                "set" | "sets" => if n == "1" { "set" } else { "sets" },
                "round" | "rounds" => if n == "1" { "round" } else { "rounds" },
                "turns" => "turns",
                "min" => "min",
                _ if rest.starts_with('v') && rest[1..].chars().all(|c| c.is_ascii_digit()) && rest.len() > 1 => {
                    parts.push(t.to_string());
                    continue;
                }
                _ => continue,
            };
            parts.push(format!("{n} {unit}"));
        }
        parts.join(" · ")
    }
}

/// One map variant file (or a built-in map's default when `path` is None).
#[derive(Clone, Debug)]
pub struct MapVariant {
    pub path: Option<PathBuf>,
    pub title: String,
    pub desc: String,
    pub source: Source,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// The map as it ships (no `--map-variant`).
    Default,
    /// `haloreach/map_variants`: Bungie's remixes (Forge World's Asylum, Pinnacle, ...) and the Anniversary maps.
    Builtin,
    /// `haloreach/hopper_map_variants`: matchmaking and community maps.
    Hopper,
    /// Our own library, saved in Forge (`reach-data/map_variants`).
    Saved,
    /// `<reach-data>/custom/`: files the player dropped in.
    Custom,
    /// The player's MCC Steam Workshop subscriptions.
    Workshop,
}

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Source::Default => "Default",
            Source::Builtin => "Bungie",
            Source::Hopper => "Matchmaking",
            Source::Saved => "Saved",
            Source::Custom => "Custom",
            Source::Workshop => "Workshop",
        }
    }

    pub fn community(self) -> bool {
        matches!(self, Source::Custom | Source::Workshop)
    }
}

/// A built-in multiplayer map and the variants made on it.
#[derive(Clone, Debug)]
pub struct BaseMap {
    /// mapinfo stem (`forge_halo`).
    pub stem: String,
    pub name: String,
    pub mapinfo_id: i32,
    /// Default first, then Bungie's, then matchmaking ones (saved maps come from `with_saved`).
    pub variants: Vec<MapVariant>,
}

#[derive(Debug, Default)]
pub struct Catalog {
    pub games: Vec<GameType>,
    pub maps: Vec<BaseMap>,
}

static CACHE: Mutex<Option<(PathBuf, Arc<Catalog>)>> = Mutex::new(None);

/// The catalog for this MCC folder (scanned on first use).
pub fn get(mcc: &Path) -> Arc<Catalog> {
    let mut g = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((p, c)) = g.as_ref() {
        if p == mcc {
            return c.clone();
        }
    }
    let t = std::time::Instant::now();
    let c = Arc::new(scan_with(mcc, &Roots::find(mcc)));
    let nv: usize = c.maps.iter().map(|m| m.variants.len() - 1).sum();
    log!("catalog: {} game types, {} maps, {} map variants in {:.0} ms", c.games.len(), c.maps.len(), nv, t.elapsed().as_secs_f64() * 1000.0);
    *g = Some((mcc.to_path_buf(), c.clone()));
    c
}

/// Scan in the background (the menu's first `get` then waits for it instead of starting over).
pub fn prefetch(mcc: &Path) {
    let mcc = mcc.to_path_buf();
    std::thread::spawn(move || {
        get(&mcc);
    });
}

/// Where a game type file lives: `game_variants`, else `hopper_game_variants` (a server or vote may name either).
pub fn game_variant_path(mcc: &Path, file: &str) -> PathBuf {
    let hr = mcc.join("haloreach");
    let p = hr.join("game_variants").join(file);
    if p.is_file() {
        return p;
    }
    let h = hr.join("hopper_game_variants").join(file);
    if h.is_file() {
        return h;
    }
    if !p.is_file() {
        // A custom / Workshop game type named by file name (votes, playlists): the cached scan knows where it is.
        let g = CACHE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(gt) = g.as_ref().and_then(|(_, c)| c.games.iter().find(|g| g.community() && g.file.eq_ignore_ascii_case(file))) {
            return gt.path.clone();
        }
    }
    p
}

/// A map variant named by path, or by file name in `map_variants`, else `hopper_map_variants`.
pub fn map_variant_path(mcc: &Path, file: &str) -> PathBuf {
    if Path::new(file).is_file() {
        return PathBuf::from(file);
    }
    let hr = mcc.join("haloreach");
    let p = hr.join("map_variants").join(file);
    let h = hr.join("hopper_map_variants").join(file);
    if !p.is_file() && h.is_file() {
        return h;
    }
    if !p.is_file() {
        let g = CACHE.lock().unwrap_or_else(|e| e.into_inner());
        let found = g.as_ref().and_then(|(_, c)| {
            c.maps.iter().flat_map(|m| &m.variants).find(|v| v.source.community() && v.path.as_ref().and_then(|p| p.file_name()).is_some_and(|f| f.to_string_lossy().eq_ignore_ascii_case(file)))
        });
        if let Some(p) = found.and_then(|v| v.path.clone()) {
            return p;
        }
    }
    p
}

/// Where community content is looked for.
#[derive(Default, Debug)]
pub struct Roots {
    /// `<reach-data>/custom` (walked whole: `maps/`, `gametypes/` and loose files).
    pub custom: Option<PathBuf>,
    /// `steamapps/workshop/content/976730` folders (one per Steam library that has it).
    pub workshop: Vec<PathBuf>,
}

/// MCC's Steam app id (Workshop content lives under `workshop/content/976730`).
const MCC_APP: &str = "976730";

impl Roots {
    /// The real ones for this MCC folder; makes the custom folder (with its README) on first use.
    pub fn find(mcc: &Path) -> Roots {
        let custom = crate::progress::data_dir().join("custom");
        ensure_custom_dir(&custom);
        Roots { custom: Some(custom), workshop: workshop_dirs(mcc) }
    }
}

const README: &str = "Community content for Halo: Reach\r\n\
\r\n\
Drop Reach files in this folder (or the two folders inside it) and they show up in PLAY and HOST GAME under\r\n\
the Community chip:\r\n\
\r\n\
\x20 maps\\       map variants made in Forge (.mvar)\r\n\
\x20 gametypes\\  game types made in the Reach game type editor (.bin)\r\n\
\r\n\
Files dropped loose in this folder are fine too, and so are sub-folders.\r\n\
Map variants are listed under the map they were made on. Files that are not Reach map variants or game\r\n\
types (other games, damaged files) are skipped; the reason is in reach-host.log (lines starting 'catalog:').\r\n\
\r\n\
Steam Workshop: if you subscribed to Reach maps or game types for MCC on Steam, those are found by themselves\r\n\
(steamapps/workshop/content/976730/...), nothing to copy.\r\n\
\r\n\
Players who join your server get your custom map and game type from the server; they do not need the files.\r\n";

fn ensure_custom_dir(dir: &Path) {
    for d in [dir.to_path_buf(), dir.join("maps"), dir.join("gametypes")] {
        let _ = std::fs::create_dir_all(d);
    }
    let r = dir.join("README.txt");
    if !r.exists() {
        let _ = std::fs::write(r, README);
    }
}

/// `<library>/steamapps/workshop/content/976730` for every Steam library we can find: the one MCC sits in (go up to
/// `steamapps`), the others listed in its `libraryfolders.vdf`, `NOBLE_STEAM_LIBRARY` (a library root, for tests),
/// and the default Steam folders.
pub fn workshop_dirs(mcc: &Path) -> Vec<PathBuf> {
    let mut libs: Vec<PathBuf> = Vec::new();
    if let Ok(v) = std::env::var("NOBLE_STEAM_LIBRARY") {
        libs.extend(v.split(';').filter(|s| !s.is_empty()).map(PathBuf::from));
    }
    if let Some(sa) = mcc.ancestors().find(|a| a.file_name().is_some_and(|n| n.eq_ignore_ascii_case("steamapps"))) {
        if let Some(lib) = sa.parent() {
            libs.push(lib.to_path_buf());
        }
        libs.extend(vdf_libraries(&sa.join("libraryfolders.vdf")));
    }
    let mut steam: Vec<PathBuf> = vec![PathBuf::from(r"C:\Program Files (x86)\Steam"), PathBuf::from(r"C:\Program Files\Steam")];
    if let Some(h) = std::env::var_os("HOME") {
        let h = PathBuf::from(h);
        steam.extend([h.join(".local/share/Steam"), h.join(".steam/steam")]);
    }
    for s in steam {
        libs.extend(vdf_libraries(&s.join("steamapps").join("libraryfolders.vdf")));
        libs.push(s);
    }
    let mut out: Vec<PathBuf> = Vec::new();
    for l in libs {
        let w = l.join("steamapps").join("workshop").join("content").join(MCC_APP);
        if w.is_dir() && !out.iter().any(|o| same_dir(o, &w)) {
            out.push(w);
        }
    }
    out
}

fn same_dir(a: &Path, b: &Path) -> bool {
    a == b || matches!((a.canonicalize(), b.canonicalize()), (Ok(x), Ok(y)) if x == y)
}

/// The `"path"  "<dir>"` lines of a `libraryfolders.vdf`.
fn vdf_libraries(vdf: &Path) -> Vec<PathBuf> {
    let Ok(t) = std::fs::read_to_string(vdf) else { return Vec::new() };
    t.lines()
        .filter_map(|l| {
            let q: Vec<&str> = l.split('"').collect(); // ["\t", "path", "\t\t", "<dir>", ""]
            (q.len() >= 4 && q[1] == "path").then(|| PathBuf::from(q[3].replace("\\\\", "\\")))
        })
        .collect()
}

/// Every `.mvar` / `.bin` under `root` (depth 5), sorted by path: (file name, path).
fn walk(root: &Path, depth: u32, out: &mut Vec<(String, PathBuf)>) {
    let mut entries: Vec<_> = std::fs::read_dir(root).into_iter().flatten().flatten().collect();
    entries.sort_by_key(|e| e.path());
    for e in entries {
        let p = e.path();
        let name = e.file_name().to_string_lossy().into_owned();
        if p.is_dir() {
            if depth > 0 {
                walk(&p, depth - 1, out);
            }
        } else {
            let l = name.to_ascii_lowercase();
            if l.ends_with(".mvar") || l.ends_with(".bin") {
                out.push((name, p));
            }
        }
    }
}

fn files(dir: &Path, ext: &str) -> Vec<(String, PathBuf)> {
    let mut v: Vec<(String, PathBuf)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path()))
        .filter(|(n, _)| n.to_ascii_lowercase().ends_with(ext))
        .collect();
    v.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
    v
}

fn scan(mcc: &Path) -> Catalog {
    scan_with(mcc, &Roots::default())
}

fn scan_with(mcc: &Path, roots: &Roots) -> Catalog {
    let hr = mcc.join("haloreach");
    let mut community: Vec<(String, PathBuf, Source)> = Vec::new();
    for (root, source) in roots.custom.iter().map(|c| (c, Source::Custom)).chain(roots.workshop.iter().map(|w| (w, Source::Workshop))) {
        let mut v = Vec::new();
        walk(root, 5, &mut v);
        community.extend(v.into_iter().map(|(n, p)| (n, p, source)));
    }
    let mut games: Vec<GameType> = Vec::new();
    let mut seen_games: Vec<Vec<u8>> = Vec::new(); // community copies of a shipped or already listed file are skipped
    for (dir, hopper) in [("game_variants", false), ("hopper_game_variants", true)] {
        for (file, path) in files(&hr.join(dir), ".bin") {
            if games.iter().any(|g| g.file.eq_ignore_ascii_case(&file)) {
                continue; // shipped copy wins
            }
            let Ok(b) = std::fs::read(&path) else { continue };
            if let Some(g) = game_type(&b, &file, path, hopper) {
                seen_games.push(b[0x30..].to_vec());
                games.push(g);
            }
        }
    }
    for (file, path, source) in community.iter().filter(|c| c.0.to_ascii_lowercase().ends_with(".bin")) {
        let Ok(b) = std::fs::read(path) else {
            log!("catalog: skipped {} (unreadable)", path.display());
            continue;
        };
        let Some(mut g) = game_type(&b, file, path.clone(), false) else {
            log!("catalog: skipped {} (not a Reach game type)", path.display());
            continue;
        };
        let body = b.get(0x30..).unwrap_or_default().to_vec();
        if seen_games.contains(&body) {
            continue;
        }
        seen_games.push(body);
        g.source = *source;
        games.push(g);
    }
    let rank = |g: &GameType| (g.community(), g.hopper);
    games.sort_by(|a, b| (rank(a), a.kind.category as u8, a.title.to_lowercase()).cmp(&(rank(b), b.kind.category as u8, b.title.to_lowercase())));

    let mut maps: Vec<BaseMap> = crate::ui::menu::MAPS
        .iter()
        .filter_map(|&(stem, name)| {
            let id = mapinfo_id(&hr.join("maps").join("info").join(format!("{stem}.mapinfo")))?;
            let default = if stem == "forge_halo" { format!("{name} (blank)") } else { name.to_string() };
            let about = if stem == "forge_halo" { "The empty Forge World canvas: build your own map on it." } else { "The map as it ships." };
            Some(BaseMap { stem: stem.into(), name: name.into(), mapinfo_id: id, variants: vec![MapVariant { path: None, title: default, desc: about.into(), source: Source::Default }] })
        })
        .collect();
    let mut seen: Vec<Vec<u8>> = Vec::new(); // identical files (hopper copies of shipped ones) are listed once
    for (dir, source) in [("map_variants", Source::Builtin), ("hopper_map_variants", Source::Hopper)] {
        for (file, path) in files(&hr.join(dir), ".mvar") {
            let Ok(b) = std::fs::read(&path) else { continue };
            let Some((title, desc, id)) = map_variant(&b) else { continue };
            let Some(m) = maps.iter_mut().find(|m| m.mapinfo_id == id) else { continue };
            let body = b.get(0x2F0..).unwrap_or_default().to_vec();
            if seen.contains(&body) {
                continue;
            }
            seen.push(body);
            let title = if title.starts_with('$') || title.trim().is_empty() { title_from_file(&file, &m.stem) } else { title };
            let desc = if desc.starts_with('$') { format!("A {} remix by Bungie.", m.name) } else { desc };
            m.variants.push(MapVariant { path: Some(path), title, desc, source });
        }
    }
    for (file, path, source) in community.iter().filter(|c| c.0.to_ascii_lowercase().ends_with(".mvar")) {
        let Ok(b) = std::fs::read(path) else {
            log!("catalog: skipped {} (unreadable)", path.display());
            continue;
        };
        let Some((title, desc, id)) = map_variant(&b) else {
            log!("catalog: skipped {} (not a Reach map variant)", path.display());
            continue;
        };
        let Some(m) = maps.iter_mut().find(|m| m.mapinfo_id == id) else {
            log!("catalog: skipped {} (base map id {id} is not a Reach multiplayer map)", path.display());
            continue;
        };
        let body = b.get(0x2F0..).unwrap_or_default().to_vec();
        if seen.contains(&body) {
            continue;
        }
        seen.push(body);
        let title = if title.starts_with('$') || title.trim().is_empty() { title_from_file(file, &m.stem) } else { title };
        let desc = if desc.is_empty() || desc.starts_with('$') { format!("A community {} map.", m.name) } else { desc };
        m.variants.push(MapVariant { path: Some(path.clone()), title, desc, source: *source });
    }
    let order = |v: &MapVariant| match v.source {
        Source::Builtin => 0,
        Source::Hopper => 1,
        _ => 2,
    };
    for m in &mut maps {
        m.variants[1..].sort_by(|a, b| (order(a), a.title.to_lowercase()).cmp(&(order(b), b.title.to_lowercase())));
    }
    Catalog { games, maps }
}

/// The catalog's map variants plus the player's saved Forge maps (read fresh: they change after a Forge session).
pub fn with_saved(c: &Catalog) -> Vec<BaseMap> {
    let mut maps = c.maps.clone();
    for s in crate::forge::list() {
        let id = std::fs::read(&s.path).ok().and_then(|b| map_variant(&b)).map(|v| v.2);
        let m = maps.iter_mut().find(|m| m.stem == s.map || Some(m.mapinfo_id) == id);
        if let Some(m) = m {
            m.variants.insert(1, MapVariant { path: Some(s.path.clone()), title: s.name.clone(), desc: "Your map, saved in Forge.".into(), source: Source::Saved });
        }
    }
    maps
}

/// `maps/info/<stem>.mapinfo`: u32 BE at +0x3C (the `levl` chunk).
fn mapinfo_id(p: &Path) -> Option<i32> {
    let b = std::fs::read(p).ok()?;
    (b.len() > 0x40 && &b[0x30..0x34] == b"levl").then(|| u32::from_be_bytes(b[0x3C..0x40].try_into().unwrap()) as i32)
}

fn utf16le(b: &[u8]) -> String {
    let u: Vec<u16> = b.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&c| c != 0).collect();
    String::from_utf16_lossy(&u).trim().to_string()
}

/// (content header at file +0x40, the chunk after `chdr`) of a BLF variant file.
fn blf<'a>(b: &'a [u8], chunk: &[u8; 4]) -> Option<(&'a [u8], &'a [u8])> {
    if b.len() < 0x40 + 0x2B0 || &b[0..4] != b"_blf" || &b[0x30..0x34] != b"chdr" {
        return None;
    }
    let next = 0x30 + u32::from_be_bytes(b[0x34..0x38].try_into().unwrap()) as usize;
    let body = b.get(next..)?;
    (body.len() > 0x10 && &body[0..4] == chunk).then_some((&b[0x40..], body))
}

/// Map variant file: (name, description, mapinfo id of its base map).
pub fn map_variant(b: &[u8]) -> Option<(String, String, i32)> {
    let (h, _) = blf(b, b"mvar")?;
    Some((utf16le(&h[0x80..0x180]), utf16le(&h[0x180..0x280]), i32::from_le_bytes(h[0x2C..0x30].try_into().unwrap())))
}

/// What a game type file holds: (name, description, engine, Megalo icon and teams bit when found).
pub fn game_variant(b: &[u8]) -> Option<(String, String, u8, Option<(u8, bool)>)> {
    let (h, body) = blf(b, b"mpvr")?;
    let (name, desc) = (utf16le(&h[0x80..0x180]), utf16le(&h[0x180..0x280]));
    let engine = h[0x2A];
    // The name and description as null-terminated UTF-16BE somewhere in the first few hundred bytes of the stream.
    let pat: Vec<u8> = name.encode_utf16().chain([0]).chain(desc.encode_utf16()).chain([0]).flat_map(|c| c.to_be_bytes()).collect();
    let bit = |i: usize| body.get(i / 8).map_or(0, |&x| (x >> (7 - i % 8)) & 1);
    let byte = |i: usize| (0..8).fold(0u8, |a, j| a << 1 | bit(i + j));
    // One byte-aligned copy of the stream per bit shift, then a plain byte search in each.
    let n = body.len().min(0x300);
    let found = (0..8)
        .filter_map(|sh| {
            let shifted: Vec<u8> = (0..n).map(|i| if sh == 0 { body[i] } else { body[i] << sh | body.get(i + 1).map_or(0, |&x| x >> (8 - sh)) }).collect();
            shifted.windows(pat.len()).position(|w| w == pat.as_slice()).map(|i| i * 8 + sh)
        })
        .min();
    let megalo = found.map(|s| {
        let e = s + pat.len() * 8;
        (byte(e), bit(e + 9) == 1)
    });
    Some((name, desc, engine, megalo))
}

fn game_type(b: &[u8], file: &str, path: PathBuf, hopper: bool) -> Option<GameType> {
    let (name, desc, engine, megalo) = game_variant(b)?;
    let f = file.to_ascii_lowercase();
    let kind = match engine {
        1 => FORGE,
        4 => FIREFIGHT,
        2 => kind_of(megalo.map(|m| m.0), &name, &f),
        _ => return None, // 3: campaign
    };
    let teams = match kind.category {
        Category::Firefight | Category::Forge => None,
        _ => megalo.map(|m| m.1).or_else(|| if f.contains("ffa") { Some(false) } else if f.contains("team") || f.contains("v4_") || f.contains("v8_") { Some(true) } else { None }),
    };
    // A multiplayer type never shows "Co-op": objective and Invasion types default to teams, the rest to FFA
    // (a community file whose header name was edited no longer matches its stream, so the teams bit is not found).
    let teams = match kind.category {
        Category::Firefight | Category::Forge => teams,
        Category::Objective | Category::Invasion => teams.or(Some(true)),
        _ => teams.or(Some(false)),
    };
    let mut title = if let Some(t) = name.strip_prefix("$hr_gvar_") { t.replace('_', " ") } else { name.clone() };
    if title.trim().is_empty() {
        title = crate::ui::menu::variant_title(file);
    }
    let title = pretty(&title);
    let desc = if desc.is_empty() || desc.starts_with('$') { kind.about.to_string() } else { desc };
    Some(GameType { file: file.to_string(), path, title, desc, kind, teams, hopper, source: if hopper { Source::Hopper } else { Source::Builtin } })
}

fn kind_of(icon: Option<u8>, name: &str, f: &str) -> Kind {
    let n = name.to_ascii_lowercase();
    if f.contains("minigame") {
        return MINIGAME;
    }
    if (n.contains("swat") || f.contains("swat")) && matches!(icon, Some(1) | None) {
        return SWAT;
    }
    match icon {
        Some(0) => CTF,
        Some(1) => SLAYER,
        Some(2) => ODDBALL,
        Some(3) => KOTH,
        Some(4) => JUGGERNAUT,
        Some(5) => TERRITORIES,
        Some(6) => ASSAULT,
        Some(7) => INFECTION,
        Some(9) => INVASION,
        Some(10) => INVASION_SLAYER,
        Some(11) => STOCKPILE,
        Some(13) | Some(14) => RACE,
        Some(15) => GRIFBALL,
        Some(17) => HEADHUNTER,
        Some(20) if n.contains("escalation") => ESCALATION,
        Some(_) => CUSTOM,
        // No icon found: guess from the file name.
        None => [
            ("ctf", CTF), ("flag", CTF), ("oddball", ODDBALL), ("koth", KOTH), ("king", KOTH), ("juggernaut", JUGGERNAUT),
            ("territories", TERRITORIES), ("assault", ASSAULT), ("bomb", ASSAULT), ("infection", INFECTION),
            ("invasion", INVASION), ("stockpile", STOCKPILE), ("race", RACE), ("rally", RACE), ("grifball", GRIFBALL),
            ("headhunter", HEADHUNTER), ("escalation", ESCALATION), ("slayer", SLAYER),
        ]
        .into_iter()
        .find(|(k, _)| f.contains(k))
        .map_or(CUSTOM, |(_, k)| k),
    }
}

/// `TU 3-PLOT TERRITORIES` → `TU 3-Plot Territories`; mixed-case titles stay as they are.
pub fn pretty(t: &str) -> String {
    if t.chars().any(|c| c.is_lowercase()) {
        return t.trim().to_string();
    }
    const KEEP: &[&str] = &["TU", "CTF", "FFA", "SWAT", "DMR", "MLG", "AR", "BTB", "HC", "CE", "CEX", "FRG", "GEB", "RHR", "2V2", "4V4", "8V8"];
    t.split_whitespace()
        .map(|w| {
            if KEEP.contains(&w) {
                return w.to_string();
            }
            let mut out = String::new();
            let mut start = true;
            for c in w.chars() {
                out.extend(if start { c.to_uppercase().collect::<Vec<_>>() } else { c.to_lowercase().collect() });
                start = !c.is_alphanumeric() && c != '\'';
            }
            out
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// `hr_forgeWorld_theCage.mvar` → `The Cage` (titles that are only localisation keys in the file).
fn title_from_file(file: &str, map_stem: &str) -> String {
    let mut s = file.trim_end_matches(".mvar").to_string();
    for p in ["hr_", "forgeworld_", "forgeWorld_", "forge_halo_", "coastline_", map_stem] {
        if let Some(r) = s.strip_prefix(p) {
            s = r.trim_start_matches('_').to_string();
        }
    }
    let s = s.trim_end_matches("_031").trim_end_matches("_default");
    let mut out = String::new();
    for (i, w) in s.split('_').filter(|w| !w.is_empty()).enumerate() {
        if i > 0 {
            out.push(' ');
        }
        for (j, c) in w.chars().enumerate() {
            if j == 0 {
                out.extend(c.to_uppercase());
            } else {
                if c.is_uppercase() {
                    out.push(' ');
                }
                out.push(c);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mcc() -> Option<PathBuf> {
        let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../work/mcc");
        let p = if p.join("haloreach").is_dir() { p } else { PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../../work/mcc") };
        p.join("haloreach").is_dir().then_some(p)
    }

    #[test]
    fn titles() {
        assert_eq!(pretty("TU 3-PLOT TERRITORIES"), "TU 3-Plot Territories");
        assert_eq!(pretty("CAPTURE THE FLAG"), "Capture The Flag");
        assert_eq!(pretty("Jump Rope"), "Jump Rope");
        assert_eq!(title_from_file("hr_forgeWorld_theCage.mvar", "forge_halo"), "The Cage");
        assert_eq!(title_from_file("hr_forgeWorld_pinnacle.mvar", "forge_halo"), "Pinnacle");
    }

    #[test]
    fn real_files() {
        let Some(mcc) = mcc() else { return eprintln!("no work/mcc: skipped") };
        let hr = mcc.join("haloreach");
        let b = std::fs::read(hr.join("map_variants/hr_forgeWorld_pinnacle.mvar")).unwrap();
        let (t, _, id) = map_variant(&b).unwrap();
        assert_eq!((t.as_str(), id), ("$HReach_MPMap_R-24_Title", 3006));
        let b = std::fs::read(hr.join("hopper_map_variants/boneyard_graveyard_031.mvar")).unwrap();
        assert_eq!(map_variant(&b).unwrap().0, "Graveyard");

        let g = |dir: &str, f: &str| {
            let p = hr.join(dir).join(f);
            game_type(&std::fs::read(&p).unwrap(), f, p, dir.starts_with("hopper")).unwrap()
        };
        let s = g("game_variants", "slayer_054.bin");
        assert_eq!((s.title.as_str(), s.kind, s.teams), ("Slayer", SLAYER, Some(false)));
        let t = g("hopper_game_variants", "slayer_team_054.bin");
        assert_eq!((t.kind, t.teams), (SLAYER, Some(true)));
        assert_eq!(g("game_variants", "ctf_054.bin").kind, CTF);
        assert_eq!(g("game_variants", "infection_054.bin").kind.category, Category::Infection);
        assert_eq!(g("game_variants", "3nvasion_054.bin").kind, INVASION);
        assert_eq!(g("game_variants", "00_basic_editing_054.bin").kind, FORGE);
        assert_eq!(g("game_variants", "hr_ff_standard_1set.bin").kind, FIREFIGHT);
        assert_eq!(g("game_variants", "SWAT_054.bin").kind, SWAT);
        assert_eq!(g("game_variants", "race_054.bin").kind, RACE);
        let gn = g("hopper_game_variants", "slayer_team_dino_blasters_054.bin");
        assert_eq!(gn.title, "Dino Blasters");

        let c = scan(&mcc);
        let fw = c.maps.iter().find(|m| m.stem == "forge_halo").unwrap();
        assert_eq!(fw.variants[0].title, "Forge World (blank)");
        let names: Vec<&str> = fw.variants.iter().filter(|v| v.source == Source::Builtin).map(|v| v.title.as_str()).collect();
        assert_eq!(names, ["Asylum", "Hemorrhage", "Paradiso", "Pinnacle", "The Cage"]);
        assert!(fw.variants.len() > 100, "Forge World has {} variants", fw.variants.len());
        assert!(c.games.len() > 400);
    }

    #[test]
    fn community_folders() {
        let Some(mcc) = mcc() else { return eprintln!("no work/mcc: skipped") };
        let hr = mcc.join("haloreach");
        let t = std::env::temp_dir().join(format!("noble-community-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&t);
        let (custom, ws) = (t.join("custom"), t.join("steamapps/workshop/content/976730"));
        ensure_custom_dir(&custom);
        assert!(custom.join("README.txt").is_file() && custom.join("maps").is_dir());
        std::fs::create_dir_all(ws.join("12345/sub")).unwrap();
        std::fs::create_dir_all(ws.join("777")).unwrap();
        let cp = |from: &str, to: PathBuf| {
            std::fs::create_dir_all(to.parent().unwrap()).unwrap();
            std::fs::copy(hr.join(from), to).unwrap();
        };
        cp("hopper_map_variants/boneyard_graveyard_031.mvar", custom.join("maps/my_graveyard.mvar"));
        let mut gy = std::fs::read(custom.join("maps/my_graveyard.mvar")).unwrap();
        gy[0x400] ^= 1; // not a byte copy of the shipped map
        std::fs::write(custom.join("maps/my_graveyard.mvar"), &gy).unwrap();
        cp("hopper_game_variants/slayer_team_dino_blasters_054.bin", custom.join("gametypes/dino.bin"));
        let mut dino = std::fs::read(custom.join("gametypes/dino.bin")).unwrap();
        dino[0x40 + 0x80..0x40 + 0x88].copy_from_slice(&"Dino X ".encode_utf16().take(4).flat_map(|c| c.to_le_bytes()).collect::<Vec<u8>>()); // not a byte copy of the shipped file
        std::fs::write(custom.join("gametypes/dino.bin"), &dino).unwrap();
        cp("map_variants/hr_forgeWorld_pinnacle.mvar", ws.join("12345/pinnacle_copy.mvar"));
        cp("game_variants/ctf_054.bin", ws.join("12345/sub/ctf_workshop.bin"));
        cp("game_variants/ctf_054.bin", custom.join("ctf_loose.bin")); // byte copy of a shipped file: not listed again
        let mut ctf = std::fs::read(ws.join("12345/sub/ctf_workshop.bin")).unwrap();
        ctf[0x40 + 0x80..0x40 + 0x84].copy_from_slice(&"WS".encode_utf16().flat_map(|c| c.to_le_bytes()).collect::<Vec<u8>>());
        std::fs::write(ws.join("12345/sub/ctf_workshop.bin"), &ctf).unwrap();
        std::fs::write(ws.join("12345/junk.mvar"), b"not a blf file at all").unwrap();
        let mut h3 = std::fs::read(hr.join("game_variants/ctf_054.bin")).unwrap();
        h3[0..4].copy_from_slice(b"BLAM"); // another game's / damaged header
        std::fs::write(ws.join("777/halo3.bin"), &h3).unwrap();
        let mut ce = std::fs::read(hr.join("map_variants/hr_forgeWorld_pinnacle.mvar")).unwrap();
        ce[0x40 + 0x2C..0x40 + 0x30].copy_from_slice(&12345i32.to_le_bytes()); // unknown base map
        std::fs::write(ws.join("777/other_map.mvar"), &ce).unwrap();
        std::fs::write(ws.join("777/readme.txt"), b"x").unwrap();

        let c = scan_with(&mcc, &Roots { custom: Some(custom.clone()), workshop: vec![ws.clone()] });
        let com: Vec<(&str, Source)> = c.games.iter().filter(|g| g.community()).map(|g| (g.file.as_str(), g.source)).collect();
        assert_eq!(com.len(), 2, "{com:?}");
        assert!(com.contains(&("dino.bin", Source::Custom)));
        assert!(c.games.iter().any(|g| g.source == Source::Workshop && g.kind == CTF));
        let fw = c.maps.iter().find(|m| m.stem == "forge_halo").unwrap();
        let cm: Vec<_> = fw.variants.iter().filter(|v| v.source.community()).collect();
        // Pinnacle is a shipped file: the same content is not listed twice; Graveyard is on another map.
        assert!(cm.is_empty(), "{cm:?}");
        let gy = c.maps.iter().flat_map(|m| &m.variants).find(|v| v.source == Source::Custom).expect("custom graveyard");
        assert_eq!(gy.title, "Graveyard");
        assert!(c.maps.iter().flat_map(|m| &m.variants).all(|v| v.path.as_ref().is_none_or(|p| !p.ends_with("junk.mvar") && !p.ends_with("other_map.mvar"))));
        assert!(c.games.iter().all(|g| !g.file.contains("halo3")));

        // Steam library discovery from libraryfolders.vdf.
        let vdf = t.join("libraryfolders.vdf");
        std::fs::write(&vdf, "\"libraryfolders\"\n{\n\t\"0\"\n\t{\n\t\t\"path\"\t\t\"/mnt/a b\"\n\t\t\"label\"\t\t\"\"\n\t}\n\t\"1\"\n\t{\n\t\t\"path\"\t\t\"D:\\\\SteamLibrary\"\n\t}\n}\n").unwrap();
        assert_eq!(vdf_libraries(&vdf), [PathBuf::from("/mnt/a b"), PathBuf::from("D:\\SteamLibrary")]);
        let lib = t.join("lib/steamapps/common/Halo The Master Chief Collection");
        std::fs::create_dir_all(t.join("lib/steamapps/workshop/content/976730")).unwrap();
        std::fs::create_dir_all(&lib).unwrap();
        assert!(workshop_dirs(&lib).iter().any(|w| w.starts_with(t.join("lib"))));
        let _ = std::fs::remove_dir_all(&t);
    }
}
