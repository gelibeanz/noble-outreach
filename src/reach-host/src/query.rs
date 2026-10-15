//! Server info query: a tiny request/response on the game's own UDP socket, answered by any hosting reach-host
//! (dedicated or player host) in net.rs's receive thread before the engine sees anything. The browser sends it
//! to every listed server for live players and ping (RTT of the query). Engine datagrams start with the engine
//! port as a u32 (1000/1001), so the magic can never be mistaken for game traffic. Wire format:
//! notes/networking.md, "Server info query".

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr, ToSocketAddrs, UdpSocket};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::log;

pub const REQ_MAGIC: [u8; 4] = *b"RQRY";
pub const REPLY_MAGIC: [u8; 4] = *b"RQRS";
pub const VERSION: u8 = 1;
const KIND_INFO_REQ: u8 = 1;
const KIND_INFO_REPLY: u8 = 2;
/// Requests are padded to this so a reply is at most a few times larger (little use for amplification).
const REQ_LEN: usize = 16;
const REPLY_HEADER: usize = 32;
/// Replies per source IP per second.
const RATE_PER_SEC: u32 = 8;

/// Game modes in the reply.
pub const MODE_MULTIPLAYER: u8 = 0;
pub const MODE_FIREFIGHT: u8 = 1;
pub const MODE_CAMPAIGN: u8 = 2;
/// Multiplayer with a Forge (sandbox, engine type 1) game variant.
pub const MODE_FORGE: u8 = 3;

/// What this instance answers with (set by main.rs on a hosting instance; clients leave it unset and stay silent).
#[derive(Clone, Debug, Default)]
pub struct Info {
    pub name: String,
    pub version: String,
    pub map: String,
    pub game_variant: String,
    pub mode: u8,
    pub max_players: u8,
    pub dedicated: bool,
    pub network_id: u64,
    pub session_id: u64,
    /// Playlist name of a `--playlist` server (optional 5th string; empty = none).
    pub playlist: String,
}

/// A decoded reply plus the measured round trip.
#[derive(Clone, Debug, Default)]
pub struct Reply {
    pub info: Info,
    pub players: u8,
    #[allow(dead_code)] // flag bit 1, never set yet
    pub password: bool,
    /// Flag bit 2: a dedicated campaign whose mission has started (the engine refuses joins until it restarts).
    pub in_progress: bool,
    /// Campaign lobby: seconds until the mission starts (0 = no countdown).
    pub lobby_secs: u8,
    /// Flag bits 3/4: a dormant dedicated server (engine not loaded) or one whose engine is still starting.
    pub waking: bool,
    pub ping_ms: u32,
}

static INFO: Mutex<Option<Info>> = Mutex::new(None);
static RATE: Mutex<Option<HashMap<IpAddr, (Instant, u32)>>> = Mutex::new(None);
static ANSWERED: AtomicU64 = AtomicU64::new(0);
static LIMITED: AtomicU64 = AtomicU64::new(0);

pub fn set_info(info: Info) {
    log!("query: answering server info queries as \"{}\" ({} on {}, {} max)", info.name, mode_name(info.mode), info.map, info.max_players);
    *INFO.lock().unwrap() = Some(info);
}

pub fn mode_name(m: u8) -> &'static str {
    match m {
        MODE_FIREFIGHT => "firefight",
        MODE_CAMPAIGN => "campaign",
        MODE_FORGE => "forge",
        _ => "multiplayer",
    }
}

/// Live player count the same way the master heartbeat reports it (slot 30 roster; a dedicated server is not a player).
fn live_players(info: &Info) -> u8 {
    let roster = crate::net::ROSTER_COUNT.load(Ordering::Relaxed);
    // No roster before the game starts: a campaign server gathering players reports its lobby count instead.
    let roster = if roster == 0 { crate::lobby::players_plus_one() } else { roster };
    let min = if info.dedicated { 0 } else { 1 };
    (if roster == 0 { min } else { (roster - 1) as u32 }).clamp(min, info.max_players as u32) as u8
}

