//! Server registry: validation, per-IP limits and expiry. Pure logic with an injected clock, so it is unit-tested.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

pub const HEARTBEAT_SECS: u64 = 15;
pub const TTL: Duration = Duration::from_secs(45);
pub const MAX_PER_IP: usize = 4;
pub const MAX_NAME: usize = 48;
pub const MAX_SHORT: usize = 64;
pub const MAX_PLAYERS: u32 = 16;
/// Hard cap on all entries so a botnet cannot grow memory without bound.
pub const MAX_TOTAL: usize = 2000;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ServerInfo {
    pub name: String,
    #[serde(default)]
    pub version: String,
    /// "ip:port" of the UDP game address; "" or ":port" / "0.0.0.0:port" = fill in the request's source IP.
    #[serde(default)]
    pub addr: String,
    /// Hex u64 (the host's engine secure address).
    pub network_id: String,
    /// Hex u64.
    pub session_id: String,
    pub map: String,
    pub game_variant: String,
    pub players: u32,
    pub max_players: u32,
    #[serde(default)]
    pub dedicated: bool,
}

#[derive(Serialize, Clone, Debug)]
pub struct Listed {
    #[serde(flatten)]
    pub info: ServerInfo,
    pub age_secs: u64,
}

struct Entry {
    info: ServerInfo,
    source: IpAddr,
    seen: Instant,
}

pub struct Registry {
    allow_private: bool,
    entries: HashMap<String, Entry>, // keyed by the game address
}

fn parse_hex(s: &str) -> Option<u64> {
    u64::from_str_radix(s.trim_start_matches("0x"), 16).ok().filter(|v| *v != 0)
}

fn is_private(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => {
            let o = v.octets();
            v.is_private() || v.is_loopback() || v.is_link_local() || v.is_unspecified() || v.is_broadcast()
                || (o[0] == 100 && (64..128).contains(&o[1])) // CGNAT / VPN overlays
        }
        IpAddr::V6(v) => v.is_loopback() || v.is_unspecified() || (v.segments()[0] & 0xfe00) == 0xfc00 || (v.segments()[0] & 0xffc0) == 0xfe80,
    }
}

fn clean(s: &str, max: usize, what: &str) -> Result<(), String> {
    if s.chars().count() > max {
        return Err(format!("{what} longer than {max} characters"));
    }
    if s.chars().any(|c| c.is_control()) {
        return Err(format!("{what} has control characters"));
    }
    Ok(())
}

impl Registry {
    pub fn new(allow_private: bool) -> Self {
        Registry { allow_private, entries: HashMap::new() }
    }

    pub fn expire(&mut self, now: Instant) {
        self.entries.retain(|_, e| now.duration_since(e.seen) < TTL);
    }

    /// Validate and store a heartbeat. `source` is the (possibly proxy-forwarded) client IP.
    pub fn register(&mut self, mut info: ServerInfo, source: IpAddr, now: Instant) -> Result<(), String> {
        self.expire(now);
        info.name = info.name.trim().to_string();
        if info.name.is_empty() {
            return Err("name is empty".into());
        }
        clean(&info.name, MAX_NAME, "name")?;
        clean(&info.version, 32, "version")?;
        clean(&info.map, MAX_SHORT, "map")?;
        clean(&info.game_variant, MAX_SHORT, "game_variant")?;
        if info.map.is_empty() || info.game_variant.is_empty() {
            return Err("map and game_variant are required".into());
        }
        if parse_hex(&info.network_id).is_none() || parse_hex(&info.session_id).is_none() {
            return Err("network_id and session_id must be non-zero hex u64".into());
        }
        if info.max_players == 0 || info.max_players > MAX_PLAYERS || info.players > info.max_players {
            return Err(format!("players/max_players out of range (max_players 1..={MAX_PLAYERS}, players <= max_players)"));
        }
        // Fill the host part from the source address when it is missing.
        let (host, port) = info.addr.rsplit_once(':').unwrap_or(("", info.addr.as_str()));
        let port: u16 = port.parse().map_err(|_| "addr needs a port".to_string())?;
        let ip: IpAddr = match host.trim() {
            "" | "0.0.0.0" | "[::]" => source,
            h => h.trim_matches(['[', ']']).parse().map_err(|_| "addr is not ip:port".to_string())?,
        };
        if port == 0 {
            return Err("addr port is 0".into());
        }
        if !self.allow_private && is_private(ip) {
            return Err("addr is a private/loopback address".into());
        }
        let addr = SocketAddr::new(ip, port);
        info.addr = addr.to_string();

        let key = info.addr.clone();
        if !self.entries.contains_key(&key) {
            if self.entries.len() >= MAX_TOTAL {
                return Err("registry full".into());
            }
            if self.entries.values().filter(|e| e.source == source).count() >= MAX_PER_IP {
                return Err(format!("more than {MAX_PER_IP} servers from one address"));
            }
        } else if self.entries[&key].source != source {
            return Err("addr already registered from another source".into());
        }
        self.entries.insert(key, Entry { info, source, seen: now });
        Ok(())
    }

