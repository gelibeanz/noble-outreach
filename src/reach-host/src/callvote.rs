//! In-game call votes (Reclaimer style). A player calls a vote from the pause menu (ui/overlay.rs) or a scoreboard
//! row; everyone answers with F5 (yes) / F6 (no) within 30 s; a majority of the connected players passes it (the
//! caller counts as yes, a kick target does not vote). One call vote at a time, a cooldown per caller. Kinds:
//! shuffle and balance the teams, kick a player (refused for the rest of this host process), skip to the next game
//! (playlist servers: opens the next-game vote, vote.rs), restart the game (dedicated servers: the same game again,
//! clients follow as after a vote restart).
//! Runs on any host, listen or dedicated: clients send calls and ballots, the host decides on its game thread
//! (`game_tick`, from vote::game_tick) and broadcasts the status every second. Wire format: the RVOT side channel,
//! kinds 5-11 (notes/networking.md "Vote").

use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::log;
use crate::vote::{header, put_str, Rd};

pub const KIND_FIRST: u8 = 5;
const KIND_CALL: u8 = 5; // client -> host: call a vote (kind, target)
const KIND_BALLOT: u8 = 6; // client -> host: yes / no on call `id`
const KIND_STATUS: u8 = 7; // host -> clients: the open call, every 1 s and after each ballot
const KIND_OUTCOME: u8 = 8; // host -> clients: passed / failed (everyone), refused (the caller only)
const KIND_CAPS_REQ: u8 = 9; // client -> host: which kinds can I call?
const KIND_CAPS: u8 = 10; // host -> client: kinds mask, my cooldown, busy
const KIND_KICKED: u8 = 11; // host -> the kicked client
pub const KIND_LAST: u8 = 11;

/// Seconds a call vote stays open.
pub const SECS: u64 = 30;
/// A player can call again this long after their last call.
const COOLDOWN: Duration = Duration::from_secs(60);

pub const FAILED: u8 = 0;
pub const PASSED: u8 = 1;
pub const REFUSED: u8 = 2;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CallKind {
    Shuffle,
    Kick,
    Skip,
    Restart,
}

pub const ALL: [CallKind; 4] = [CallKind::Shuffle, CallKind::Kick, CallKind::Skip, CallKind::Restart];

impl CallKind {
    pub fn from_byte(b: u8) -> Option<Self> {
        ALL.get(b as usize).copied()
    }
    fn bit(self) -> u8 {
        1 << self as u8
    }
    pub fn title(self) -> &'static str {
        match self {
            CallKind::Shuffle => "Shuffle teams",
            CallKind::Kick => "Kick player",
            CallKind::Skip => "Skip to next game",
            CallKind::Restart => "Restart game",
        }
    }
    /// The question on the vote panel.
    pub fn question(self, target: &str) -> String {
        match self {
            CallKind::Shuffle => "Shuffle and balance the teams?".into(),
            CallKind::Kick => format!("Kick {target}?"),
            CallKind::Skip => "End this game and vote for the next one?".into(),
            CallKind::Restart => "Restart this game from the start?".into(),
        }
    }
    /// Why the host cannot run this kind right now (pause menu, greyed rows).
    pub fn unavailable(self) -> &'static str {
        match self {
            CallKind::Shuffle => "Needs a team game with 2+ players",
            CallKind::Kick => "Nobody else to kick",
            CallKind::Skip => "Needs a server with a playlist",
            CallKind::Restart => "Needs a dedicated server",
        }
    }
    fn parse(s: &str) -> Option<Self> {
        match s {
            "shuffle" => Some(CallKind::Shuffle),
            "kick" => Some(CallKind::Kick),
            "skip" => Some(CallKind::Skip),
            "restart" => Some(CallKind::Restart),
            _ => None,
        }
    }
}

/// The open call vote, as broadcast.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Status {
    pub id: u16,
    pub kind: u8,
    pub secs_left: u8,
    pub yes: u8,
    pub no: u8,
    /// Players who may vote (connected, minus a kick target).
    pub voters: u8,
    /// Yes votes needed to pass (a majority of `voters`).
    pub needed: u8,
    pub caller: String,
    pub target: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Outcome {
    pub id: u16,
    pub kind: u8,
    /// FAILED, PASSED or REFUSED.
    pub result: u8,
    pub yes: u8,
    pub no: u8,
    pub caller: String,
    pub target: String,
    /// What happened / why it was refused.
    pub text: String,
}

// ---- wire ---------------------------------------------------------------------------------------------------

fn id_of(p: &[u8]) -> u16 {
    u16::from_le_bytes([p[6], p[7]])
}

pub fn encode_call(kind: CallKind, target: &str) -> Vec<u8> {
    let mut o = header(KIND_CALL, 0);
    o.extend_from_slice(&[kind as u8, 0, 0, 0]);
    put_str(&mut o, target, 32);
    o
}

fn decode_call(p: &[u8]) -> Option<(u8, String)> {
    let mut r = Rd(p, 8);
    let k = r.u8()?;
    r.1 += 3;
    Some((k, r.str()?))
}

