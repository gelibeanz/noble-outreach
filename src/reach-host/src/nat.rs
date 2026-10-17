//! NAT traversal for listen servers (menu HOST): master-assisted UDP hole punching plus a UPnP port mapping.
//! Host: registers its game socket with the master's rendezvous (reach-master rendezvous.rs) every 10 s, which tells
//! it its NAT-mapped public endpoint (listed on the master instead of `:port`), and tries UPnP on its router.
//! Joiner: tries the host directly (PUNCH → PUNCH_ACK); if that stays silent it asks the rendezvous to introduce it,
//! then both sides send PUNCH packets until one gets through. The engine's join runs at the same time and retries.
//! All packets go over the game socket (net.rs owns it) so they open the same NAT mapping the game uses.
//! Wire format and limits: notes/networking.md "NAT punch".

use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::log;

pub const MAGIC: [u8; 4] = *b"RNAT";
pub const VERSION: u8 = 1;
pub const LEN: usize = 40;
pub const KIND_REGISTER: u8 = 1;
pub const KIND_REGISTERED: u8 = 2;
pub const KIND_INTRO_REQ: u8 = 3;
pub const KIND_INTRO_REPLY: u8 = 4;
pub const KIND_INTRO_PEER: u8 = 5;
/// Peer to peer, game socket to game socket: a punch, and its answer (token echoed).
pub const KIND_PUNCH: u8 = 6;
pub const KIND_PUNCH_ACK: u8 = 7;
const STATUS_OK: u8 = 0;
const STATUS_UNKNOWN_HOST: u8 = 1;

const REGISTER_EVERY: Duration = Duration::from_secs(10);
const PUNCHES: u32 = 10;
const PUNCH_GAP: Duration = Duration::from_millis(100);

#[derive(Clone, Copy, Debug, PartialEq)]
struct Packet {
    kind: u8,
    status: u8,
    token: u64,
    addr: Option<SocketAddr>,
}

impl Packet {
    fn encode(&self) -> [u8; LEN] {
        let mut b = [0u8; LEN];
        b[..4].copy_from_slice(&MAGIC);
        b[4] = VERSION;
        b[5] = self.kind;
        b[6] = self.status;
        b[8..16].copy_from_slice(&self.token.to_le_bytes());
        match self.addr {
            Some(SocketAddr::V4(a)) => {
                b[7] = 4;
                b[16..18].copy_from_slice(&a.port().to_le_bytes());
                b[20..24].copy_from_slice(&a.ip().octets());
            }
            Some(SocketAddr::V6(a)) => {
                b[7] = 6;
                b[16..18].copy_from_slice(&a.port().to_le_bytes());
                b[20..36].copy_from_slice(&a.ip().octets());
            }
            None => {}
        }
        b
    }

    fn decode(b: &[u8]) -> Option<Packet> {
        if b.len() < LEN || b[..4] != MAGIC || b[4] != VERSION {
            return None;
        }
        let port = u16::from_le_bytes([b[16], b[17]]);
        let addr = match b[7] {
            4 => Some(SocketAddr::from(([b[20], b[21], b[22], b[23]], port))),
            6 => Some(SocketAddr::from((<[u8; 16]>::try_from(&b[20..36]).unwrap(), port))),
            _ => None,
        };
        Some(Packet { kind: b[5], status: b[6], token: u64::from_le_bytes(b[8..16].try_into().unwrap()), addr })
    }
}

// ---- Options (`--rendezvous ip:port|off`, `--no-upnp`) -------------------------------------------------------------

static RV_OVERRIDE: Mutex<Option<String>> = Mutex::new(None);
static NO_UPNP: AtomicBool = AtomicBool::new(false);

