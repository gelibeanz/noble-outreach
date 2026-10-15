# Networking: two reach-host instances in one match

Milestone 2 (2026-10-15): two `reach-host` processes on one PC play the same Slayer match on Forge World
over our own UDP transport. Run it with `scripts/run-pair.sh`. Addresses are for the frozen DLL
(image base `0x180000000`); our own words, no decompiled code.

## How the engine forms a networked custom game

The engine runs its own session protocol (join request, membership, simulation) over the host's
transport slots. The host never has to build sessions itself: it only has to fill the launch options
the right way on each side and carry datagrams.

### Identity

- Every machine has a **secure address**: a u64. The engine takes it from **launch options +0x58**
  (`EngineInitializeGame` copies it to `0x180d1ade8`; `InitLocalSecureAddressFromHost` `0x1803a90c4`
  installs it as the local secure address `0x184e0e732` when a host object exists). So +0x58 is "my own
  network id", not the session host's.
- A transport address of type 8 carries that u64 as its first 8 bytes. Slot 41's `network_id` is the
  destination's secure address; slot 43 must report the sender's secure address. On receive the engine
  builds `{network_id, port = the local socket's port, type 8}` itself (`TransportEndpointRecvFrom`
  0x18043aa78), so a reply goes to the same engine port number on the peer. Verified at run time: the
  client's first packet went to `network_id` = the host's +0x58 value.
- Launch options **+0x50** is the session id. With flags bit 3 set, `EngineInitializeGame` copies it to
  `0x180cfb5e8` and the first session the engine creates takes it as its id
  (`GenerateSessionIdUsingLaunchInstanceId` 0x1803a9058, one-shot via `0x184e38ce1`). The joiner asks for
  this id, so both sides must use the **same** +0x50.

### Roles (launch state machine, `TickExternalLaunchStateMachine` 0x18000eea4, state `0x180c1a114`)

| State | Function | What it does |
|---|---|---|
| 1 | inline | Creates a local session (`HostLocalSessionForLaunch` 0x180399fc4) and waits until `GetNetworkSessionStatus` (0x1802a4088) = 2 |
| 2 | `LaunchStateSelectGameModePath` 0x18000de8c | mode 3 → 6, mode 5 (Firefight) → 7, campaign → 4/5, film → 3 |
| 6 / 7 | `LaunchStateSetupMultiplayerSession` 0x18000e6fc / `LaunchStateSetupFirefightSession` 0x18000e97c | **flags bit 3 set (host):** apply map + game variant to the session, then state 8 if +0xE8 ≥ 2 else 10. **Bit 3 clear (joiner):** state 9 |
| 8 | `LaunchStateWaitForPeersJoined` 0x18000ebac | Host waits until `GetSessionJoinedPlayerCount` (0x1802a3838) = +0xE8. After 45 s it moves on, or exits with reason 2 if still alone |
| 9 | `LaunchStateJoinHostSession` 0x18000ec74 | Joiner: `BeginJoinSessionFromLaunch` 0x1802ac4f4(…, &options+0x50, &options+0x2F8, 16 zero bytes) → `RequestJoinRemoteSession` 0x18039b6d0. Polls the join; success → 11 |
| 10 | `LaunchStateWaitForSessionStart` 0x18000ed68 | Host: when status 5, starts the game → 11 |
| 11 | — | in game |

Timeouts: 60 s per state, 90 s overall (`external_launch_*_timeout`, exit reason 4). The joiner must
connect within 45 s of the host reaching state 8.

The join request is queued in globals at `0x182a49098` (session id `0x182a490a4`, 16-byte key
`0x182a490ac`, host secure address `0x182a490bc`) and handled by `TickSessionJoinRequests` 0x18039a0bc,
which calls `StartJoinRemoteSessionWithLocalUsers` 0x1803923dc → `RegisterSessionDescriptorSlot`
0x180400830 (8 descriptor slots of 0x600 bytes at `0x182d4f558`). Peers enter the membership by secure
address (`AddSessionPeerBySecureAddress` 0x18039cf80), which opens an observer channel for that address
(table `0x182d1a640`, 0x22 channels of 0x18d0 bytes). The 16-byte key is all zero in our launch and the
engine accepts it over host-routed transport.

### Launch-option fields for a networked game