pub fn encode_ballot(id: u16, yes: bool) -> Vec<u8> {
    let mut o = header(KIND_BALLOT, id);
    o.extend_from_slice(&[yes as u8, 0, 0, 0]);
    o
}

pub fn encode_status(m: &Status) -> Vec<u8> {
    let mut o = header(KIND_STATUS, m.id);
    o.extend_from_slice(&[m.kind, m.secs_left, m.yes, m.no, m.voters, m.needed, 0, 0]);
    put_str(&mut o, &m.caller, 32);
    put_str(&mut o, &m.target, 32);
    o
}

fn decode_status(p: &[u8]) -> Option<Status> {
    let mut r = Rd(p, 8);
    let b: Vec<u8> = (0..8).map(|_| r.u8()).collect::<Option<_>>()?;
    Some(Status { id: id_of(p), kind: b[0], secs_left: b[1], yes: b[2], no: b[3], voters: b[4], needed: b[5], caller: r.str()?, target: r.str()? })
}

pub fn encode_outcome(m: &Outcome) -> Vec<u8> {
    let mut o = header(KIND_OUTCOME, m.id);
    o.extend_from_slice(&[m.kind, m.result, m.yes, m.no]);
    put_str(&mut o, &m.caller, 32);
    put_str(&mut o, &m.target, 32);
    put_str(&mut o, &m.text, 120);
    o
}

fn decode_outcome(p: &[u8]) -> Option<Outcome> {
    let mut r = Rd(p, 8);
    let (kind, result, yes, no) = (r.u8()?, r.u8()?, r.u8()?, r.u8()?);
    Some(Outcome { id: id_of(p), kind, result, yes, no, caller: r.str()?, target: r.str()?, text: r.str()? })
}

fn encode_caps(mask: u8, cooldown: u8, busy: bool) -> Vec<u8> {
    let mut o = header(KIND_CAPS, 0);
    o.extend_from_slice(&[mask, cooldown, busy as u8, 0]);
    o
}

fn encode_kicked(text: &str) -> Vec<u8> {
    let mut o = header(KIND_KICKED, 0);
    o.extend_from_slice(&[0, 0, 0, 0]);
    put_str(&mut o, text, 120);
    o
}

// ---- engine player table (game thread) ------------------------------------------------------------------------

/// One player of the engine's table (TLS +0x18 data array, datums of 0x490 bytes; notes/frontend.md "Tab scoreboard").
pub struct Player {
    pub datum: usize,
    pub name: String,
    pub team: i8,
    pub xuid: u64,
}

impl Player {
    /// Our instance number (xuid = player 0 xuid + instance, options.rs) and so its net id.
    fn net_id(&self) -> u64 {
        crate::net::net_id(self.xuid.wrapping_sub(crate::options::xuid(0)) as u32)
    }
    fn is_host(&self) -> bool {
        self.xuid == crate::options::xuid(0)
    }
}

/// Engine TLS game-state block of the calling thread (game thread only).
unsafe fn game_tls(base: usize) -> usize {
    let idx = ((base + 0xc17b18) as *const u32).read() as usize;
    let tls: *const usize;
    core::arch::asm!("mov {}, gs:[0x58]", out(reg) tls);
    if tls.is_null() {
        return 0;
    }
    *tls.add(idx)
}

unsafe fn players(base: usize) -> Vec<Player> {
    let block = game_tls(base);
    let mut out = Vec::new();
    if block == 0 {
        return out;
    }
    let arr = ((block + 0x18) as *const usize).read();
    if arr == 0 {
        return out;
    }
    let data = ((arr + 0x50) as *const usize).read();
    if data == 0 {
        return out;
    }
    for i in 0..16usize {
        let p = data + i * 0x490;
        if (p as *const u16).read_volatile() == 0 {
            continue;
        }
        let name: Vec<u16> = (0..16).map(|k| ((p + 0xb0 + 2 * k) as *const u16).read_volatile()).take_while(|&c| c != 0).collect();
        out.push(Player {
            datum: p,
            name: String::from_utf16_lossy(&name),
            team: ((p + 0xad) as *const i8).read_volatile(),
            xuid: ((p + 0x08) as *const u64).read_unaligned(),
        });
    }
    out
}

// ---- host -------------------------------------------------------------------------------------------------------

/// The listen host's own calls and ballots (it has no address of its own).
const LOCAL: SocketAddr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0));

struct Req {
    from: SocketAddr,
    kind: u8,
    target: String,
}

struct Round {
    id: u16,
    kind: CallKind,
    target: String,
    target_id: Option<u64>,
    caller: String,
    opened: Instant,
    ballots: HashMap<SocketAddr, bool>,
    last_send: Option<Instant>,
    dirty: bool,
}

struct Server {
    reqs: Vec<Req>,
    round: Option<Round>,
    next_id: u16,
    last_call: Vec<(SocketAddr, Instant)>,
    /// Datagrams to resend: (packet, None = every peer, sends left, next send).
    sends: Vec<(Vec<u8>, Option<SocketAddr>, u8, Instant)>,
    /// Kicked names: kicked again if they come back under a new id (a `--join-server` joiner picks a new instance).
    banned: Vec<String>,
    caps_at: Option<Instant>,
}

