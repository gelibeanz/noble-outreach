//! Per-player game statistics from the engine's game results (kills, deaths, assists, medals, kills per weapon,
//! Firefight score) and the end-of-game carnage report. The host (listen or dedicated) is authoritative: when the
//! engine finalises its game results it builds the report and broadcasts it to every peer, so every client shows
//! the same table (ui/carnage.rs). A client without a report from its host (older host, offline game) uses its own.
//! Layout and addresses: notes/frontend.md "Game statistics". Wire format: notes/networking.md "Carnage report".
//! Datagrams start with `RCRN`, so they never reach the engine queues.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::log;

// ---- engine layout (frozen DLL) -----------------------------------------------------------------------------------

/// Game-results player entries: 16 × 0xD8, u8 in use at +0, u64 xuid at +7.
const RVA_RESULT_PLAYERS: usize = 0x28c82a8;
/// Statistics: 16 × 0x708, entry k at +k*0x708 (same order as the player entries).
const RVA_RESULT_STATS: usize = 0x28c9200;
const STATS_SIZE: usize = 0x708;
/// u8, set by BuildGameResultsForHost (0x1800f92e8) when the game is over and its results are final.
const RVA_RESULTS_FINAL: usize = 0x28b83b1;
/// Statistic words (s16, top bit masked off by the engine's own export): table 0x180b016f0.
pub const STAT_SCORE: usize = 7; // in-game-total-score
pub const STAT_KILLS: usize = 8;
pub const STAT_ASSISTS: usize = 9;
pub const STAT_DEATHS: usize = 10;
pub const STAT_BETRAYALS: usize = 11;
pub const STAT_SUICIDES: usize = 12;
pub const STAT_BEST_SPREE: usize = 13;
/// Medal counts: 145 s16 from word 162 (byte +0x144), indexed by the medal's index in the globals' medal block.
const MEDALS_AT: usize = 0x144;
pub const MEDALS: usize = 145;
/// Kills per damage reporting type: 84 entries of 12 bytes from +0x300 (u8 used, then s16 kills, deaths,
/// betrayals, suicides, headshots).
const WEAPONS_AT: usize = 0x300;
pub const WEAPONS: usize = 84;
/// Engine functions used once per game on the game thread to name the medal indices: string id of a name
/// (`StringIdFromName` 0x18009db08) and medal index of a string id (`MedalIndexFromStringId` 0x180100bac).
const RVA_STRING_ID: usize = 0x9db08;
const RVA_MEDAL_INDEX: usize = 0x100bac;
/// The engine's own {name, MCC stat id} list of named medals (0x74 entries of 16 bytes).
const RVA_MEDAL_NAMES: usize = 0x991410;
const MEDAL_NAME_COUNT: usize = 0x74;

/// Damage reporting types 0..83 (the tag enum at 0x1809c0738, which the weapon statistics follow).
pub const WEAPON_NAMES: [&str; WEAPONS] = [
    "Guardians", "Scripting", "AI Suicide", "Magnum", "Assault Rifle", "DMR", "Shotgun", "Sniper Rifle",
    "Rocket Launcher", "Spartan Laser", "Frag Grenade", "Grenade Launcher", "DMR", "Magnum", "Golf Club",
    "Machine Gun", "Target Locator", "Plasma Pistol", "Needler", "Plasma Rifle", "Plasma Repeater", "Needle Rifle",
    "Spiker", "Plasma Launcher", "Gravity Hammer", "Energy Blade", "Plasma Grenade", "Concussion Rifle",
    "Energy Sword", "Focus Rifle", "Fuel Rod Gun", "Plasma Turret", "Revenant", "Revenant Gunner", "Wraith Gunner",
    "Shade", "Shade Flak", "Shade Anti-Air", "Warthog", "Warthog Gunner", "Scorpion", "Falcon", "Falcon Gunner",
    "Forklift", "Scorpion Gunner", "Falcon Grenadier", "Falcon Grenadier", "Pickup", "Semi", "Cargo Truck",
    "Electric Cart", "Military Truck", "Falling", "Collision", "Explosion", "Birthday Party", "Flag", "Bomb",
    "Bomb Explosion", "Ball", "Teleporter", "Transfer Damage", "Armor Lock", "Airstrike", "Turret",
    "Plasma Cannon", "Plasma Mortar", "Plasma Turret", "Shade Turret", "Machine Gun Turret", "Plasma Turret",
    "Gauss Turret", "Rocket Turret", "Wraith Turret", "Excavator", "SMG", "Battle Rifle", "Carbine", "Beam Rifle",
    "Flak Cannon", "Missile Launcher", "Brute Shot", "Flamethrower", "Sentinel Beam",
];