/// net.rs hook: true when `pkt` is a query datagram (answered or dropped here, never queued for the engine).
pub fn handle(sock: &UdpSocket, pkt: &[u8], from: SocketAddr) -> bool {
    if pkt.len() < 4 || (pkt[..4] != REQ_MAGIC && pkt[..4] != REPLY_MAGIC) {
        return false;
    }
    if pkt[..4] == REPLY_MAGIC || pkt.len() < 12 || pkt[4] != VERSION || pkt[5] != KIND_INFO_REQ {
        return true;
    }
    let Some(info) = INFO.lock().unwrap().clone() else { return true };
    if !allow(from.ip()) {
        if LIMITED.fetch_add(1, Ordering::Relaxed) < 3 {
            log!("query: rate limit hit for {}", from.ip());
        }
        return true;
    }
    let token = u32::from_le_bytes(pkt[8..12].try_into().unwrap());
    let reply = encode_reply(&info, live_players(&info), token);
    let _ = sock.send_to(&reply, from);
    let k = ANSWERED.fetch_add(1, Ordering::Relaxed) + 1;
    if k <= 5 || k % 500 == 0 {
        log!("query: #{k} answered {from} ({} bytes)", reply.len());
    }
    true
}

fn allow(ip: IpAddr) -> bool {
    let mut g = RATE.lock().unwrap();
    let m = g.get_or_insert_with(HashMap::new);
    let now = Instant::now();
    if m.len() > 4096 {
        m.retain(|_, (t, _)| now - *t < Duration::from_secs(1));
    }
    let e = m.entry(ip).or_insert((now, 0));
    if now - e.0 >= Duration::from_secs(1) {
        *e = (now, 0);
    }
    e.1 += 1;
    e.1 <= RATE_PER_SEC
}

fn put_str(out: &mut Vec<u8>, s: &str, max: usize) {
    let mut n = s.len().min(max);
    while !s.is_char_boundary(n) {
        n -= 1;
    }
    out.push(n as u8);
    out.extend_from_slice(&s.as_bytes()[..n]);
}

pub fn encode_request(token: u32) -> [u8; REQ_LEN] {
    let mut r = [0u8; REQ_LEN];
    r[..4].copy_from_slice(&REQ_MAGIC);
    r[4] = VERSION;
    r[5] = KIND_INFO_REQ;
    r[8..12].copy_from_slice(&token.to_le_bytes());
    r
}

fn encode_reply(i: &Info, players: u8, token: u32) -> Vec<u8> {
    let mut o = Vec::with_capacity(REPLY_HEADER + 160);
    o.extend_from_slice(&REPLY_MAGIC);
    o.push(VERSION);
    o.push(KIND_INFO_REPLY);
    // flags: bit 0 dedicated, bit 1 password (never set yet), bit 2 campaign in progress (joins refused),
    // bit 3 dormant (no engine loaded, dormant.rs), bit 4 engine starting (joins not answered yet)
    o.push(i.dedicated as u8 | if crate::lobby::IN_PROGRESS.load(Ordering::Relaxed) { 4 } else { 0 } | crate::dormant::query_flags());
    o.push(i.mode);
    o.extend_from_slice(&token.to_le_bytes());
    o.push(players);
    o.push(i.max_players);
    o.push(crate::lobby::SECS_LEFT.load(Ordering::Relaxed).min(255) as u8); // campaign lobby countdown, 0 = none
    o.push(0);
    o.extend_from_slice(&i.network_id.to_le_bytes());
    o.extend_from_slice(&i.session_id.to_le_bytes());
    put_str(&mut o, &i.name, 48);
    put_str(&mut o, &i.version, 32);
    put_str(&mut o, &i.map, 64);
    put_str(&mut o, &i.game_variant, 64);
    put_str(&mut o, &i.playlist, 48); // optional: older decoders stop after the 4th string
    o
}

/// (token, reply without ping) or None for anything that is not a v1 info reply.
fn decode_reply(p: &[u8]) -> Option<(u32, Reply)> {
    if p.len() < REPLY_HEADER || p[..4] != REPLY_MAGIC || p[4] != VERSION || p[5] != KIND_INFO_REPLY {
        return None;
    }
    let mut at = REPLY_HEADER;
    let mut s = || {
        let n = *p.get(at)? as usize;
        let v = String::from_utf8_lossy(p.get(at + 1..at + 1 + n)?).into_owned();
        at += 1 + n;
        Some(v)
    };
    let (name, version, map, game_variant) = (s()?, s()?, s()?, s()?);
    let playlist = s().unwrap_or_default();
    let info = Info {
        name,
        version,
        map,
        game_variant,
        mode: p[7],
        max_players: p[13],
        dedicated: p[6] & 1 != 0,
        network_id: u64::from_le_bytes(p[16..24].try_into().ok()?),
        session_id: u64::from_le_bytes(p[24..32].try_into().ok()?),
        playlist,
    };
    Some((u32::from_le_bytes(p[8..12].try_into().ok()?), Reply { info, players: p[12], password: p[6] & 2 != 0, in_progress: p[6] & 4 != 0, lobby_secs: p[14], waking: p[6] & 0x18 != 0, ping_ms: 0 }))
}

