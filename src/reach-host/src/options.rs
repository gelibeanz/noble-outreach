//! Builds a multiplayer custom-game, Firefight or campaign launch-options block (0x2BF30 bytes) with the engine's own
//! data-access object: map variant from a map id or `.mvar`, game variant from a `.bin`, each copied in
//! by the variant's own method. Field layout and addresses: notes/launch-options.md,
//! notes/campaign-firefight.md.

use std::ffi::c_void;
use std::mem::transmute;
use std::path::{Path, PathBuf};

use crate::log;

pub const SIZE: usize = 0x2BF30;

// Data-access vtable slots (vtable 0x180994e60).
const DA_MAP_VARIANT_FROM_FILE: usize = 1;
const DA_MAP_VARIANT_FROM_MAP_ID: usize = 2;
const DA_GAME_VARIANT_FROM_FILE: usize = 4;
// Variant object vtable slots (map variant 0x180994fc8, game variant 0x180995048).
const VARIANT_COPY_TO_OPTIONS: usize = 7;
const MAP_VARIANT_GET_MAP_ID: usize = 13;
/// Engine's mapinfo-id → built-in MP map index switch (e.g. 3006 → 0xd1); -1 if unknown.
const RVA_MAP_INDEX_FROM_MAPINFO_ID: usize = 0x38c30;

const LOCAL_XUID: u64 = 0x0009_0000_0000_0001; // same as host slot 88
const LOCAL_MACHINE: u64 = 0x5245_4143_4830_0001; // our own peer/host address (any non-zero id)

/// Game mode (options +0x0C) and the mode-specific fields.
#[derive(Clone, Copy, PartialEq)]
pub enum Mode {
    Multiplayer,
    /// Mode 5 → launch state 7. Needs a Firefight game variant; the map is set on the session by id.
    Firefight,
    /// Mode 1 → launch state 4. No variants: the engine builds its own campaign game variant.
    Campaign { insertion: i32 },
}

pub struct Spec {
    pub mode: Mode,
    /// Options +0x34: 0 easy, 1 normal, 2 heroic, 3 legendary.
    pub difficulty: i32,
    /// Options +0x48: skull bits in MCC's shared skull enum (`SKULLS`); campaign and Firefight only.
    pub skulls: u64,
    pub map: Option<String>,
    pub game_variant: Option<PathBuf>,
    pub map_variant: Option<PathBuf>,
    pub net: Option<NetGame>,
}

/// A networked custom game: `players` instances, instance 0 hosts. Instance n is player n with xuid
/// `xuid(n)` and network id `net::net_id(n)`. Layout and roles: notes/networking.md.
pub struct NetGame {
    pub instance: u32,
    pub players: u32,
    pub session_id: u64,
    /// The session host has no local player (dedicated server). On the host: empty player table, nobody
    /// signed in. On a client: only its own player entry is listed. See notes/dedicated.md.
    pub dedicated: bool,
    /// Listen server (menu HOST): the host plays and starts at once; clients join in progress like on a
    /// dedicated server (only the host is in the player table, +0xE8 = 1). See notes/dedicated.md "Listen server".
    pub listen: bool,
}

/// Reach's skulls: (CLI / settings key, name, bit in options +0x48). +0x48 uses MCC's skull enum shared by every
/// game (libmcc `e_skull`); `ConvertLaunchSkullsToReach` (0x18003a22c, launch states 4 and 7) maps those bits to
/// Reach's own order (iron 0, black eye 1, tough luck 2, catch 3, fog/cloud 4, famine 5, thunderstorm 6, tilt 7,
/// mythic 8, assassin 9 (not in Reach's menu), blind 10, cowbell 11, grunt birthday party 12, IWHBYD 13) and hands
/// them to the session (0x1802a5338, rcx = Reach bits). notes/campaign-firefight.md "Skulls".
pub const SKULLS: &[(&str, &str, u32)] = &[
    ("iron", "Iron", 18),
    ("blackeye", "Black Eye", 3),
    ("toughluck", "Tough Luck", 36),
    ("catch", "Catch", 7),
    ("cloud", "Cloud", 13),
    ("famine", "Famine", 11),
    ("thunderstorm", "Thunderstorm", 34),
    ("tilt", "Tilt", 35),
    ("mythic", "Mythic", 23),
    ("blind", "Blind", 4),
    ("cowbell", "Cowbell", 8),
    ("gruntbirthday", "Grunt Birthday Party", 16),
    ("iwhbyd", "IWHBYD", 19),
];

