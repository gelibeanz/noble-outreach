//! Playlist voting (Reclaimer style) over our UDP side channel. A dedicated server with `--playlist` offers 3
//! weighted-random playlist entries plus "None of the above" (one re-roll) when the first player joins a fresh server
//! (launch state 8 is held meanwhile) and when a game ends (host slot 26). Clients vote with keys 1-4 / mouse / pad
//! (ui/vote.rs). After 20 s (`--vote-secs`), or 2 s after everyone voted, the majority wins (ties random, no votes =
//! random option). The same entry keeps the game running; another entry restarts the server process into it
//! (playlist.rs `vote-next.json`) and every client relaunches itself with the new map/mode and rejoins.
//! Wire format: notes/networking.md "Vote". Datagrams start with `RVOT`, so they never reach the engine queues.

use std::collections::HashMap;
use std::net::{SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicU8, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::log;
use crate::playlist;

pub const MAGIC: [u8; 4] = *b"RVOT";
pub const VERSION: u8 = 1;
const KIND_OPEN: u8 = 1; // server -> clients: options, seconds left, tally (repeated every second)
const KIND_CAST: u8 = 2; // client -> server: my choice
const KIND_RESULT: u8 = 3; // server -> clients: winner, and whether the server restarts into it
const KIND_CALL: u8 = 4; // client -> server: open a vote now (pause menu VOTE FOR A NEW GAME)
/// Choice index of "None of the above".
pub const NONE: u8 = 3;

/// `--vote-secs N` (default 20).
pub static SECS: AtomicU32 = AtomicU32::new(20);

/// One option as sent to clients.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Opt {
    pub mode: u8,
    pub name: String,
    pub map: String,
    pub game_variant: String,
}

#[derive(Clone, Debug, Default)]
pub struct OpenMsg {
    pub id: u16,
    pub secs_left: u8,
    pub none_allowed: bool,
    /// Choice 4 is "Keep playing this game" (a vote called mid-game), not "None of the above". Flags byte bit 1.
    pub keep: bool,
    pub voters: u8,
    pub tally: [u8; 4],
    pub playlist: String,
    pub options: Vec<Opt>,
}

#[derive(Clone, Debug, Default)]
pub struct ResultMsg {
    pub id: u16,
    pub winner: u8,
    pub restart: bool,
    pub difficulty: u8,
    pub opt: Opt,
}

// ---- wire ---------------------------------------------------------------------------------------------------

fn put_str(o: &mut Vec<u8>, s: &str, max: usize) {
    let mut n = s.len().min(max);
    while !s.is_char_boundary(n) {
        n -= 1;
    }
    o.push(n as u8);
    o.extend_from_slice(&s.as_bytes()[..n]);
}

struct Rd<'a>(&'a [u8], usize);

impl Rd<'_> {
    fn u8(&mut self) -> Option<u8> {
        let v = *self.0.get(self.1)?;
        self.1 += 1;
        Some(v)
    }
    fn str(&mut self) -> Option<String> {
        let n = self.u8()? as usize;
        let v = String::from_utf8_lossy(self.0.get(self.1..self.1 + n)?).into_owned();
        self.1 += n;
        Some(v)
    }
}

fn header(kind: u8, id: u16) -> Vec<u8> {
    let mut o = Vec::with_capacity(256);
    o.extend_from_slice(&MAGIC);
    o.push(VERSION);
    o.push(kind);
    o.extend_from_slice(&id.to_le_bytes());
    o
}

fn put_opt(o: &mut Vec<u8>, p: &Opt) {
    o.push(p.mode);
    put_str(o, &p.name, 48);
    put_str(o, &p.map, 64);
    put_str(o, &p.game_variant, 64);
}

fn get_opt(r: &mut Rd) -> Option<Opt> {
    Some(Opt { mode: r.u8()?, name: r.str()?, map: r.str()?, game_variant: r.str()? })
}

