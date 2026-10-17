//! Server discovery client: register this instance with a master server, list servers, pick one to join.
//! Hand-rolled HTTP/1.1 over `TcpStream`; https:// wraps it in rustls (ring + webpki-roots). JSON bodies.
//! The master's API and limits: notes/master-server.md.

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::log;

pub const HEARTBEAT_SECS: u64 = 15;

/// Masters tried in order when neither `--master`/`REACH_MASTER` nor the settings name one. The first that answers
/// wins. Set at build time with `NOBLE_DEFAULT_MASTERS` (comma list; build.rs reruns when it changes); empty by
/// default, so a public build has no master until the player adds one (Direct Connect, Favourites and LAN work
/// without).
pub const DEFAULT_MASTERS: &str = match option_env!("NOBLE_DEFAULT_MASTERS") {
    Some(v) => v,
    None => "",
};

/// Shown where a master list would be when none is configured.
pub const NO_MASTER: &str = "no master configured: add one in Settings, or use Direct Connect";

pub fn default_masters() -> Vec<String> {
    split_list(DEFAULT_MASTERS)
}
/// Per-master timeout while looking for one that answers.
const FETCH_TIMEOUT: Duration = Duration::from_secs(3);

fn split_list(spec: &str) -> Vec<String> {
    spec.split(',').map(|u| u.trim().trim_end_matches('/').to_string()).filter(|u| !u.is_empty()).collect()
}

/// Masters to use, in order: `--master` (main.rs puts it in `REACH_MASTER`, so a relaunched menu keeps it), the
/// settings' master URL (comma-separated list allowed), else `default_masters()`.
pub fn master_list() -> Vec<String> {
    if let Some(v) = std::env::var("REACH_MASTER").ok().map(|v| split_list(&v)).filter(|v| !v.is_empty()) {
        return v;
    }
    let s = crate::ui::settings::get().master_url;
    let defaults = default_masters();
    // A saved value made only of default masters (early settings files stored the old single default) means
    // "use the defaults", so it picks up masters added to the build later.
    let saved = split_list(&s);
    if !saved.is_empty() && !saved.iter().all(|u| defaults.contains(u)) {
        return saved;
    }
    defaults
}

/// A `--master`/`--join-server` value: comma-separated URLs, or empty for `master_list()`.
pub fn master_list_from(spec: &str) -> Vec<String> {
    let v = split_list(spec);
    if v.is_empty() { master_list() } else { v }
}