/// `iron,blind` → +0x48 bits.
pub fn parse_skulls(list: &str) -> Result<u64, String> {
    let mut bits = 0u64;
    for k in list.split(',').map(str::trim).filter(|k| !k.is_empty()) {
        let s = SKULLS.iter().find(|s| s.0.eq_ignore_ascii_case(k)).ok_or_else(|| format!("unknown skull {k} (expected {})", SKULLS.iter().map(|s| s.0).collect::<Vec<_>>().join(",")))?;
        bits |= 1 << s.2;
    }
    Ok(bits)
}

pub fn skull_names(bits: u64) -> String {
    SKULLS.iter().filter(|s| bits >> s.2 & 1 != 0).map(|s| s.1).collect::<Vec<_>>().join(", ")
}

pub fn xuid(instance: u32) -> u64 {
    LOCAL_XUID + instance as u64
}

impl Spec {
    pub fn wanted(&self) -> bool {
        self.map.is_some() || self.game_variant.is_some() || self.map_variant.is_some()
    }
}

unsafe fn vfn(obj: *mut c_void, slot: usize) -> usize {
    *(*(obj as *const *const usize)).add(slot)
}

struct Block(Vec<u8>);

impl Block {
    fn put(&mut self, off: usize, bytes: &[u8], what: &str) {
        self.0[off..off + bytes.len()].copy_from_slice(bytes);
        let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        log!("  options+{off:#07x} = {hex} ({what})");
    }
    fn u8(&mut self, off: usize, v: u8, what: &str) {
        self.put(off, &[v], what)
    }
    fn u16(&mut self, off: usize, v: u16, what: &str) {
        self.put(off, &v.to_le_bytes(), what)
    }
    fn i32(&mut self, off: usize, v: i32, what: &str) {
        self.put(off, &v.to_le_bytes(), what)
    }
    fn u64(&mut self, off: usize, v: u64, what: &str) {
        self.put(off, &v.to_le_bytes(), what)
    }
}

/// `<name>` → `maps/info/<name>.mapinfo` id (u32 BE at +0x3C, the `levl` chunk), or a number:
/// >= 1000 is a mapinfo id, smaller is already a map index.
fn resolve_map(mcc: &Path, base: usize, name: &str) -> Result<u32, String> {
    let name = if name == "forge_world" { "forge_halo" } else { name };
    let num = name.strip_prefix("0x").map_or_else(|| name.parse().ok(), |h| u32::from_str_radix(h, 16).ok());
    let mapinfo_id = match num {
        Some(n) if n < 1000 => return Ok(n),
        Some(n) => n,
        None => {
            let p = mcc.join("haloreach").join("maps").join("info").join(format!("{name}.mapinfo"));
            let b = std::fs::read(&p).map_err(|e| format!("read {}: {e}", p.display()))?;
            if b.len() < 0x40 || &b[0x30..0x34] != b"levl" {
                return Err(format!("{} is not a mapinfo file", p.display()));
            }
            u32::from_be_bytes(b[0x3C..0x40].try_into().unwrap())
        }
    };
    let index_of: unsafe extern "system" fn(u32) -> i32 = unsafe { transmute(base + RVA_MAP_INDEX_FROM_MAPINFO_ID) };
    let index = unsafe { index_of(mapinfo_id) };
    log!("map {name}: mapinfo id {mapinfo_id} -> map index {index:#x}");
    if index < 0 {
        return Err(format!("map {name} (mapinfo id {mapinfo_id}) is not a built-in map"));
    }
    Ok(index as u32)
}