static SERVER: Mutex<Server> = Mutex::new(Server { reqs: Vec::new(), round: None, next_id: 1, last_call: Vec::new(), sends: Vec::new(), banned: Vec::new(), caps_at: None });
/// After a shuffle: game-tick passes until the team bytes are logged again (0 = off).
static SHUFFLE_CHECK: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Kinds this host can run now (bit per CallKind), refreshed every second on the game thread.
static CAPS: AtomicU8 = AtomicU8::new(0);

fn dedicated() -> bool {
    crate::host::DEDICATED.load(Ordering::Relaxed)
}

/// Who may vote on a call against `target_id`: peers heard from in the last 4 s (minus the target), plus the
/// listen host's own player.
fn electorate(target_id: Option<u64>) -> Vec<SocketAddr> {
    let mut v = crate::net::active_peer_addrs(Duration::from_secs(4));
    if let Some(t) = target_id.and_then(crate::net::peer_addr_of) {
        v.retain(|a| *a != t);
    }
    if !dedicated() {
        v.push(LOCAL);
    }
    v
}

fn needed(voters: usize) -> usize {
    voters / 2 + 1
}

fn cooldown_left(s: &Server, from: SocketAddr) -> Duration {
    s.last_call.iter().find(|(a, _)| *a == from).map_or(Duration::ZERO, |(_, t)| COOLDOWN.saturating_sub(t.elapsed()))
}

fn caps_of(ps: &[Player]) -> u8 {
    let mp = crate::query::info().is_some_and(|i| i.mode == crate::query::MODE_MULTIPLAYER);
    let mut m = 0;
    if mp && ps.iter().filter(|p| p.team >= 0).count() >= 2 {
        m |= CallKind::Shuffle.bit();
    }
    if ps.iter().any(|p| !p.is_host()) && ps.len() >= 2 {
        m |= CallKind::Kick.bit();
    }
    if crate::vote::IS_SERVER.load(Ordering::Relaxed) {
        m |= CallKind::Skip.bit();
    }
    if dedicated() {
        m |= CallKind::Restart.bit();
    }
    m
}

fn name_of(ps: &[Player], from: SocketAddr) -> String {
    let id = if from == LOCAL { Some(crate::net::net_id(0)) } else { crate::net::peer_id_of(from) };
    id.and_then(|id| ps.iter().find(|p| p.net_id() == id)).map_or_else(|| if from == LOCAL { "Host".into() } else { "A player".into() }, |p| p.name.clone())
}

/// Send `pkt` to `to` (None = every peer) 3 times over ~1 s; the listen host's own copy is delivered at once.
fn send3(s: &mut Server, pkt: Vec<u8>, to: Option<SocketAddr>) {
    if to.is_none() || to == Some(LOCAL) {
        if !dedicated() {
            receive(&pkt);
        }
        if to == Some(LOCAL) {
            return;
        }
    }
    s.sends.push((pkt, to, 3, Instant::now()));
}

fn status_of(r: &Round) -> Status {
    let voters = electorate(r.target_id);
    let yes = voters.iter().filter(|a| r.ballots.get(a) == Some(&true)).count();
    let no = voters.iter().filter(|a| r.ballots.get(a) == Some(&false)).count();
    Status {
        id: r.id,
        kind: r.kind as u8,
        secs_left: (SECS as f32 - r.opened.elapsed().as_secs_f32()).ceil().clamp(0.0, 255.0) as u8,
        yes: yes as u8,
        no: no as u8,
        voters: voters.len() as u8,
        needed: needed(voters.len()) as u8,
        caller: r.caller.clone(),
        target: r.target.clone(),
    }
}

fn refuse(s: &mut Server, req: &Req, kind: u8, text: &str) {
    log!("callvote: refused {} from {}: {text}", CallKind::from_byte(kind).map_or("?", |k| k.title()), req.from);
    let o = Outcome { id: 0, kind, result: REFUSED, text: text.into(), target: req.target.clone(), ..Default::default() };
    let pkt = encode_outcome(&o);
    send3(s, pkt, Some(req.from));
}