| Offset | Host (instance 0) | Joiner (instance n) |
|---|---|---|
| +0x00 flags | 0x08 (bit 3 = create and host) | **0x00** (bit 3 clear = join) |
| +0x50 | session id (same everywhere) | same |
| +0x58 | own secure address (network id of instance 0) | own network id (instance n) |
| +0x60 17×u64 | peer secure addresses, [0] = host, [i] = instance i | same table |
| +0xE8 | player count (host waits for this many) | same |
| +0xF0 16×0x20 | per player {xuid, secure address, team, 1, peer index, −1}; empty = {0,0,−1,0,−1,−1} | same table |
| +0x2F0 | number of player entries (team-assignment loops `0x1803915a8`, `0x180450a4c` match xuids against it) | same |
| +0x2F8 | 0 | **host's secure address** (join target) |

Player +0x10 is the team the host applies when the variant has teams (matched by xuid). Player +0x18 is
compared with +0x2F4 in `0x18040ba68`; we write the peer index there (MCC's 1-player capture has 0).
Map/game variants are only needed on the host; the joiner gets them from the session, though ours carries
them too. Slot 88 must return each instance's own xuid (the one in its player entry).

The 17-entry peer table at +0x60 is never read by the engine as far as static references show (no data
references); the join only needs +0x2F8 and the session protocol learns the rest. We fill it anyway.

## Our transport (`src/reach-host/src/net.rs`)

- Instance n: network id `0x5245414348000001 + n`, xuid `0x0009000000000001 + n`, UDP
  `127.0.0.1:47800 + n` (`--local-pair host|client`), or `--net-role host|client --instance N
  --players N --bind ip:port --peer N=ip:port … --session-id hex`.
- One UDP socket per instance carries both engine ports. Datagram = `[u32 engine port][u64 sender
  network id][payload ≤ 1192]`. The sender id in the header avoids a reverse address lookup.
- A receive thread queues packets per engine port (cap 4096, oldest dropped). Slot 43 pops the queue for
  the port it is asked for and returns 0 when empty, never blocks. Slot 41 looks the id up in the peer
  list (unknown id with a single peer: routed to that peer, logged), returns bytes sent, −2 would block,
  −3 other error, −1 unknown peer.
- Logs: the first 8 packets each way (id, port, length, first bytes) and every 10 s a `net heartbeat`
  line: packet/byte counts, launch state, session status, session state, roster size from slot 30.
- Slot 30 (`update_session_roster`) logs the roster whenever its player count changes.

## Pair run (2026-10-15, `scripts/run-pair.sh`)

Both instances in `work/prefix-net`; the client is started with `PROTON_VERB=runinprefix` (umu's default
`waitforexitandrun` blocks until the first process in the prefix exits). Each instance has its own folder
`work/run-net/{host,client}` (exe copy, log, reach-data, frame captures).

- Client t=2.1 s: first packet, 55 bytes to the host on engine port 1001 (the join request); host answers
  on 1001, then the link handshake runs on port 1000.
- Both: slot 30 roster `instance 0x5245414348530001 host 0x5245414348000001, 2 players` with both
  xuids/machines, launch state 11, map loaded ~7 s after the join, then 60 fps.
- Steady state: 600 packets per 10 s each way (60 Hz), no drops, no send failures, stable past 80 s.
- Back-buffer captures from both instances show the Forge World match with the HUD; the client also shows
  the spawn/loadout screen. Seeing the other Spartan was not checked (players cannot move yet).

## Open points

- Meaning of player +0x14/+0x18/+0x1C and +0x2F4; teams for team variants not tested (Slayer FFA used).
- The 16-byte session key (zero) and unsigned variants are accepted in this setup; whether that holds
  across real machines is untested (only 127.0.0.1).
- More than 2 players, late joins, a client leaving, host migration, and the host's end-of-game path
  (slot 26 next game options) are untested.
- Where the engine turns a secure address into a type-8 transport address was not pinned down statically
  (the observer channel keeps the secure address; the send path reads it); the run shows the id is used
  unchanged.

## Server info query (2026-10-15, `src/reach-host/src/query.rs`)

A request/response on the game's own UDP socket, so it works through the same playit tunnel as the game.
net.rs's receive thread hands every datagram to `query::handle` first; query datagrams are answered or dropped
there and never reach the engine queues. Engine datagrams start with the engine port as a u32 (1000/1001), so
the magics below can never collide. Any instance 0 (dedicated server or player host) answers; clients stay silent.
All integers little-endian.

Request (16 bytes; padded so a reply is at most ~10x larger):

| Off | Size | Field |
|---|---|---|
| 0 | 4 | magic `RQRY` |
| 4 | 1 | version = 1 |
| 5 | 1 | kind = 1 (info request) |
| 6 | 2 | reserved 0 |
| 8 | 4 | token (echoed; the client matches replies by it) |
| 12 | 4 | padding 0 (responders accept ≥ 12 bytes) |

Reply (32-byte header + 4 strings):

| Off | Size | Field |
|---|---|---|
| 0 | 4 | magic `RQRS` |
| 4 | 1 | version = 1 |
| 5 | 1 | kind = 2 (info reply) |
| 6 | 1 | flags: bit 0 dedicated, bit 1 password (never set yet), bit 2 campaign in progress (joins refused, lobby.rs), bit 3 dormant (no engine loaded, dormant.rs), bit 4 engine starting (woken, joins not answered yet) |
| 7 | 1 | mode: 0 multiplayer, 1 Firefight, 2 campaign |
| 8 | 4 | token from the request |
| 12 | 1 | players now (slot 30 roster, same rule as the master heartbeat: a dedicated server is not a player) |
| 13 | 1 | max players |
| 14 | 1 | campaign lobby countdown in seconds (0 = none; dedicated campaign gathering players) |
| 15 | 1 | reserved 0 |
| 16 | 8 | host network id (secure address of instance 0) |
| 24 | 8 | session id (launch options +0x50) — enough to join without the master |
| 32 | … | 4 × (u8 length + UTF-8): name (≤48), version (≤32), map (≤64), game variant file (≤64) |

- Unknown version/kind and stray `RQRS` packets are dropped silently; a future v2 client can fall back to v1.
- Rate limit: 8 replies per source IP per second (window resets each second; table pruned above 4096 IPs).
- Client side: `query::query_many` sends to every target from one ephemeral socket, resends once at half the
  timeout (browser: 1.5 s), RTT = ping. `query::query_broadcast` does LAN discovery (255.255.255.255 to 47800–47803
  and 47900). `query::resolve` turns `host:port` into an address, preferring IPv4.
- Hosts log `query: answering … as "<name>"` at start and the first 5 answers (`query: #k answered <addr>`).
- Older reach-host builds without the handler treat a query as an engine datagram from an unknown id: they log
  `net: new peer …` with a junk id and queue it on a port the engine never reads (harmless, but one bogus peer entry).
- Checked 2026-10-15: local dedicated on 127.0.0.1:48100 answered in 0–2 ms (78-byte reply); a live dedicated server
  through a playit UDP tunnel answered in 11–14 ms with live players (master listed 0, query said 3–4).

## Bye packet (2026-10-15, `src/reach-host/src/net.rs`)

Without it, a player who leaves is only noticed by the engine's own silence timeout (~16 s: the observer channel
dies, then the session update removes the peer). Same socket as the game, same rule as the query: the first u32
is never an engine port, so it cannot collide. Handled in net.rs's receive thread; never reaches the engine.

