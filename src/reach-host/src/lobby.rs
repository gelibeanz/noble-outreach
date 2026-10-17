//! Campaign lobby gather on a dedicated server. A campaign session refuses joins once the mission runs
//! (`CanSessionAcceptAnyJoinRequest` reason 0x1f, notes/campaign-firefight.md), so the server holds launch state 8
//! until the players have gathered: it starts when the session is full or `--lobby-wait N` seconds (default 45)
//! after the first player joined. State 8 (`LaunchStateWaitForPeersJoined` 0x18000ebac) moves on when
//! `GetSessionJoinedPlayerCount` (0x1802a3838, machines, so the server itself is 1) equals the engine's copy of
//! launch options +0xE8 (global 0x182497288). We keep that at an impossible count while gathering and set it to the
//! current count to start. Runs on the game thread (called from throttle.rs's paced pump), so the engine call is safe.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use crate::log;

const LAUNCH_STATE: usize = 0xc1a114;
const OPTIONS_PLAYER_COUNT: usize = 0x2497288; // engine copy of launch options (0x1824971a0) + 0xE8
const RVA_JOINED_COUNT: usize = 0x2a3838;
const HOLD: i32 = 64; // more machines than a session can hold

/// `--lobby-wait N` (seconds after the first player joined).
pub static WAIT_SECS: AtomicU32 = AtomicU32::new(45);
/// Set by main.rs for a dedicated campaign server.
pub static ENABLED: AtomicBool = AtomicBool::new(false);
pub static MAX_PLAYERS: AtomicU32 = AtomicU32::new(4);
/// Listen host (menu HOST CO-OP): the host's own machine counts as a player, the wait starts as soon as state 8 is
/// reached, and the host can start early (Enter, `start_now`).
pub static LISTEN: AtomicBool = AtomicBool::new(false);
static START_NOW: AtomicBool = AtomicBool::new(false);
/// Engine is in launch state 8 and we hold it (for the host's "waiting for friends" panel).
static HOLDING: AtomicBool = AtomicBool::new(false);
/// The mission has started: joins are refused by the engine (query flag bit 2, the browser shows it).
pub static IN_PROGRESS: AtomicBool = AtomicBool::new(false);
/// Players gathered so far and seconds left (0 = no countdown yet), for the server query.
pub static GATHERED: AtomicU32 = AtomicU32::new(0);
pub static SECS_LEFT: AtomicU32 = AtomicU32::new(0);
static FIRST_MS: AtomicU64 = AtomicU64::new(0); // game thread only; 0 = nobody yet
static RELEASED: AtomicBool = AtomicBool::new(false);

/// Listen host: start the mission now with whoever is in (Enter on the lobby panel). True = consumed.
pub fn start_now() -> bool {
    if !HOLDING.load(Ordering::Relaxed) || RELEASED.load(Ordering::Relaxed) {
        return false;
    }
    START_NOW.store(true, Ordering::Relaxed);
    true
}

/// Lobby panel: (players in, session size, seconds left, we are the listen host) while gathering. A listen host uses
/// its own state; a client joined to a gathering dedicated server shows what the server's `RLBY` packets say.
pub fn waiting_view() -> Option<(u32, u32, u32, bool)> {
    if LISTEN.load(Ordering::Relaxed) {
        if !HOLDING.load(Ordering::Relaxed) || RELEASED.load(Ordering::Relaxed) {
            return None;
        }
        return Some((GATHERED.load(Ordering::Relaxed).max(1), MAX_PLAYERS.load(Ordering::Relaxed), SECS_LEFT.load(Ordering::Relaxed), true));
    }
    let v = *CLIENT_VIEW.lock().unwrap_or_else(|e| e.into_inner());
    let (players, max, secs, at) = v?;
    (at.elapsed() < std::time::Duration::from_secs(3) && secs > 0).then_some((players, max, secs, false))
}

// ---- dedicated server -> clients: lobby status ---------------------------------------------------------------
// `RLBY` v1, players u8, max u8, seconds left u8 (0 = mission starting). Sent about once a second while a dedicated
// campaign server gathers, and three times when the mission starts. Clients without this code ignore it.
const MAGIC: [u8; 4] = *b"RLBY";
static CLIENT_VIEW: std::sync::Mutex<Option<(u32, u32, u32, std::time::Instant)>> = std::sync::Mutex::new(None);
static LAST_SEND_MS: AtomicU64 = AtomicU64::new(0);

