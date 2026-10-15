//! reach-master: master server for reach-host. Servers POST /api/servers every 15 s, clients GET it.
//! Notes and deploy plan: notes/master-server.md.

mod client;
mod registry;
mod rendezvous;

use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use axum::extract::{ConnectInfo, DefaultBodyLimit, Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use registry::{Listed, Registry, ServerInfo, HEARTBEAT_SECS, TTL};
use serde_json::{json, Value};

struct App {
    reg: Mutex<Registry>,
    trust_proxy: bool,
    /// `--client-dir`: files reach-launcher downloads (None = the /api/client routes answer 404).
    client: Option<Mutex<client::ClientDir>>,
    /// `--rendezvous-public`: the NAT-punch UDP address clients use (None = /api/rendezvous answers 404).
    rendezvous_public: Option<String>,
}

/// Client IP: the socket peer, or with --trust-proxy the Cloudflare / X-Forwarded-For header.
fn source_ip(app: &App, peer: SocketAddr, h: &HeaderMap) -> IpAddr {
    if app.trust_proxy {
        let hdr = |n: &str| h.get(n).and_then(|v| v.to_str().ok());
        if let Some(ip) = hdr("cf-connecting-ip").or_else(|| hdr("x-forwarded-for").and_then(|v| v.split(',').next())) {
            if let Ok(ip) = ip.trim().parse() {
                return ip;
            }
        }
    }
    peer.ip()
}

async fn post_server(
    State(app): State<Arc<App>>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: Result<Json<ServerInfo>, axum::extract::rejection::JsonRejection>,
) -> (StatusCode, Json<Value>) {
    let Json(info) = match body {
        Ok(b) => b,
        Err(e) => return (StatusCode::BAD_REQUEST, Json(json!({"error": e.body_text()}))),
    };
    let ip = source_ip(&app, peer, &headers);
    match app.reg.lock().unwrap().register(info, ip, Instant::now()) {
        Ok(()) => (StatusCode::OK, Json(json!({"ttl_secs": TTL.as_secs(), "heartbeat_secs": HEARTBEAT_SECS}))),
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({"error": e}))),
    }
}

/// `GET /api/whoami` → `{"ip": "<caller's IP>"}` (same rule as registration): a listen host shows it to its friends.
async fn whoami(State(app): State<Arc<App>>, ConnectInfo(peer): ConnectInfo<SocketAddr>, headers: HeaderMap) -> Json<Value> {
    Json(json!({"ip": source_ip(&app, peer, &headers).to_string()}))
}

async fn get_servers(State(app): State<Arc<App>>) -> Json<Vec<Listed>> {
    Json(app.reg.lock().unwrap().list(Instant::now()))
}

async fn client_manifest(State(app): State<Arc<App>>) -> Response {
    let Some(c) = &app.client else { return (StatusCode::NOT_FOUND, "no client dir").into_response() };
    let r = c.lock().unwrap().manifest(); // blocking hash on first sight of a file; the dir holds a few files
    match r {
        // no-store: Cloudflare must not cache the manifest (or the files below), or friends get a stale client.
        Ok(m) => ([(header::CACHE_CONTROL, "no-store")], Json(m)).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("client dir: {e}")).into_response(),
    }
}

