//! Minimal UPnP IGD client (SSDP discovery + SOAP over plain HTTP): ask the home router to forward our UDP game port
//! while a listen server runs, and remove the mapping when it stops. No dependencies; only what reach-host needs:
//! GetExternalIPAddress, AddPortMapping, DeletePortMapping on a WANIPConnection (v1/v2) or WANPPPConnection service.
//! Behaviour and the results seen on real routers: notes/networking.md "UPnP".

use std::io::{Read, Write};
use std::net::{IpAddr, SocketAddr, TcpStream, ToSocketAddrs, UdpSocket};
use std::time::{Duration, Instant};

use crate::log;

const SSDP: &str = "239.255.255.250:1900";
const SEARCH: &[&str] = &[
    "urn:schemas-upnp-org:service:WANIPConnection:1",
    "urn:schemas-upnp-org:service:WANIPConnection:2",
    "urn:schemas-upnp-org:service:WANPPPConnection:1",
    "urn:schemas-upnp-org:device:InternetGatewayDevice:1",
];
/// Lease asked for; the mapping is removed on exit, the lease only matters if we crash. Some routers accept only
/// permanent leases (error 725): then we ask again with 0.
pub const LEASE_SECS: u32 = 3600;
const DESCRIPTION: &str = "Noble Outreach";

#[derive(Clone, Debug)]
pub struct Gateway {
    /// Absolute control URL of the WAN connection service.
    pub control: String,
    pub service: String,
    /// Our address on the interface that reaches the router (the mapping's internal client).
    pub local_ip: IpAddr,
    /// Router model from the description, for the log.
    pub name: String,
}

/// SSDP search, then the first answering router's description. `wait` bounds the search.
pub fn discover(wait: Duration) -> Result<Gateway, String> {
    let sock = UdpSocket::bind("0.0.0.0:0").map_err(|e| format!("ssdp bind: {e}"))?;
    sock.set_read_timeout(Some(Duration::from_millis(250))).ok();
    let dest: SocketAddr = SSDP.parse().unwrap();
    for st in SEARCH {
        let m = format!("M-SEARCH * HTTP/1.1\r\nHOST: {SSDP}\r\nMAN: \"ssdp:discover\"\r\nMX: 2\r\nST: {st}\r\n\r\n");
        sock.send_to(m.as_bytes(), dest).map_err(|e| format!("ssdp send: {e}"))?;
    }
    let t0 = Instant::now();
    let mut buf = [0u8; 2048];
    let mut tried = Vec::new();
    let mut last_err = "no UPnP router answered".to_string();
    while t0.elapsed() < wait {
        let Ok((n, from)) = sock.recv_from(&mut buf) else { continue };
        let text = String::from_utf8_lossy(&buf[..n]);
        let Some(loc) = header(&text, "location") else { continue };
        if tried.contains(&loc) {
            continue;
        }
        tried.push(loc.clone());
        log!("upnp: answer from {from}: {loc}");
        match describe(&loc) {
            Ok(g) => return Ok(g),
            Err(e) => last_err = format!("{loc}: {e}"),
        }
    }
    Err(last_err)
}

fn header(text: &str, name: &str) -> Option<String> {
    text.lines().find_map(|l| {
        let (k, v) = l.split_once(':')?;
        k.trim().eq_ignore_ascii_case(name).then(|| v.trim().to_string())
    })
}

/// `http://host:port/path` → (host:port, path).
fn split_url(url: &str) -> Result<(String, String), String> {
    let rest = url.strip_prefix("http://").ok_or_else(|| format!("not an http URL: {url}"))?;
    let (hp, path) = rest.find('/').map_or((rest, "/"), |i| (&rest[..i], &rest[i..]));
    let hp = if hp.contains(':') { hp.to_string() } else { format!("{hp}:80") };
    Ok((hp, path.to_string()))
}