/// Display names for the engine's medal keys; anything else is shown from its key in title case.
const MEDAL_TITLES: &[(&str, &str)] = &[
    ("medal_extermination", "Extermination"), ("medal_perfection", "Perfection"),
    ("medal_multikill_2", "Double Kill"), ("medal_multikill_3", "Triple Kill"), ("medal_multikill_4", "Overkill"),
    ("medal_multikill_5", "Killtacular"), ("medal_multikill_6", "Killtrocity"), ("medal_multikill_7", "Killimanjaro"),
    ("medal_multikill_8", "Killtastrophe"), ("medal_multikill_9", "Killpocalypse"), ("medal_multikill_10", "Killionaire"),
    ("medal_kill_spree_5", "Killing Spree"), ("medal_kill_spree_10", "Killing Frenzy"),
    ("medal_kill_spree_15", "Running Riot"), ("medal_kill_spree_20", "Rampage"), ("medal_kill_spree_25", "Untouchable"),
    ("medal_kill_spree_30", "Invincible"), ("medal_kill_spree_35", "Inconceivable"),
    ("medal_kill_spree_40", "Unfrigginbelievable"),
    ("medal_sniper_5", "Sniper Spree"), ("medal_sniper_10", "Sharpshooter"), ("medal_sniper_15", "Be the Bullet"),
    ("medal_shotgun_5", "Shotgun Spree"), ("medal_shotgun_10", "Open Season"), ("medal_shotgun_15", "Buck Wild"),
    ("medal_splatter_5", "Splatter Spree"), ("medal_splatter_10", "Vehicular Manslaughter"),
    ("medal_splatter_15", "Sunday Driver"), ("medal_sword_5", "Sword Spree"), ("medal_sword_10", "Slice 'n Dice"),
    ("medal_sword_15", "Cutting Crew"), ("medal_juggernaut_5", "Juggernaut Spree"), ("medal_juggernaut_10", "Unstoppable"),
    ("medal_juggernaut_15", "Walking Tank"), ("medal_zombie_5", "Infection Spree"), ("medal_zombie_10", "Mmmm... Brains"),
    ("medal_zombie_15", "Zombie Apocalypse"), ("medal_survivor_5", "Hell's Janitor"), ("medal_survivor_10", "Hell's Survivor"),
    ("medal_survivor_15", "Hell's Last Stand"), ("medal_koth_5", "Hail to the King"),
    ("sticky_grenade_5x", "Sticky Spree"), ("sticky_grenade_10x", "Sticky Fingers"), ("medal_sticky_15", "Corrected"),
    ("hammer_kill_5x", "Hammer Spree"), ("hammer_kill_10x", "Dream Crusher"), ("medal_hammerkill_15", "Wrecking Crew"),
    ("assist_5", "Assist Spree"), ("assist_10", "Sidekick"), ("assist_15", "Second Gunman"),
    ("laser_5", "Laser Spree"), ("laser_10", "Red Menace"), ("laser_15", "Sunburst"),
    ("wheelman_5", "Wheelman Spree"), ("wheelman_10", "Road Hog"), ("wheelman_15", "Road Rage"),
    ("wingman_5x", "Wingman Spree"), ("wingman_10x", "Broseidon"), ("wingman_15x", "Lord of Wingmen"),
    ("medal_shotgun_kill_sword", "Bulltrue"), ("medal_vehicle_impact_kill", "Splatter"),
    ("medal_vehicle_hijack", "Hijack"), ("medal_aircraft_hijack", "Skyjack"),
    ("medal_deadplayer_kill_player", "Kill from the Grave"), ("medal_player_kill_spreeplayer", "Killjoy"),
    ("medal_spartanlaser_kill", "Laser Kill"), ("medal_stickygrenade_kill", "Stick"), ("medal_sniper_kill", "Sniper Kill"),
    ("medal_bashbehind_kill", "Beat Down"), ("medal_bash_kill", "Pummel"), ("medal_driver_assist_gunner", "Wheelman"),
    ("medal_assault_bomb_planted", "Bomb Planted"), ("medal_ctf_flag_captured", "Flag Score"),
    ("medal_oddball_carrier_kill_player", "Ball Carrier Kill"), ("medal_ctf_flag_carrier_kill_player", "Flag Carrier Kill"),
    ("medal_oddball_player_kill_oddball", "Killed Ball Carrier"), ("medal_assault_player_kill_carrier", "Killed Bomb Carrier"),
    ("medal_ctf_flag_player_kill_carrier", "Killed Flag Carrier"), ("medal_juggernaut_player_kill_juggernaut", "Juggernaut Kill"),
    ("vip_player_kill_vip", "VIP Kill"), ("medal_infection_survive", "Last Man Standing"), ("headshot_kill", "Headshot"),
    ("supercombine_kill", "Supercombine"), ("emp_assist", "EMP Assist"), ("survival_hero", "Hero"), ("assist", "Assist"),
    ("first_blood", "First Strike"), ("sprinting_kill", "Sprint Kill"), ("pull_kill", "Pull"),
    ("ff_medal_kill_spree_10", "Firefight Spree 10"), ("ff_medal_kill_spree_20", "Firefight Spree 20"),
    ("ff_medal_kill_spree_30", "Firefight Spree 30"), ("ff_medal_kill_spree_40", "Firefight Spree 40"),
    ("ff_medal_kill_spree_50", "Firefight Spree 50"), ("ff_medal_kill_spree_100", "Firefight Spree 100"),
    ("ff_medal_kill_spree_500", "Firefight Spree 500"), ("ff_medal_kill_spree_1000", "Firefight Spree 1000"),
    ("medal_flame_kill", "Firestarter"),
];