| Off | Size | Field |
|---|---|---|
| 0 | 4 | magic `RBYE` |
| 4 | 1 | version = 1 |
| 5 | 1 | kind = 1 (leaving) |
| 6 | 2 | reserved 0 |
| 8 | 8 | sender network id (same as the engine datagram header) |

- **Sent** 3 times, 15 ms apart, to every known peer: pause overlay LEAVE GAME, window closed (X / Alt+F4), and any
  host (listen or dedicated) shutting down through its window.
- **Host receiving it** (instance 0): stops sending to that peer (slot 41 swallows the engine's packets for it) and
  queues the id. On the game thread (throttle.rs hook before each `ProcessEngineMessageQueue`; dedicated paced,
  listen/pair host unpaced) `drop_left_peers` finds the peer in the session and calls the engine's lost-peer
  removal, the same call its timeout path makes:
  - session object `*0x182a1fe08`; state +0x60ef8 (6 = hosting with live peers; only then is it called);
    membership at +0x30: peer mask +0x5c (17 bits), our own index +0x40, peer i at +0x60 + i*0x158 whose first
    u64 is its secure address (`AddSessionPeerBySecureAddress` 0x18039cf80 writes it), observer channel index at
    +0x4470 + i*0xC.
  - `UpdateNetworkSessionHost` 0x180393830 (per-session update, host part): for each peer whose observer channel state (int at 0x182d1b960 +
    channel*0x18d0) is 1 (dead) it calls `RemoveDisconnectedSessionPeer` 0x180395f4c `(session, peer index)`,
    which (for a joined peer) calls `RemoveSessionPeer` 0x180395de8 `(session, peer, reason 8)`: host slot 25 (+0xC8, not
    in host-slots.md yet: `(this, u64 xuid, bool booted = reason 0xd)`) per player of that peer, membership
    update to everyone, peer bit cleared.
  - Boot is the other route (engine msgs 0x10/0x11 with a xuid → `BootSessionPlayer` 0x18038ea2c → 0x180394f94 →
    same `RemoveSessionPeer`, reasons 0 / 0xd); not used.
- The master heartbeat beats again within 1 s when the roster count changes, so the list updates at once.
- **Client receiving it from the host** (net id of instance 0): the game is over; main returns to the menu (fresh
  process) with "The host left the game." on the server browser.
- A peer that said bye and then talks again more than 2 s later (rejoined) is un-marked.

## Vote (playlist voting, 2026-10-15, `src/reach-host/src/vote.rs`)

Same side channel as the query: raw datagrams on the game socket, picked off in net.rs's receive thread before the
engine queues (magic `RVOT`). The server sends to every peer heard from in the last 4 s; clients accept vote
datagrams only from their host's address, the server accepts casts only from known peer addresses. All integers
little-endian. Header (8 bytes): magic `RVOT`, u8 version = 1, u8 kind, u16 round id (new id per round, also after
a re-roll). Strings are u8 length + UTF-8.

| Kind | Direction | Body after the header |
|---|---|---|
| 1 open / tally | server → clients, every 1 s while a round is open and at once after each cast | u8 seconds left, u8 option count (3), u8 flags (bit 0: "None of the above" offered), u8 voters (peers connected), 4 × u8 tally (options 1-3, none), playlist name (≤48), then per option: u8 mode (query.rs mode byte), name (≤48), map (≤64), game variant file (≤64) |
| 2 cast | client → server | u8 choice (0-2, 3 = none of the above), 3 × 0 |
| 3 result | server → clients, 3 times 400 ms apart | u8 winner (0-2), u8 restart (1 = the server restarts into it), u8 difficulty (0-3), u8 0, then one option as in kind 1 |

- A client may change its vote (the latest cast counts). Casts for an old round id, or "none" when not offered, are ignored.
- Unknown versions are dropped silently. Rules and the restart/follow flow: notes/dedicated.md "Playlists and voting".
- The info query reply (above) carries the playlist name as an optional 5th string after the variant; decoders
  that stop after 4 strings are unaffected (no version bump).

## NAT punch (2026-10-15, `src/reach-host/src/nat.rs`, master side `src/reach-master/src/rendezvous.rs`)

Lets a friend's listen server (menu HOST, `--listen`) be joined without a port forward on routers with
endpoint-independent UDP mapping (most home routers). Same idea as Reclaimer's master-assisted hole punching.
Every packet goes over the game socket (net.rs's receive thread hands `RNAT` datagrams to `nat::handle` before
query/vote/dormant/engine), so the NAT mapping punched is the one the game uses.

Packet (all kinds, both directions, exactly 40 bytes so no reply is larger than its request; integers LE):

| Off | Size | Field |
|---|---|---|
| 0 | 4 | magic `RNAT` |
| 4 | 1 | version = 1 (others dropped) |
| 5 | 1 | kind |
| 6 | 1 | status: 0 ok, 1 unknown host, 2 refused (private address, table/IP limit) |
| 7 | 1 | address family: 0 none, 4, 6 |
| 8 | 8 | token (random per process; echoed) |
| 16 | 2 | port |
| 18 | 2 | reserved |
| 20 | 16 | IP (IPv4 in the first 4 bytes) |
| 36 | 4 | padding |

| Kind | From → to | Address field |
|---|---|---|
| 1 REGISTER | host → rendezvous, at start (3 tries 1 s apart) then every 10 s | — |
| 2 REGISTERED | rendezvous → host | the host's public endpoint as seen |
| 3 INTRO_REQ | joiner → rendezvous | target host's public endpoint (the listing address) |
| 4 INTRO_REPLY | rendezvous → joiner | target; status 0 = introduced, 1 = not registered |
| 5 INTRO_PEER | rendezvous → host | the joiner's public endpoint (token = joiner's) |
| 6 PUNCH | peer ↔ peer | — (10 packets, 100 ms apart) |
| 7 PUNCH_ACK | answer to every PUNCH (≤ 50/s total) | — (token echoed) |