fn map_id(index: u32) -> [u8; 16] {
    let mut id = [0u8; 16];
    id[0..4].copy_from_slice(&index.to_le_bytes());
    id[6..8].copy_from_slice(&0x8888u16.to_le_bytes());
    id
}

/// Build the block. `data` is the object from `CreateDataAccess`, `base` the engine's load address.
pub unsafe fn build(spec: &Spec, mcc: &Path, data: *mut c_void, base: usize) -> Result<Vec<u8>, String> {
    if data.is_null() {
        return Err("no data-access object".into());
    }
    let mut b = Block(vec![0u8; SIZE]);
    log!("building launch options ({})", match spec.mode {
        Mode::Multiplayer => "multiplayer custom game",
        Mode::Firefight => "firefight",
        Mode::Campaign { .. } => "campaign",
    });

    // Campaign: no variants (launch state 4 makes its own campaign game variant and never reads a map variant).
    let (id, index, variants) = if let Mode::Campaign { .. } = spec.mode {
        let name = spec.map.as_deref().ok_or("--campaign needs a mission")?;
        let index = resolve_map(mcc, base, name)?;
        (map_id(index), index, None)
    } else {
        let (id, index, mv, gv) = build_variants(spec, mcc, data, base)?;
        (id, index, Some((gv, mv)))
    };
    write_header(&mut b, spec, &id, index);
    write_players(&mut b, spec);
    if let Some((game_variant, map_variant)) = variants {
        let copy: unsafe extern "system" fn(*mut c_void, *mut u8) = transmute(vfn(game_variant, VARIANT_COPY_TO_OPTIONS));
        copy(game_variant, b.0.as_mut_ptr());
        log!("  options+0x00300: game variant copied by its slot 7 (0xfc04 bytes), type {}", u32::from_le_bytes(b.0[0x300..0x304].try_into().unwrap()));
        crate::forge::note_launch(spec.map.as_deref().unwrap_or(""), u32::from_le_bytes(b.0[0x300..0x304].try_into().unwrap()));
        let copy: unsafe extern "system" fn(*mut c_void, *mut u8) = transmute(vfn(map_variant, VARIANT_COPY_TO_OPTIONS));
        copy(map_variant, b.0.as_mut_ptr());
        let mv = 0x1D308;
        log!(
            "  options+0x1d308: map variant copied by its slot 7 (0xd9bc bytes), mapinfo id {} at +0x2b4",
            i32::from_le_bytes(b.0[mv + 0x2B4..mv + 0x2B8].try_into().unwrap())
        );
    }
    // The variant objects stay alive (engine heap); not worth freeing for a one-shot launch.
    Ok(b.0)
}