    /// Live servers, most players first, then by name.
    pub fn list(&mut self, now: Instant) -> Vec<Listed> {
        self.expire(now);
        let mut v: Vec<Listed> = self
            .entries
            .values()
            .map(|e| Listed { info: e.info.clone(), age_secs: now.duration_since(e.seen).as_secs() })
            .collect();
        v.sort_by(|a, b| b.info.players.cmp(&a.info.players).then_with(|| a.info.name.cmp(&b.info.name)));
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(name: &str, addr: &str, players: u32) -> ServerInfo {
        ServerInfo {
            name: name.into(),
            version: "0.1".into(),
            addr: addr.into(),
            network_id: "5245414348000001".into(),
            session_id: "5245414348530001".into(),
            map: "forge_world".into(),
            game_variant: "slayer_054.bin".into(),
            players,
            max_players: 16,
            dedicated: false,
        }
    }
    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn expiry() {
        let mut r = Registry::new(false);
        let t = Instant::now();
        r.register(info("a", "8.8.8.8:47800", 1), ip("8.8.8.8"), t).unwrap();
        assert_eq!(r.list(t + Duration::from_secs(44)).len(), 1);
        // a heartbeat refreshes it
        r.register(info("a", "8.8.8.8:47800", 2), ip("8.8.8.8"), t + Duration::from_secs(40)).unwrap();
        assert_eq!(r.list(t + Duration::from_secs(80)).len(), 1);
        assert_eq!(r.list(t + Duration::from_secs(86)).len(), 0);
    }

    #[test]
    fn per_ip_limit() {
        let mut r = Registry::new(false);
        let t = Instant::now();
        for p in 0..MAX_PER_IP {
            r.register(info("s", &format!(":{}", 47800 + p), 0), ip("8.8.8.8"), t).unwrap();
        }
        assert!(r.register(info("s", ":50000", 0), ip("8.8.8.8"), t).is_err());
        // re-posting an existing one is fine, another IP is fine
        r.register(info("s", ":47800", 3), ip("8.8.8.8"), t).unwrap();
        r.register(info("s", ":50000", 0), ip("1.1.1.1"), t).unwrap();
        // expired entries free the slot
        r.register(info("s", ":50001", 0), ip("8.8.8.8"), t + Duration::from_secs(60)).unwrap();
    }

    #[test]
    fn address_fill_and_hijack() {
        let mut r = Registry::new(false);
        let t = Instant::now();
        r.register(info("a", ":47800", 1), ip("8.8.8.8"), t).unwrap();
        assert_eq!(r.list(t)[0].info.addr, "8.8.8.8:47800");
        assert!(r.register(info("b", "8.8.8.8:47800", 1), ip("1.1.1.1"), t).is_err());
    }

    #[test]
    fn validation() {
        let mut r = Registry::new(false);
        let t = Instant::now();
        let s = ip("8.8.8.8");
        assert!(r.register(info("", ":1", 0), s, t).is_err());
        assert!(r.register(info(&"x".repeat(MAX_NAME + 1), ":1", 0), s, t).is_err());
        assert!(r.register(info("a\nb", ":1", 0), s, t).is_err());
        assert!(r.register(info("a", "nonsense", 0), s, t).is_err());
        assert!(r.register(info("a", ":0", 0), s, t).is_err());
        assert!(r.register(info("a", ":1", 17), s, t).is_err());
        let mut bad = info("a", ":1", 0);
        bad.network_id = "zz".into();
        assert!(r.register(bad, s, t).is_err());
        let mut bad = info("a", ":1", 0);
        bad.session_id = "0".into();
        assert!(r.register(bad, s, t).is_err());
    }

    #[test]
    fn private_addresses() {
        let t = Instant::now();
        for a in ["127.0.0.1:1", "10.0.0.5:1", "192.168.1.2:1", "100.64.0.9:1"] {
            assert!(Registry::new(false).register(info("a", a, 0), ip("8.8.8.8"), t).is_err(), "{a}");
        }
        // source-filled private IP is rejected too, unless allowed
        assert!(Registry::new(false).register(info("a", ":1", 0), ip("127.0.0.1"), t).is_err());
        Registry::new(true).register(info("a", ":1", 0), ip("127.0.0.1"), t).unwrap();
    }

    #[test]
    fn sorted() {
        let mut r = Registry::new(false);
        let t = Instant::now();
        r.register(info("b", ":1", 2), ip("1.1.1.1"), t).unwrap();
        r.register(info("a", ":2", 2), ip("1.1.1.2"), t).unwrap();
        r.register(info("c", ":3", 5), ip("1.1.1.3"), t).unwrap();
        let names: Vec<_> = r.list(t).into_iter().map(|l| l.info.name).collect();
        assert_eq!(names, ["c", "a", "b"]);
    }
}