/// Medal key per medal index (filled once per game on the game thread), None = not named by the engine.
static MEDAL_KEYS: Mutex<Vec<Option<String>>> = Mutex::new(Vec::new());

/// Display name of medal index `i` ("Medal 37" when the engine has no name for it).
pub fn medal_name(i: usize) -> String {
    let key = MEDAL_KEYS.lock().unwrap_or_else(|e| e.into_inner()).get(i).cloned().flatten();
    match key {
        Some(k) => title_of(&k),
        None => format!("Medal {i}"),
    }
}

fn title_of(key: &str) -> String {
    if let Some(&(_, t)) = MEDAL_TITLES.iter().find(|(k, _)| *k == key) {
        return t.to_string();
    }
    let k = key.strip_prefix("medal_").unwrap_or(key);
    k.split('_').filter(|w| !w.is_empty()).map(|w| {
        let mut c = w.chars();
        c.next().map(|f| f.to_uppercase().chain(c).collect::<String>()).unwrap_or_default()
    }).collect::<Vec<_>>().join(" ")
}

/// Game thread, once a map is loaded: map every named medal to its index (the medal block lives in the globals
/// tag, so this only reads tags). Once per game.
pub unsafe fn init_medal_names(base: usize) {
    let string_id: unsafe extern "system" fn(*const u8) -> u32 = std::mem::transmute(base + RVA_STRING_ID);
    let medal_index: unsafe extern "system" fn(u32) -> i32 = std::mem::transmute(base + RVA_MEDAL_INDEX);
    let mut keys = vec![None; MEDALS];
    let mut named = 0;
    for k in 0..MEDAL_NAME_COUNT {
        let p = ((base + RVA_MEDAL_NAMES + k * 16) as *const *const u8).read();
        if p.is_null() {
            continue;
        }
        let sid = string_id(p);
        if sid == u32::MAX {
            continue;
        }
        let i = medal_index(sid);
        if (0..MEDALS as i32).contains(&i) {
            keys[i as usize] = Some(std::ffi::CStr::from_ptr(p as _).to_string_lossy().into_owned());
            named += 1;
        }
    }
    log!("stats: {named} medals named by the engine");
    *MEDAL_KEYS.lock().unwrap_or_else(|e| e.into_inner()) = keys;
}