pub fn encode_open(m: &OpenMsg) -> Vec<u8> {
    let mut o = header(KIND_OPEN, m.id);
    o.extend_from_slice(&[m.secs_left, m.options.len() as u8, m.none_allowed as u8 | (m.keep as u8) << 1, m.voters]);
    o.extend_from_slice(&m.tally);
    put_str(&mut o, &m.playlist, 48);
    for p in &m.options {
        put_opt(&mut o, p);
    }
    o
}

fn decode_open(p: &[u8]) -> Option<OpenMsg> {
    let mut r = Rd(p, 8);
    let secs_left = r.u8()?;
    let n = r.u8()?.min(3) as usize;
    let flags = r.u8()?;
    let (none_allowed, keep) = (flags & 1 != 0, flags & 2 != 0);
    let voters = r.u8()?;
    let tally = [r.u8()?, r.u8()?, r.u8()?, r.u8()?];
    let playlist = r.str()?;
    let options = (0..n).map(|_| get_opt(&mut r)).collect::<Option<Vec<_>>>()?;
    Some(OpenMsg { id: u16::from_le_bytes([p[6], p[7]]), secs_left, none_allowed, keep, voters, tally, playlist, options })
}

pub fn encode_cast(id: u16, choice: u8) -> Vec<u8> {
    let mut o = header(KIND_CAST, id);
    o.extend_from_slice(&[choice, 0, 0, 0]);
    o
}

pub fn encode_result(m: &ResultMsg) -> Vec<u8> {
    let mut o = header(KIND_RESULT, m.id);
    o.extend_from_slice(&[m.winner, m.restart as u8, m.difficulty, 0]);
    put_opt(&mut o, &m.opt);
    o
}

fn decode_result(p: &[u8]) -> Option<ResultMsg> {
    let mut r = Rd(p, 8);
    let (winner, restart, difficulty, _) = (r.u8()?, r.u8()? & 1 != 0, r.u8()?, r.u8()?);
    Some(ResultMsg { id: u16::from_le_bytes([p[6], p[7]]), winner, restart, difficulty, opt: get_opt(&mut r)? })
}

fn opt_of(i: usize) -> Opt {
    let e = &playlist::get().unwrap().entries[i];
    Opt { mode: playlist::mode_byte(e), name: e.name.clone(), map: e.map.clone(), game_variant: e.game_variant.clone().unwrap_or_default() }
}

fn difficulty_byte(s: Option<&str>) -> u8 {
    match s {
        Some("easy") => 0,
        Some("heroic") => 2,
        Some("legendary") => 3,
        _ => 1,
    }
}

// ---- server -------------------------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Debug)]
enum Reason {
    PreGame,
    GameEnd,
    /// A player asked for one mid-game (pause menu); the current game is not among the options.
    Called,
}

struct Round {
    id: u16,
    reason: Reason,
    options: Vec<usize>,
    none_allowed: bool,
    deadline: Instant,
    votes: HashMap<SocketAddr, u8>,
    last_send: Option<Instant>,
    early: bool,
}

/// Server state, all under one lock (main thread ticks, receive thread records votes).
struct Server {
    round: Option<Round>,
    next_id: u16,
    /// RESULT resends: (message, sends left, next send, restart afterwards into entry).
    result: Option<(ResultMsg, u8, Instant, Option<usize>)>,
}

static SERVER: Mutex<Server> = Mutex::new(Server { round: None, next_id: 1, result: None });
static IS_SERVER: AtomicBool = AtomicBool::new(false);
/// Open request from the game thread / slot 26 / a player: 0 none, 1 pre-game, 2 game end, 3 called.
static OPEN_REQ: AtomicU8 = AtomicU8::new(0);
/// Pre-game vote: 0 not started (hold state 8 once a player is in), 1 running (hold), 2 done (release).
static PREGAME: AtomicU8 = AtomicU8::new(0);
/// Machines in the session while in launch state 8 (game thread), -1 = not in state 8.
static JOINED: AtomicI32 = AtomicI32::new(-1);