/// Game thread, every loop pass on a host (vote::game_tick).
pub unsafe fn game_tick(base: usize) {
    if !crate::net::is_host() {
        return;
    }
    let mut s = SERVER.lock().unwrap();
    let now = Instant::now();
    // Resends.
    for (pkt, to, left, at) in s.sends.iter_mut() {
        if now >= *at {
            match to {
                Some(a) => crate::net::send_raw(*a, pkt),
                None => _ = crate::vote::broadcast(pkt),
            }
            *left -= 1;
            *at = now + Duration::from_millis(400);
        }
    }
    s.sends.retain(|x| x.2 > 0);
    let check = SHUFFLE_CHECK.load(Ordering::Relaxed);
    if check > 0 {
        SHUFFLE_CHECK.store(if check >= 300 { 0 } else { check + 1 }, Ordering::Relaxed);
        if check == 300 {
            log!("callvote: shuffle: after {}", team_bytes(base));
        }
    }
    let caps_due = s.caps_at.is_none_or(|t| now - t >= Duration::from_secs(1));
    if s.reqs.is_empty() && s.round.is_none() && !caps_due {
        return;
    }
    let ps = players(base);
    if caps_due {
        s.caps_at = Some(now);
        CAPS.store(caps_of(&ps), Ordering::Relaxed);
        // A kicked player back under a new id: drop them again.
        let back: Vec<&Player> = ps.iter().filter(|p| !p.is_host() && s.banned.iter().any(|b| b.eq_ignore_ascii_case(&p.name))).collect();
        for p in back {
            if crate::net::peer_addr_of(p.net_id()).is_some() {
                log!("callvote: kicked player {} came back; dropping them again", p.name);
                if let Some(a) = crate::net::kick(p.net_id()) {
                    let pkt = encode_kicked("You were kicked from this server by a vote.");
                    s.sends.push((pkt, Some(a), 3, now));
                }
            }
        }
    }
    // New calls.
    for req in std::mem::take(&mut s.reqs) {
        let Some(kind) = CallKind::from_byte(req.kind) else { continue };
        let caller = name_of(&ps, req.from);
        if s.round.is_some() {
            refuse(&mut s, &req, req.kind, "A vote is already running");
            continue;
        }
        let wait = cooldown_left(&s, req.from);
        if !wait.is_zero() {
            refuse(&mut s, &req, req.kind, &format!("You can call another vote in {} s", wait.as_secs() + 1));
            continue;
        }
        if CAPS.load(Ordering::Relaxed) & kind.bit() == 0 {
            refuse(&mut s, &req, req.kind, kind.unavailable());
            continue;
        }
        let mut target_id = None;
        let mut target = String::new();
        if kind == CallKind::Kick {
            match ps.iter().find(|p| p.name.eq_ignore_ascii_case(req.target.trim())) {
                None => {
                    refuse(&mut s, &req, req.kind, &format!("No player called \"{}\"", req.target));
                    continue;
                }
                Some(p) if p.is_host() => {
                    refuse(&mut s, &req, req.kind, "The host cannot be kicked");
                    continue;
                }
                Some(p) if p.name == caller => {
                    refuse(&mut s, &req, req.kind, "You cannot kick yourself");
                    continue;
                }
                Some(p) => {
                    target_id = Some(p.net_id());
                    target = p.name.clone();
                }
            }
        }
        let id = s.next_id;
        s.next_id = s.next_id.wrapping_add(1).max(1);
        s.last_call.retain(|(a, _)| *a != req.from);
        s.last_call.push((req.from, now));
        let mut ballots = HashMap::new();
        ballots.insert(req.from, true);
        log!("callvote: {id} open: {} called by {caller}{}", kind.title(), if target.is_empty() { String::new() } else { format!(" against {target}") });
        s.round = Some(Round { id, kind, target, target_id, caller, opened: now, ballots, last_send: None, dirty: true });
    }
    // The open call.
    let Some(r) = s.round.as_mut() else { return };
    let st = status_of(r);
    let pass = st.yes >= st.needed;
    let fail = st.no as usize > (st.voters as usize).saturating_sub(st.needed as usize) || r.opened.elapsed() >= Duration::from_secs(SECS);
    if !pass && !fail {
        if r.dirty || r.last_send.is_none_or(|t| now - t >= Duration::from_secs(1)) {
            r.dirty = false;
            r.last_send = Some(now);
            let pkt = encode_status(&st);
            crate::vote::broadcast(&pkt);
            if !dedicated() {
                receive(&pkt);
            }
        }
        return;
    }
    let r = s.round.take().unwrap();
    log!("callvote: {} {}: {} yes, {} no of {} voters ({} needed)", r.id, if pass { "passed" } else { "failed" }, st.yes, st.no, st.voters, st.needed);
    let text = if pass { apply(base, &mut s, &r, &ps) } else if r.opened.elapsed() >= Duration::from_secs(SECS) { "Not enough votes in time".into() } else { "Voted down".into() };
    let o = Outcome { id: r.id, kind: r.kind as u8, result: if pass { PASSED } else { FAILED }, yes: st.yes, no: st.no, caller: r.caller.clone(), target: r.target.clone(), text };
    send3(&mut s, encode_outcome(&o), None);
}

/// A passed call: do it (game thread). Returns the line shown under the result.
unsafe fn apply(base: usize, s: &mut Server, r: &Round, ps: &[Player]) -> String {
    match r.kind {
        CallKind::Shuffle => shuffle(base, ps),
        CallKind::Kick => {
            let Some(id) = r.target_id else { return "Nobody to kick".into() };
            s.banned.push(r.target.clone());
            match crate::net::kick(id) {
                Some(a) => {
                    let pkt = encode_kicked("You were kicked from this server by a vote.");
                    s.sends.push((pkt, Some(a), 3, Instant::now()));
                    log!("callvote: kicked {} ({a})", r.target);
                    format!("{} was kicked", r.target)
                }
                None => format!("{} already left", r.target),
            }
        }
        CallKind::Skip => {
            if crate::vote::open_skip() {
                "Vote for the next game".into()
            } else {
                "A game vote is already running".into()
            }
        }
        CallKind::Restart => {
            if crate::vote::restart_same() {
                "Restarting the game ...".into()
            } else {
                "The server is already changing games".into()
            }
        }
    }
}