/// Statistics block of the game-results entry with `xuid` (looked up as 0x1800fed6c does).
pub fn block_of(base: usize, xuid: u64) -> Option<usize> {
    (0..16).find(|k| unsafe {
        let e = base + RVA_RESULT_PLAYERS + k * 0xd8;
        (e as *const u8).read_volatile() != 0 && ((e + 7) as *const u64).read_unaligned() == xuid
    }).map(|k| base + RVA_RESULT_STATS + k * STATS_SIZE)
}

/// Statistic word `n` of a block (the engine masks the top bit when it exports them).
pub fn word(block: usize, n: usize) -> i32 {
    unsafe { (((block + 2 * n) as *const u16).read_volatile() & 0x7fff) as i32 }
}

/// Medals of a block, most first: (medal index, count).
pub fn medals(block: usize) -> Vec<(u8, u16)> {
    let mut v: Vec<(u8, u16)> = (0..MEDALS)
        .map(|i| (i as u8, unsafe { ((block + MEDALS_AT + 2 * i) as *const u16).read_volatile() & 0x7fff }))
        .filter(|m| m.1 > 0)
        .collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    v
}

/// Weapon (damage reporting type) with the most kills: (index, kills, headshots).
pub fn best_weapon(block: usize) -> Option<(u8, u16, u16)> {
    (0..WEAPONS)
        .map(|i| unsafe {
            let e = block + WEAPONS_AT + i * 12;
            let rd = |o: usize| ((e + o) as *const u16).read_unaligned() & 0x7fff;
            (i as u8, rd(2), rd(10))
        })
        .filter(|w| w.1 > 0)
        .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)))
}

/// Firefight score of `xuid`, as `campaign_metagame_get_player_score` (0x1800a1718) computes it: the metagame
/// player record (TLS +0x190 → 8 × 0x2C, xuid at +0, score i32 at +8) minus the start-of-game record (TLS +0x488 →
/// +4 + 8 × 0x20, xuid at +0, i32 at +0x10), at least 0. The engine scales it by the time multiplier only when
/// metagame +0x19E bit 1 is set (campaign scoring), which Firefight does not use.
pub fn firefight_score(tls_block: usize, xuid: u64) -> Option<i32> {
    unsafe {
        let rd = |a: usize| (a as *const usize).read_volatile();
        let meta = rd(tls_block + 0x190);
        if meta == 0 {
            return None;
        }
        let rec = (0..8).map(|i| meta + i * 0x2c).find(|&r| (r as *const u64).read_unaligned() == xuid)?;
        let score = ((rec + 8) as *const i32).read_unaligned();
        let start = rd(tls_block + 0x488);
        let less = if start == 0 {
            0
        } else {
            (0..8).map(|i| start + 4 + i * 0x20).find(|&r| (r as *const u64).read_unaligned() == xuid)
                .map_or(0, |r| ((r + 0x10) as *const i32).read_unaligned())
        };
        Some((score - less).max(0))
    }
}

/// Game mode of the running game (game options TLS +0x48 → +0x10).
pub fn options_mode(tls_block: usize) -> Option<u8> {
    unsafe {
        let o = (tls_block as *const usize).add(0x48 / 8).read_volatile();
        (o != 0).then(|| ((o + 0x10) as *const u8).read_volatile())
    }
}

/// Engine game-options mode byte of a Firefight game (multiplayer is 3; seen live).
pub const MODE_SURVIVAL: u8 = 2;

// ---- carnage report -------------------------------------------------------------------------------------------