/// main.rs on a dedicated server with a playlist, after net::start.
pub fn start_server() {
    let Some(p) = playlist::get() else { return };
    IS_SERVER.store(true, Ordering::Relaxed);
    if p.voted {
        PREGAME.store(2, Ordering::Relaxed); // the players just voted for this entry
    }
    log!("vote: server ready, {} s per vote{}", SECS.load(Ordering::Relaxed), if p.voted { ", no pre-game vote (started by vote)" } else { ", pre-game vote when the first player joins" });
}

pub fn parse(argv: &[String]) -> Result<(), String> {
    if let Some(i) = argv.iter().position(|a| a == "--vote-secs") {
        let v = argv.get(i + 1).ok_or("--vote-secs needs N")?;
        SECS.store(v.parse::<u32>().map_err(|e| format!("--vote-secs: {e}"))?.clamp(5, 120), Ordering::Relaxed);
    }
    Ok(())
}

/// Host slot 26 (a game ended, the engine asks for the next one): open the end-of-game vote.
pub fn game_ended() {
    if IS_SERVER.load(Ordering::Relaxed) {
        let _ = OPEN_REQ.compare_exchange(0, 2, Ordering::Relaxed, Ordering::Relaxed);
    }
}

/// Game thread, once per loop pass (throttle.rs), dedicated servers: hold launch state 8 for the pre-game vote.
/// Same lever as lobby.rs: state 8 moves on when `GetSessionJoinedPlayerCount` (0x1802a3838, machines incl. the
/// server) equals the engine copy of launch options +0xE8 (0x182497288); an impossible count holds it.
pub unsafe fn game_tick(base: usize) {
    const LAUNCH_STATE: usize = 0xc1a114;
    const OPTIONS_PLAYER_COUNT: usize = 0x2497288;
    const RVA_JOINED_COUNT: usize = 0x2a3838;
    const HOLD: i32 = 64;
    if !IS_SERVER.load(Ordering::Relaxed) {
        return;
    }
    if ((base + LAUNCH_STATE) as *const u32).read_volatile() != 8 {
        JOINED.store(-1, Ordering::Relaxed);
        return;
    }
    let count: unsafe extern "system" fn() -> i32 = std::mem::transmute(base + RVA_JOINED_COUNT);
    let machines = count();
    JOINED.store(machines, Ordering::Relaxed);
    // A dedicated campaign gathers in lobby.rs, which owns the lever: the vote then runs during the gather without
    // holding (if the gather ends first, a different winner restarts the server as usual).
    let lobby = crate::lobby::ENABLED.load(Ordering::Relaxed);
    let target = (base + OPTIONS_PLAYER_COUNT) as *mut i32;
    match PREGAME.load(Ordering::Relaxed) {
        0 => {
            if machines >= 2 && PREGAME.compare_exchange(0, 1, Ordering::Relaxed, Ordering::Relaxed).is_ok() {
                let _ = OPEN_REQ.compare_exchange(0, 1, Ordering::Relaxed, Ordering::Relaxed);
            }
            if !lobby {
                target.write_volatile(HOLD);
            }
        }
        1 if !lobby => target.write_volatile(HOLD),
        // Released: the original rule (first player in).
        2 if !lobby => target.write_volatile(machines.max(2)),
        _ => {}
    }
}

fn open_round(s: &mut Server, reason: Reason, none_allowed: bool, exclude: &[usize]) {
    let p = playlist::get().unwrap();
    let options = playlist::pick(&p.entries, 3, exclude);
    let id = s.next_id;
    s.next_id = s.next_id.wrapping_add(1).max(1);
    let secs = SECS.load(Ordering::Relaxed);
    let list: Vec<String> = options.iter().enumerate().map(|(k, &i)| format!("{} \"{}\"", k + 1, p.entries[i].name)).collect();
    log!("vote: round {id} open ({}): {}{}, {secs} s", match reason { Reason::PreGame => "pre-game", Reason::GameEnd => "game over", Reason::Called => "called by a player" }, list.join(", "), if none_allowed { ", 4 none of the above" } else { "" });
    s.round = Some(Round { id, reason, options, none_allowed, deadline: Instant::now() + Duration::from_secs(secs as u64), votes: HashMap::new(), last_send: None, early: false });
}

