//! Per-player ping (and the host's kill/death/assist counts) for the Tab scoreboard. The host (instance 0: dedicated
//! or listen server) pings every peer once a second over the game socket and broadcasts a table {player xuid → round
//! trip ms, kills, deaths, assists}; clients show what the table says. The counts come from the host because a
//! client's own game-results statistics miss deaths it did not see (notes/frontend.md "Tab scoreboard").
//! Wire format: notes/networking.md "Ping". Datagrams start with `RPNG`, so they never reach the engine queues.
//! A peer's engine player is found by xuid: instance n plays as xuid `options::xuid(n)` and has net id
//! `net::net_id(n)`, so the net id in a pong names the player (datum +0x08 holds the same xuid).

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::log;

pub const MAGIC: [u8; 4] = *b"RPNG";
const VERSION: u8 = 1;
const KIND_PING: u8 = 1;
const KIND_PONG: u8 = 2;
const KIND_TABLE: u8 = 3;
/// Ping / pong: header 8 + u64 send time (µs on the host's clock) + u64 sender net id (pong; 0 in a ping).
const PING_LEN: usize = 24;
/// Table entry: u64 xuid, u16 ms (0xFFFF = not measured yet), i16 kills, deaths, assists (kills -32768 = the host
/// has no counts for this player).
const ENTRY: usize = 16;
const MAX_ENTRIES: usize = 16;
const NO_PING: u16 = 0xFFFF;
pub const NO_COUNTS: i16 = i16::MIN;
/// A table older than this is not shown (host gone, or an older host without pings).
const STALE: Duration = Duration::from_secs(5);

/// One player in the table.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Entry {
    pub ms: Option<u16>,
    pub kills: i16,
    pub deaths: i16,
    pub assists: i16,
}

/// Session size for the scoreboard title "(n/max)" (`--players`, the server's max for a joiner; 16 offline).
pub static MAX_PLAYERS: AtomicU32 = AtomicU32::new(16);
static HOST: AtomicBool = AtomicBool::new(false);
/// Host: smoothed RTT per xuid.
static RTT: Mutex<Option<HashMap<u64, f32>>> = Mutex::new(None);
/// Both sides: the latest table and when it arrived (host: built).
static TABLE: Mutex<Option<(HashMap<u64, Entry>, Instant)>> = Mutex::new(None);

fn epoch() -> Instant {
    static T0: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    *T0.get_or_init(Instant::now)
}

fn now_us() -> u64 {
    epoch().elapsed().as_micros() as u64
}

fn header(kind: u8, n: u8) -> [u8; 8] {
    [MAGIC[0], MAGIC[1], MAGIC[2], MAGIC[3], VERSION, kind, n, 0]
}

pub fn encode_ping(t_us: u64) -> [u8; PING_LEN] {
    let mut p = [0u8; PING_LEN];
    p[..8].copy_from_slice(&header(KIND_PING, 0));
    p[8..16].copy_from_slice(&t_us.to_le_bytes());
    p
}

fn encode_table(t: &[(u64, Entry)]) -> Vec<u8> {
    let t = &t[..t.len().min(MAX_ENTRIES)];
    let mut o = Vec::with_capacity(8 + t.len() * ENTRY);
    o.extend_from_slice(&header(KIND_TABLE, t.len() as u8));
    for (x, e) in t {
        o.extend_from_slice(&x.to_le_bytes());
        o.extend_from_slice(&e.ms.unwrap_or(NO_PING).to_le_bytes());
        for v in [e.kills, e.deaths, e.assists] {
            o.extend_from_slice(&v.to_le_bytes());
        }
    }
    o
}