pub const MAGIC: [u8; 4] = *b"RCRN";
const VERSION: u8 = 1;
const KIND_REPORT: u8 = 1;
const MAX_PLAYERS: usize = 16;
const MAX_MEDALS: usize = 6;
const NAME_MAX: usize = 24;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Line {
    pub xuid: u64,
    pub name: String,
    /// -1 in free-for-all.
    pub team: i8,
    /// Slayer/objective score (Firefight: its score).
    pub score: i32,
    pub kills: i16,
    pub deaths: i16,
    pub assists: i16,
    pub betrayals: i16,
    pub suicides: i16,
    pub best_spree: i16,
    /// Top medals (index, count), most first.
    pub medals: Vec<(u8, u16)>,
    pub medal_total: u16,
    /// Weapon with the most kills (damage reporting type, kills).
    pub weapon: Option<(u8, u16)>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Report {
    /// Per host process: game number (clients drop repeats).
    pub id: u16,
    pub teams: bool,
    pub firefight: bool,
    pub team_scores: [i32; 8],
    /// Best first (as the scoreboard sorts).
    pub players: Vec<Line>,
}

fn put_str(o: &mut Vec<u8>, s: &str) {
    let mut n = s.len().min(NAME_MAX);
    while !s.is_char_boundary(n) {
        n -= 1;
    }
    o.push(n as u8);
    o.extend_from_slice(&s.as_bytes()[..n]);
}

pub fn encode(r: &Report) -> Vec<u8> {
    let players = &r.players[..r.players.len().min(MAX_PLAYERS)];
    let mut o = Vec::with_capacity(48 + players.len() * 80);
    o.extend_from_slice(&MAGIC);
    o.extend_from_slice(&[VERSION, KIND_REPORT, players.len() as u8, r.teams as u8 | (r.firefight as u8) << 1]);
    o.extend_from_slice(&r.id.to_le_bytes());
    for t in r.team_scores {
        o.extend_from_slice(&(t.clamp(i16::MIN as i32, i16::MAX as i32) as i16).to_le_bytes());
    }
    for p in players {
        o.extend_from_slice(&p.xuid.to_le_bytes());
        put_str(&mut o, &p.name);
        o.push(p.team as u8);
        o.extend_from_slice(&p.score.to_le_bytes());
        for v in [p.kills, p.deaths, p.assists, p.betrayals, p.suicides, p.best_spree] {
            o.extend_from_slice(&v.to_le_bytes());
        }
        let (wi, wk) = p.weapon.unwrap_or((0xff, 0));
        o.push(wi);
        o.extend_from_slice(&wk.to_le_bytes());
        o.extend_from_slice(&p.medal_total.to_le_bytes());
        let m = &p.medals[..p.medals.len().min(MAX_MEDALS)];
        o.push(m.len() as u8);
        for (i, c) in m {
            o.push(*i);
            o.extend_from_slice(&c.to_le_bytes());
        }
    }
    o
}

pub fn decode(p: &[u8]) -> Option<Report> {
    if p.len() < 26 || p[..4] != MAGIC || p[4] != VERSION || p[5] != KIND_REPORT {
        return None;
    }
    let n = (p[6] as usize).min(MAX_PLAYERS);
    let mut at = 8;
    let mut take = |k: usize| -> Option<&[u8]> {
        let s = p.get(at..at + k)?;
        at += k;
        Some(s)
    };
    let id = u16::from_le_bytes(take(2)?.try_into().ok()?);
    let mut team_scores = [0i32; 8];
    for t in team_scores.iter_mut() {
        *t = i16::from_le_bytes(take(2)?.try_into().ok()?) as i32;
    }
    let mut players = Vec::with_capacity(n);
    for _ in 0..n {
        let xuid = u64::from_le_bytes(take(8)?.try_into().ok()?);
        let len = take(1)?[0] as usize;
        let name = String::from_utf8_lossy(take(len)?).into_owned();
        let team = take(1)?[0] as i8;
        let score = i32::from_le_bytes(take(4)?.try_into().ok()?);
        let mut v = [0i16; 6];
        for x in v.iter_mut() {
            *x = i16::from_le_bytes(take(2)?.try_into().ok()?);
        }
        let wi = take(1)?[0];
        let wk = u16::from_le_bytes(take(2)?.try_into().ok()?);
        let medal_total = u16::from_le_bytes(take(2)?.try_into().ok()?);
        let nm = (take(1)?[0] as usize).min(MAX_MEDALS);
        let mut medals = Vec::with_capacity(nm);
        for _ in 0..nm {
            let i = take(1)?[0];
            medals.push((i, u16::from_le_bytes(take(2)?.try_into().ok()?)));
        }
        players.push(Line {
            xuid, name, team, score, kills: v[0], deaths: v[1], assists: v[2], betrayals: v[3], suicides: v[4], best_spree: v[5],
            medals, medal_total, weapon: (wi != 0xff).then_some((wi, wk)),
        });
    }
    Some(Report { id, teams: p[7] & 1 != 0, firefight: p[7] & 2 != 0, team_scores, players })
}

/// Build the report from this machine's live data (scoreboard rows for names, teams and scores; game results for
/// the rest).
pub fn snapshot(id: u16) -> Report {
    let b = crate::ui::scoreboard::board();
    let base = crate::crash::ENGINE_BASE.load(Ordering::Relaxed);
    let players = b.rows.iter().map(|r| {
        let blk = if base != 0 { block_of(base, r.xuid) } else { None };
        let w = |n: usize| blk.map_or(0, |a| word(a, n)) as i16;
        let medals = blk.map(medals).unwrap_or_default();
        Line {
            xuid: r.xuid,
            name: r.name.clone(),
            team: r.team,
            score: r.score,
            kills: r.kills as i16,
            deaths: r.deaths as i16,
            assists: r.assists as i16,
            betrayals: w(STAT_BETRAYALS),
            suicides: w(STAT_SUICIDES),
            best_spree: w(STAT_BEST_SPREE),
            medal_total: medals.iter().map(|m| m.1 as u32).sum::<u32>().min(u16::MAX as u32) as u16,
            medals: medals.into_iter().take(MAX_MEDALS).collect(),
            weapon: blk.and_then(best_weapon).map(|(i, k, _)| (i, k)),
        }
    }).collect();
    Report { id, teams: b.teams, firefight: b.firefight, team_scores: b.team_scores, players }
}

/// The report the UI shows, when it arrived, and whether it came from the host.
static REPORT: Mutex<Option<(Report, Instant, bool)>> = Mutex::new(None);
/// Host: the report being sent (sends left, next send).
static SENDING: Mutex<Option<(Vec<u8>, u32, Instant)>> = Mutex::new(None);
static GAME_ID: AtomicU32 = AtomicU32::new(0);
/// Last seen value of the results-final flag (0/1), 2 = unknown.
static LAST_FINAL: AtomicU32 = AtomicU32::new(2);
static LAST_TICK: AtomicUsize = AtomicUsize::new(0);

/// Between two games in one process (seamless.rs).
pub fn reset() {
    *REPORT.lock().unwrap_or_else(|e| e.into_inner()) = None;
    *SENDING.lock().unwrap_or_else(|e| e.into_inner()) = None;
    LAST_FINAL.store(2, Ordering::Relaxed);
    MEDAL_KEYS.lock().unwrap_or_else(|e| e.into_inner()).clear();
}

/// The latest report and how long ago it arrived.
pub fn report() -> Option<(Report, Duration)> {
    REPORT.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map(|(r, at, _)| (r.clone(), at.elapsed()))
}

fn epoch_ms() -> usize {
    static T0: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    T0.get_or_init(Instant::now).elapsed().as_millis() as usize
}

/// Render thread (every frame) and the host's ping thread (every second): notice the game's results becoming
/// final, build the report, and on a host send it (5 times, 1 s apart; late joiners of the post-game get it from
/// the repeats). Cheap: does nothing more than once every 250 ms.
pub fn tick() {
    let now = epoch_ms();
    let last = LAST_TICK.load(Ordering::Relaxed);
    if now.saturating_sub(last) < 250 || LAST_TICK.compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed).is_err() {
        return;
    }
    let base = crate::crash::ENGINE_BASE.load(Ordering::Relaxed);
    if base == 0 {
        return;
    }
    let fin = unsafe { ((base + RVA_RESULTS_FINAL) as *const u8).read_volatile() != 0 } as u32;
    let prev = LAST_FINAL.swap(fin, Ordering::Relaxed);
    let host = crate::ping::is_host();
    if fin == 1 && prev == 0 {
        let id = (GAME_ID.fetch_add(1, Ordering::Relaxed) + 1) as u16;
        let r = snapshot(id);
        log_report("game over, results final", &r);
        let mut g = REPORT.lock().unwrap_or_else(|e| e.into_inner());
        // A client keeps its host's report if one already arrived for this game.
        if host || !g.as_ref().is_some_and(|(_, at, from_host)| *from_host && at.elapsed() < Duration::from_secs(60)) {
            *g = Some((r.clone(), Instant::now(), false));
        }
        drop(g);
        if host {
            *SENDING.lock().unwrap_or_else(|e| e.into_inner()) = Some((encode(&r), 5, Instant::now()));
        }
    } else if fin == 0 && prev == 1 {
        log!("stats: results no longer final (next game)");
    }
    let mut s = SENDING.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((pkt, left, at)) = s.as_mut() {
        if Instant::now() >= *at {
            let peers = crate::net::active_peer_addrs(Duration::from_secs(8));
            for a in &peers {
                crate::net::send_raw(*a, pkt);
            }
            if *left == 5 {
                log!("stats: carnage report sent to {} peer(s) ({} bytes)", peers.len(), pkt.len());
            }
            *left -= 1;
            *at = Instant::now() + Duration::from_secs(1);
            if *left == 0 {
                *s = None;
            }
        }
    }
}