/// `host:port` (hostnames resolved, IPv4 preferred).
pub fn resolve(addr: &str) -> Result<SocketAddr, String> {
    let all: Vec<SocketAddr> = addr.trim().to_socket_addrs().map_err(|e| format!("{addr}: {e}"))?.collect();
    all.iter().find(|a| a.is_ipv4()).or(all.first()).copied().ok_or_else(|| format!("{addr}: no address"))
}

/// Query every target from one socket; one resend for silent ones at half time. Index-aligned results.
pub fn query_many(targets: &[SocketAddr], timeout: Duration) -> Vec<Option<Reply>> {
    query_many_progress(targets, timeout, None)
}

/// `query_many`, counting every first answer into `progress` as it arrives (the server browser's "X of Y").
pub fn query_many_progress(targets: &[SocketAddr], timeout: Duration, progress: Option<&AtomicUsize>) -> Vec<Option<Reply>> {
    let mut out: Vec<Option<Reply>> = vec![None; targets.len()];
    if targets.is_empty() {
        return out;
    }
    let Ok(sock) = UdpSocket::bind(SocketAddr::from(([0, 0, 0, 0], 0))) else { return out };
    let base = (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(1) & 0xFFFF_0000) | 0x8000;
    let mut sent = vec![Instant::now(); targets.len()];
    let send = |sock: &UdpSocket, i: usize, sent: &mut Vec<Instant>| {
        sent[i] = Instant::now();
        let _ = sock.send_to(&encode_request(base + i as u32), targets[i]);
    };
    for i in 0..targets.len() {
        send(&sock, i, &mut sent);
    }
    let start = Instant::now();
    let mut resent = false;
    let mut buf = [0u8; 512];
    while start.elapsed() < timeout && out.iter().any(|r| r.is_none()) {
        if !resent && start.elapsed() >= timeout / 2 {
            resent = true;
            for i in 0..targets.len() {
                if out[i].is_none() {
                    send(&sock, i, &mut sent);
                }
            }
        }
        let _ = sock.set_read_timeout(Some(Duration::from_millis(50)));
        // Windows reports an ICMP port-unreachable from an earlier send as a recv error; just keep reading.
        if let Ok((n, _)) = sock.recv_from(&mut buf) {
            if let Some((tok, mut r)) = decode_reply(&buf[..n]) {
                let i = tok.wrapping_sub(base) as usize;
                if i < targets.len() && out[i].is_none() {
                    r.ping_ms = sent[i].elapsed().as_millis() as u32;
                    out[i] = Some(r);
                    if let Some(p) = progress {
                        p.fetch_add(1, Ordering::Relaxed);
                    }
                }
            }
        }
    }
    out
}

/// LAN discovery: broadcast the query to `ports` on 255.255.255.255 and collect every reply.
pub fn query_broadcast(ports: &[u16], timeout: Duration) -> Vec<(SocketAddr, Reply)> {
    let mut out = Vec::new();
    let Ok(sock) = UdpSocket::bind(SocketAddr::from(([0, 0, 0, 0], 0))) else { return out };
    let _ = sock.set_broadcast(true);
    let token = 0x4c41_4e00;
    let t0 = Instant::now();
    for p in ports {
        let _ = sock.send_to(&encode_request(token), SocketAddr::from(([255, 255, 255, 255], *p)));
    }
    let mut buf = [0u8; 512];
    while t0.elapsed() < timeout {
        let _ = sock.set_read_timeout(Some(Duration::from_millis(50)));
        if let Ok((n, from)) = sock.recv_from(&mut buf) {
            if let Some((tok, mut r)) = decode_reply(&buf[..n]) {
                if tok == token && !out.iter().any(|(a, _)| *a == from) {
                    r.ping_ms = t0.elapsed().as_millis() as u32;
                    out.push((from, r));
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let i = Info { name: "Test Server".into(), version: "0.1".into(), map: "forge_world".into(), game_variant: "slayer_054.bin".into(), mode: 1, max_players: 8, dedicated: true, network_id: 7, session_id: 9, playlist: "Slayer".into() };
        let (tok, r) = decode_reply(&encode_reply(&i, 3, 42)).unwrap();
        assert_eq!((tok, r.players, r.info.max_players, r.info.mode, r.info.dedicated), (42, 3, 8, 1, true));
        assert_eq!((r.info.name.as_str(), r.info.map.as_str(), r.info.session_id), ("Test Server", "forge_world", 9));
        assert_eq!(r.info.playlist, "Slayer");
        assert!(decode_reply(&encode_request(1)).is_none());
    }
}