fn tally(r: &Round) -> [u8; 4] {
    let mut t = [0u8; 4];
    for &v in r.votes.values() {
        t[v as usize] = t[v as usize].saturating_add(1);
    }
    t
}

fn open_msg(r: &Round, voters: usize) -> OpenMsg {
    OpenMsg {
        id: r.id,
        secs_left: r.deadline.saturating_duration_since(Instant::now()).as_secs_f32().ceil().min(255.0) as u8,
        none_allowed: r.none_allowed,
        keep: r.reason == Reason::Called,
        voters: voters.min(255) as u8,
        tally: tally(r),
        playlist: playlist::get().map(|p| p.name.clone()).unwrap_or_default(),
        options: r.options.iter().map(|&i| opt_of(i)).collect(),
    }
}

fn broadcast(pkt: &[u8]) -> usize {
    let peers = crate::net::active_peer_addrs(Duration::from_secs(4));
    for a in &peers {
        crate::net::send_raw(*a, pkt);
    }
    peers.len()
}

/// Listen host, campaign mission over (mission.rs): tell the joiners to follow into `map`, as a vote restart does
/// (RESULT with restart set, sent 3 times over ~1 s). Returns how many peers were told.
pub fn announce_next_mission(map: &str, name: &str, difficulty: &str) -> usize {
    let msg = ResultMsg {
        id: 0x8000 | (map.bytes().fold(0u16, |h, b| h.wrapping_mul(31).wrapping_add(b as u16)) & 0x7fff),
        winner: 0,
        restart: true,
        difficulty: difficulty_byte(Some(difficulty)),
        opt: Opt { mode: crate::query::MODE_CAMPAIGN, name: name.into(), map: map.into(), game_variant: String::new() },
    };
    let pkt = encode_result(&msg);
    let n = broadcast(&pkt);
    std::thread::spawn(move || {
        for _ in 0..2 {
            std::thread::sleep(Duration::from_millis(400));
            broadcast(&pkt);
        }
    });
    n
}

/// Main thread, every ~10 ms on a dedicated server.
pub fn tick() {
    if !IS_SERVER.load(Ordering::Relaxed) {
        return;
    }
    let mut s = SERVER.lock().unwrap();
    let now = Instant::now();
    // Result resends, then the restart.
    if let Some((msg, left, at, restart)) = &mut s.result {
        if now >= *at {
            broadcast(&encode_result(msg));
            *left -= 1;
            *at = now + Duration::from_millis(400);
            if *left == 0 {
                let restart = *restart;
                s.result = None;
                if let Some(e) = restart {
                    match playlist::write_next(e) {
                        Ok(()) => log!("vote: restarting the server into entry {} \"{}\" (exit code 3, supervisor restarts)", e + 1, playlist::get().unwrap().entries[e].name),
                        Err(err) => log!("vote: {err}; restarting anyway (random entry next)"),
                    }
                    std::process::exit(3);
                }
            }
        }
        return;
    }
    let req = OPEN_REQ.swap(0, Ordering::Relaxed);
    if s.round.is_none() {
        match req {
            1 => open_round(&mut s, Reason::PreGame, true, &[]),
            2 => open_round(&mut s, Reason::GameEnd, true, &[]),
            3 => {
                let cur = playlist::get().unwrap().current;
                open_round(&mut s, Reason::Called, true, &[cur]);
            }
            _ => return,
        }
    }
    let voters = crate::net::active_peer_addrs(Duration::from_secs(4)).len();
    let r = s.round.as_mut().unwrap();
    // Pre-game: everyone left before the vote ended; wait for the next first player.
    if r.reason == Reason::PreGame && JOINED.load(Ordering::Relaxed) == 1 && voters == 0 {
        log!("vote: round {} cancelled, nobody left on the server", r.id);
        s.round = None;
        PREGAME.store(0, Ordering::Relaxed);
        return;
    }
    // Everyone voted: close 2 s later, but not in the first 6 s (late joiners still get to vote).
    let opened = r.deadline - Duration::from_secs(SECS.load(Ordering::Relaxed) as u64);
    if !r.early && voters > 0 && r.votes.len() >= voters && now - opened >= Duration::from_secs(6) {
        r.early = true;
        r.deadline = r.deadline.min(now + Duration::from_secs(2));
        log!("vote: round {}: everyone voted, closing in 2 s", r.id);
    }
    if now >= r.deadline {
        decide(&mut s);
        return;
    }
    if r.last_send.is_none_or(|t| now - t >= Duration::from_secs(1)) {
        r.last_send = Some(now);
        let m = open_msg(r, voters);
        broadcast(&encode_open(&m));
    }
}

