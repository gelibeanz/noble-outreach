# Master server (milestone 3b)

`src/reach-master`: small axum/tokio HTTP service. Hosts register by heartbeat, clients list servers.
Build: `cargo build --release -p reach-master`; test: `cargo test -p reach-master` (6 unit tests: expiry,
per-IP limit, address fill/hijack, validation, private addresses, sort order). Check: `scripts/run-browse-test.sh`.

## API (JSON, plain HTTP)
- `POST /api/servers` body `ServerInfo` `{name, version, addr, network_id, session_id, map, game_variant, players,
  max_players, dedicated}` -> `200 {ttl_secs: 45, heartbeat_secs: 15}` or `400 {error}`.
  `addr` = "ip:port" of the UDP game socket; empty IP (":47800") = master uses the request's source IP.
  `network_id`/`session_id` = hex u64 (non-zero).
- `GET /api/servers` -> array of the same fields plus `age_secs`, sorted by players desc then name.
- `GET /api/whoami` -> `{ip}`: the caller's IP as the master sees it (same rule as registration, honours
  `--trust-proxy`). A listen host (menu HOST) shows it as its internet address; a private/loopback answer is shown as unknown.
- `GET /api/rendezvous` -> `{version: 1, addr: "host:port"}`: the NAT-punch UDP address (404 when
  `--rendezvous-public` is not set; clients then skip the punch). A name (playit) is resolved by clients.
- `GET /healthz` -> `ok`.
- Servers re-post every 15 s; an entry expires 45 s after its last heartbeat.

## Limits
Body max 4 KiB; name <= 48 chars, map/variant <= 64, no control characters; max_players 1..16, players <= max;
max 4 entries per source IP, 2000 total; an addr already registered from another source IP is refused (no hijack);
private/loopback/link-local/CGNAT (100.64/10, also used by VPN overlays) addresses refused unless `--allow-private`.

## Config
`--listen ip:port` (env `REACH_MASTER_LISTEN`, default 0.0.0.0:47880), `--allow-private` (`REACH_MASTER_ALLOW_PRIVATE=1`),
`--trust-proxy` (`REACH_MASTER_TRUST_PROXY=1`): take the client IP from `CF-Connecting-IP` / `X-Forwarded-For`
(only behind the tunnel, otherwise spoofable; without it every client behind Cloudflare shares one IP and the per-IP
limit and addr fill are wrong).

## Which master (2026-10-15)
`browser::default_masters()` = the build-time `NOBLE_DEFAULT_MASTERS` (comma list), empty by default: then the
browser shows Favourites, LAN and Direct Connect only (`browser::NO_MASTER` message), `--master ""` from the menu
counts as no master (no heartbeat, no punch; UPnP and the port-forward hint still run). Private builds set the list
through the env (see the private deploy notes). `browser::master_list()` order: `--master` (main.rs copies it to env
`REACH_MASTER` in menu mode so a relaunched menu keeps it; a launcher may also set `REACH_MASTER`), then the
settings' master URL (empty = defaults; a saved value made only of default masters, as early settings files stored,
counts as empty), then
the defaults. Any of them may be a comma-separated list. Lists are fetched first-that-answers (3 s per master); the
host heartbeat (`--master a,b`) beats to the first that accepts and stays with it. Menu HOST passes the whole list.

## reach-host side (`src/reach-host/src/browser.rs`)
Hand-rolled HTTP/1.1 over `TcpStream` (http:// only, no TLS/OpenSSL; serde_json is the only new dependency).
- `--browse [url[,url]]` (no url = default list): prints the table with a live UDP query per server (players + ping
  column) and exits before any engine work (also writes `browse.txt` next to the exe,
  because Proton drops console stdout).
- `--master <url> [--server-name N] [--public-addr ip:port]` on a hosting instance: heartbeat thread every 15 s with
  map, variant file name, session id, network id, bind port, players from the slot 30 roster (1 until then), max players,
  `dedicated` from `--dedicated`. Behind NAT give `--public-addr` (the tunnel or forwarded address).
- `--join-server <url> <index|name>`: fetch, pick (index, exact or prefix name), then behave as
  `--net-role client --peer 0=<addr> --session-id <id> --map <map> --game-variant <MCC game_variants/file>`.
  Refused if the variant file is not in the player's MCC folder or the server's network id is not instance 0's.

## Deployment
- Docker: `docker build -f src/reach-master/Dockerfile -t reach-master .` from the project root (static musl binary on
  `scratch`, port 47880, runs as nobody). Behind a reverse proxy or Cloudflare Tunnel set
  `REACH_MASTER_TRUST_PROXY=1`. reach-host speaks https:// (rustls), so a TLS hostname works.
- Game UDP (47800+) is not proxied by the master: hosts forward their port, or use a UDP tunnel and pass the
  tunnel's public address as `--public-addr`. Same for a dedicated server.
- Master does not verify that a registered address is reachable or really a Reach server.

## Known limits
Net ids are fixed per instance (`net_id(0)` for the host), so `--join-server` only works with servers whose
network_id is that value (all of ours). Joiner gets instance 1 and port 47801 by default; two joiners on one PC need
`--instance`/`--bind` given explicitly.

## NAT punch rendezvous (2026-10-15, `src/reach-master/src/rendezvous.rs`)
UDP next to the HTTP API; protocol in notes/networking.md "NAT punch". Config: `--rendezvous-listen ip:port|off`
(env `REACH_MASTER_RENDEZVOUS_LISTEN`, default `0.0.0.0:47881`), `--rendezvous-public host:port` (env
`REACH_MASTER_RENDEZVOUS_PUBLIC`, what `/api/rendezvous` advertises), `--rendezvous-proxy-protocol` (env
`REACH_MASTER_RENDEZVOUS_PROXY=1`: take the client's real address from a PROXY protocol v2 header in front of each
datagram, as a playit tunnel sends it; datagrams without the header use the socket source). Hosts are keyed by
their public endpoint, expire 30 s after the last REGISTER; max 2000 hosts, 4 per IP, 20 packets per IP per second
(rest dropped silently), private/CGNAT endpoints refused unless `--allow-private`. Replies go to the socket source
("route"), so through playit they return via the agent's flow for that client. Introductions only go to endpoints
that registered themselves, and every reply is the size of its request (no reflection amplification).
Unit tests: codec, register/intro/expiry, routing through a proxy, limits, PROXY v2 parsing.

Running it behind carrier NAT: publish `47881/udp` and bring it in through a UDP tunnel (Cloudflare tunnels do not
carry UDP). With playit: a **UDP** tunnel to **127.0.0.1:47881** with Proxy Protocol **v2** (v1 is ignored on UDP),
the master started with `--rendezvous-proxy-protocol` and `REACH_MASTER_RENDEZVOUS_PUBLIC=<tunnel host>:<port>`.
Check: `curl <master>/api/rendezvous` (404 = not advertised). Without Proxy Protocol the master would see every
client as the tunnel agent and punch the wrong address.
