//! Listen server (menu HOST) address info: the LAN address, the internet address (from `--public-addr`, else the
//! master's `/api/whoami`), and a one-line port-forward hint. Shown on the Host screen, as a banner for ~10 s after
//! the host spawns, in the pause overlay's "Hosting" section, and as a 5 s "<name> joined" toast.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::log;

struct Info {
    port: u16,
    lan: Option<String>,
    /// Internet address, or why it is unknown.
    public: Result<String, String>,
    /// When the host's player first spawned (banner start).
    spawned: Option<Instant>,
    /// Players seen so far, and the latest "joined" toast.
    known: Vec<String>,
    toast: Option<(String, Instant)>,
}

static INFO: Mutex<Option<Info>> = Mutex::new(None);

/// Banner: fully visible for BANNER_SECS - FADE_SECS, then fades out.
const BANNER_SECS: f32 = 10.0;
const FADE_SECS: f32 = 2.0;
const TOAST_SECS: f32 = 5.0;

/// One-line hint for friends who cannot reach the host directly (no NAT punching).
pub fn reach_hint(port: u16) -> String {
    format!("Friends outside your LAN: forward UDP {port} on your router, or use a UDP tunnel (e.g. playit.gg) behind CG-NAT.")
}

/// This PC's LAN IPv4 (the interface that would route to the internet; no packet is sent).
pub fn lan_ip() -> Option<std::net::IpAddr> {
    let s = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("8.8.8.8:80").ok()?;
    let ip = s.local_addr().ok()?.ip();
    (!ip.is_unspecified()).then_some(ip)
}

fn is_public(ip: &std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v) => !(v.is_private() || v.is_loopback() || v.is_link_local() || v.octets()[0] == 100 && (64..128).contains(&v.octets()[1])),
        std::net::IpAddr::V6(v) => !(v.is_loopback() || (v.segments()[0] & 0xfe00) == 0xfc00),
    }
}

/// Called by main for a `--listen` host once its socket is bound. `--public-addr` (a tunnel) wins; otherwise
/// the master is asked (in the background) which IP it sees us from.
pub fn start(port: u16, master: Option<String>, public_addr: String) {
    let lan = lan_ip().map(|ip| format!("{ip}:{port}"));
    let public = if !public_addr.is_empty() {
        Ok(public_addr)
    } else if master.is_some() {
        Err("asking the master ...".into())
    } else {
        Err("unknown (no master)".into())
    };
    log!("listen: hosting on UDP {port}, LAN {}, internet {}", lan.as_deref().unwrap_or("?"), public.as_ref().unwrap_or_else(|e| e));
    let asking = public.is_err() && master.is_some();
    *INFO.lock().unwrap_or_else(|e| e.into_inner()) = Some(Info { port, lan, public, spawned: None, known: Vec::new(), toast: None });
    let Some(master) = master.filter(|_| asking) else { return };
    std::thread::spawn(move || {
        let r = (0..3)
            .find_map(|i| {
                if i > 0 {
                    std::thread::sleep(Duration::from_secs(5));
                }
                crate::browser::whoami(&master).ok()
            })
            .ok_or_else(|| "unknown (master unreachable)".to_string())
            .and_then(|ip| match ip.parse::<std::net::IpAddr>() {
                Ok(a) if is_public(&a) => Ok(if a.is_ipv6() { format!("[{a}]:{port}") } else { format!("{a}:{port}") }),
                Ok(a) => Err(format!("unknown (the master sees {a}, a local address)")),
                Err(_) => Err(format!("unknown (master said {ip:?})")),
            });
        log!("listen: internet address {}", r.as_ref().unwrap_or_else(|e| e));
        if let Some(i) = INFO.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            i.public = r;
        }
    });
}

/// Hosting over (seamless.rs: back to the menu in this process): no banner, no toasts.
pub fn stop() {
    *INFO.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

/// The host's player spawned (game thread): starts the banner.
pub fn note_spawned() {
    if let Some(i) = INFO.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        if i.spawned.is_none() {
            i.spawned = Some(Instant::now());
            log!("listen: host spawned, showing the address banner for {BANNER_SECS} s");
        }
    }
}

/// Current player names (render thread, each frame while hosting): a new name starts a "joined" toast.
pub fn note_players(names: &[String]) {
    let mut g = INFO.lock().unwrap_or_else(|e| e.into_inner());
    let Some(i) = g.as_mut() else { return };
    for n in names {
        if !i.known.contains(n) {
            if !i.known.is_empty() || i.spawned.is_some() && names.len() > 1 {
                log!("listen: {n} joined");
                i.toast = Some((n.clone(), Instant::now()));
            }
            i.known.push(n.clone());
        }
    }
}

pub fn hosting() -> bool {
    INFO.lock().unwrap_or_else(|e| e.into_inner()).is_some()
}

/// Address lines and hint. None when not hosting.
pub fn lines() -> Option<(Vec<String>, String)> {
    let g = INFO.lock().unwrap_or_else(|e| e.into_inner());
    let i = g.as_ref()?;
    let mut v = vec![format!("HOSTING · UDP port {}", i.port)];
    if let Some(l) = &i.lan {
        v.push(format!("LAN address  {l}"));
    }
    // A NAT-assisted listen host (nat.rs) lists the UPnP-mapped or punched address; show that one.
    let public = crate::nat::listing_addr().map_or_else(|| i.public.clone().unwrap_or_else(|e| e), |a| a);
    v.push(format!("Internet address  {public}"));
    let Some(router) = crate::nat::host_status() else { return Some((v, reach_hint(i.port))) };
    let hint = match router.as_str() {
        "Needs port forward" => reach_hint(i.port),
        s if s.starts_with("Checking") => "Checking whether friends can reach you without a port forward ...".into(),
        _ => "Friends can join from the server browser or this address; no port forward needed on most routers.".into(),
    };
    v.push(format!("Router  {router}"));
    Some((v, hint))
}

/// Banner opacity (0 = hidden): 10 s after spawn, fading over the last 2 s.
pub fn banner_alpha() -> f32 {
    let g = INFO.lock().unwrap_or_else(|e| e.into_inner());
    let Some(t) = g.as_ref().and_then(|i| i.spawned) else { return 0.0 };
    let s = t.elapsed().as_secs_f32();
    ((BANNER_SECS - s) / FADE_SECS).clamp(0.0, 1.0)
}

/// "<name> joined" toast and its opacity, for 5 s after a join.
pub fn toast() -> Option<(String, f32)> {
    let g = INFO.lock().unwrap_or_else(|e| e.into_inner());
    let (n, t) = g.as_ref()?.toast.as_ref()?;
    let s = t.elapsed().as_secs_f32();
    (s < TOAST_SECS).then(|| (n.clone(), ((TOAST_SECS - s) / 1.0).clamp(0.0, 1.0)))
}
