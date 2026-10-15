//! UDP rendezvous for NAT hole punching (listen hosts behind home routers). A host sends REGISTER from its game
//! socket; we learn its NAT-mapped public endpoint and tell it. A joiner sends INTRO_REQ (also from its game socket)
//! naming the host's public endpoint; we send the joiner the host's endpoint and the host the joiner's, then both
//! sides send PUNCH packets to each other and the normal join runs. Wire format, limits and the playit setup:
//! notes/networking.md "NAT punch" and notes/master-server.md.
//!
//! Behind a playit UDP tunnel every datagram comes from the local playit agent; with `--rendezvous-proxy-protocol`
//! the real client address is taken from the PROXY protocol v2 header playit puts in front of each datagram.
//! "public" = the client's real internet endpoint (from the header, else the socket source); "route" = the socket
//! source we must send to so the packet goes back the way it came (through the agent's flow).

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::{Duration, Instant};

pub const MAGIC: [u8; 4] = *b"RNAT";
pub const VERSION: u8 = 1;
/// Every packet in both directions is exactly this long, so no reply is larger than its request.
pub const LEN: usize = 40;

pub const KIND_REGISTER: u8 = 1;
pub const KIND_REGISTERED: u8 = 2;
pub const KIND_INTRO_REQ: u8 = 3;
pub const KIND_INTRO_REPLY: u8 = 4;
pub const KIND_INTRO_PEER: u8 = 5;

pub const STATUS_OK: u8 = 0;
pub const STATUS_UNKNOWN_HOST: u8 = 1;
pub const STATUS_REFUSED: u8 = 2;

/// A host re-registers every 10 s (that also keeps its router's mapping alive); entries expire after 30 s.
pub const HOST_TTL: Duration = Duration::from_secs(30);
pub const MAX_HOSTS: usize = 2000;
pub const MAX_HOSTS_PER_IP: usize = 4;
/// Packets per source IP per second; more are dropped silently.
pub const RATE_PER_SEC: u32 = 20;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Packet {
    pub kind: u8,
    pub status: u8,
    pub token: u64,
    pub addr: Option<SocketAddr>,
}

impl Packet {
    pub fn encode(&self) -> [u8; LEN] {
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

    /// None for anything that is not a v1 RNAT packet of the right size.
    pub fn decode(b: &[u8]) -> Option<Packet> {
        if b.len() < LEN || b[..4] != MAGIC || b[4] != VERSION {
            return None;
        }
        let port = u16::from_le_bytes([b[16], b[17]]);
        let addr = match b[7] {
            4 => Some(SocketAddr::new(IpAddr::V4(Ipv4Addr::new(b[20], b[21], b[22], b[23])), port)),
            6 => Some(SocketAddr::new(IpAddr::V6(Ipv6Addr::from(<[u8; 16]>::try_from(&b[20..36]).unwrap())), port)),
            _ => None,
        };
        Some(Packet { kind: b[5], status: b[6], token: u64::from_le_bytes(b[8..16].try_into().unwrap()), addr })
    }
}

const PROXY_SIG: [u8; 12] = [0x0D, 0x0A, 0x0D, 0x0A, 0x00, 0x0D, 0x0A, 0x51, 0x55, 0x49, 0x54, 0x0A];

/// Strip a PROXY protocol v2 header: (client source address if the header carries one, the payload after it).
/// Returns None for a malformed header; a datagram without the signature comes back unchanged.
pub fn strip_proxy_v2(b: &[u8]) -> Option<(Option<SocketAddr>, &[u8])> {
    if b.len() < 16 || b[..12] != PROXY_SIG {
        return Some((None, b));
    }
    if b[12] >> 4 != 2 {
        return None;
    }
    let len = u16::from_be_bytes([b[14], b[15]]) as usize;
    let body = b.get(16..16 + len)?;
    let rest = &b[16 + len..];
    if b[12] & 0x0F == 0 {
        return Some((None, rest)); // LOCAL command (health check): no address
    }
    let src = match b[13] >> 4 {
        1 if body.len() >= 12 => Some(SocketAddr::new(
            IpAddr::V4(Ipv4Addr::new(body[0], body[1], body[2], body[3])),
            u16::from_be_bytes([body[8], body[9]]),
        )),
        2 if body.len() >= 36 => Some(SocketAddr::new(
            IpAddr::V6(Ipv6Addr::from(<[u8; 16]>::try_from(&body[0..16]).unwrap())),
            u16::from_be_bytes([body[32], body[33]]),
        )),
        _ => None,
    };
    Some((src, rest))
}

fn is_private(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => {
            let o = v.octets();
            v.is_private() || v.is_loopback() || v.is_link_local() || v.is_unspecified() || v.is_broadcast() || (o[0] == 100 && (64..128).contains(&o[1]))
        }
        IpAddr::V6(v) => v.is_loopback() || v.is_unspecified() || (v.segments()[0] & 0xfe00) == 0xfc00 || (v.segments()[0] & 0xffc0) == 0xfe80,
    }
}