/// Map variant (from a .mvar file, else the map's default) and game variant objects, plus the map id/index.
unsafe fn build_variants(spec: &Spec, mcc: &Path, data: *mut c_void, base: usize) -> Result<([u8; 16], u32, *mut c_void, *mut c_void), String> {
    let map_variant = if let Some(p) = &spec.map_variant {
        let file = std::fs::read(p).map_err(|e| format!("read {}: {e}", p.display()))?;
        let f: unsafe extern "system" fn(*mut c_void, *const u8, u32) -> *mut c_void =
            transmute(vfn(data, DA_MAP_VARIANT_FROM_FILE));
        let mv = f(data, file.as_ptr(), file.len() as u32);
        log!("data access slot 1 (map variant from file {}, {} bytes) -> {mv:?}", p.display(), file.len());
        if mv.is_null() {
            return Err(format!("engine rejected map variant {}", p.display()));
        }
        mv
    } else {
        let name = spec.map.as_deref().ok_or("--game-variant needs --map or --map-variant")?;
        let id = map_id(resolve_map(mcc, base, name)?);
        let f: unsafe extern "system" fn(*mut c_void, *const [u8; 16]) -> *mut c_void =
            transmute(vfn(data, DA_MAP_VARIANT_FROM_MAP_ID));
        let mv = f(data, &id);
        log!("data access slot 2 (map variant from map id) -> {mv:?}");
        if mv.is_null() {
            return Err("engine returned no map variant".into());
        }
        mv
    };
    let get_id: unsafe extern "system" fn(*mut c_void) -> *const [u8; 16] = transmute(vfn(map_variant, MAP_VARIANT_GET_MAP_ID));
    let id = *get_id(map_variant);
    let index = u32::from_le_bytes(id[0..4].try_into().unwrap());
    if let Some(name) = &spec.map {
        if spec.map_variant.is_some() && resolve_map(mcc, base, name)? != index {
            log!("warning: --map {name} differs from the map variant's map index {index:#x}; using the variant's");
        }
    }

    let default_gv = if spec.mode == Mode::Firefight { "hr_ff_standard_1set.bin" } else { "slayer_054.bin" };
    let gv_path = spec.game_variant.clone().unwrap_or_else(|| mcc.join("haloreach").join("game_variants").join(default_gv));
    let file = std::fs::read(&gv_path).map_err(|e| format!("read {}: {e}", gv_path.display()))?;
    let f: unsafe extern "system" fn(*mut c_void, *const u8, u32) -> *mut c_void = transmute(vfn(data, DA_GAME_VARIANT_FROM_FILE));
    let game_variant = f(data, file.as_ptr(), file.len() as u32);
    log!("data access slot 4 (game variant from file {}, {} bytes) -> {game_variant:?}", gv_path.display(), file.len());
    if game_variant.is_null() {
        return Err(format!("engine rejected game variant {}", gv_path.display()));
    }
    Ok((id, index, map_variant, game_variant))
}

fn write_header(b: &mut Block, spec: &Spec, id: &[u8; 16], index: u32) {

    // Header. Bytes 1..=4, +0x34 and +0x44 take the values MCC sent for a Slayer custom game.
    b.u8(0x00, 0x08, "flags lo: bit 3 multiplayer");
    b.u8(0x01, 0x03, "flags hi (MCC value)");
    b.u8(0x02, 0x18, "option byte (MCC value)");
    b.u8(0x03, 0x10, "option byte, 16 = max players? (MCC value)");
    b.u8(0x04, 0x03, "option byte (MCC value; 4 disables team lookup)");
    match spec.mode {
        Mode::Multiplayer => b.i32(0x0C, 3, "game mode: multiplayer"),
        Mode::Firefight => b.i32(0x0C, 5, "game mode: firefight"),
        Mode::Campaign { insertion } => {
            b.i32(0x0C, 1, "game mode: campaign");
            // Built-in campaign id {campaign 1 (halo3.campaign cmpn id), 1, 0x1111}: PreloadCustomMapInfo skips
            // host slot 17 for this form; launch state 4 resolves (campaign id, map id) to the scenario.
            let mut cid = [0u8; 16];
            cid[0..4].copy_from_slice(&1u32.to_le_bytes());
            cid[4..6].copy_from_slice(&1u16.to_le_bytes());
            cid[6..8].copy_from_slice(&0x1111u16.to_le_bytes());
            b.put(0x24, &cid, "campaign id {1, 1, 0x1111}");
            b.i32(0x3C, insertion, "campaign insertion point");
        }
    }
    b.i32(0x10, index as i32, "map index");
    b.put(0x14, id, "map id {index, 0, 0x8888}");
    b.i32(0x34, spec.difficulty, "difficulty (0 easy .. 3 legendary)");
    if spec.skulls != 0 && spec.mode != Mode::Multiplayer {
        b.u64(0x48, spec.skulls, &format!("skulls: {}", skull_names(spec.skulls)));
    }
    b.u16(0x44, 0xFFFF, "(MCC value)");
}