fn decide(s: &mut Server) {
    let r = s.round.take().unwrap();
    let t = tally(&r);
    let total: u32 = t.iter().map(|&v| v as u32).sum();
    let choices = if r.none_allowed { 4 } else { 3 };
    let pick = if total == 0 {
        (playlist::rand_u64() % 3) as u8
    } else {
        let max = *t[..choices].iter().max().unwrap();
        let tied: Vec<u8> = (0..choices as u8).filter(|&k| t[k as usize] == max).collect();
        tied[(playlist::rand_u64() % tied.len() as u64) as usize]
    };
    log!("vote: round {} closed, tally {}/{}/{}/{} ({} vote(s)){}", r.id, t[0], t[1], t[2], t[3], total, if total == 0 { ", no votes: random option" } else { "" });
    if r.reason == Reason::Called && (pick == NONE || total == 0) {
        // Called mid-game: "Keep playing" won, or nobody voted: carry on, never a random map.
        let p = playlist::get().unwrap();
        let e = p.current;
        log!("vote: round {}: {}; keeping \"{}\"", r.id, if total == 0 { "no votes" } else { "keep playing won" }, p.entries[e].name);
        let msg = ResultMsg { id: r.id, winner: NONE, restart: false, difficulty: difficulty_byte(p.entries[e].difficulty.as_deref()), opt: opt_of(e) };
        s.result = Some((msg, 3, Instant::now(), None));
        return;
    }
    if pick == NONE {
        log!("vote: \"none of the above\" won, re-rolling once");
        let exclude = r.options.clone();
        open_round(s, r.reason, false, &exclude);
        return;
    }
    let p = playlist::get().unwrap();
    let e = r.options[pick as usize];
    let restart = e != p.current;
    log!("vote: winner {} \"{}\" ({} on {}){}", pick + 1, p.entries[e].name, p.entries[e].mode, p.entries[e].map,
        if restart { "; restarting the server into it" } else { "; already playing it, carrying on" });
    let msg = ResultMsg { id: r.id, winner: pick, restart, difficulty: difficulty_byte(p.entries[e].difficulty.as_deref()), opt: opt_of(e) };
    s.result = Some((msg, 3, Instant::now(), restart.then_some(e)));
    if !restart && r.reason == Reason::PreGame {
        PREGAME.store(2, Ordering::Relaxed); // release launch state 8
    }
}

fn record_cast(pkt: &[u8], from: SocketAddr) {
    if pkt.len() < 9 || !crate::net::is_peer_addr(from) {
        return;
    }
    let (id, choice) = (u16::from_le_bytes([pkt[6], pkt[7]]), pkt[8]);
    let mut s = SERVER.lock().unwrap();
    let voters = crate::net::active_peer_addrs(Duration::from_secs(4)).len();
    let Some(r) = s.round.as_mut() else { return };
    if r.id != id || choice > NONE || (choice == NONE && !r.none_allowed) {
        return;
    }
    if r.votes.insert(from, choice) != Some(choice) {
        let t = tally(r);
        log!("vote: {from} voted {} (tally {}/{}/{}/{})", choice + 1, t[0], t[1], t[2], t[3]);
        // Echo the new tally at once.
        let m = open_msg(r, voters);
        r.last_send = Some(Instant::now());
        drop(s);
        broadcast(&encode_open(&m));
    }
}