- **Host** (`--listen` without `--public-addr`): `nat::start_host` finds the rendezvous (`--rendezvous host:port|off`,
  env `REACH_RENDEZVOUS`, else the first master answering `GET /api/rendezvous`), registers, and tries UPnP at the
  same time. INTRO_PEER → a punch burst to the joiner. The master heartbeat waits up to 5 s for this, then lists
  the UPnP address (router's public IP:port) if mapped, else the punched endpoint, else the old `:port` fill.
- **Joiner** (any `--net-role client` with a peer 0, so menu JOIN, DIRECT CONNECT and `--join-server`):
  `nat::start_client` sends 3 PUNCHes directly (250 ms apart); an ACK = "Connected to the host directly". Otherwise
  (and only for a public host address) INTRO_REQ (3 tries 700 ms apart), punch burst on INTRO_REPLY; a PUNCH or ACK
  from the host = "Connected to the host (NAT punch)". The engine's own join runs in parallel and retries, so
  nothing waits on this. Status toast bottom left (ui/overlay.rs), log lines `nat: ...`.
  `REACH_NAT_FORCE_PUNCH=1` skips the direct try and the public check (test knob for one-PC runs).
- Host banner / pause section line "Router": "Router port opened via UPnP" / "Using NAT punch (via the master)" /
  "Checking the router ..." / "Needs port forward" (no UPnP and no rendezvous answer).
- Old hosts without nat.rs treat a PUNCH as an engine datagram from a junk id (one bogus `net: new peer`, harmless).
- Will not work: symmetric NAT on both ends (port changes per destination), carrier NAT that maps per destination,
  and two players behind the same router without hairpinning (use the LAN tab). Those need a relay or a tunnel.

### UPnP (`src/reach-host/src/upnp.rs`)
SSDP M-SEARCH (WANIPConnection:1/2, WANPPPConnection:1, IGD:1) → device description → SOAP
GetExternalIPAddress, AddPortMapping (UDP port → our LAN IP, lease 3600 s, retried with 0 on error 725),
DeletePortMapping in `net::send_bye` (window close, leave game, host shutdown; a crash leaves it until the lease
ends). If the router's external IP is not public (carrier NAT), no mapping is added. `--no-upnp` turns it off.
`reach-host --upnp-test PORT` adds and removes one mapping and writes `upnp-test.txt` (no engine).

Tested on a TP-Link Deco X50 (2026-10-15): WANIPConnection:1, control `/ctl/IPConn`; GetExternalIPAddress returned a
CG-NAT address, AddPortMapping UDP 49100 OK (lease 3600), DeletePortMapping OK. So UPnP works on that router but
cannot help behind an ISP's carrier NAT: such listen hosts need the punch (if the ISP's NAT is
endpoint-independent) or a tunnel.