fn log_report(what: &str, r: &Report) {
    let list: Vec<String> = r.players.iter().map(|p| {
        let medals: Vec<String> = p.medals.iter().map(|(i, c)| format!("{} x{c}", medal_name(*i as usize))).collect();
        let w = p.weapon.map_or("-".into(), |(i, k)| format!("{} ({k})", WEAPON_NAMES.get(i as usize).unwrap_or(&"?")));
        format!("{} (team {}) score {} K{} D{} A{} betrayals {} suicides {} spree {} weapon {w} medals {} [{}]",
            p.name, p.team, p.score, p.kills, p.deaths, p.assists, p.betrayals, p.suicides, p.best_spree, p.medal_total, medals.join(", "))
    }).collect();
    let teams = if r.teams { format!(", team scores {:?}", &r.team_scores[..4]) } else { String::new() };
    log!("stats: {what}: report {}{}{teams}: {}", r.id, if r.firefight { " (firefight)" } else { "" }, list.join("; "));
}

/// net.rs receive thread: true when `pkt` is a carnage datagram (handled here, never queued for the engine).
pub fn handle(pkt: &[u8], from: SocketAddr) -> bool {
    if pkt.len() < 4 || pkt[..4] != MAGIC {
        return false;
    }
    if crate::ping::is_host() || crate::net::host_addr() != Some(from) {
        return true;
    }
    let Some(r) = decode(pkt) else { return true };
    let mut g = REPORT.lock().unwrap_or_else(|e| e.into_inner());
    if g.as_ref().is_some_and(|(o, _, from_host)| *from_host && o.id == r.id && o.players == r.players) {
        return true;
    }
    log_report("carnage report from the host", &r);
    *g = Some((r, Instant::now(), true));
    true
}