pub fn parse(argv: &[String]) -> Result<(), String> {
    if let Some(i) = argv.iter().position(|a| a == "--rendezvous") {
        *RV_OVERRIDE.lock().unwrap() = Some(argv.get(i + 1).ok_or("--rendezvous needs host:port or off")?.clone());
    }
    NO_UPNP.store(argv.iter().any(|a| a == "--no-upnp"), Ordering::Relaxed);
    Ok(())
}

/// The rendezvous address: `--rendezvous` (or env `REACH_RENDEZVOUS`), else the first master that advertises one
/// (`GET /api/rendezvous`). None when off or no master has one.
fn find_rendezvous(masters: &[String]) -> Option<SocketAddr> {
    let spec = RV_OVERRIDE.lock().unwrap().clone().or_else(|| std::env::var("REACH_RENDEZVOUS").ok());
    let spec = match spec {
        Some(s) if s == "off" => return None,
        Some(s) => s,
        None => masters.iter().find_map(|m| match crate::browser::rendezvous(m) {
            Ok(a) => Some(a),
            Err(e) => {
                log!("nat: master {m} has no rendezvous: {e}");
                None
            }
        })?,
    };
    let all: Vec<SocketAddr> = spec.to_socket_addrs().map_err(|e| log!("nat: rendezvous {spec}: {e}")).ok()?.collect();
    let a = all.iter().find(|a| a.is_ipv4()).or(all.first()).copied();
    log!("nat: rendezvous {spec} -> {}", a.map_or("none".into(), |a| a.to_string()));
    a
}

// ---- Shared state ------------------------------------------------------------------------------------------------

static RV: OnceLock<SocketAddr> = OnceLock::new();
static TOKEN: AtomicU64 = AtomicU64::new(0);

fn token() -> u64 {
    let t = TOKEN.load(Ordering::Relaxed);
    if t != 0 {
        return t;
    }
    let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1);
    let t = (seed ^ (std::process::id() as u64).rotate_left(32)).wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    TOKEN.store(t, Ordering::Relaxed);
    t
}

fn send(to: SocketAddr, p: Packet) {
    crate::net::send_raw(to, &p.encode());
}

/// PUNCH packets to `to`, PUNCHES of them PUNCH_GAP apart, from a short-lived thread.
fn punch_burst(to: SocketAddr, why: &'static str) {
    log!("nat: punching {to} ({why})");
    std::thread::spawn(move || {
        for _ in 0..PUNCHES {
            send(to, Packet { kind: KIND_PUNCH, status: 0, token: token(), addr: None });
            std::thread::sleep(PUNCH_GAP);
        }
    });
}

#[derive(Clone, Debug, PartialEq)]
enum Upnp {
    Off,
    Trying,
    /// Mapped; the router's external IP (may itself be private when the ISP uses carrier NAT).
    Mapped(IpAddr),
    Failed(String),
}

struct HostState {
    port: u16,
    /// Our public endpoint as the rendezvous sees it (None until REGISTERED).
    punched: Option<SocketAddr>,
    rv_state: &'static str,
    upnp: Upnp,
    gateway: Option<crate::upnp::Gateway>,
    started: Instant,
}

static HOST: Mutex<Option<HostState>> = Mutex::new(None);

struct ClientState {
    host: SocketAddr,
    text: String,
    changed: Instant,
    connected: bool,
    /// When the status was first drawn (the engine's first frame can come after the connection is made).
    shown: Option<Instant>,
}

static CLIENT: Mutex<Option<ClientState>> = Mutex::new(None);

fn set_client(text: &str, connected: bool) {
    if let Some(c) = CLIENT.lock().unwrap().as_mut() {
        if c.connected && !connected {
            return; // a late "no answer" must not overwrite "connected"
        }
        if c.text != text {
            log!("nat: {text}");
            c.text = text.into();
            c.changed = Instant::now();
            c.shown = None;
        }
        c.connected |= connected;
    }
}

fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => {
            let o = v.octets();
            !(v.is_private() || v.is_loopback() || v.is_link_local() || v.is_unspecified() || (o[0] == 100 && (64..128).contains(&o[1])))
        }
        IpAddr::V6(v) => !(v.is_loopback() || v.is_unspecified() || (v.segments()[0] & 0xfe00) == 0xfc00),
    }
}

// ---- Receive (net.rs's receive thread) -------------------------------------------------------------------------

/// An RNAT datagram on the game socket: handled here, never reaches the engine. False for anything else.
pub fn handle(pkt: &[u8], from: SocketAddr) -> bool {
    if pkt.len() < 4 || pkt[..4] != MAGIC {
        return false;
    }
    let Some(p) = Packet::decode(pkt) else { return true };
    let from_rv = RV.get() == Some(&from);
    static LOGGED: AtomicU64 = AtomicU64::new(0);
    match p.kind {
        KIND_REGISTERED if from_rv => {
            let mut g = HOST.lock().unwrap();
            if let Some(h) = g.as_mut() {
                if p.status != STATUS_OK {
                    if h.rv_state != "refused" {
                        log!("nat: rendezvous refused our registration (status {})", p.status);
                    }
                    h.rv_state = "refused";
                } else if h.punched != p.addr {
                    log!("nat: registered with the rendezvous, public endpoint {}", p.addr.map_or("?".into(), |a| a.to_string()));
                    h.punched = p.addr;
                    h.rv_state = "registered";
                }
            }
        }
        KIND_INTRO_PEER if from_rv => {
            if let Some(j) = p.addr {
                punch_burst(j, "introduced by the rendezvous");
            }
        }
        KIND_INTRO_REPLY if from_rv => {
            let host = CLIENT.lock().unwrap().as_ref().map(|c| c.host);
            match (p.status, p.addr) {
                (STATUS_OK, Some(h)) => {
                    set_client("Connecting via NAT punch ...", false);
                    punch_burst(host.unwrap_or(h), "rendezvous introduced us");
                }
                (STATUS_UNKNOWN_HOST, _) => set_client("Host is not registered for NAT punch; trying direct ...", false),
                (s, _) => set_client(&format!("Rendezvous refused the introduction (status {s}); trying direct ..."), false),
            }
        }
        KIND_PUNCH => {
            if LOGGED.fetch_add(1, Ordering::Relaxed) < 12 {
                log!("nat: punch from {from}");
            }
            // Answer every punch (same size, so no amplification); a joiner also learns the path is open.
            if allow_ack() {
                send(from, Packet { kind: KIND_PUNCH_ACK, status: 0, token: p.token, addr: None });
            }
            if CLIENT.lock().unwrap().as_ref().is_some_and(|c| c.host == from) {
                set_client("Connected to the host (NAT punch)", true);
            }
        }
        KIND_PUNCH_ACK => {
            if LOGGED.fetch_add(1, Ordering::Relaxed) < 12 {
                log!("nat: punch ack from {from}");
            }
            let direct = p.token == token() && CLIENT.lock().unwrap().as_ref().is_some_and(|c| c.host == from);
            if direct {
                let punched = CLIENT.lock().unwrap().as_ref().is_some_and(|c| c.text.contains("NAT punch"));
                set_client(if punched { "Connected to the host (NAT punch)" } else { "Connected to the host directly" }, true);
            }
        }
        _ => {}
    }
    true
}

/// At most 50 PUNCH_ACKs per second in total.
fn allow_ack() -> bool {
    static WIN: Mutex<(Option<Instant>, u32)> = Mutex::new((None, 0));
    let mut w = WIN.lock().unwrap();
    if w.0.is_none_or(|t| t.elapsed() >= Duration::from_secs(1)) {
        *w = (Some(Instant::now()), 0);
    }
    w.1 += 1;
    w.1 <= 50
}

// ---- Host side -------------------------------------------------------------------------------------------------

/// Transport closed (seamless.rs): forget the host / joiner state; the registration thread ends on its own.
pub fn stop() {
    *HOST.lock().unwrap() = None;
    *CLIENT.lock().unwrap() = None;
}