/// One HTTP/1.1 request with `Connection: close`: (status, body, our local IP on that connection).
fn http(method: &str, url: &str, headers: &str, body: &str) -> Result<(u16, String, IpAddr), String> {
    let (hp, path) = split_url(url)?;
    let addr = hp.to_socket_addrs().map_err(|e| format!("resolve {hp}: {e}"))?.next().ok_or("no address")?;
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_secs(3)).map_err(|e| format!("connect {addr}: {e}"))?;
    s.set_read_timeout(Some(Duration::from_secs(5))).ok();
    let local = s.local_addr().map_err(|e| e.to_string())?.ip();
    let req = format!("{method} {path} HTTP/1.1\r\nHost: {hp}\r\nConnection: close\r\nContent-Length: {}\r\n{headers}\r\n{body}", body.len());
    s.write_all(req.as_bytes()).map_err(|e| format!("send: {e}"))?;
    let mut raw = Vec::new();
    let _ = s.take(1 << 20).read_to_end(&mut raw); // some routers reset instead of closing; keep what arrived
    let text = String::from_utf8_lossy(&raw).into_owned();
    let (head, body) = text.split_once("\r\n\r\n").ok_or("malformed HTTP answer")?;
    let status = head.split_whitespace().nth(1).and_then(|c| c.parse().ok()).ok_or("malformed status line")?;
    let body = if head.to_ascii_lowercase().contains("transfer-encoding: chunked") { dechunk(body) } else { body.to_string() };
    Ok((status, body, local))
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

/// Text of the first `<tag>…</tag>` (namespace prefixes on the tag allowed), from `from` on.
fn tag(xml: &str, name: &str, from: usize) -> Option<(String, usize)> {
    let mut i = from;
    while let Some(p) = xml[i..].find('<') {
        let start = i + p + 1;
        let end = start + xml[start..].find('>')?;
        let t = &xml[start..end];
        let bare = t.rsplit(':').next().unwrap_or(t);
        if bare == name && !t.starts_with('/') {
            let close = end + 1 + xml[end + 1..].find("</")?;
            return Some((xml[end + 1..close].trim().to_string(), close));
        }
        i = end;
    }
    None
}

fn describe(location: &str) -> Result<Gateway, String> {
    let (status, xml, local_ip) = http("GET", location, "", "")?;
    if status != 200 {
        return Err(format!("description answered {status}"));
    }
    let name = tag(&xml, "modelName", 0).or_else(|| tag(&xml, "friendlyName", 0)).map(|t| t.0).unwrap_or_default();
    let base = tag(&xml, "URLBase", 0).map(|t| t.0).unwrap_or_else(|| {
        let (hp, _) = split_url(location).unwrap_or_default();
        format!("http://{hp}")
    });
    // Prefer an IP connection over PPP; take the first of each kind in document order.
    for want in ["WANIPConnection", "WANPPPConnection"] {
        let mut i = 0;
        while let Some((st, at)) = tag(&xml, "serviceType", i) {
            i = at;
            if st.contains(want) {
                let (ctl, _) = tag(&xml, "controlURL", at).ok_or("service without controlURL")?;
                let control = if ctl.starts_with("http://") {
                    ctl
                } else {
                    format!("{}/{}", base.trim_end_matches('/'), ctl.trim_start_matches('/'))
                };
                return Ok(Gateway { control, service: st, local_ip, name });
            }
        }
    }
    Err("the router lists no WANIPConnection/WANPPPConnection service".into())
}