// ---- client -------------------------------------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
pub struct View {
    pub open: Option<OpenMsg>,
    /// When the last OPEN arrived (the panel hides 3 s after they stop).
    pub open_at: Option<Instant>,
    /// When this round's first OPEN arrived.
    pub round_at: Option<Instant>,
    pub mine: Option<u8>,
    pub result: Option<(ResultMsg, Instant)>,
}

static CLIENT: Mutex<View> = Mutex::new(View { open: None, open_at: None, round_at: None, mine: None, result: None });
static FOLLOWING: AtomicBool = AtomicBool::new(false);

/// Client: what the vote panel shows (None = nothing to show).
pub fn view() -> Option<View> {
    let v = CLIENT.lock().unwrap().clone();
    let open = v.open_at.is_some_and(|t| t.elapsed() < Duration::from_secs(3));
    let res = v.result.as_ref().is_some_and(|(_, t)| t.elapsed() < Duration::from_secs(10)) || FOLLOWING.load(Ordering::Relaxed);
    (open || res || call_notice().is_some()).then_some(v)
}

/// Client: a vote restart is being followed (main.rs must not return to the menu meanwhile).
pub fn following() -> bool {
    FOLLOWING.load(Ordering::Relaxed)
}

/// Client: when this player last asked the server for a vote (pause menu).
static CALLED_AT: Mutex<Option<Instant>> = Mutex::new(None);

/// Client joined to a server (not hosting): the pause menu offers VOTE FOR A NEW GAME.
pub fn can_call() -> bool {
    !IS_SERVER.load(Ordering::Relaxed) && crate::net::host_addr().is_some()
}

/// Client: ask the server to open a vote now. A server without a playlist (or an older one) ignores it;
/// `call_notice` then says so.
pub fn call() {
    if let Some(h) = crate::net::host_addr() {
        crate::net::send_raw(h, &header(KIND_CALL, 0));
        *CALLED_AT.lock().unwrap() = Some(Instant::now());
        log!("vote: asked the server for a vote");
    }
}

/// Client: text for a short notice after `call` (None = nothing to say).
pub fn call_notice() -> Option<&'static str> {
    let at = (*CALLED_AT.lock().unwrap())?;
    let v = CLIENT.lock().unwrap();
    let answered = v.round_at.is_some_and(|r| r >= at) || v.result.as_ref().is_some_and(|(_, r)| *r >= at);
    let e = at.elapsed();
    if answered || e > Duration::from_secs(9) {
        None
    } else if e < Duration::from_secs(3) {
        Some("Asking the server for a vote ...")
    } else {
        Some("No vote: this server has no playlist voting, or a vote just ran")
    }
}

/// Client: vote for option `choice` (0..2, 3 = none of the above). False if no vote is open or it is not offered.
pub fn cast(choice: u8) -> bool {
    let mut v = CLIENT.lock().unwrap();
    let Some(o) = &v.open else { return false };
    if v.open_at.is_none_or(|t| t.elapsed() > Duration::from_secs(3)) || (choice as usize >= o.options.len() && !(choice == NONE && o.none_allowed)) {
        return false;
    }
    let id = o.id;
    v.mine = Some(choice);
    drop(v);
    if let Some(h) = crate::net::host_addr() {
        crate::net::send_raw(h, &encode_cast(id, choice));
    }
    log!("vote: cast {} in round {id}", choice + 1);
    true
}