/// Listen host without `--public-addr`: register with the rendezvous and try UPnP, both in the background.
pub fn start_host(port: u16, masters: Vec<String>) {
    let upnp = if NO_UPNP.load(Ordering::Relaxed) { Upnp::Off } else { Upnp::Trying };
    *HOST.lock().unwrap() = Some(HostState { port, punched: None, rv_state: "looking", upnp: upnp.clone(), gateway: None, started: Instant::now() });
    if upnp == Upnp::Trying {
        std::thread::spawn(move || try_upnp(port));
    }
    let gen = crate::net::generation(); // the registration ends with our transport (seamless.rs)
    std::thread::spawn(move || {
        let Some(rv) = find_rendezvous(&masters) else {
            if let Some(h) = HOST.lock().unwrap().as_mut() {
                h.rv_state = "none";
            }
            return;
        };
        let _ = RV.set(rv);
        if let Some(h) = HOST.lock().unwrap().as_mut() {
            h.rv_state = "registering";
        }
        // Re-register every 10 s: keeps our entry and our router's UDP mapping to the rendezvous alive.
        let mut n = 0u32;
        while crate::net::generation() == gen {
            send(rv, Packet { kind: KIND_REGISTER, status: 0, token: token(), addr: None });
            n += 1;
            let registered = HOST.lock().unwrap().as_ref().is_some_and(|h| h.punched.is_some());
            // Until the first answer: resend after 1 s (3 tries), then fall back to the normal period.
            std::thread::sleep(if registered || n > 3 { REGISTER_EVERY } else { Duration::from_secs(1) });
            if n == 4 && !HOST.lock().unwrap().as_ref().is_some_and(|h| h.punched.is_some()) {
                log!("nat: no answer from the rendezvous at {rv}");
                if let Some(h) = HOST.lock().unwrap().as_mut() {
                    h.rv_state = "no answer";
                }
            }
        }
    });
}

fn try_upnp(port: u16) {
    let r = crate::upnp::discover(Duration::from_secs(3)).and_then(|g| {
        log!("upnp: router {:?} ({}), our LAN IP {}", g.name, g.service, g.local_ip);
        // Behind carrier NAT (router's WAN IP private/100.64/10) a mapping cannot help: do not leave one behind.
        let ext = crate::upnp::external_ip(&g)?;
        if !is_public(ext) {
            return Err(format!("router's external IP {ext} is not public (carrier NAT); no mapping added"));
        }
        let lease = crate::upnp::add_udp(&g, port)?;
        log!("upnp: UDP {port} mapped (lease {lease} s), router's external IP {ext}");
        Ok((g, ext))
    });
    let mut gh = HOST.lock().unwrap();
    let Some(h) = gh.as_mut() else { return };
    match r {
        Ok((g, ext)) => {
            h.upnp = Upnp::Mapped(ext);
            h.gateway = Some(g);
        }
        Err(e) => {
            log!("upnp: no mapping: {e}");
            h.upnp = Upnp::Failed(e);
        }
    }
}

/// Remove our UPnP mapping (window closed, leave game, host shutdown; called from net::send_bye). Blocks ≤ ~3 s.
pub fn on_exit() {
    let g = HOST.lock().unwrap().as_mut().and_then(|h| h.gateway.take().map(|g| (g, h.port)));
    if let Some((g, port)) = g {
        match crate::upnp::delete_udp(&g, port) {
            Ok(()) => log!("upnp: UDP {port} mapping removed"),
            Err(e) => log!("upnp: removing UDP {port} failed: {e}"),
        }
    }
}

/// Address to list on the master: the UPnP-mapped one (public router IP), else the punched endpoint. None until
/// one is known (or neither works).
pub fn listing_addr() -> Option<String> {
    let g = HOST.lock().unwrap();
    let h = g.as_ref()?;
    if let Upnp::Mapped(ip) = h.upnp {
        if is_public(ip) {
            return Some(SocketAddr::new(ip, h.port).to_string());
        }
    }
    h.punched.map(|a| a.to_string())
}