struct Host {
    route: SocketAddr,
    seen: Instant,
}

pub struct Rendezvous {
    allow_private: bool,
    hosts: HashMap<SocketAddr, Host>, // keyed by the host's public endpoint
    rate: HashMap<IpAddr, (Instant, u32)>,
    pub stats: [u64; 4], // registers, intros ok, intros unknown, dropped
}

impl Rendezvous {
    pub fn new(allow_private: bool) -> Self {
        Rendezvous { allow_private, hosts: HashMap::new(), rate: HashMap::new(), stats: [0; 4] }
    }

    fn allow(&mut self, ip: IpAddr, now: Instant) -> bool {
        if self.rate.len() > 4096 {
            self.rate.retain(|_, (t, _)| now.duration_since(*t) < Duration::from_secs(1));
        }
        let e = self.rate.entry(ip).or_insert((now, 0));
        if now.duration_since(e.0) >= Duration::from_secs(1) {
            *e = (now, 0);
        }
        e.1 += 1;
        e.1 <= RATE_PER_SEC
    }

    pub fn host_count(&mut self, now: Instant) -> usize {
        self.hosts.retain(|_, h| now.duration_since(h.seen) < HOST_TTL);
        self.hosts.len()
    }

    /// One datagram from `public` (arrived from `route`). Returns the packets to send: (destination route, bytes).
    pub fn handle(&mut self, pkt: &[u8], public: SocketAddr, route: SocketAddr, now: Instant) -> Vec<(SocketAddr, [u8; LEN])> {
        let Some(p) = Packet::decode(pkt) else {
            self.stats[3] += 1;
            return Vec::new();
        };
        if !self.allow(public.ip(), now) {
            self.stats[3] += 1;
            return Vec::new();
        }
        let refused = |kind| vec![(route, Packet { kind, status: STATUS_REFUSED, token: p.token, addr: None }.encode())];
        match p.kind {
            KIND_REGISTER => {
                if !self.allow_private && is_private(public.ip()) {
                    return refused(KIND_REGISTERED);
                }
                self.hosts.retain(|_, h| now.duration_since(h.seen) < HOST_TTL);
                if !self.hosts.contains_key(&public)
                    && (self.hosts.len() >= MAX_HOSTS || self.hosts.keys().filter(|k| k.ip() == public.ip()).count() >= MAX_HOSTS_PER_IP)
                {
                    return refused(KIND_REGISTERED);
                }
                self.hosts.insert(public, Host { route, seen: now });
                self.stats[0] += 1;
                vec![(route, Packet { kind: KIND_REGISTERED, status: STATUS_OK, token: p.token, addr: Some(public) }.encode())]
            }
            KIND_INTRO_REQ => {
                let Some(target) = p.addr else { return refused(KIND_INTRO_REPLY) };
                let host = self.hosts.get(&target).filter(|h| now.duration_since(h.seen) < HOST_TTL);
                match host {
                    Some(h) => {
                        self.stats[1] += 1;
                        vec![
                            (h.route, Packet { kind: KIND_INTRO_PEER, status: STATUS_OK, token: p.token, addr: Some(public) }.encode()),
                            (route, Packet { kind: KIND_INTRO_REPLY, status: STATUS_OK, token: p.token, addr: Some(target) }.encode()),
                        ]
                    }
                    None => {
                        self.stats[2] += 1;
                        vec![(route, Packet { kind: KIND_INTRO_REPLY, status: STATUS_UNKNOWN_HOST, token: p.token, addr: Some(target) }.encode())]
                    }
                }
            }
            _ => {
                self.stats[3] += 1;
                Vec::new()
            }
        }
    }
}