fn broadcast_status(players: u32, max: u32, secs: u32) {
    let pkt = [MAGIC[0], MAGIC[1], MAGIC[2], MAGIC[3], 1, players.min(255) as u8, max.min(255) as u8, secs.min(255) as u8];
    for a in crate::net::active_peer_addrs(std::time::Duration::from_secs(4)) {
        crate::net::send_raw(a, &pkt);
    }
}

/// net.rs receive thread: true when `pkt` is a lobby status datagram (handled here, never queued for the engine).
pub fn handle(pkt: &[u8], from: std::net::SocketAddr) -> bool {
    if pkt.len() < 8 || pkt[..4] != MAGIC {
        return false;
    }
    if pkt[4] == 1 && crate::net::host_addr() == Some(from) {
        let v = (pkt[5] as u32, pkt[6] as u32, pkt[7] as u32, std::time::Instant::now());
        let prev = CLIENT_VIEW.lock().unwrap_or_else(|e| e.into_inner()).replace(v);
        if prev.is_none_or(|p| p.0 != v.0) {
            log!("lobby: server has {}/{} players, mission starts in {} s", v.0, v.1, v.2);
        }
    }
    true
}

pub fn parse(argv: &[String]) -> Result<(), String> {
    if let Some(i) = argv.iter().position(|a| a == "--lobby-wait") {
        let v = argv.get(i + 1).ok_or("--lobby-wait needs N (seconds)")?;
        WAIT_SECS.store(v.parse().map_err(|e| format!("--lobby-wait: {e}"))?, Ordering::Relaxed);
    }
    JIP.store(argv.iter().any(|a| a == "--campaign-jip"), Ordering::Relaxed);
    Ok(())
}

fn now_ms() -> u64 {
    static T0: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    T0.get_or_init(std::time::Instant::now).elapsed().as_millis() as u64 + 1
}

/// Game thread, once per loop pass.
pub unsafe fn tick(base: usize) {
    if !ENABLED.load(Ordering::Relaxed) {
        return;
    }
    let state = ((base + LAUNCH_STATE) as *const u32).read_volatile();
    HOLDING.store(state == 8, Ordering::Relaxed);
    if state != 8 {
        if state >= 10 && !JIP.load(Ordering::Relaxed) && !IN_PROGRESS.swap(true, Ordering::Relaxed) {
            SECS_LEFT.store(0, Ordering::Relaxed);
            log!("lobby: mission started with {} player(s); the engine refuses joins from now on", GATHERED.load(Ordering::Relaxed));
        }
        return;
    }
    let count: unsafe extern "system" fn() -> i32 = std::mem::transmute(base + RVA_JOINED_COUNT);
    let machines = count();
    let listen = LISTEN.load(Ordering::Relaxed);
    // A dedicated server is a machine without a player; a listen host is one of the players.
    let players = if listen { machines.max(1) as u32 } else { (machines - 1).max(0) as u32 };
    let max = MAX_PLAYERS.load(Ordering::Relaxed).max(1);
    let target = (base + OPTIONS_PLAYER_COUNT) as *mut i32;
    if RELEASED.load(Ordering::Relaxed) {
        target.write_volatile(machines); // a join finishing in the same pass changes the count; follow it
        return;
    }
    let prev = GATHERED.swap(players, Ordering::Relaxed);
    let wait = WAIT_SECS.load(Ordering::Relaxed) as u64 * 1000;
    if players == 0 {
        FIRST_MS.store(0, Ordering::Relaxed);
        SECS_LEFT.store(0, Ordering::Relaxed);
        if prev != 0 {
            log!("lobby: everyone left, waiting for the first player again");
        }
        target.write_volatile(HOLD);
        return;
    }
    if FIRST_MS.load(Ordering::Relaxed) == 0 {
        FIRST_MS.store(now_ms(), Ordering::Relaxed);
    }
    let waited = now_ms() - FIRST_MS.load(Ordering::Relaxed);
    let left = wait.saturating_sub(waited);
    SECS_LEFT.store(left.div_ceil(1000).max(1) as u32, Ordering::Relaxed);
    if players != prev {
        log!("lobby: {players}/{max} players gathered, mission starts in {} s or when full", left.div_ceil(1000));
    }
    if !listen && now_ms() - LAST_SEND_MS.load(Ordering::Relaxed) >= 1000 {
        LAST_SEND_MS.store(now_ms(), Ordering::Relaxed);
        broadcast_status(players, max, SECS_LEFT.load(Ordering::Relaxed));
    }
    // Test hook: `REACH_LOBBY_START_AFTER=N` presses "start now" N s into a listen host's lobby.
    if listen && std::env::var("REACH_LOBBY_START_AFTER").ok().and_then(|v| v.parse::<u64>().ok()).is_some_and(|n| waited >= n * 1000) {
        start_now();
    }
    let now = START_NOW.load(Ordering::Relaxed);
    if players >= max || left == 0 || now {
        let why = if players >= max { "server full" } else if now { "host started it" } else { "lobby wait over" };
        log!("lobby: starting the mission with {players}/{max} players ({why})");
        RELEASED.store(true, Ordering::Relaxed);
        target.write_volatile(machines);
        if !listen {
            broadcast_status(players, max, 0); // clients drop the panel at once
        }
    } else {
        target.write_volatile(HOLD);
    }
}