fn decode_table(p: &[u8]) -> Option<HashMap<u64, Entry>> {
    let n = *p.get(6)? as usize;
    if n > MAX_ENTRIES || p.len() < 8 + n * ENTRY {
        return None;
    }
    let u16_at = |e: &[u8], o: usize| u16::from_le_bytes([e[o], e[o + 1]]);
    Some((0..n).map(|i| {
        let e = &p[8 + i * ENTRY..];
        let ms = u16_at(e, 8);
        let x = u64::from_le_bytes(e[..8].try_into().unwrap());
        (x, Entry { ms: (ms != NO_PING).then_some(ms), kills: u16_at(e, 10) as i16, deaths: u16_at(e, 12) as i16, assists: u16_at(e, 14) as i16 })
    }).collect())
}

/// main.rs (start_networking): a networked game started. The host starts its ping thread; a client only answers.
pub fn start(instance: u32, players: u32) {
    MAX_PLAYERS.store(players.clamp(1, 16), Ordering::Relaxed);
    if instance != 0 || HOST.swap(true, Ordering::Relaxed) {
        return;
    }
    std::thread::spawn(|| {
        let mut logged = 0u32;
        let mut last_counts = Vec::new();
        loop {
            std::thread::sleep(Duration::from_secs(1));
            let ping = encode_ping(now_us());
            for a in crate::net::active_peer_addrs(Duration::from_secs(4)) {
                crate::net::send_raw(a, &ping);
            }
            let rtt: HashMap<u64, f32> = RTT.lock().unwrap_or_else(|e| e.into_inner()).clone().unwrap_or_default();
            let dedicated = crate::host::DEDICATED.load(Ordering::Relaxed);
            let me = crate::host::LOCAL_XUID.load(Ordering::Relaxed);
            // Everyone in the engine's player table with the host's counts; a listen host's own row shows 0 ms.
            let mut table: Vec<(u64, Entry)> = crate::ui::scoreboard::stats()
                .into_iter()
                .map(|(x, k, d, a)| {
                    let ms = if x == me && !dedicated { Some(0) } else { rtt.get(&x).map(|v| v.round().clamp(0.0, 9999.0) as u16) };
                    (x, Entry { ms, kills: k, deaths: d, assists: a })
                })
                .collect();
            // No player table on this thread's view yet (dedicated before the map): still send the pings measured.
            for (&x, v) in &rtt {
                if !table.iter().any(|t| t.0 == x) {
                    table.push((x, Entry { ms: Some(v.round().clamp(0.0, 9999.0) as u16), kills: NO_COUNTS, deaths: 0, assists: 0 }));
                }
            }
            table.sort_by_key(|t| t.0);
            let counts: Vec<(u64, i16, i16, i16)> = table.iter().map(|(x, e)| (*x, e.kills, e.deaths, e.assists)).collect();
            if (logged < 5 && !rtt.is_empty()) || counts != last_counts {
                logged += 1;
                let list: Vec<String> = table.iter()
                    .map(|(x, e)| format!("{x:#x} {} K{} D{} A{}", e.ms.map_or("-".into(), |m| format!("{m} ms")), e.kills, e.deaths, e.assists))
                    .collect();
                log!("ping: table {}", list.join(", "));
                last_counts = counts;
            }
            let pkt = encode_table(&table);
            for a in crate::net::active_peer_addrs(Duration::from_secs(4)) {
                crate::net::send_raw(a, &pkt);
            }
            *TABLE.lock().unwrap_or_else(|e| e.into_inner()) = Some((table.into_iter().collect(), Instant::now()));
        }
    });
    log!("ping: host pings its peers every second (RPNG)");
}