/// net.rs receive thread: true when `pkt` is a vote datagram (handled here, never queued for the engine).
pub fn handle(_sock: &UdpSocket, pkt: &[u8], from: SocketAddr) -> bool {
    if pkt.len() < 8 || pkt[..4] != MAGIC {
        return false;
    }
    if pkt[4] != VERSION {
        return true;
    }
    if IS_SERVER.load(Ordering::Relaxed) {
        if pkt[5] == KIND_CAST {
            record_cast(pkt, from);
        } else if pkt[5] == KIND_CALL && crate::net::is_peer_addr(from) {
            // Ignored while a round or its result is running (tick only opens when there is none).
            if OPEN_REQ.compare_exchange(0, 3, Ordering::Relaxed, Ordering::Relaxed).is_ok() {
                log!("vote: {from} asked for a vote");
            }
        }
        return true;
    }
    if crate::net::host_addr() != Some(from) {
        return true;
    }
    match pkt[5] {
        KIND_OPEN => {
            if let Some(m) = decode_open(pkt) {
                let mut v = CLIENT.lock().unwrap();
                if v.open.as_ref().is_none_or(|o| o.id != m.id) {
                    log!("vote: round {} open from the server: {}{} ({} s)", m.id,
                        m.options.iter().enumerate().map(|(k, o)| format!("{} \"{}\"", k + 1, o.name)).collect::<Vec<_>>().join(", "),
                        if m.none_allowed { ", 4 none of the above" } else { "" }, m.secs_left);
                    v.mine = None;
                    v.result = None;
                    v.round_at = Some(Instant::now());
                }
                let since = v.round_at.map(|t| t.elapsed()).unwrap_or_default();
                v.open = Some(m);
                v.open_at = Some(Instant::now());
                let auto = (v.mine.is_none() && since >= Duration::from_secs(2))
                    .then(|| std::env::var("REACH_VOTE_AUTO").ok().and_then(|s| s.parse::<u8>().ok()))
                    .flatten();
                drop(v);
                // Test aid: REACH_VOTE_AUTO=n votes for option n (1-4) two seconds into each round.
                if let Some(n) = auto.filter(|n| (1..=4).contains(n)) {
                    cast(n - 1);
                }
            }
        }
        KIND_RESULT => {
            if let Some(m) = decode_result(pkt) {
                let mut v = CLIENT.lock().unwrap();
                if v.result.as_ref().is_some_and(|(r, _)| r.id == m.id) {
                    return true;
                }
                log!("vote: result: option {} \"{}\" ({} on {}){}", m.winner + 1, m.opt.name, crate::query::mode_name(m.opt.mode), m.opt.map,
                    if m.restart { ", the server restarts into it; following" } else { "" });
                v.open = None;
                v.open_at = None;
                v.result = Some((m.clone(), Instant::now()));
                drop(v);
                if m.restart && !FOLLOWING.swap(true, Ordering::Relaxed) {
                    std::thread::spawn(move || follow(m));
                }
            }
        }
        _ => {}
    }
    true
}

/// Arguments this process's game was launched with. A game started from the menu runs in the same process with the
/// menu's join/host flags appended, so the process arguments alone would relaunch into a local game (main.rs sets it).
pub static LAUNCH_ARGS: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();