/// What a client sees when it tries to join a campaign server whose mission has started.
pub const IN_PROGRESS_MSG: &str = "Campaign in progress: wait for the next mission or the server restart";

/// `--campaign-jip` (experimental, server): let joins past the two "session closed" checks on the session
/// parameter byte (+1: 1 = open, 2 = campaign in progress):
/// 1. `CanSessionAcceptAnyJoinRequest` 0x180397354 (reason 0x1f): `jnz` at 0x18039737d → `jmp`.
/// 2. `GetSessionJoinRequestRefuseReason` 0x1803975d8 (reason 5): the `mov sil, r13b` at 0x180397653 that marks
///    the session not joinable → nops. See notes/campaign-firefight.md.
pub static JIP: AtomicBool = AtomicBool::new(false);

/// Between two games in one process (seamless.rs): back to "no lobby"; main sets it up again from the arguments.
pub fn reset() {
    for f in [&ENABLED, &LISTEN, &START_NOW, &HOLDING, &IN_PROGRESS, &RELEASED] {
        f.store(false, Ordering::Relaxed);
    }
    GATHERED.store(0, Ordering::Relaxed);
    SECS_LEFT.store(0, Ordering::Relaxed);
    FIRST_MS.store(0, Ordering::Relaxed);
    WAIT_SECS.store(45, Ordering::Relaxed);
    *CLIENT_VIEW.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

unsafe fn patch_bytes(site: *mut u8, want: &[u8], new: &[u8]) -> bool {
    use windows::Win32::System::Memory::{VirtualProtect, PAGE_EXECUTE_READWRITE, PAGE_PROTECTION_FLAGS};
    if std::slice::from_raw_parts(site, want.len()) != want {
        return false;
    }
    let mut old = PAGE_PROTECTION_FLAGS(0);
    if VirtualProtect(site as _, new.len(), PAGE_EXECUTE_READWRITE, &mut old).is_err() {
        return false;
    }
    std::ptr::copy_nonoverlapping(new.as_ptr(), site, new.len());
    let _ = VirtualProtect(site as _, new.len(), old, &mut old);
    true
}

pub unsafe fn install_jip(base: usize) {
    if !JIP.load(Ordering::Relaxed) {
        return;
    }
    let a = patch_bytes((base + 0x39737d) as *mut u8, &[0x75, 0x07], &[0xEB]);
    let b = patch_bytes((base + 0x397653) as *mut u8, &[0x41, 0x8A, 0xF5], &[0x90, 0x90, 0x90]);
    log!("campaign-jip: join-in-progress checks patched (experimental): 0x18039737d {a}, 0x180397653 {b}");
}

/// Players gathered in the lobby + 1 (the ROSTER_COUNT convention), 0 when there is no lobby.
pub fn players_plus_one() -> u64 {
    match GATHERED.load(Ordering::Relaxed) {
        0 => 0,
        n if !IN_PROGRESS.load(Ordering::Relaxed) => n as u64 + 1,
        _ => 0,
    }
}