// ---- team shuffle -----------------------------------------------------------------------------------------------

/// Teams the shuffle uses: red, blue.
const TEAMS: [i8; 2] = [0, 1];

/// New team per player (same order as `ps`): a random order dealt alternately to red and blue, so team sizes differ
/// by at most one. Tries a few orders for one that changes something.
fn deal(ps: &[(String, i8)], mut rand: impl FnMut() -> u64) -> Vec<i8> {
    let mut best = Vec::new();
    for _ in 0..8 {
        let mut order: Vec<usize> = (0..ps.len()).collect();
        for i in (1..order.len()).rev() {
            order.swap(i, (rand() % (i as u64 + 1)) as usize);
        }
        let first = (rand() % 2) as usize;
        let mut t = vec![0i8; ps.len()];
        for (k, &i) in order.iter().enumerate() {
            t[i] = TEAMS[(k + first) % 2];
        }
        let changed = t.iter().zip(ps).any(|(a, b)| *a != b.1);
        best = t;
        if changed {
            break;
        }
    }
    best
}

unsafe fn shuffle(base: usize, ps: &[Player]) -> String {
    let team_players: Vec<&Player> = ps.iter().filter(|p| p.team >= 0).collect();
    let list: Vec<(String, i8)> = team_players.iter().map(|p| (p.name.clone(), p.team)).collect();
    let teams = deal(&list, crate::playlist::rand_u64);
    log!("callvote: shuffle: before {}", team_bytes(base));
    let moves: Vec<(&Player, i8)> = team_players.iter().zip(&teams).filter(|(p, t)| p.team != **t).map(|(p, t)| (*p, *t)).collect();
    let moved = set_teams(base, &moves);
    log!("callvote: shuffle: {} move(s) written: {}", moved, moves.iter().map(|(p, t)| format!("{} {} -> {t}", p.name, p.team)).collect::<Vec<_>>().join(", "));
    SHUFFLE_CHECK.store(1, Ordering::Relaxed); // log the team bytes again a few seconds later (after the respawn)
    if moved == 0 {
        "Teams already balanced".into()
    } else {
        format!("{moved} player(s) changed teams")
    }
}

/// Session object (`*0x182a1fe08`): membership player mask at +0x173c, player records of 0xb0 bytes at +0x1740
/// (+0x04 xuid, +0x0c peer index, +0x20 the player's configuration of 0x80 bytes: name first, team at +0x78 and
/// +0x79), membership update number at +0x38 and the players-changed counter at +0x4468. notes/networking.md "Vote".
const RVA_SESSION: usize = 0x2a1fe08;

/// Membership record of the player with `xuid`.
unsafe fn member(session: usize, xuid: u64) -> Option<usize> {
    let mask = ((session + 0x173c) as *const u32).read_volatile();
    (0..16usize).filter(|i| mask >> i & 1 != 0).map(|i| session + 0x1740 + i * 0xb0).find(|&m| ((m + 4) as *const u64).read_unaligned() == xuid)
}

/// Host: move players to new teams the way the engine's own membership changes do: write each player's team into
/// the session membership, then bump the membership update number (+0x38, replicates the membership to every peer)
/// and the players-changed counter (+0x4468, the simulation then diffs each player's configuration and applies the
/// change on every machine: the unit dies and respawns on the new team). All moves go in one bump: a team that is
/// empty even for a moment ends the game. Returns how many players were written.
unsafe fn set_teams(base: usize, moves: &[(&Player, i8)]) -> usize {
    let session = ((base + RVA_SESSION) as *const usize).read_volatile();
    if session == 0 {
        return 0;
    }
    let mut n = 0;
    for (p, t) in moves {
        match member(session, p.xuid) {
            Some(m) => {
                ((m + 0x20 + 0x78) as *mut i8).write_volatile(*t);
                ((m + 0x20 + 0x79) as *mut i8).write_volatile(*t);
                n += 1;
            }
            None => log!("callvote: {} is not in the session membership; team not changed", p.name),
        }
    }
    if n > 0 {
        for off in [0x38, 0x4468] {
            let u = (session + off) as *mut u32;
            u.write_volatile(u.read_volatile().wrapping_add(1));
        }
    }
    n
}