### Verified (2026-10-15, `scripts/run-nat-test.sh`, prefix work/prefix-nat, ports 49100/49110/49111)
Local master (`--allow-private`, rendezvous 127.0.0.1:49111), listen host on 49100, joiner via `--join-server`
with `REACH_NAT_FORCE_PUNCH=1`: host registered at 0.7 s (public 127.0.0.1:49100, listed as that); joiner
INTRO_REQ at 0.85 s → master answered with 2 packets → both sides logged punch / punch ack bursts (host
"punching 127.0.0.1:34056 (introduced by the rendezvous)"); joiner "Connected to the host (NAT punch)" at 0.86 s;
host roster 2 players at 25.7 s, steady traffic, joiner in game (captures: joiner toast at 2 s, joiner on Forge World
at 30 s, host banner with "Router  Using NAT punch (via the master)"). Real NAT (two different networks) not tested.

## Wake (lazy engine start, 2026-10-15, `src/reach-host/src/dormant.rs`)

An empty dedicated server runs "dormant": only our socket, the info query and the master heartbeat, no engine
(notes/dedicated.md "Lazy engine start"). Same side-channel rule as the query: first u32 is never an engine port.
Handled in net.rs's receive thread; never reaches the engine.

| Off | Size | Field |
|---|---|---|
| 0 | 4 | magic `RWAK` |
| 4 | 1 | version = 1 |
| 5 | 1 | kind = 1 (wake) |
| 6 | 2 | reserved 0 |
| 8 | 8 | sender network id (0 = none; unused so far) |

- **Client:** sends it 3 times from a throwaway socket right after its arguments are final (menu or command line),
  before loading its own engine, so the server's engine loads in parallel. Then, just before `initialize_game`
  (where launch state 9 sends the join), it polls the info query once a second: while flag bit 3 or 4 is set it
  logs "server waking, please wait", re-sends the wake every ~0.5 s and waits (up to 90 s). No reply (older server)
  = go on at once. `REACH_NO_WAKE=1` skips both (behaves like an older client; test aid).
- **Server:** a wake, or the first engine datagram (an older client's join), wakes it. Engine datagrams are
  dropped (not queued) from then until the woken engine first reaches launch state 8, so no stale join requests
  pile up; the joiner's engine re-sends its join about once a second while in launch state 9 (60 s state
  timeout), which carries an older client through.
- Anyone can wake a server (as with a join packet); a woken server that sees no peer for 120 s while still in
  launch state 8 exits (code 3) and its supervisor restarts it dormant.