/// First master in `urls` that answers: (its url, its list).
pub fn fetch_any(urls: &[String]) -> Result<(String, Vec<ServerInfo>), String> {
    let mut errs = Vec::new();
    for u in urls {
        match fetch_timeout(u, FETCH_TIMEOUT) {
            Ok(l) => return Ok((u.clone(), l)),
            Err(e) => {
                log!("browser: master {u} failed: {e}");
                errs.push(format!("{u}: {e}"));
            }
        }
    }
    Err(if errs.is_empty() { NO_MASTER.into() } else { errs.join("; ") })
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ServerInfo {
    pub name: String,
    #[serde(default)]
    pub version: String,
    /// "ip:port"; empty ip = the master fills in our public IP.
    #[serde(default)]
    pub addr: String,
    pub network_id: String,
    pub session_id: String,
    pub map: String,
    pub game_variant: String,
    pub players: u32,
    pub max_players: u32,
    #[serde(default)]
    pub dedicated: bool,
}

trait ReadWrite: Read + Write {}
impl<T: Read + Write> ReadWrite for T {}

/// One HTTP request, `Connection: close`. Returns (status, body). http:// plain, https:// via rustls (ring, bundled roots).
fn http(method: &str, url: &str, body: Option<&str>, timeout: Duration) -> Result<(u16, String), String> {
    let (tls, rest) = if let Some(r) = url.strip_prefix("https://") {
        (true, r)
    } else if let Some(r) = url.strip_prefix("http://") {
        (false, r)
    } else {
        return Err("master URL must start with http:// or https://".into());
    };
    let (hostport, path) = rest.split_once('/').map_or((rest, "/"), |(h, p)| (h, &rest[rest.len() - p.len() - 1..]));
    let def_port = if tls { 443 } else { 80 };
    let hostport = if hostport.contains(':') { hostport.to_string() } else { format!("{hostport}:{def_port}") };
    let addr = hostport.to_socket_addrs().map_err(|e| format!("resolve {hostport}: {e}"))?.next().ok_or("no address")?;
    let s = TcpStream::connect_timeout(&addr, timeout).map_err(|e| format!("connect {addr}: {e}"))?;
    s.set_read_timeout(Some(timeout)).ok();
    s.set_write_timeout(Some(timeout)).ok();
    let name = hostport.rsplit_once(':').map_or(hostport.as_str(), |(h, _)| h).to_string();
    let host = if hostport.ends_with(&format!(":{def_port}")) { name.clone() } else { hostport.clone() };
    let mut s: Box<dyn ReadWrite> = if tls {
        let mut roots = rustls::RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        let cfg = rustls::ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .map_err(|e| format!("tls: {e}"))?
            .with_root_certificates(roots)
            .with_no_client_auth();
        let sni = rustls::pki_types::ServerName::try_from(name).map_err(|e| format!("tls name: {e}"))?;
        let conn = rustls::ClientConnection::new(Arc::new(cfg), sni).map_err(|e| format!("tls: {e}"))?;
        Box::new(rustls::StreamOwned::new(conn, s))
    } else {
        Box::new(s)
    };
    let mut req = format!("{method} {path} HTTP/1.1\r\nHost: {host}\r\nUser-Agent: reach-host\r\nConnection: close\r\nAccept: application/json\r\n");
    if let Some(b) = body {
        req += &format!("Content-Type: application/json\r\nContent-Length: {}\r\n", b.len());
    }
    req += "\r\n";
    s.write_all(req.as_bytes()).and_then(|_| body.map_or(Ok(()), |b| s.write_all(b.as_bytes()))).map_err(|e| format!("send: {e}"))?;
    let mut raw = Vec::new();
    // Servers that drop the TLS socket without close_notify surface as UnexpectedEof; data read so far is complete.
    match (&mut s).take(1 << 20).read_to_end(&mut raw) {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof && !raw.is_empty() => {}
        Err(e) => return Err(format!("read: {e}")),
    }
    let text = String::from_utf8_lossy(&raw);
    let (head, body) = text.split_once("\r\n\r\n").ok_or("malformed response")?;
    let status = head.split_whitespace().nth(1).and_then(|c| c.parse().ok()).ok_or("malformed status line")?;
    let chunked = head.to_ascii_lowercase().contains("transfer-encoding: chunked");
    Ok((status, if chunked { dechunk(body) } else { body.to_string() }))
}

fn dechunk(mut b: &str) -> String {
    let mut out = String::new();
    while let Some((len, rest)) = b.split_once("\r\n") {
        let n = usize::from_str_radix(len.split(';').next().unwrap_or("").trim(), 16).unwrap_or(0);
        if n == 0 || rest.len() < n {
            break;
        }
        out.push_str(&rest[..n]);
        b = rest[n..].trim_start_matches("\r\n");
    }
    out
}

fn api(master: &str) -> String {
    format!("{}/api/servers", master.trim_end_matches('/'))
}

/// Server list, as the master returns it (most players first).
pub fn fetch(master: &str) -> Result<Vec<ServerInfo>, String> {
    fetch_timeout(master, Duration::from_secs(5))
}

fn fetch_timeout(master: &str, timeout: Duration) -> Result<Vec<ServerInfo>, String> {
    let (status, body) = http("GET", &api(master), None, timeout)?;
    if status != 200 {
        return Err(format!("master answered {status}: {body}"));
    }
    serde_json::from_str(&body).map_err(|e| format!("bad list from master: {e}"))
}

/// `GET <master>/api/whoami`: the IP the master sees us from (a listen host shows it as its internet address).
pub fn whoami(master: &str) -> Result<String, String> {
    let url = format!("{}/api/whoami", master.trim_end_matches('/'));
    let (status, body) = http("GET", &url, None, Duration::from_secs(5))?;
    if status != 200 {
        return Err(format!("master answered {status}: {body}"));
    }
    let v: serde_json::Value = serde_json::from_str(&body).map_err(|e| format!("bad whoami answer: {e}"))?;
    v["ip"].as_str().map(String::from).ok_or_else(|| "whoami answer has no ip".into())
}

/// `GET <master>/api/rendezvous`: the master's NAT-punch UDP address ("host:port", maybe a name), nat.rs.
pub fn rendezvous(master: &str) -> Result<String, String> {
    let url = format!("{}/api/rendezvous", master.trim_end_matches('/'));
    let (status, body) = http("GET", &url, None, FETCH_TIMEOUT)?;
    if status != 200 {
        return Err(format!("master answered {status}"));
    }
    let v: serde_json::Value = serde_json::from_str(&body).map_err(|e| format!("bad rendezvous answer: {e}"))?;
    v["addr"].as_str().filter(|a| !a.is_empty()).map(String::from).ok_or_else(|| "rendezvous answer has no addr".into())
}

pub fn heartbeat(master: &str, info: &ServerInfo) -> Result<(), String> {
    let (status, body) = http("POST", &api(master), Some(&serde_json::to_string(info).unwrap()), Duration::from_secs(5))?;
    if status == 200 { Ok(()) } else { Err(format!("master answered {status}: {body}")) }
}

/// Text table for `--browse`.
pub fn table(list: &[ServerInfo]) -> String {
    let mut s = format!("{:>3}  {:<28} {:<22} {:<14} {:<16} {:>7}  {}\n", "#", "name", "address", "map", "variant", "players", "");
    for (i, v) in list.iter().enumerate() {
        s += &format!(
            "{:>3}  {:<28} {:<22} {:<14} {:<16} {:>4}/{:<2}  {}\n",
            i, v.name, v.addr, v.map, v.game_variant, v.players, v.max_players, if v.dedicated { "dedicated" } else { "" }
        );
    }
    if list.is_empty() {
        s += "(no servers)\n";
    }
    s
}

/// `--browse [url[,url]]`: print the list (with a live UDP query per server for players and ping) and return.
pub fn browse(master: &str) -> Result<(), String> {
    let (master, mut list) = fetch_any(&master_list_from(master))?;
    let master = master.as_str();
    let targets: Vec<_> = list.iter().map(|s| crate::query::resolve(&s.addr).ok()).collect();
    let addrs: Vec<_> = targets.iter().map(|a| a.unwrap_or_else(|| std::net::SocketAddr::from(([0, 0, 0, 0], 9)))).collect();
    let replies = crate::query::query_many(&addrs, Duration::from_millis(1500));
    let mut pings = Vec::new();
    for (s, r) in list.iter_mut().zip(&replies) {
        if let Some(r) = r {
            s.players = r.players as u32;
        }
        pings.push(r.as_ref().map_or("-".to_string(), |r| format!("{} ms", r.ping_ms)));
    }
    let mut t = table(&list);
    t = t.lines().enumerate().map(|(i, l)| match i {
        0 => format!("{l}  ping\n"),
        i if i <= pings.len() => format!("{l}  {}\n", pings[i - 1]),
        _ => format!("{l}\n"),
    }).collect();
    print!("{t}");
    // Proton drops a console exe's stdout, so keep a copy next to the exe for scripts.
    let _ = std::fs::write(std::env::current_exe().unwrap_or_default().with_file_name("browse.txt"), &t);
    log!("browse: {} server(s) from {master}", list.len());
    Ok(())
}

/// `--join-server` selector: a list index or a (case-insensitive) name, exact then prefix.
pub fn pick(list: &[ServerInfo], sel: &str) -> Result<ServerInfo, String> {
    if let Ok(i) = sel.parse::<usize>() {
        return list.get(i).cloned().ok_or_else(|| format!("no server #{i} (the list has {})", list.len()));
    }
    let l = sel.to_lowercase();
    list.iter()
        .find(|v| v.name.to_lowercase() == l)
        .or_else(|| list.iter().find(|v| v.name.to_lowercase().starts_with(&l)))
        .cloned()
        .ok_or_else(|| format!("no server named {sel:?}"))
}

/// What a hosting instance reports. Player count is read live from the slot 30 roster.
pub struct Registration {
    pub master: String,
    pub info: ServerInfo,
}

pub fn parse_hex(s: &str) -> Result<u64, String> {
    u64::from_str_radix(s.trim_start_matches("0x"), 16).map_err(|e| format!("bad hex id {s:?}: {e}"))
}

/// `host:port` with a DNS name → `ipv4:port` (IPv6 only if there's no IPv4); None for numeric or empty input.
fn resolve_public(addr: &str) -> Option<String> {
    let (host, _) = addr.rsplit_once(':')?;
    if host.is_empty() || host.parse::<std::net::IpAddr>().is_ok() || host.starts_with('[') {
        return None;
    }
    let all: Vec<_> = addr.to_socket_addrs().ok()?.collect();
    all.iter().find(|a| a.is_ipv4()).or(all.first()).map(|a| a.to_string())
}

/// Background thread: POST every 15 s. A failed beat is logged (first few) and retried.
pub fn start_heartbeat(reg: Registration) {
    let gen = crate::net::generation(); // the thread ends with our transport (seamless.rs)
    std::thread::spawn(move || {
        let mut info = reg.info;
        let public = info.addr.clone();
        // No --public-addr (":port"): a listen host lists the address nat.rs found (UPnP mapping or the punched
        // endpoint) once known; wait a few seconds for it so the list does not show a stale ":port" entry first.
        let nat = public.starts_with(':');
        while nat && crate::nat::host_pending() {
            std::thread::sleep(Duration::from_millis(200));
        }
        // `--master a,b`: beat to the first that answers and stay with it while it does.
        let masters = master_list_from(&reg.master);
        let mut cur = 0usize;
        let mut fails = 0u32;
        let mut last = u32::MAX;
        while crate::net::generation() == gen {
            // A tunnel hostname (e.g. a playit UDP tunnel's name:port) is resolved on every beat, preferring IPv4,
            // since the master and clients take a numeric ip:port and the tunnel's IP can change.
            if let Some(a) = crate::nat::listing_addr().filter(|_| nat) {
                if info.addr != a {
                    log!("master: listing NAT address {a}");
                }
                info.addr = a;
            } else if let Some(a) = resolve_public(&public) {
                if info.addr != a {
                    log!("master: public address {public} -> {a}");
                }
                info.addr = a;
            }
            let roster = match crate::net::ROSTER_COUNT.load(std::sync::atomic::Ordering::Relaxed) {
                0 => crate::lobby::players_plus_one(), // campaign lobby: no roster until the mission starts
                r => r,
            };
            // ROSTER_COUNT is the slot 30 count + 1 (0 = no roster yet). A dedicated server is not a player itself,
            // so it reports 0 while empty; a player host always counts itself.
            let min = if info.dedicated { 0 } else { 1 };
            info.players = if roster == 0 { min } else { (roster - 1) as u32 }.clamp(min, info.max_players);
            let mut res = Err(String::new());
            for k in 0..masters.len() {
                let i = (cur + k) % masters.len();
                res = heartbeat(&masters[i], &info);
                if res.is_ok() {
                    if i != cur {
                        log!("master: switched to {}", masters[i]);
                        cur = i;
                    }
                    break;
                }
            }
            match res {
                Ok(()) => {
                    if fails > 0 || last != info.players {
                        log!("master: registered at {} ({} of {} players)", masters[cur], info.players, info.max_players);
                    }
                    fails = 0;
                    last = info.players;
                }
                Err(e) => {
                    fails += 1;
                    if fails <= 3 || fails % 20 == 0 {
                        log!("master: heartbeat failed ({fails}x): {e}");
                    }
                }
            }
            // Beat again early when the roster changes (a player joined, or left with a bye), so the list is current.
            for _ in 0..HEARTBEAT_SECS {
                std::thread::sleep(Duration::from_secs(1));
                if crate::net::ROSTER_COUNT.load(std::sync::atomic::Ordering::Relaxed) != roster {
                    break;
                }
            }
        }
    });
}