/// Every player's team fields, for the log: name=+0xAD/+0x128/+0x129/+0x1A8/+0x1A9/unit team.
unsafe fn team_bytes(base: usize) -> String {
    let block = game_tls(base);
    players(base)
        .iter()
        .map(|p| {
            let b = |o: usize| ((p.datum + o) as *const i8).read_volatile();
            let unit = ((p.datum + 0x28) as *const u32).read_volatile();
            let ut = object(block, unit).map_or(-2, |o| ((o + 0x1ca) as *const i8).read_volatile());
            format!("{}={}/{}/{}/{}/{}/u{}", p.name, b(0xad), b(0x128), b(0x129), b(0x1a8), b(0x1a9), ut)
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Session membership players (session +0x173c mask, records of 0xb0 at +0x1740: +0x04 xuid, +0x0c peer, +0x20
/// configuration of 0x80 with the name first and the team bytes at +0x78/+0x79), for the log.
unsafe fn members_dump(base: usize) -> String {
    let session = ((base + 0x2a1fe08) as *const usize).read();
    if session == 0 {
        return "no session".into();
    }
    let mask = ((session + 0x173c) as *const u32).read();
    let mut out = format!("mask {mask:#x}");
    for i in 0..16usize {
        if mask >> i & 1 == 0 {
            continue;
        }
        let m = session + 0x1740 + i * 0xb0;
        let n: Vec<u16> = (0..16).map(|k| ((m + 0x20 + 2 * k) as *const u16).read()).take_while(|&c| c != 0).collect();
        out += &format!(" | {i}: {} xuid {:#x} peer {} team {}/{}", String::from_utf16_lossy(&n), ((m + 4) as *const u64).read_unaligned(),
            ((m + 0xc) as *const i32).read_unaligned(), ((m + 0x98) as *const i8).read(), ((m + 0x99) as *const i8).read());
    }
    out
}

/// Object data of handle `h` (TLS +0x10 object array, +0x50 entries of 0x18 bytes, object pointer at +0x10).
unsafe fn object(block: usize, h: u32) -> Option<usize> {
    if block == 0 || h == u32::MAX {
        return None;
    }
    let arr = ((block + 0x10) as *const usize).read();
    if arr == 0 {
        return None;
    }
    let data = ((arr + 0x50) as *const usize).read();
    let o = ((data + (h & 0xffff) as usize * 0x18 + 0x10) as *const usize).read();
    (o != 0).then_some(o)
}

// ---- client -------------------------------------------------------------------------------------------------------

/// What the call-vote panel shows.
#[derive(Clone, Debug, Default)]
pub struct View {
    pub status: Option<Status>,
    /// This player's ballot on the open call.
    pub mine: Option<bool>,
    pub outcome: Option<Outcome>,
}

struct Client {
    status: Option<(Status, Instant)>,
    mine: Option<(u16, bool)>,
    outcome: Option<(Outcome, Instant)>,
    caps: Option<(u8, u8, bool, Instant)>,
    kicked: Option<String>,
    fake: bool,
    /// When this player last called a vote: the next new call counts as its yes.
    called: Option<Instant>,
}

static CLIENT: Mutex<Client> = Mutex::new(Client { status: None, mine: None, outcome: None, caps: None, kicked: None, fake: false, called: None });

/// The panel: the open call (status younger than 3 s), else the last outcome for 6 s.
pub fn view() -> Option<View> {
    fake_tick();
    let c = CLIENT.lock().unwrap();
    let status = c.status.as_ref().filter(|(_, t)| t.elapsed() < Duration::from_secs(3)).map(|(s, _)| s.clone());
    let outcome = c.outcome.as_ref().filter(|(_, t)| t.elapsed() < Duration::from_secs(6)).map(|(o, _)| o.clone());
    if status.is_none() && outcome.is_none() {
        return None;
    }
    let mine = status.as_ref().and_then(|s| c.mine.filter(|m| m.0 == s.id).map(|m| m.1));
    Some(View { status, mine, outcome })
}

/// In a network game (host or joined): the pause menu offers CALL A VOTE.
pub fn in_network_game() -> bool {
    crate::net::is_host() || crate::net::host_addr().is_some()
}

/// Call a vote. Listen host / server: queued for its own game thread; client: sent to the host. False when not in a
/// network game. The answer comes as a status (vote opens) or an outcome (refused, with the reason).
pub fn call_vote(kind: CallKind, target: Option<&str>) -> bool {
    let t = target.unwrap_or("").to_string();
    CLIENT.lock().unwrap().called = Some(Instant::now());
    if crate::net::is_host() {
        log!("callvote: calling {} {t}", kind.title());
        SERVER.lock().unwrap().reqs.push(Req { from: LOCAL, kind: kind as u8, target: t });
        true
    } else if let Some(h) = crate::net::host_addr() {
        crate::net::send_raw(h, &encode_call(kind, &t));
        log!("callvote: asked the host for a vote: {} {t}", kind.title());
        true
    } else {
        false
    }
}

/// Yes / no on the open call. False when none is open.
pub fn cast(yes: bool) -> bool {
    let mut c = CLIENT.lock().unwrap();
    let Some((st, at)) = &c.status else { return false };
    if at.elapsed() > Duration::from_secs(3) {
        return false;
    }
    let id = st.id;
    if c.mine == Some((id, yes)) {
        return true;
    }
    c.mine = Some((id, yes));
    let fake = c.fake;
    drop(c);
    log!("callvote: voted {} on {id}", if yes { "yes" } else { "no" });
    if fake {
        return true;
    }
    if crate::net::is_host() {
        let mut s = SERVER.lock().unwrap();
        if let Some(r) = s.round.as_mut().filter(|r| r.id == id) {
            r.ballots.insert(LOCAL, yes);
            r.dirty = true;
        }
    } else if let Some(h) = crate::net::host_addr() {
        crate::net::send_raw(h, &encode_ballot(id, yes));
    }
    true
}

/// input.rs: F5 (yes) / F6 (no). True = consumed (a call vote is open).
pub fn key(yes: bool) -> bool {
    view().is_some_and(|v| v.status.is_some()) && cast(yes)
}

/// Pause menu opened: ask the host which kinds it can run (a host knows already).
pub fn request_caps() {
    if !crate::net::is_host() {
        if let Some(h) = crate::net::host_addr() {
            crate::net::send_raw(h, &header(KIND_CAPS_REQ, 0));
        }
    }
}

/// (kinds mask, seconds until I may call again, a call is running). None = the host never answered (an older host
/// without call votes).
pub fn caps() -> Option<(u8, u8, bool)> {
    if crate::net::is_host() {
        let s = SERVER.lock().unwrap();
        return Some((CAPS.load(Ordering::Relaxed), cooldown_left(&s, LOCAL).as_secs().min(255) as u8, s.round.is_some()));
    }
    let c = CLIENT.lock().unwrap();
    c.caps.filter(|x| x.3.elapsed() < Duration::from_secs(30)).map(|x| (x.0, x.1, x.2))
}

pub fn kind_available(mask: u8, kind: CallKind) -> bool {
    mask & kind.bit() != 0
}

/// Client: the host kicked us (main.rs returns to the menu with this text).
/// Between two games in one process (seamless.rs): drop the open call, queued requests and this player's vote
/// state, so a later "host left" is not reported as a kick. Bans and cooldowns stay for the process.
pub fn reset() {
    let mut s = SERVER.lock().unwrap();
    s.reqs.clear();
    s.round = None;
    s.sends.clear();
    drop(s);
    let mut c = CLIENT.lock().unwrap();
    let called = c.called;
    *c = Client { status: None, mine: None, outcome: None, caps: None, kicked: None, fake: c.fake, called };
}

pub fn kicked() -> Option<String> {
    CLIENT.lock().unwrap().kicked.clone()
}

/// A host datagram for this client (or the listen host's own copy).
fn receive(pkt: &[u8]) {
    match pkt[5] {
        KIND_STATUS => {
            if let Some(m) = decode_status(pkt) {
                let mut c = CLIENT.lock().unwrap();
                if c.status.as_ref().is_none_or(|(s, _)| s.id != m.id) {
                    log!("callvote: vote {} open: {} (called by {}{}), {} s, {} of {} needed", m.id, CallKind::from_byte(m.kind).map_or("?", |k| k.title()), m.caller,
                        if m.target.is_empty() { String::new() } else { format!(", target {}", m.target) }, m.secs_left, m.needed, m.voters);
                    if c.called.take().is_some_and(|t| t.elapsed() < Duration::from_secs(3)) {
                        c.mine = Some((m.id, true)); // the caller counts as yes
                    }
                }
                c.status = Some((m, Instant::now()));
            }
        }
        KIND_OUTCOME => {
            if let Some(m) = decode_outcome(pkt) {
                let mut c = CLIENT.lock().unwrap();
                if c.outcome.as_ref().is_some_and(|(o, t)| *o == m && t.elapsed() < Duration::from_secs(3)) {
                    return; // resend
                }
                log!("callvote: vote {} {}: {} ({} yes, {} no)", m.id, ["failed", "passed", "refused"].get(m.result as usize).unwrap_or(&"?"), m.text, m.yes, m.no);
                if c.status.as_ref().is_some_and(|(s, _)| s.id == m.id) {
                    c.status = None;
                }
                c.outcome = Some((m, Instant::now()));
            }
        }
        KIND_CAPS if pkt.len() >= 11 => {
            CLIENT.lock().unwrap().caps = Some((pkt[8], pkt[9], pkt[10] != 0, Instant::now()));
        }
        KIND_KICKED => {
            let text = Rd(pkt, 12).str().unwrap_or_default();
            let mut c = CLIENT.lock().unwrap();
            if c.kicked.is_none() {
                log!("callvote: kicked by the host: {text}");
                c.kicked = Some(text);
                crate::net::HOST_LEFT.store(true, Ordering::Relaxed); // main.rs: back to the menu
            }
        }
        _ => {}
    }
}

/// net.rs receive thread (via vote::handle): an RVOT datagram of kinds 5-11.
pub fn handle(pkt: &[u8], from: SocketAddr) {
    if crate::net::is_host() {
        if !crate::net::is_peer_addr(from) || crate::net::is_banned(0, from) {
            return;
        }
        match pkt[5] {
            KIND_CALL => {
                if let Some((kind, target)) = decode_call(pkt) {
                    SERVER.lock().unwrap().reqs.push(Req { from, kind, target });
                }
            }
            KIND_BALLOT if pkt.len() >= 9 => {
                let mut s = SERVER.lock().unwrap();
                if let Some(r) = s.round.as_mut().filter(|r| r.id == id_of(pkt)) {
                    if r.ballots.insert(from, pkt[8] != 0) != Some(pkt[8] != 0) {
                        log!("callvote: {from} voted {} on {}", if pkt[8] != 0 { "yes" } else { "no" }, r.id);
                        r.dirty = true;
                    }
                }
            }
            KIND_CAPS_REQ => {
                let s = SERVER.lock().unwrap();
                let pkt = encode_caps(CAPS.load(Ordering::Relaxed), cooldown_left(&s, from).as_secs().min(255) as u8, s.round.is_some());
                drop(s);
                crate::net::send_raw(from, &pkt);
            }
            _ => {}
        }
        return;
    }
    if crate::net::host_addr() == Some(from) {
        receive(pkt);
    }
}

// ---- test aids --------------------------------------------------------------------------------------------------

/// `REACH_VOTE_FAKE=call` (or `REACH_VOTE_FAKE=1`): show a made-up open call vote (panel captures without a server).
fn fake_tick() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    let Ok(v) = std::env::var("REACH_VOTE_FAKE") else { return };
    if !(v == "1" || v.split(',').any(|x| x == "call")) {
        return;
    }
    ONCE.call_once(|| log!("callvote: REACH_VOTE_FAKE: showing a made-up call vote"));
    let mut c = CLIENT.lock().unwrap();
    c.fake = true;
    let st = Status { id: 1, kind: CallKind::Shuffle as u8, secs_left: 25, yes: 2, no: 0, voters: 9, needed: 5, caller: "Kat".into(), target: String::new() };
    c.status = Some((st, Instant::now()));
}

/// Test script `vote ...` (input.rs, game thread): `call <shuffle|kick|skip|restart> [target]`, `yes`, `no`,
/// `teams` (log every player's team fields), `members` (log the session membership), `team <name> <t>` (move one player as the shuffle does).
pub fn script_command(a: &[String]) {
    let base = crate::crash::ENGINE_BASE.load(Ordering::Relaxed);
    match a.first().map(|s| s.as_str()) {
        Some("call") => {
            let Some(k) = a.get(1).and_then(|k| CallKind::parse(k)) else { return log!("callvote: script: unknown kind {:?}", a.get(1)) };
            call_vote(k, a.get(2).map(|s| s.as_str()));
        }
        Some("yes") => log!("callvote: script yes -> {}", cast(true)),
        Some("no") => log!("callvote: script no -> {}", cast(false)),
        Some("teams") if base != 0 => unsafe { log!("callvote: teams {}", team_bytes(base)) },
        Some("team") if base != 0 => unsafe {
            let t: i8 = a.get(2).and_then(|t| t.parse().ok()).unwrap_or(0);
            match players(base).iter().find(|p| a.get(1).is_some_and(|n| p.name.eq_ignore_ascii_case(n))) {
                Some(p) => {
                    set_teams(base, &[(p, t)]);
                    log!("callvote: script: {} -> team {t}", p.name);
                }
                None => log!("callvote: script: no player {:?}", a.get(1)),
            }
        },
        Some("members") if base != 0 => unsafe { log!("callvote: members {}", members_dump(base)) },
        _ => log!("callvote: script: unknown command {}", a.join(" ")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let c = encode_call(CallKind::Kick, "Kat");
        assert_eq!((&c[..4], c[5]), (&b"RVOT"[..], KIND_CALL));
        assert_eq!(decode_call(&c), Some((CallKind::Kick as u8, "Kat".into())));
        let b = encode_ballot(9, true);
        assert_eq!((b[5], id_of(&b), b[8]), (KIND_BALLOT, 9, 1));
        let s = Status { id: 4, kind: 0, secs_left: 25, yes: 2, no: 1, voters: 9, needed: 5, caller: "Kat".into(), target: String::new() };
        assert_eq!(decode_status(&encode_status(&s)), Some(s));
        let o = Outcome { id: 4, kind: 1, result: PASSED, yes: 5, no: 1, caller: "A".into(), target: "B".into(), text: "B was kicked".into() };
        assert_eq!(decode_outcome(&encode_outcome(&o)), Some(o));
        assert_eq!(CallKind::from_byte(3), Some(CallKind::Restart));
        assert_eq!(CallKind::from_byte(4), None);
    }

    #[test]
    fn majority_and_deal() {
        assert_eq!((needed(1), needed(2), needed(3), needed(9)), (1, 2, 2, 5));
        let ps: Vec<(String, i8)> = (0..5).map(|i| (format!("p{i}"), 0)).collect();
        let mut x = 7u64;
        let t = deal(&ps, || {
            x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            x >> 33
        });
        let red = t.iter().filter(|&&v| v == 0).count();
        assert!(red == 2 || red == 3);
    }
}