/// The UDP task: one socket, PROXY v2 stripped when `proxy` is on, a stats line every 10 min while busy.
pub async fn serve(sock: tokio::net::UdpSocket, allow_private: bool, proxy: bool) {
    let mut rv = Rendezvous::new(allow_private);
    let mut buf = [0u8; 512];
    let mut last_log = Instant::now();
    let mut logged = 0u32;
    loop {
        let (n, route) = match sock.recv_from(&mut buf).await {
            Ok(v) => v,
            Err(_) => continue, // ICMP errors from earlier sends surface here on some platforms
        };
        let (public, pkt) = match if proxy { strip_proxy_v2(&buf[..n]) } else { Some((None, &buf[..n])) } {
            Some((src, rest)) => (src.unwrap_or(route), rest),
            None => continue,
        };
        let now = Instant::now();
        let out = rv.handle(pkt, public, route, now);
        if logged < 20 && !out.is_empty() {
            logged += 1;
            let k = Packet::decode(pkt).map_or(0, |p| p.kind);
            eprintln!("rendezvous: kind {k} from {public} (via {route}) -> {} packet(s)", out.len());
        }
        for (to, b) in out {
            let _ = sock.send_to(&b, to).await;
        }
        if now.duration_since(last_log) >= Duration::from_secs(600) {
            last_log = now;
            let s = rv.stats;
            eprintln!("rendezvous: {} host(s); registers {}, intros {} ok / {} unknown, dropped {}", rv.host_count(now), s[0], s[1], s[2], s[3]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(s: &str) -> SocketAddr {
        s.parse().unwrap()
    }
    fn pkt(kind: u8, token: u64, addr: Option<&str>) -> [u8; LEN] {
        Packet { kind, status: 0, token, addr: addr.map(a) }.encode()
    }

    #[test]
    fn codec_roundtrip() {
        for addr in [Some(a("1.2.3.4:5678")), Some(a("[2001:db8::1]:9")), None] {
            let p = Packet { kind: 3, status: 1, token: 0xdead_beef_0102, addr };
            assert_eq!(Packet::decode(&p.encode()), Some(p));
        }
        assert_eq!(Packet::decode(&[0u8; 39]), None);
        let mut b = pkt(1, 1, None);
        b[4] = 2;
        assert_eq!(Packet::decode(&b), None);
    }

    #[test]
    fn register_and_intro() {
        let mut r = Rendezvous::new(false);
        let t = Instant::now();
        let host = a("8.8.8.8:40000");
        let out = r.handle(&pkt(KIND_REGISTER, 7, None), host, host, t);
        assert_eq!(out.len(), 1);
        let reg = Packet::decode(&out[0].1).unwrap();
        assert_eq!((reg.kind, reg.status, reg.token, reg.addr), (KIND_REGISTERED, STATUS_OK, 7, Some(host)));

        let joiner = a("9.9.9.9:50000");
        let out = r.handle(&pkt(KIND_INTRO_REQ, 42, Some("8.8.8.8:40000")), joiner, joiner, t);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].0, host);
        let peer = Packet::decode(&out[0].1).unwrap();
        assert_eq!((peer.kind, peer.token, peer.addr), (KIND_INTRO_PEER, 42, Some(joiner)));
        let rep = Packet::decode(&out[1].1).unwrap();
        assert_eq!((out[1].0, rep.kind, rep.status, rep.addr), (joiner, KIND_INTRO_REPLY, STATUS_OK, Some(host)));

        // unknown host, and expiry
        let out = r.handle(&pkt(KIND_INTRO_REQ, 1, Some("8.8.8.8:1")), joiner, joiner, t);
        assert_eq!(Packet::decode(&out[0].1).unwrap().status, STATUS_UNKNOWN_HOST);
        let out = r.handle(&pkt(KIND_INTRO_REQ, 1, Some("8.8.8.8:40000")), joiner, joiner, t + HOST_TTL);
        assert_eq!(Packet::decode(&out[0].1).unwrap().status, STATUS_UNKNOWN_HOST);
    }

    #[test]
    fn routes_through_proxy() {
        // Via playit: public endpoint from the PROXY header, replies to the agent's flow socket.
        let mut r = Rendezvous::new(false);
        let t = Instant::now();
        let (host, hroute) = (a("8.8.8.8:40000"), a("127.0.0.1:30001"));
        let (joiner, jroute) = (a("9.9.9.9:50000"), a("127.0.0.1:30002"));
        r.handle(&pkt(KIND_REGISTER, 1, None), host, hroute, t);
        let out = r.handle(&pkt(KIND_INTRO_REQ, 2, Some("8.8.8.8:40000")), joiner, jroute, t);
        assert_eq!((out[0].0, out[1].0), (hroute, jroute));
    }

    #[test]
    fn limits() {
        let t = Instant::now();
        let mut r = Rendezvous::new(false);
        let p = a("127.0.0.1:1");
        assert_eq!(Packet::decode(&r.handle(&pkt(KIND_REGISTER, 1, None), p, p, t)[0].1).unwrap().status, STATUS_REFUSED);
        for port in 0..MAX_HOSTS_PER_IP as u16 {
            let h = SocketAddr::new(a("8.8.4.4:1").ip(), 1000 + port);
            assert_eq!(Packet::decode(&r.handle(&pkt(KIND_REGISTER, 1, None), h, h, t)[0].1).unwrap().status, STATUS_OK);
        }
        let h = a("8.8.4.4:2000");
        assert_eq!(Packet::decode(&r.handle(&pkt(KIND_REGISTER, 1, None), h, h, t)[0].1).unwrap().status, STATUS_REFUSED);
        // rate limit: 20 per second per IP, then silence
        let j = a("1.1.1.1:5");
        let answered = (0..30).filter(|_| !r.handle(&pkt(KIND_INTRO_REQ, 1, Some("8.8.8.8:1")), j, j, t).is_empty()).count();
        assert_eq!(answered, RATE_PER_SEC as usize);
        assert!(!r.handle(&pkt(KIND_INTRO_REQ, 1, Some("8.8.8.8:1")), j, j, t + Duration::from_secs(1)).is_empty());
    }

    #[test]
    fn proxy_v2() {
        let mut b = PROXY_SIG.to_vec();
        b.extend_from_slice(&[0x21, 0x12, 0, 12, 8, 8, 8, 8, 10, 0, 0, 1, 0x9c, 0x40, 0xba, 0xc9]);
        b.extend_from_slice(&pkt(KIND_REGISTER, 5, None));
        let (src, rest) = strip_proxy_v2(&b).unwrap();
        assert_eq!(src, Some(a("8.8.8.8:40000")));
        assert_eq!(Packet::decode(rest).unwrap().token, 5);
        let plain = pkt(KIND_REGISTER, 5, None);
        assert_eq!(strip_proxy_v2(&plain).unwrap(), (None, &plain[..]));
        assert!(strip_proxy_v2(&b[..20]).is_none());
    }
}