/// SOAP call: Ok(body) on 200, Err("<code> <description>") on a UPnP fault.
fn soap(g: &Gateway, action: &str, args: &[(&str, String)]) -> Result<String, String> {
    let mut a = String::new();
    for (k, v) in args {
        a += &format!("<{k}>{v}</{k}>");
    }
    let body = format!(
        "<?xml version=\"1.0\"?>\r\n<s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\" s:encodingStyle=\"http://schemas.xmlsoap.org/soap/encoding/\"><s:Body><u:{action} xmlns:u=\"{}\">{a}</u:{action}></s:Body></s:Envelope>\r\n",
        g.service
    );
    let headers = format!("Content-Type: text/xml; charset=\"utf-8\"\r\nSOAPAction: \"{}#{action}\"\r\n", g.service);
    let (status, resp, _) = http("POST", &g.control, &headers, &body)?;
    if status == 200 {
        return Ok(resp);
    }
    let code = tag(&resp, "errorCode", 0).map(|t| t.0).unwrap_or_else(|| format!("HTTP {status}"));
    let desc = tag(&resp, "errorDescription", 0).map(|t| t.0).unwrap_or_default();
    Err(format!("{code} {desc}").trim().to_string())
}

pub fn external_ip(g: &Gateway) -> Result<IpAddr, String> {
    let r = soap(g, "GetExternalIPAddress", &[])?;
    let ip = tag(&r, "NewExternalIPAddress", 0).map(|t| t.0).unwrap_or_default();
    ip.parse().map_err(|_| format!("router gave external IP {ip:?}"))
}

/// Forward external UDP `port` to our `port`. Returns the lease granted (0 = permanent).
pub fn add_udp(g: &Gateway, port: u16) -> Result<u32, String> {
    let args = |lease: u32| {
        vec![
            ("NewRemoteHost", String::new()),
            ("NewExternalPort", port.to_string()),
            ("NewProtocol", "UDP".into()),
            ("NewInternalPort", port.to_string()),
            ("NewInternalClient", g.local_ip.to_string()),
            ("NewEnabled", "1".into()),
            ("NewPortMappingDescription", DESCRIPTION.into()),
            ("NewLeaseDuration", lease.to_string()),
        ]
    };
    match soap(g, "AddPortMapping", &args(LEASE_SECS)) {
        Ok(_) => Ok(LEASE_SECS),
        Err(e) if e.starts_with("725") => soap(g, "AddPortMapping", &args(0)).map(|_| 0),
        Err(e) => Err(e),
    }
}

pub fn delete_udp(g: &Gateway, port: u16) -> Result<(), String> {
    soap(g, "DeletePortMapping", &[("NewRemoteHost", String::new()), ("NewExternalPort", port.to_string()), ("NewProtocol", "UDP".into())]).map(|_| ())
}

/// `--upnp-test PORT`: discover, read the external IP, add the mapping, remove it, log each answer. No engine.
pub fn self_test(port: u16) -> String {
    let mut out = Vec::new();
    let mut say = |s: String| {
        log!("upnp-test: {s}");
        out.push(s);
    };
    match discover(Duration::from_secs(3)) {
        Err(e) => say(format!("discovery failed: {e}")),
        Ok(g) => {
            say(format!("router {:?}, service {}, control {}, our LAN IP {}", g.name, g.service, g.control, g.local_ip));
            say(format!("GetExternalIPAddress: {:?}", external_ip(&g)));
            let add = add_udp(&g, port);
            say(format!("AddPortMapping UDP {port}: {add:?}"));
            if add.is_ok() {
                say(format!("DeletePortMapping UDP {port}: {:?}", delete_udp(&g, port)));
            }
        }
    }
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags() {
        let x = "<root><device><s:serviceType>urn:schemas-upnp-org:service:WANIPConnection:1</s:serviceType><controlURL>/ctl/IPConn</controlURL></device></root>";
        let (st, at) = tag(x, "serviceType", 0).unwrap();
        assert!(st.ends_with("WANIPConnection:1"));
        assert_eq!(tag(x, "controlURL", at).unwrap().0, "/ctl/IPConn");
        assert_eq!(header("HTTP/1.1 200 OK\r\nLOCATION: http://192.168.1.1:5000/desc.xml\r\n", "location").unwrap(), "http://192.168.1.1:5000/desc.xml");
    }
}