/// Client: the server restarts into `m`. Wait until it answers the info query with the new map and variant, then
/// relaunch this exe with the new mode flags (same identity, bind and peer) and exit.
fn follow(m: ResultMsg) {
    let Some(host) = crate::net::host_addr() else { return };
    let argv: Vec<String> = LAUNCH_ARGS.get().cloned().unwrap_or_else(|| std::env::args().skip(1).collect());
    let mcc = argv.iter().position(|a| a == "--mcc").and_then(|i| argv.get(i + 1)).map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("MCC_PATH").map(Into::into)).unwrap_or_default();
    // --join-server picks the map from the master listing again; otherwise swap the mode flags.
    let new_argv = if argv.iter().any(|a| a == "--join-server") {
        argv.clone()
    } else {
        let e = playlist::Entry {
            name: m.opt.name.clone(),
            mode: match m.opt.mode {
                crate::query::MODE_FIREFIGHT => "firefight",
                crate::query::MODE_CAMPAIGN => "campaign",
                _ => "slayer",
            }
            .into(),
            map: m.opt.map.clone(),
            game_variant: (!m.opt.game_variant.is_empty()).then(|| m.opt.game_variant.clone()),
            map_variant: None, // joiners get the map variant from the session
            difficulty: Some(["easy", "normal", "heroic", "legendary"][m.difficulty.min(3) as usize].into()),
            weight: 1,
        };
        playlist::replace_mode_args(&argv, &playlist::entry_args(&e, &mcc))
    };
    std::thread::sleep(Duration::from_secs(2)); // the old server sends the result 3 times over ~1 s, then exits
    let t0 = Instant::now();
    loop {
        if let Some(r) = &crate::query::query_many(&[host], Duration::from_millis(800))[0] {
            if r.info.map == m.opt.map && (m.opt.game_variant.is_empty() || r.info.game_variant == m.opt.game_variant) {
                log!("vote: server is back on {} after {:.1} s; relaunching to rejoin", r.info.map, t0.elapsed().as_secs_f32());
                break;
            }
        }
        if t0.elapsed() > Duration::from_secs(120) {
            log!("vote: server did not come back on {} within 120 s; relaunching anyway", m.opt.map);
            break;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    let exe = std::env::current_exe().unwrap_or_default();
    let _ = std::fs::copy(exe.with_file_name("reach-host.log"), exe.with_file_name("reach-host-prevote.log"));
    match std::process::Command::new(&exe).args(&new_argv).env("REACH_VOTE_PREV_PID", std::process::id().to_string()).spawn() {
        Ok(c) => log!("vote: relaunched as process {} with {}", c.id(), new_argv.join(" ")),
        Err(e) => log!("vote: could not relaunch {}: {e}", exe.display()),
    }
    // Not process::exit: its engine/DLL teardown can hang under Wine, leaving this game's window open next to the
    // relaunched one. Nothing here needs flushing (the log is written per line).
    unsafe {
        use windows::Win32::System::Threading::{GetCurrentProcess, TerminateProcess};
        let _ = TerminateProcess(GetCurrentProcess(), 0);
    }
    std::process::exit(0);
}

/// Start of a process relaunched by `follow`: wait (up to 10 s) for the old one to exit so its UDP port is free.
pub fn await_previous() {
    use windows::Win32::System::Threading::{OpenProcess, TerminateProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE};
    let Some(pid) = std::env::var("REACH_VOTE_PREV_PID").ok().and_then(|v| v.parse::<u32>().ok()) else { return };
    std::env::remove_var("REACH_VOTE_PREV_PID");
    unsafe {
        if let Ok(h) = OpenProcess(PROCESS_SYNCHRONIZE | PROCESS_TERMINATE, false, pid) {
            let r = WaitForSingleObject(h, 10_000);
            if r.0 == 0 {
                log!("vote: previous process {pid} exited");
            } else {
                // Stuck in its exit: end it, or its window stays open next to ours.
                let k = TerminateProcess(h, 0);
                let gone = WaitForSingleObject(h, 3_000).0 == 0;
                log!("vote: previous process {pid} still running after 10 s; terminated ({k:?}, gone {gone})");
            }
            let _ = windows::Win32::Foundation::CloseHandle(h);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let o = OpenMsg {
            id: 7,
            secs_left: 12,
            none_allowed: true,
            keep: true,
            voters: 2,
            tally: [0, 2, 0, 1],
            playlist: "Slayer".into(),
            options: vec![Opt { mode: 0, name: "A".into(), map: "forge_world".into(), game_variant: "slayer_054.bin".into() }; 3],
        };
        let d = decode_open(&encode_open(&o)).unwrap();
        assert!(d.keep && d.none_allowed);
        assert_eq!((d.id, d.secs_left, d.none_allowed, d.voters, d.tally, d.options.len()), (7, 12, true, 2, [0, 2, 0, 1], 3));
        assert_eq!(d.options[2], o.options[2]);
        let r = ResultMsg { id: 7, winner: 1, restart: true, difficulty: 2, opt: o.options[0].clone() };
        let e = decode_result(&encode_result(&r)).unwrap();
        assert_eq!((e.id, e.winner, e.restart, e.difficulty, e.opt.map.as_str()), (7, 1, true, 2, "forge_world"));
        let c = encode_cast(7, 1);
        assert_eq!((&c[..4], c[5], c[8]), (&MAGIC[..], KIND_CAST, 1));
    }
}