/// `REACH_CARNAGE_FAKE`: a made-up team game for layout captures (also names the medals it uses).
pub fn sample_report() -> Report {
    {
        let mut k = MEDAL_KEYS.lock().unwrap_or_else(|e| e.into_inner());
        if k.is_empty() {
            *k = vec![None; MEDALS];
            for (i, key) in [(0, "medal_multikill_2"), (1, "medal_kill_spree_5"), (2, "headshot_kill"), (3, "assist"),
                (4, "first_blood"), (5, "medal_multikill_3"), (6, "revenge"), (7, "medal_bashbehind_kill")] {
                k[i] = Some(key.into());
            }
        }
    }
    let me = crate::host::LOCAL_XUID.load(Ordering::Relaxed);
    let l = |x: u64, n: &str, team: i8, s: i32, k: i16, d: i16, a: i16, w: u8, m: &[(u8, u16)]| Line {
        xuid: x, name: n.into(), team, score: s, kills: k, deaths: d, assists: a, betrayals: 0, suicides: 0, best_spree: k.min(7),
        medals: m.to_vec(), medal_total: m.iter().map(|x| x.1).sum(), weapon: Some((w, (k as u16 * 2 / 3).max(1))),
    };
    Report {
        id: 0,
        teams: true,
        firefight: false,
        team_scores: [50, 43, 0, 0, 0, 0, 0, 0],
        players: vec![
            l(me, "Noble Six", 0, 19, 19, 8, 6, 5, &[(0, 3), (2, 7), (1, 1), (4, 1)]),
            l(2, "Kat", 0, 17, 17, 10, 4, 4, &[(2, 5), (0, 2), (3, 4)]),
            l(3, "Jorge", 0, 14, 14, 12, 7, 15, &[(3, 7), (7, 2)]),
            l(4, "Emile", 1, 16, 16, 13, 3, 6, &[(5, 1), (0, 2), (6, 3), (1, 1)]),
            l(5, "Jun", 1, 15, 15, 11, 5, 7, &[(2, 6)]),
            l(6, "Carter", 1, 12, 12, 16, 2, 4, &[]),
        ],
    }
}