/// Multiplayer alternates teams; Firefight and campaign put everyone on team 0 (team 1 spawns as an Elite in Firefight).
fn team(spec: &Spec, player: u32) -> i32 {
    if spec.mode == Mode::Multiplayer { (player % 2) as i32 } else { 0 }
}

fn write_players(b: &mut Block, spec: &Spec) {
    match &spec.net {
        None => {
            let instance = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_nanos() as u64) | 1;
            b.u64(0x50, instance, "game instance id (random per launch)");
            b.u64(0x58, LOCAL_MACHINE, "local secure address");
            // Player options at +0x60: 17 peer addresses, player count, 16 players of 0x20, count.
            b.u64(0x60, LOCAL_MACHINE, "peer 0 address");
            b.i32(0xE8, 1, "player count");
            b.u64(0xF0, LOCAL_XUID, "player 0 xuid");
            b.u64(0xF8, LOCAL_MACHINE, "player 0 peer address");
            b.i32(0x100, 0, "player 0 team");
            b.i32(0x104, 1, "player 0 +0x14 (MCC value)");
            b.i32(0x10C, -1, "player 0 +0x1c (MCC value)");
        }
        Some(n) if n.dedicated => {
            let host = crate::net::net_id(0);
            let me = crate::net::net_id(n.instance);
            if n.instance != 0 {
                b.u8(0x00, 0x00, "flags lo: bit 3 clear = join, not host");
            } else if spec.mode == Mode::Firefight {
                // No bit 6 here: it also sets game option +0x110dc, and with that flag set the Firefight script's
                // `survival_mode_meets_set_start_requirements` (0x1801b17b8) wants at least +0x110dd (2..8) players
                // in game, so one client on a dedicated server never got a first wave. Without it the session keeps
                // its default max players and the game end goes to the lobby, i.e. the process exits and restarts.
                log!("  dedicated firefight: flags hi bit 6 left clear (set start would need 2+ players)");
            } else {
                // Bit 6 makes launch state 6 apply +0x03 (session max players) and +0x0B, and sets the session
                // flag that becomes game option +0x110dc: at the end of a game the life cycle then goes to its
                // "next game" state (asks host slot 26) instead of back to the lobby (which exits a hosted engine).
                b.u8(0x01, 0x03 | 0x40, "flags hi: bit 6 = apply max players, next game after game end");
                b.u8(0x03, n.players as u8, "session max players");
                // Game option +0x110dd: the options check (0x180057c90) wants 0xFF while +0x110dc is clear and 2..8 when set.
                b.u8(0x0B, 2, "with bit 6: game option +0x110dd, must be 2..8 (meaning unknown)");
            }
            b.u64(0x50, n.session_id, "game instance id = session id (same on every instance)");
            b.u64(0x58, me, "local secure address (our network id)");
            b.u64(0x60, host, "peer 0 address (dedicated host)");
            // The session cannot build game options with zero players (the game ends at once and the host exits),
            // so the server waits in launch state 8 until 2 machines (itself + the first player) are in; main.rs
            // keeps that state from timing out. See notes/dedicated.md.
            b.i32(0xE8, if n.instance == 0 { 2 } else { 0 }, "machines to wait for: server + first player (held by main.rs)");
            if n.instance != 0 {
                b.u64(0xF0, xuid(n.instance), "player 0 xuid (this client)");
                b.u64(0xF8, me, "player 0 peer address");
                b.i32(0x100, team(spec, n.instance), "player 0 team");
                b.i32(0x104, 1, "player 0 +0x14 (MCC value)");
                b.i32(0x108, 0, "player 0 +0x18");
                b.i32(0x10C, -1, "player 0 +0x1c (MCC value)");
            }
            b.u64(0x2F8, host, "host secure address to join (read in launch state 9)");
        }
        Some(n) if n.listen => {
            // Host only (its clients use the dedicated joiner layout). Same session setup as a dedicated server,
            // but player 0 is the host's own local player and state 8 needs no one else (+0xE8 = 1 machine).
            let host = crate::net::net_id(0);
            if spec.mode == Mode::Firefight {
                log!("  listen firefight: flags hi bit 6 left clear (set start would need 2+ players)");
            } else {
                b.u8(0x01, 0x03 | 0x40, "flags hi: bit 6 = apply max players, next game after game end");
                b.u8(0x03, n.players as u8, "session max players");
                b.u8(0x0B, 2, "with bit 6: game option +0x110dd, must be 2..8");
            }
            b.u64(0x50, n.session_id, "game instance id = session id (same on every instance)");
            b.u64(0x58, host, "local secure address (our network id)");
            b.u64(0x60, host, "peer 0 address (this host)");
            if matches!(spec.mode, Mode::Campaign { .. }) {
                // Campaign refuses joins once the mission runs, so the host gathers friends first (lobby.rs). With
                // +0xE8 <= 1 launch state 4 skips state 8 altogether (cmp at 0x18000e446), so ask for 2 machines;
                // lobby.rs then holds state 8 and releases it with the real count.
                b.i32(0xE8, 2, "machines to wait for: host + 1, held by lobby.rs until the host starts the mission");
            } else {
                b.i32(0xE8, 1, "machines to wait for: the host alone, start at once (late join)");
            }
            b.u64(0xF0, xuid(0), "player 0 xuid (host's local player)");
            b.u64(0xF8, host, "player 0 peer address");
            b.i32(0x100, team(spec, 0), "player 0 team");
            b.i32(0x104, 1, "player 0 +0x14 (MCC value)");
            b.i32(0x108, 0, "player 0 +0x18");
            b.i32(0x10C, -1, "player 0 +0x1c (MCC value)");
            b.u64(0x2F8, host, "host secure address");
        }
        Some(n) => {
            let host = crate::net::net_id(0);
            let me = crate::net::net_id(n.instance);
            if n.instance != 0 {
                // Joiner: bit 3 clear sends the launch state machine from state 6 to state 9, which asks the
                // session to join session +0x50 at the host's secure address +0x2F8.
                b.u8(0x00, 0x00, "flags lo: bit 3 clear = join, not host");
            }
            b.u64(0x50, n.session_id, "game instance id = session id (same on every instance)");
            b.u64(0x58, me, "local secure address (our network id)");
            for i in 0..n.players {
                b.u64(0x60 + 8 * i as usize, crate::net::net_id(i), &format!("peer {i} address"));
            }
            b.i32(0xE8, n.players as i32, "player count (host waits in state 8 until this many joined)");
            for i in 0..n.players {
                let p = 0xF0 + 0x20 * i as usize;
                b.u64(p, xuid(i), &format!("player {i} xuid"));
                b.u64(p + 8, crate::net::net_id(i), &format!("player {i} peer address"));
                b.i32(p + 0x10, team(spec, i), &format!("player {i} team"));
                b.i32(p + 0x14, 1, &format!("player {i} +0x14 (MCC value)"));
                b.i32(p + 0x18, i as i32, &format!("player {i} +0x18 (peer index?)"));
                b.i32(p + 0x1C, -1, &format!("player {i} +0x1c (MCC value)"));
            }
            b.u64(0x2F8, host, "host secure address to join (read in launch state 9)");
        }
    }
    let used = match &spec.net {
        None => 1,
        Some(n) if n.dedicated => (n.instance != 0) as usize,
        Some(n) if n.listen => 1,
        Some(n) => n.players as usize,
    };
    for p in (0xF0 + 0x20 * used..0x2F0).step_by(0x20) {
        for f in [0x10, 0x18, 0x1C] {
            b.0[p + f..p + f + 4].copy_from_slice(&(-1i32).to_le_bytes());
        }
    }
    log!("  options+{:#x}..0x2f0: remaining players empty (+0x10, +0x18, +0x1c = -1, like MCC)", 0xF0 + 0x20 * used);
    b.i32(0x2F0, used as i32, "player table count (team assignment loop)");
}