/// net.rs receive thread: true when `pkt` is a ping datagram (handled here, never queued for the engine).
pub fn handle(pkt: &[u8], from: SocketAddr) -> bool {
    if pkt.len() < 8 || pkt[..4] != MAGIC {
        return false;
    }
    if pkt[4] != VERSION {
        return true;
    }
    match pkt[5] {
        // Client: echo the host's ping with our net id.
        KIND_PING if pkt.len() >= PING_LEN && crate::net::host_addr() == Some(from) => {
            let mut p = [0u8; PING_LEN];
            p.copy_from_slice(&pkt[..PING_LEN]);
            p[5] = KIND_PONG;
            p[16..24].copy_from_slice(&crate::net::my_id().to_le_bytes());
            crate::net::send_raw(from, &p);
        }
        // Host: one round trip from a peer.
        KIND_PONG if pkt.len() >= PING_LEN && HOST.load(Ordering::Relaxed) && crate::net::is_peer_addr(from) => {
            let sent = u64::from_le_bytes(pkt[8..16].try_into().unwrap());
            let id = u64::from_le_bytes(pkt[16..24].try_into().unwrap());
            let Some(n) = id.checked_sub(crate::net::NET_ID_BASE).filter(|&n| (1..=0xFFFF).contains(&n)) else { return true };
            let ms = now_us().saturating_sub(sent) as f32 / 1000.0;
            let xuid = crate::options::xuid(n as u32);
            let mut g = RTT.lock().unwrap_or_else(|e| e.into_inner());
            let m = g.get_or_insert_with(HashMap::new);
            let first = !m.contains_key(&xuid);
            let v = m.entry(xuid).or_insert(ms);
            *v = *v * 0.7 + ms * 0.3; // smoothed, so the chip does not flicker
            if first {
                log!("ping: first pong from net id {id:#x} (xuid {xuid:#x}) at {from}: {ms:.1} ms");
            }
        }
        // Client: the host's table.
        KIND_TABLE if crate::net::host_addr() == Some(from) => {
            if let Some(t) = decode_table(pkt) {
                let prev = TABLE.lock().unwrap_or_else(|e| e.into_inner()).replace((t.clone(), Instant::now()));
                let counts = |m: &HashMap<u64, Entry>| {
                    let mut v: Vec<(u64, i16, i16, i16)> = m.iter().map(|(&x, e)| (x, e.kills, e.deaths, e.assists)).collect();
                    v.sort();
                    v
                };
                if !t.is_empty() && prev.is_none_or(|(p, _)| counts(&p) != counts(&t)) {
                    let mut list: Vec<String> = t.iter()
                        .map(|(x, e)| format!("{x:#x} {} K{} D{} A{}", e.ms.map_or("-".into(), |m| format!("{m} ms")), e.kills, e.deaths, e.assists))
                        .collect();
                    list.sort();
                    log!("ping: table from the host: {}", list.join(", "));
                }
            }
        }
        _ => {}
    }
    true
}

/// The latest table's entry for `xuid` (host: its own; client: the host's broadcast), None if stale or absent.
pub fn entry(xuid: u64) -> Option<Entry> {
    let g = TABLE.lock().unwrap_or_else(|e| e.into_inner());
    let (t, at) = g.as_ref()?;
    if at.elapsed() > STALE {
        return None;
    }
    t.get(&xuid).copied()
}

/// True on a client that has a fresh table from its host (its counts beat the client's own).
pub fn host_counts() -> bool {
    !HOST.load(Ordering::Relaxed) && TABLE.lock().unwrap_or_else(|e| e.into_inner()).as_ref().is_some_and(|(_, at)| at.elapsed() <= STALE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_roundtrip() {
        let e = Entry { ms: Some(53), kills: 3, deaths: -1, assists: 2 };
        let t = vec![(0x0009_0000_0000_0001u64, Entry { ms: None, kills: 0, deaths: 0, assists: 0 }), (0x0009_0000_0000_0002, e)];
        let p = encode_table(&t);
        assert_eq!(p.len(), 8 + 2 * ENTRY);
        let d = decode_table(&p).unwrap();
        assert_eq!(d.get(&0x0009_0000_0000_0002), Some(&e));
        assert_eq!(d.get(&0x0009_0000_0000_0001).unwrap().ms, None);
        assert!(decode_table(&p[..p.len() - 1]).is_none());
        let ping = encode_ping(1234);
        assert_eq!((&ping[..4], ping[5]), (&MAGIC[..], KIND_PING));
    }
}