/// Still waiting on UPnP or the first rendezvous answer (at most 5 s after start).
pub fn host_pending() -> bool {
    let g = HOST.lock().unwrap();
    let Some(h) = g.as_ref() else { return false };
    h.started.elapsed() < Duration::from_secs(5) && (h.upnp == Upnp::Trying || matches!(h.rv_state, "looking" | "registering"))
}

/// One line for the hosting banner / pause section, None when not a NAT-assisted listen host.
pub fn host_status() -> Option<String> {
    let g = HOST.lock().unwrap();
    let h = g.as_ref()?;
    let upnp_ok = matches!(h.upnp, Upnp::Mapped(ip) if is_public(ip));
    Some(if upnp_ok {
        "Router port opened via UPnP".into()
    } else if h.punched.is_some() {
        "Using NAT punch (via the master)".into()
    } else if h.upnp == Upnp::Trying || matches!(h.rv_state, "looking" | "registering") {
        "Checking the router ...".into()
    } else {
        "Needs port forward".into()
    })
}

// ---- Joiner side -----------------------------------------------------------------------------------------------

/// Client of a listen/dedicated host at `host`: direct first, then the rendezvous. Runs in the background while the
/// engine loads (its join request goes out ~2 s after start and is retried).
pub fn start_client(host: SocketAddr, masters: Vec<String>) {
    *CLIENT.lock().unwrap() = Some(ClientState { host, text: String::new(), changed: Instant::now(), connected: false, shown: None });
    set_client("Contacting the host ...", false);
    // Test knob: skip the direct try and the public-address check, to exercise the punch path on one PC.
    let force = std::env::var_os("REACH_NAT_FORCE_PUNCH").is_some();
    std::thread::spawn(move || {
        for _ in 0..if force { 0 } else { 3 } {
            send(host, Packet { kind: KIND_PUNCH, status: 0, token: token(), addr: None });
            std::thread::sleep(Duration::from_millis(250));
            if connected() {
                return;
            }
        }
        if !is_public(host.ip()) && !force {
            set_client("No answer yet from the host (LAN address, no NAT punch)", false);
            return;
        }
        let Some(rv) = find_rendezvous(&masters) else {
            set_client("No answer yet from the host; no rendezvous for NAT punch", false);
            return;
        };
        let _ = RV.set(rv);
        set_client("Asking the master to introduce us (NAT punch) ...", false);
        for _ in 0..3 {
            send(rv, Packet { kind: KIND_INTRO_REQ, status: 0, token: token(), addr: Some(host) });
            std::thread::sleep(Duration::from_millis(700));
            if connected() {
                return;
            }
        }
        std::thread::sleep(Duration::from_secs(2));
        if !connected() {
            set_client("No answer from the host: it may need a port forward or a UDP tunnel", false);
        }
    });
}

fn connected() -> bool {
    CLIENT.lock().unwrap().as_ref().is_some_and(|c| c.connected)
}

/// Joiner status line and its opacity: shown while connecting and for 5 s after the last change.
pub fn client_status() -> Option<(String, f32)> {
    let mut g = CLIENT.lock().unwrap();
    let c = g.as_mut()?;
    let s = c.shown.get_or_insert_with(Instant::now).elapsed().as_secs_f32();
    let keep = if c.connected { 6.0 } else { 12.0 };
    (s < keep && !c.text.is_empty()).then(|| (c.text.clone(), (keep - s).clamp(0.0, 1.0)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codec() {
        let p = Packet { kind: KIND_INTRO_REQ, status: 0, token: 77, addr: Some("8.8.8.8:4000".parse().unwrap()) };
        assert_eq!(Packet::decode(&p.encode()), Some(p));
    }
}