async fn client_file(State(app): State<Arc<App>>, Path(name): Path<String>) -> Response {
    let Some(p) = app.client.as_ref().and_then(|c| c.lock().unwrap().file(&name)) else {
        return (StatusCode::NOT_FOUND, "no such file").into_response();
    };
    // Streamed, so a few friends downloading at once stay inside the container's memory limit.
    match tokio::fs::File::open(&p).await {
        Ok(f) => {
            let len = f.metadata().await.map(|m| m.len()).unwrap_or(0);
            let body = axum::body::Body::from_stream(tokio_util::io::ReaderStream::new(f));
            ([
                (header::CONTENT_TYPE, "application/octet-stream".to_string()),
                (header::CONTENT_LENGTH, len.to_string()),
                (header::CACHE_CONTROL, "no-store".to_string()),
            ], body).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{name}: {e}")).into_response(),
    }
}

/// `GET /api/rendezvous` → `{"version": 1, "addr": "host:port"}`: where hosts and joiners send their NAT-punch
/// packets (rendezvous.rs). A name (a UDP tunnel `host:port`) is passed through; clients resolve it.
async fn rendezvous_info(State(app): State<Arc<App>>) -> Response {
    match &app.rendezvous_public {
        Some(a) => Json(json!({"version": rendezvous::VERSION, "addr": a})).into_response(),
        None => (StatusCode::NOT_FOUND, "no rendezvous configured").into_response(),
    }
}

fn router(app: Arc<App>) -> Router {
    Router::new()
        .route("/api/servers", get(get_servers).post(post_server))
        .route("/api/client/manifest", get(client_manifest))
        .route("/api/client/files/{name}", get(client_file))
        .route("/api/whoami", get(whoami))
        .route("/api/rendezvous", get(rendezvous_info))
        .route("/healthz", get(|| async { "ok" }))
        .layer(DefaultBodyLimit::max(4096))
        .with_state(app)
}

fn usage() -> ! {
    eprintln!("usage: reach-master [--listen ip:port] [--allow-private] [--trust-proxy] [--client-dir DIR]\n    [--rendezvous-listen ip:port|off] [--rendezvous-public host:port] [--rendezvous-proxy-protocol]\n  env: REACH_MASTER_LISTEN, REACH_MASTER_ALLOW_PRIVATE=1, REACH_MASTER_TRUST_PROXY=1, REACH_MASTER_CLIENT_DIR,\n    REACH_MASTER_RENDEZVOUS_LISTEN (default 0.0.0.0:47881), REACH_MASTER_RENDEZVOUS_PUBLIC, REACH_MASTER_RENDEZVOUS_PROXY=1");
    std::process::exit(2)
}

#[tokio::main]
async fn main() {
    let flag = |n: &str| std::env::var(n).map_or(false, |v| v == "1" || v == "true");
    let mut listen = std::env::var("REACH_MASTER_LISTEN").unwrap_or_else(|_| "0.0.0.0:47880".into());
    let mut allow_private = flag("REACH_MASTER_ALLOW_PRIVATE");
    let mut trust_proxy = flag("REACH_MASTER_TRUST_PROXY");
    let mut client_dir = std::env::var_os("REACH_MASTER_CLIENT_DIR").map(std::path::PathBuf::from);
    let mut rv_listen = std::env::var("REACH_MASTER_RENDEZVOUS_LISTEN").unwrap_or_else(|_| "0.0.0.0:47881".into());
    let mut rv_public = std::env::var("REACH_MASTER_RENDEZVOUS_PUBLIC").ok().filter(|v| !v.trim().is_empty());
    let mut rv_proxy = flag("REACH_MASTER_RENDEZVOUS_PROXY");
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--listen" => listen = it.next().unwrap_or_else(|| usage()),
            "--allow-private" => allow_private = true,
            "--trust-proxy" => trust_proxy = true,
            "--client-dir" => client_dir = Some(it.next().unwrap_or_else(|| usage()).into()),
            "--rendezvous-listen" => rv_listen = it.next().unwrap_or_else(|| usage()),
            "--rendezvous-public" => rv_public = Some(it.next().unwrap_or_else(|| usage())),
            "--rendezvous-proxy-protocol" => rv_proxy = true,
            _ => usage(),
        }
    }
    if let Some(d) = &client_dir {
        eprintln!("client files from {}", d.display());
    }
    let client = client_dir.map(|d| Mutex::new(client::ClientDir::new(d)));
    if rv_listen != "off" {
        match tokio::net::UdpSocket::bind(&rv_listen).await {
            Ok(s) => {
                eprintln!("rendezvous on udp {rv_listen} (public {}, proxy-protocol {rv_proxy})", rv_public.as_deref().unwrap_or("not advertised"));
                tokio::spawn(rendezvous::serve(s, allow_private, rv_proxy));
            }
            Err(e) => eprintln!("rendezvous: bind udp {rv_listen}: {e} (NAT punch off)"),
        }
    }
    let app = Arc::new(App { reg: Mutex::new(Registry::new(allow_private)), trust_proxy, client, rendezvous_public: rv_public });
    let listener = tokio::net::TcpListener::bind(&listen).await.unwrap_or_else(|e| {
        eprintln!("bind {listen}: {e}");
        std::process::exit(1)
    });
    eprintln!("reach-master listening on {listen} (allow-private {allow_private}, trust-proxy {trust_proxy})");
    axum::serve(listener, router(app).into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(async { let _ = tokio::signal::ctrl_c().await; })
        .await
        .unwrap();
}