// ---- debug ------------------------------------------------------------------------------------------------------

/// `REACH_STATS_LOG=1`: log every statistic word that changed since the last call, per player (layout discovery).
/// Game thread (scoreboard::game_tick), at most twice a second.
pub unsafe fn debug_tick(base: usize, tls_block: usize) {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    static PREV: Mutex<Vec<Vec<u16>>> = Mutex::new(Vec::new());
    static LAST: AtomicUsize = AtomicUsize::new(0);
    if !*ON.get_or_init(|| std::env::var_os("REACH_STATS_LOG").is_some()) {
        return;
    }
    let now = epoch_ms();
    if now.saturating_sub(LAST.load(Ordering::Relaxed)) < 500 {
        return;
    }
    LAST.store(now, Ordering::Relaxed);
    let mut prev = PREV.lock().unwrap_or_else(|e| e.into_inner());
    prev.resize(16, vec![0u16; STATS_SIZE / 2]);
    for k in 0..16 {
        let e = base + RVA_RESULT_PLAYERS + k * 0xd8;
        if (e as *const u8).read_volatile() == 0 {
            continue;
        }
        let xuid = ((e + 7) as *const u64).read_unaligned();
        let blk = base + RVA_RESULT_STATS + k * STATS_SIZE;
        let cur: Vec<u16> = (0..STATS_SIZE / 2).map(|n| ((blk + 2 * n) as *const u16).read_volatile()).collect();
        // Word 14 (seconds alive) moves every second: not news.
        let changed: Vec<String> = (0..cur.len()).filter(|&n| cur[n] != prev[k][n] && n != 14)
            .map(|n| format!("w{n}(+{:#x})={}", 2 * n, cur[n] as i16)).collect();
        if !changed.is_empty() {
            let ff = firefight_score(tls_block, xuid);
            log!("stats: entry {k} xuid {xuid:#x} changed {} | ff score {ff:?} mode {:?} final {}", changed.join(" "),
                options_mode(tls_block), ((base + RVA_RESULTS_FINAL) as *const u8).read_volatile());
        }
        prev[k] = cur;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_roundtrip() {
        let line = |n: &str, x: u64| Line {
            xuid: x, name: n.into(), team: 1, score: 12, kills: 13, deaths: 4, assists: 2, betrayals: 0, suicides: 1,
            best_spree: 6, medals: vec![(11, 2), (3, 1)], medal_total: 3, weapon: Some((4, 9)),
        };
        let r = Report { id: 3, teams: true, firefight: false, team_scores: [25, 18, 0, 0, 0, 0, 0, 0],
            players: vec![line("Noble Six", 0x0009_0000_0000_0001), line("A very long player name that is cut", 2)] };
        let d = decode(&encode(&r)).unwrap();
        assert_eq!(d.players[0], r.players[0]);
        assert_eq!(d.players[1].name.len(), NAME_MAX);
        assert_eq!((d.id, d.teams, d.firefight, d.team_scores), (3, true, false, r.team_scores));
        let e = encode(&r);
        assert!(decode(&e[..e.len() - 1]).is_none());
        // 16 players with full medal lists stay under one Ethernet MTU.
        let big = Report { players: (0..16).map(|i| { let mut l = line("Player name 16 ch", i); l.medals = vec![(1, 1); 6]; l }).collect(), ..r };
        assert!(encode(&big).len() < 1400, "{}", encode(&big).len());
    }

    #[test]
    fn titles() {
        assert_eq!(title_of("medal_multikill_2"), "Double Kill");
        assert_eq!(title_of("triple_double"), "Triple Double");
        assert_eq!(title_of("medal_some_new_thing"), "Some New Thing");
    }
}
