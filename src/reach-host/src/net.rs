//! Our peer transport behind host slots 41 (send) and 43 (receive). One UDP socket per instance carries
//! both engine ports (1000 = link endpoint 0, 1001 = endpoint 1). Datagram = [u32 engine port LE]
//! [u64 sender network id LE][payload]. A receive thread queues packets per engine port; slot 43 pops
//! from the queue of the port it asks for and never blocks. Peers not given on the command line are learned
//! from the first datagram they send (a dedicated server does not know its joiners in advance); a known id
//! seen from a new address is re-pointed there (a client that restarted). Design notes: notes/networking.md.

use std::collections::{HashMap, VecDeque};
use std::net::{SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use crate::log;

/// Network id (the engine's "secure address", launch options +0x58) of instance `n`.
pub const NET_ID_BASE: u64 = 0x5245_4143_4800_0001; // "REACH\0\0" + 1
pub const BASE_PORT: u16 = 47800;
const HEADER: usize = 12;
const MAX_PAYLOAD: usize = 1192;
const QUEUE_CAP: usize = 4096;
const LOG_FIRST: u64 = 8;

pub fn net_id(instance: u32) -> u64 {
    NET_ID_BASE + instance as u64
}

struct Net {
    sock: UdpSocket,
    my_id: u64,
    bind: SocketAddr,
    peers: Mutex<Vec<Peer>>,
    queues: Mutex<HashMap<u32, VecDeque<(u64, Vec<u8>)>>>,
}

struct Peer {
    id: u64,
    addr: SocketAddr,
    last_seen: Option<Instant>,
    /// When this peer said bye (we stop sending to it; it comes back if it talks again 2 s later).
    left: Option<Instant>,
}

/// "Bye" datagram (16 bytes): magic `RBYE`, version 1, kind 1 (leaving), 2 reserved, u64 sender network id.
/// Sent a few times when a player leaves or closes the window; never reaches the engine. notes/networking.md.
const BYE_MAGIC: [u8; 4] = *b"RBYE";
const BYE_VERSION: u8 = 1;
const BYE_KIND_LEAVING: u8 = 1;
/// Net ids that said bye, for the game thread to drop from the session (host side, `drop_left_peers`).
static BYE_QUEUE: Mutex<Vec<u64>> = Mutex::new(Vec::new());
/// Client: the host (instance 0) said bye, so the game is over for us.
pub static HOST_LEFT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

static NET: RwLock<Option<Arc<Net>>> = RwLock::new(None);
/// Bumped by `stop`: the receive thread and the side-channel threads of the old transport (master heartbeat,
/// rendezvous registration) end when it changes (seamless.rs: one process, several games).
static GEN: AtomicU32 = AtomicU32::new(0);
static RECV_THREAD: Mutex<Option<std::thread::JoinHandle<()>>> = Mutex::new(None);

fn net() -> Option<Arc<Net>> {
    NET.read().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Current transport generation (see `GEN`).
pub fn generation() -> u32 {
    GEN.load(Ordering::Relaxed)
}

/// Transport running with this identity and bind address (and, for a client, this host address)?
pub fn running_as(my_id: u64, bind: SocketAddr, host: Option<SocketAddr>) -> bool {
    net().is_some_and(|n| {
        n.my_id == my_id && n.bind == bind && host.is_none_or(|h| n.peers.lock().unwrap().iter().any(|p| p.id == net_id(0) && p.addr == h))
    })
}

pub fn running() -> bool {
    net().is_some()
}

/// Close the socket and end the receive thread, so the next game can bind again (seamless.rs).
pub fn stop() {
    let old = NET.write().unwrap_or_else(|e| e.into_inner()).take();
    if old.is_none() {
        return;
    }
    GEN.fetch_add(1, Ordering::Relaxed);
    drop(old);
    if let Some(t) = RECV_THREAD.lock().unwrap_or_else(|e| e.into_inner()).take() {
        let _ = t.join(); // recv timeout is 200 ms
    }
    log!("net: transport stopped");
}

/// Between two games over the same transport (a vote follow, the next mission): forget the old game's packets and
/// the host's last answer (the 20 s join watchdog starts again).
pub fn reset_game() {
    HOST_LEFT.store(false, Ordering::Relaxed);
    BYE_QUEUE.lock().unwrap().clear();
    ROSTER_COUNT.store(0, Ordering::Relaxed);
    if let Some(n) = net() {
        n.queues.lock().unwrap().clear();
        for p in n.peers.lock().unwrap().iter_mut() {
            p.last_seen = None;
            p.left = None;
        }
    }
}
static SENT: AtomicU64 = AtomicU64::new(0);
static SENT_BYTES: AtomicU64 = AtomicU64::new(0);
static RECV: AtomicU64 = AtomicU64::new(0);
static RECV_BYTES: AtomicU64 = AtomicU64::new(0);
static DELIVERED: AtomicU64 = AtomicU64::new(0);
static DROPPED: AtomicU64 = AtomicU64::new(0);
static SEND_FAIL: AtomicU64 = AtomicU64::new(0);
/// Engine base, for the session/launch-state readout in the stats line.
pub static ENGINE_BASE: AtomicUsize = AtomicUsize::new(0);
/// Last roster entry count from host slot 30 (+1, so 0 = never called).
pub static ROSTER_COUNT: AtomicU64 = AtomicU64::new(0);

fn hex(b: &[u8]) -> String {
    b.iter().take(24).map(|x| format!("{x:02x}")).collect()
}

/// Bind `bind` and remember the peers. Starts the receive thread.
pub fn start(my_id: u64, bind: SocketAddr, peers: Vec<(u64, SocketAddr)>) -> Result<(), String> {
    let sock = UdpSocket::bind(bind).map_err(|e| format!("bind {bind}: {e}"))?;
    sock.set_read_timeout(Some(Duration::from_millis(200))).map_err(|e| format!("set_read_timeout: {e}"))?;
    let recv = sock.try_clone().map_err(|e| format!("clone socket: {e}"))?;
    log!("net: id {my_id:#x} bound {bind}");
    for (id, a) in &peers {
        log!("net: peer {id:#x} at {a}");
    }
    let peers = peers.into_iter().map(|(id, addr)| Peer { id, addr, last_seen: None, left: None }).collect();
    let mut slot = NET.write().unwrap_or_else(|e| e.into_inner());
    if slot.is_some() {
        return Err("net started twice".into());
    }
    let net = Arc::new(Net { sock, my_id, bind, peers: Mutex::new(peers), queues: Mutex::new(HashMap::new()) });
    *slot = Some(net.clone());
    drop(slot);
    let gen = GEN.load(Ordering::Relaxed);
    *RECV_THREAD.lock().unwrap_or_else(|e| e.into_inner()) = Some(std::thread::spawn(move || recv_loop(recv, net, gen)));
    Ok(())
}

fn recv_loop(sock: UdpSocket, net: Arc<Net>, gen: u32) {
    let net = &*net;
    let mut buf = vec![0u8; 2048];
    let mut last = Instant::now();
    let mut prev = [0u64; 4];
    while GEN.load(Ordering::Relaxed) == gen {
        match sock.recv_from(&mut buf) {
            // Server info queries (query.rs) are answered here and never reach the engine.
            Ok((n, from)) if crate::query::handle(&sock, &buf[..n], from) => {}
            // NAT punch / rendezvous packets (nat.rs) likewise.
            Ok((n, from)) if crate::nat::handle(&buf[..n], from) => {}
            // Playlist votes (vote.rs) likewise.
            Ok((n, from)) if crate::vote::handle(&sock, &buf[..n], from) => {}
            // Scoreboard ping (ping.rs).
            Ok((n, from)) if crate::ping::handle(&buf[..n], from) => {}
            // Campaign lobby status from a dedicated server (lobby.rs).
            Ok((n, from)) if crate::lobby::handle(&buf[..n], from) => {}
            // Wake packets for a dormant dedicated server (dormant.rs).
            Ok((n, from)) if crate::dormant::handle(&buf[..n], from) => {}
            // A kicked player (call vote, callvote.rs): nothing of theirs reaches the engine again in this process.
            Ok((n, from)) if n >= HEADER && buf[..4] != BYE_MAGIC && is_banned(u64::from_le_bytes(buf[4..12].try_into().unwrap()), from) => {}
            // Dormant / engine still starting: wake on the first engine datagram, drop it (the joiner retries).
            Ok((n, from)) if n >= HEADER && buf[..4] != BYE_MAGIC && crate::dormant::gate_engine_datagram(from) => {}
            Ok((n, from)) if n >= 16 && buf[..4] == BYE_MAGIC => {
                if buf[4] == BYE_VERSION && buf[5] == BYE_KIND_LEAVING {
                    got_bye(net, u64::from_le_bytes(buf[8..16].try_into().unwrap()), from);
                }
            }
            Ok((n, from)) if n >= HEADER => {
                let port = u32::from_le_bytes(buf[0..4].try_into().unwrap());
                let id = u64::from_le_bytes(buf[4..12].try_into().unwrap());
                let payload = buf[HEADER..n].to_vec();
                note_peer(net, id, from);
                let k = RECV.fetch_add(1, Ordering::Relaxed) + 1;
                RECV_BYTES.fetch_add(payload.len() as u64, Ordering::Relaxed);
                if k <= LOG_FIRST {
                    log!("net: recv #{k} from {id:#x} ({from}) port {port} len {}: {}", payload.len(), hex(&payload));
                }
                let mut q = net.queues.lock().unwrap();
                let q = q.entry(port).or_default();
                if q.len() >= QUEUE_CAP {
                    q.pop_front();
                    DROPPED.fetch_add(1, Ordering::Relaxed);
                }
                q.push_back((id, payload));
            }
            Ok((n, from)) => log!("net: short datagram ({n} bytes) from {from}"),
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
            Err(e) => {
                // Windows reports an ICMP port-unreachable from an earlier send as a recv error (peer not up yet).
                static SEEN: AtomicU64 = AtomicU64::new(0);
                if SEEN.fetch_add(1, Ordering::Relaxed) < 3 {
                    log!("net: recv error {e}");
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        if last.elapsed() >= Duration::from_secs(10) {
            last = Instant::now();
            let now = [SENT.load(Ordering::Relaxed), RECV.load(Ordering::Relaxed), SENT_BYTES.load(Ordering::Relaxed), RECV_BYTES.load(Ordering::Relaxed)];
            log!(
                "net heartbeat: sent {} pkts/{} B (+{} in 10 s), recv {} pkts/{} B (+{}), delivered {}, dropped {}, send failures {}, {}",
                now[0], now[2], now[0] - prev[0], now[1], now[3], now[1] - prev[1],
                DELIVERED.load(Ordering::Relaxed), DROPPED.load(Ordering::Relaxed), SEND_FAIL.load(Ordering::Relaxed),
                session_state()
            );
            prev = now;
            let peers = net.peers.lock().unwrap();
            let list: Vec<String> = peers
                .iter()
                .map(|p| match p.last_seen {
                    Some(t) => format!("{:#x}@{} {:.1}s ago", p.id, p.addr, t.elapsed().as_secs_f32()),
                    None => format!("{:#x}@{} never", p.id, p.addr),
                })
                .collect();
            log!("net peers: {}", list.join(", "));
        }
    }
}

/// Record a datagram from `id` at `from`: learn a new peer, follow a peer that moved, and log a peer
/// coming back after 5 s of silence (left and rejoined).
fn note_peer(net: &Net, id: u64, from: SocketAddr) {
    let mut peers = net.peers.lock().unwrap();
    let now = Instant::now();
    match peers.iter_mut().find(|p| p.id == id) {
        Some(p) => {
            if p.addr != from {
                log!("net: peer {id:#x} moved {} -> {from}", p.addr);
                p.addr = from;
            }
            if p.left.is_some_and(|t| now - t > Duration::from_secs(2)) {
                log!("net: peer {id:#x} is back after saying bye");
                p.left = None;
            }
            if p.last_seen.is_some_and(|t| now - t > Duration::from_secs(5)) {
                log!("net: peer {id:#x} back after {:.1} s of silence", (now - p.last_seen.unwrap()).as_secs_f32());
            }
            p.last_seen = Some(now);
        }
        None => {
            log!("net: new peer {id:#x} at {from}");
            peers.push(Peer { id, addr: from, last_seen: Some(now), left: None });
        }
    }
}

/// Launch state machine, session state and roster size, read straight from engine globals
/// (racy reads of aligned ints from another thread; fine for a log line).
pub fn session_state() -> String {
    let base = ENGINE_BASE.load(Ordering::Relaxed);
    if base == 0 {
        return String::new();
    }
    unsafe {
        let rd32 = |rva: usize| ((base + rva) as *const u32).read_volatile();
        let launch = rd32(0xc1a114);
        let status = rd32(0x2a1fd48);
        let session = ((base + 0x2a1fe08) as *const usize).read_volatile();
        let sstate = if session != 0 { ((session + 0x60ef8) as *const i32).read_volatile() } else { -1 };
        let roster = ROSTER_COUNT.load(Ordering::Relaxed);
        let roster = if roster == 0 { "none".to_string() } else { (roster - 1).to_string() };
        format!("launch state {launch}, net status {status}, session {session:#x} state {sstate}, roster players {roster}")
    }
}

/// Host slot 41. Returns bytes sent, -1 unknown peer, -2 would block, -3 other error.
pub fn send(id: u64, data: &[u8], port: u32) -> i32 {
    let Some(net) = net() else { return -1 };
    let peers = net.peers.lock().unwrap();
    let addr = match peers.iter().find(|p| p.id == id) {
        // Said bye: swallow the engine's packets until its own session drops the peer.
        Some(p) if p.left.is_some() => return data.len() as i32,
        Some(p) => p.addr,
        None if peers.len() == 1 && peers[0].left.is_none() => {
            static WARNED: AtomicU64 = AtomicU64::new(0);
            if WARNED.fetch_add(1, Ordering::Relaxed) < 3 {
                log!("net: send to unknown id {id:#x}, routing to the only peer {:#x}", peers[0].id);
            }
            peers[0].addr
        }
        None => {
            static WARNED: AtomicU64 = AtomicU64::new(0);
            if WARNED.fetch_add(1, Ordering::Relaxed) < 3 {
                log!("net: send to unknown id {id:#x} (len {}, port {port}): unreachable", data.len());
            }
            SEND_FAIL.fetch_add(1, Ordering::Relaxed);
            return -1;
        }
    };
    drop(peers);
    if data.len() > MAX_PAYLOAD {
        log!("net: payload {} > {MAX_PAYLOAD}, dropped", data.len());
        return -3;
    }
    let mut pkt = Vec::with_capacity(HEADER + data.len());
    pkt.extend_from_slice(&port.to_le_bytes());
    pkt.extend_from_slice(&net.my_id.to_le_bytes());
    pkt.extend_from_slice(data);
    match net.sock.send_to(&pkt, addr) {
        Ok(_) => {
            let k = SENT.fetch_add(1, Ordering::Relaxed) + 1;
            SENT_BYTES.fetch_add(data.len() as u64, Ordering::Relaxed);
            if k <= LOG_FIRST {
                log!("net: send #{k} to {id:#x} ({addr}) port {port} len {}: {}", data.len(), hex(data));
            }
            data.len() as i32
        }
        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => -2,
        Err(e) => {
            if SEND_FAIL.fetch_add(1, Ordering::Relaxed) < 3 {
                log!("net: send to {addr} failed: {e}");
            }
            -3
        }
    }
}

/// Host slot 43: next queued packet for `port`, or 0 when none.
pub fn recv(buf: &mut [u8], id_out: &mut u64, port: u32) -> i32 {
    let Some(net) = net() else { return 0 };
    let Some((id, data)) = net.queues.lock().unwrap().get_mut(&port).and_then(|q| q.pop_front()) else { return 0 };
    let n = data.len().min(buf.len());
    buf[..n].copy_from_slice(&data[..n]);
    *id_out = id;
    DELIVERED.fetch_add(1, Ordering::Relaxed);
    n as i32
}

/// A peer said bye. Host: stop sending to it and queue it for the game thread to drop from the session at once
/// (instead of the engine's ~16 s silence timeout). Client: if it was the host, the game is over.
fn got_bye(net: &Net, id: u64, from: SocketAddr) {
    let mut peers = net.peers.lock().unwrap();
    let Some(p) = peers.iter_mut().find(|p| p.id == id) else { return };
    if p.left.is_some() {
        return; // repeats of the same bye
    }
    p.left = Some(Instant::now());
    drop(peers);
    log!("net: bye from {id:#x} ({from})");
    if id == net_id(0) && net.my_id != id {
        HOST_LEFT.store(true, Ordering::Relaxed);
    } else if net.my_id == net_id(0) {
        BYE_QUEUE.lock().unwrap().push(id);
    }
}

/// Tell every peer we are leaving (pause overlay LEAVE GAME, window closed, host shutting down). UDP may drop a
/// datagram, so it goes out 3 times, 15 ms apart. Blocks ~30 ms; no-op without a network game.
pub fn send_bye() {
    let Some(net) = net() else { return };
    let addrs: Vec<SocketAddr> = net.peers.lock().unwrap().iter().filter(|p| p.left.is_none()).map(|p| p.addr).collect();
    let mut pkt = Vec::with_capacity(16);
    pkt.extend_from_slice(&BYE_MAGIC);
    pkt.extend_from_slice(&[BYE_VERSION, BYE_KIND_LEAVING, 0, 0]);
    pkt.extend_from_slice(&net.my_id.to_le_bytes());
    for round in 0..3 {
        if round > 0 {
            std::thread::sleep(Duration::from_millis(15));
        }
        for a in &addrs {
            let _ = net.sock.send_to(&pkt, a);
        }
    }
    log!("net: bye sent to {} peer(s)", addrs.len());
    crate::nat::on_exit(); // listen host: remove our UPnP port mapping
}

/// Host, game thread (throttle.rs's hook before each message pump): drop peers that said bye from the game
/// session through the engine's own lost-peer path. The session update (`UpdateNetworkSessionHost` 0x180393830) calls
/// `RemoveDisconnectedSessionPeer` (0x180395f4c, `(session, peer index)`, removal reason 8) for every peer whose
/// observer channel has died; we call it as soon as the bye arrives. Session object: `*0x182a1fe08`; state
/// +0x60ef8 (6 = host, peers live); peer mask +0x5c (17 bits); peer i at +0x60 + i*0x158, its secure address
/// (= our net id) the first u64; our own peer index +0x40.
pub unsafe fn drop_left_peers(base: usize) {
    const SESSION: usize = 0x2a1fe08;
    const RVA_REMOVE_PEER: usize = 0x395f4c;
    let ids: Vec<u64> = std::mem::take(&mut *BYE_QUEUE.lock().unwrap());
    if ids.is_empty() {
        return;
    }
    let session = ((base + SESSION) as *const usize).read();
    let state = if session != 0 { ((session + 0x60ef8) as *const i32).read() } else { -1 };
    if state != 6 {
        log!("net: bye from {} peer(s) but session state is {state}; left to the engine's timeout", ids.len());
        return;
    }
    let mask = ((session + 0x5c) as *const u32).read();
    let me = ((session + 0x40) as *const i32).read();
    let remove: unsafe extern "system" fn(usize, i32) = std::mem::transmute(base + RVA_REMOVE_PEER);
    for id in ids {
        let found = (0..17).find(|&i| mask >> i & 1 != 0 && i != me && ((session + 0x60 + i as usize * 0x158) as *const u64).read() == id);
        match found {
            Some(i) => {
                remove(session, i);
                log!("net: dropped peer {id:#x} (session peer {i}) after its bye");
            }
            None => log!("net: bye from {id:#x}, not in the session (mask {mask:#x})"),
        }
    }
}

/// Side channels (vote.rs): addresses of peers heard from within `max_age` (and not gone with a bye).
pub fn active_peer_addrs(max_age: Duration) -> Vec<SocketAddr> {
    let Some(net) = net() else { return Vec::new() };
    let peers = net.peers.lock().unwrap();
    peers.iter().filter(|p| p.left.is_none() && p.last_seen.is_some_and(|t| t.elapsed() < max_age)).map(|p| p.addr).collect()
}

/// Client: has anything arrived from the host (peer 0) yet?
pub fn host_heard() -> bool {
    let Some(net) = net() else { return false };
    let peers = net.peers.lock().unwrap();
    peers.iter().any(|p| p.id == net_id(0) && p.last_seen.is_some())
}

/// Side channels: is `addr` one of our peers?
pub fn is_peer_addr(addr: SocketAddr) -> bool {
    net().is_some_and(|n| n.peers.lock().unwrap().iter().any(|p| p.addr == addr))
}

/// Side channels: the host's (instance 0's) address, as a client.
pub fn host_addr() -> Option<SocketAddr> {
    let net = net()?;
    let peers = net.peers.lock().unwrap();
    peers.iter().find(|p| p.id == net_id(0)).map(|p| p.addr)
}

/// Side channels: our own network id (0 without a network game).
pub fn my_id() -> u64 {
    net().map_or(0, |n| n.my_id)
}

/// Side channels: one raw datagram on the game socket (no engine header).
pub fn send_raw(addr: SocketAddr, pkt: &[u8]) {
    if let Some(net) = net() {
        let _ = net.sock.send_to(pkt, addr);
    }
}

/// Kicked peers (net ids and addresses), refused for the rest of this process.
static BANNED: Mutex<(Vec<u64>, Vec<SocketAddr>)> = Mutex::new((Vec::new(), Vec::new()));

/// Was this id or address kicked?
pub fn is_banned(id: u64, from: SocketAddr) -> bool {
    let b = BANNED.lock().unwrap();
    b.0.contains(&id) || b.1.contains(&from)
}

/// Host: kick peer `id` (call vote). Stops sending to it, drops it from the game session on the game thread
/// (`drop_left_peers`, the same engine path as a bye) and refuses its datagrams from now on. Returns its address.
pub fn kick(id: u64) -> Option<SocketAddr> {
    let net = net()?;
    let mut peers = net.peers.lock().unwrap();
    let p = peers.iter_mut().find(|p| p.id == id)?;
    p.left.get_or_insert_with(Instant::now);
    let addr = p.addr;
    drop(peers);
    let mut b = BANNED.lock().unwrap();
    b.0.push(id);
    b.1.push(addr);
    drop(b);
    BYE_QUEUE.lock().unwrap().push(id);
    log!("net: kicked peer {id:#x} ({addr}); refused from now on");
    Some(addr)
}

/// Are we the session host (instance 0: listen host or dedicated server)?
pub fn is_host() -> bool {
    net().is_some_and(|n| n.my_id == net_id(0))
}

/// Side channels: network id of the peer at `addr`.
pub fn peer_id_of(addr: SocketAddr) -> Option<u64> {
    net()?.peers.lock().unwrap().iter().find(|p| p.addr == addr).map(|p| p.id)
}

/// Side channels: address of peer `id` (None once it said bye or was kicked).
pub fn peer_addr_of(id: u64) -> Option<SocketAddr> {
    net()?.peers.lock().unwrap().iter().find(|p| p.id == id && p.left.is_none()).map(|p| p.addr)
}
