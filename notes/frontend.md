# Frontend (milestone 4 MVP)

Our own menu and pause overlay inside `reach-host`, in the spirit of Reclaimer's UI (see
`frontend-reference.md`): dark translucent panels, thin uppercase headings, one orange accent. egui 0.35
drawn by `egui-directx11` 0.13 (precompiled shaders, same `windows` 0.62 as the host); cross-compiles with
zig like the rest.

## When it shows
- **Main menu**: `reach-host --mcc <dir>` with no launch arguments (no `--map/--game-variant/--map-variant`,
  `--options`, `--net-role/--local-pair/--join-server`, `--dedicated`, `--no-launch`, `--hidden`). Any of those
  skips the menu exactly as before. Background (HOME / PLAY / HOST): the player's Spartan live from the preview
  engine, or still install art (`ui/menu_scene.rs`, toggle Graphics > Live menu background; notes/customization.md).
- **Pause overlay**: in game, Esc (or pad Start) fires the engine's pause action, which calls host slot 7 and
  draws nothing itself. Slot 7 opens the overlay. The game keeps running underneath.

## Code (`src/reach-host/src/ui/`)
| File | What |
|---|---|
| `mod.rs` | egui context + renderer (one global, created on first draw), Win32 message → egui event translation, `draw()` (one egui frame into a render target view), `back_buffer_rtv()`, `relaunch_to_menu()`, the `REACH_UI_SHOT` test hook |
| `theme.rs` | colours (`ACCENT`, `PANEL`, `DIM`…), egui style, widgets: `backdrop`, `panel`, `heading`, `caption`, `nav_item`, `primary_button`, `ghost_button`, `list_header`/`list_row` |
| `menu.rs` | main menu screens (Home, Play, Server browser, Host game, Settings), its own frame loop (`run`), built-in MP map list, variant names, the shared `settings_form` |
| `overlay.rs` | slot 7 handler, `end_frame` draw hook, RESUME / SETTINGS / LEAVE GAME |
| `settings.rs` | `Settings` (player name, mouse sensitivity, master URL, last picks, host form, graphics/audio/game/mouse/pad) in `<exe dir>\reach-data\settings.json` |
| `settings_screen.rs` | the SETTINGS screen (menu and pause overlay): categories with value summaries, rows (toggle, ◀ value ▶ picker, slider, two key slots), help panel, RESTORE DEFAULTS, keyboard navigation, key capture |
| `customize.rs` | the CUSTOMIZE screen: Spartan/Elite tabs, piece and colour rows, 3D preview (flat fallback), picker (names or Reach colour swatches), hover = try on, click = equip. Data and engine side: [customization.md](customization.md) |
| `settings_apply.rs` | applies changes: input.rs at once, window mode on the window thread, profile + engine msg 5 in a running game, msg 14 on resize |

Hooks elsewhere (kept small): `main.rs` (menu before engine load, args re-parsed with the menu's choice, engine
pointer for the overlay, return-to-menu on game exit / leave), `host.rs` (slot 7 override, overlay call at the
top of slot 1 `end_frame`, `capture_back_buffer` made `pub`), `input.rs` (`set_ui_active`, `set_mouse_sens`, gating).

## How it works
- **The menu produces command-line arguments.** PLAY → `--map M --game-variant <mcc>\haloreach\game_variants\V`;
  JOIN → the same `--net-role client --peer 0=addr --session-id … --players … --map … --game-variant …
  [--to-dedicated]` that `--join-server` builds (same network-id and variant checks); HOST →
  `--net-role host --players N --bind 0.0.0.0:47800 --map … --game-variant … [--master URL --server-name N]`.
  `main.rs` appends them to argv and calls `parse_args` again, so every menu path is also a scriptable CLI path.
- **Menu rendering**: before the DLL is loaded, `menu::run` loops on our window: pump messages, resize the
  swap chain to the client area if needed, clear, egui frame, `Present(1)`. On launch it calls `ClearState` +
  `Flush` and hands the untouched swap chain to the engine as before.
- **Overlay rendering**: `host.rs` slot 1 (`end_frame`, engine render thread, before the engine presents)
  calls `overlay::end_frame(swapchain)`. While open it makes an RTV on the back buffer and draws with the
  immediate context. Around the draw it swaps in our own `ID3DDeviceContextState`
  (`ID3D11DeviceContext1::SwapDeviceContextState`), so the engine's whole pipeline state comes back unchanged.
  Verified: the engine frame after RESUME renders normally.
- **Input**: `ui::active()` while the menu or overlay is up. `ui::window_message` runs before
  `input::window_message` and turns mouse, wheel, key and char messages into egui events (client pixels,
  scaled to points at draw time; layout is designed for 720 lines and scales with the window height).
  `input::set_ui_active(true)` releases every key, frees the cursor, ignores raw input and click-to-capture,
  and makes slot 36 report player 0's device with no keys/mouse (so the engine does not pause for a missing
  device). Open/close are posted to the window (`WM_UI_OPEN/CLOSE`) because capture and cursor belong to the
  window thread.
- **Seamless (2026-10-17):** LEAVE GAME, host left, the 20 s join watchdog and a menu game's end now end the
  engine in place and show the menu in the same window and process (notes/engine-interface.md "Game after game in
  one process"; a "Leaving the game" card is drawn in between, `ui/loading.rs`). The relaunch below is the
  fallback (`REACH_RELAUNCH=1`, or when the engine cannot be ended in place).
- **LEAVE GAME** (2026-10-15 fix): posts engine message 13 (quit), then main sends the bye packet to every
  peer (notes/networking.md) and returns to the menu at once: starts a new `reach-host --mcc …`
  (env `REACH_UI_RELAUNCHED=1`) and TerminateProcess-es itself (no waiting for the game thread, no DLL detach; msg 13
  never reliably ended the thread, and on a networked client the engine kept running its post-game states). The
  game's log is copied to `reach-host-game.log` first. A menu-launched game also returns to the menu when its thread
  exits on its own. Same for every role (solo, client, listen host, dedicated window). A client whose host says bye
  returns to the menu with `REACH_UI_NOTICE` = "The host left the game.", shown on the server browser. Closing the
  window (X / Alt+F4) sends bye and terminates. `REACH_UI_HIDDEN=1` keeps a relaunched menu's window hidden
  (unattended checks).
  Verified 2026-10-15 (work/run-leave): dedicated Firefight ff10_prototype + 1 client leaving → server
  `net: dropped peer … after its bye` and roster 0 within ~0.05 s, client menu captured ~2.3 s after LEAVE; listen
  host (Firefight) leaving → joiner `host left the game` within 0.1 s, its menu showed the notice ~2 s later.
- **Settings**: mouse sensitivity applies at once (and at start-up unless `--mouse-sens` is given). The player
  name is put in `REACH_NAME` (read by slot 88's name buffer) for menu-launched games when `REACH_NAME` is unset;
  CLI runs keep their old naming.

## Settings screen (2026-10-15)
Menu SETTINGS takes the whole window; the pause overlay's SETTINGS draws the same screen over the game. Changes
save and apply when the pointer is released. Keys: Up/Down rows, Left/Right change, Enter toggle / listen for a
key, Tab / Shift+Tab pages, Esc cancel listening or leave. Key slots: click, press a key or mouse button (wheel
too); Esc cancels, right-click clears; keys the engine does not poll are refused; clashes inside one context
(play / Forge / Theater) are drawn red.

| Setting | Goes to |
|---|---|
| Display mode, window size | our window (`settings_apply::apply_display`): windowed = centred WS_OVERLAPPEDWINDOW, borderless = WS_POPUP over the monitor, fullscreen = borderless + `SetFullscreenState(TRUE)`; then engine msg 14 |
| Rendering resolution | swap-chain size = client × % (host slot 2 `ResizeBuffers`, DXGI stretches); UI pointer events rescaled |
| Frame-rate limit, VSync | slot 32 +0x15 / +0x14, live via msg 5 |
| GPU line | DXGI adapter description of our device |
| Quality | none: Reach reads no quality fields (host-slots.md, slot 32) |
| Master / music / effects volume | profile +0xA5C / +0xA60 / +0xA64, plus directly the engine globals and the mastering voice (`audio::apply_volume`, at engine start and on change; host-slots.md "Volumes") |
| FOV, vehicle FOV | profile +0x14 / +0x18 (live, verified) |
| Crosshair position, Spartan voice | profile +0x1C, +0x27 |
| Mouse sensitivity, acceleration, smoothing, invert look | input.rs (host owns mouse look; profile +0x1E kept 0) |
| Invert flight (mouse) | profile +0x22 |
| Key bindings | `Settings.mouse.bindings` (only non-defaults) → profile +0x42C (66 × 0x18) → msg 5 + flip of +0x1D2 forces `RebuildPlayerInputBindings` (live, verified) |
| Pad look sensitivity, invert, invert flight, auto-centre | profile +0x1B5/+0x1B6, +0x1D, +0x21, +0x23 |
| Dead zones | profile +0x1B8 / +0x1BC (engine accepts 0..0.2) |
| Button / stick layout, swap triggers | profile +0x1C8 (MCC preset numbers), +0x1C9, +0x1D7 (slot 116 stays null) |
| Vibration, crouch lock, clench protection | profile +0x1F (inverted) + our XInput rumble gate, +0x24, +0x26 |

Test hooks: `REACH_UI_SHOT=settings-graphics,settings-audio,settings-game,settings-player,settings-keyboard,
settings-controller` (menu pages), `pause-settings` (with `pause`: overlay settings capture); test-script command
`set fov <deg>` / `set fps <code>` / `set vsync 0|1` / `set bind <mcc action> <key>` saves and applies like the UI.

## Add a screen
1. Add a variant to `Screen` in `menu.rs`, a `nav_item` line in `Menu::ui`'s left column, its title in the
   `match self.screen` heading block, and a `fn my_screen(&mut self, ui: &mut Ui)` called from the content match.
2. Lay out with `theme::caption`/`heading`, lists with `list_header` + `list_row` inside a `ScrollArea`, the one
   main action with `primary_button`, others with `ghost_button`. Sub-areas: `ui.scope_builder(UiBuilder::new().max_rect(r), …)`.
3. To launch something, build CLI arguments and call `self.launch(args)`.
4. Add its name to the `REACH_UI_SHOT` step match in `menu::run` to capture it unattended.
Overlay pages: add a `Page` variant in `overlay.rs` and a branch in `draw_ui`.

## Checks (2026-10-15, `scripts/run-ui.sh`, prefix `work/prefix-ui`, files in `work/run-ui/`)
- `REACH_UI_SHOT=home,play-screen,browser,host,settings` → `ui-<screen>.bmp` for each, then quits.
  Browser shot taken against a local `reach-master --listen 127.0.0.1:47885 --allow-private` with three fake
  entries (rows, selection, detail panel, footer counts).
- `REACH_UI_SHOT=play,pause,resume,leave … --test-input scripts/test-ui-pause.txt`: menu launched Slayer on
  Forge World, name "Player" from settings reached slot 88, Esc → slot 7 → `ui-pause.bmp` (overlay over the
  live frame), RESUME → `ui-after-resume.bmp` (normal frame, HUD intact, clock running), Esc → LEAVE →
  msg 13 → state 10 → 8 s fallback → new process → `ui-relaunched.bmp` (menu).

## Not done / unverified
- Real mouse/keyboard on the menu (clicks, typing, wheel) were not exercised: the checks drive screens through
  `REACH_UI_SHOT`. JOIN, DIRECT CONNECT and START HOSTING buttons were not launched (args mirror the verified
  CLI paths). Settings SAVE and the overlay SETTINGS page were not clicked.
- Direct connect to an address not in the master list assumes the default session id and 2 players
  (`--players 2`, map placeholder `forge_world`; the joiner takes map/variant from the host's session).
- No map images or controller navigation of the UI (ping, favourites and multiple masters: see Server browser below).
- The game is not paused under the overlay (no sim pause message is sent; it is multiplayer).
- Msg 13 does not end the game thread within 8 s; no longer waited for (LEAVE GAME relaunches at once).

## Firefight / campaign in the menu (2026-10-15)
- PLAY and HOST GAME have mode tabs MULTIPLAYER / FIREFIGHT / CAMPAIGN (`PlayMode` in `menu.rs`). Firefight: map
  list (Beachhead ff50_park, Overlook ff10_prototype, Courtyard, Waterfront, Corvette, Glacier ff60_icecave, Holdout,
  Outpost ff60_airview, Unearthed; names checked against each mapinfo's English title), `hr_ff_*` game types from
  `game_variants`, difficulty row. Campaign: 11 missions (m70_a is the credits, m40 has no map index; m70_bonus =
  Lone Wolf, mission names from the mapinfo titles), difficulty, insertion fixed at 0 (`--insertion 0`).
- Args: `--firefight M --difficulty D --game-variant V` / `--campaign M --difficulty D --insertion 0`; HOST adds
  `--net-role host --players N --bind 0.0.0.0:47800 [--master … --server-name …]`. Co-op players default 2 (slider 2..4,
  `coop_players`, not saved); multiplayer keeps the saved max players. Mode, map and difficulty are not persisted.
- Server browser: "Mode" column = Firefight (map ff*), Campaign (map m<digit>*), else the game type.
  `main.rs --join-server` infers `--firefight` / `--campaign` from the listed map the same way (a Firefight or
  campaign host without `--game-variant` registers `hr_ff_standard_1set.bin` / `slayer_054.bin` as its variant; the
  joiner needs that file in its MCC folder).
- Checked: `REACH_UI_SHOT=play-ff,play-camp,host-camp` captures (run-menu / prefix-menu); `play-ff-start` launched
  `--firefight ff50_park --difficulty normal --game-variant …hr_ff_standard_1set.bin` from the menu: map loaded at
  8.9 s, waves begun 0 by 32 s. Not exercised: real clicks on the tabs, hosting/joining Firefight or campaign from
  the menu (args mirror the verified CLI co-op), MULTIPLAYER tab capture after the change, Firefight on a dedicated.

## Server browser (2026-10-15)
- Tabs INTERNET (master list) / FAVOURITES / LAN (broadcast query), mode filter button (all / multiplayer / Firefight /
  campaign), hide full / empty, search; columns ★, server, mode, map, players, ping; click a caption to sort, again
  to flip. Click the ★ to star/unstar; double-click a row or JOIN GAME to join.
- Refresh (on open and REFRESH) runs on a thread: first master that answers (rows show at once), then the UDP info
  query (notes/networking.md) to every listed server and favourite: live players, ping, map/mode/session. No answer =
  master data stays, ping "—", detail says "Master data: the server did not answer the query".
- SERVER ADDRESS: `host:port` (tunnel hostnames such as `name.example.net:25195` re-resolved on every refresh) → ADD TO
  FAVOURITES saves it in `settings.json` `favourites` (as typed) and queries it; CONNECT joins it directly. A favourite
  that answered the query can be joined without the master (the reply carries session id and network id).
- Joins: dedicated servers get a random instance 1..65535 and an OS-chosen port (as `--join-server`); `ff*` maps
  join with `--firefight`, `m<digit>*` with `--campaign … --insertion 0`.
- Test hook: `REACH_UI_SHOT=browser,browser-fav` with `REACH_UI_ADD=host:port[,…]` adds favourites by address then
  captures the Favourites tab. Checked in `work/run-browser2` (prefix-browser2): `client/ui-browser.png` (local
  dedicated with 0 ms ping, live data), `client2/ui-browser.png` and `ui-browser-fav3.png` (default masters,
  a live dedicated Firefight server with 11–14 ms; favourites by hostname), `client/ui-browser-fallback.png` (server stopped, master
  entry still listed: master data, no ping).

## Tab scoreboard (2026-10-15)
- **Why nothing drew:** the engine does process the "show scores" action (engine action 0x3b, held while Tab / pad
  Back is down) and even reports its pre-spawn scoreboard to the host (slot 3 state 8 / 9, sent from the end of
  `ProcessEngineMessageQueue` 0x18000f26c when the UI window manager `0x182a0da38` has a screen for a local user), but
  no engine UI screen ever renders under our host (captures during state 8 show the plain game view). The scoreboard
  screen ids (string ids 0x800d0 `scoreboard`, 0x800d1 `scoreboard_half`, table at 0x1809c1fc8) never appear as code
  immediates; the screen loader is `0x1802c9efc` (window manager, render window, screen id, ...), with id-taking
  wrappers 0x1802ccb84 / 0x1802ee120. Making the engine's own gui screens render was not pursued.
- **Ours (ui/scoreboard.rs, Reclaimer-style since 2026-10-17):** while action 0x3b is held for player 0 (keyboard/mouse
  record at 0x18287ff94 +0x224 + 0x3b×0x10, pad via the index table at +0x100; +2 = held frames; device from
  0x182dfbe74) the overlay's passive layer draws a centred panel: "SCOREBOARD (n/max)", dim column headers PLAYER /
  SCORE / KILLS / DEATHS / ASSISTS / PING. Team games: a header row per team ("RED TEAM (n)" in team colour, team
  score under SCORE), best team first, then that team's rows as solid team-colour bars; free-for-all: one list of
  grey bars. Each row: name, centred numbers, a ping chip (dark box, 4 signal bars, "53 ms"; green < 100 ms, amber
  < 180, red), and a "≡" menu button; the local row has a gold outline and no button. No VOICE column, mute,
  emblems, ranks or service tags (no voice chat; emblems are open; the service tag is not in the player datum).
- **Mouse mode:** right-click while Tab is held turns it on: the UI takes the mouse (cursor freed, game input withheld,
  as for the pause overlay) and the board stays up without Tab. Esc, Tab or another right-click closes it. While Tab
  is held, right mouse is not passed to the engine (it would zoom; input.rs `fill_input_state`). "≡" opens a menu:
  **Vote to kick** (`vote::call_vote(CallKind::Kick, Some(name))`, feat/votes) and **Copy name** (Windows clipboard).
  The result shows as the hint line under the list for 4 s.
- **Data:** game-thread TLS block (captured in slot 36, and on a dedicated server in throttle.rs's pump hook):
  +0x18 players data array → +0x50 datums of 0x490 bytes (u16 salt at +0 non-zero = used; +0x08 u64 xuid; +0xAD i8
  team, -1 none; +0xB0 UTF-16 name[16]; +0x1C8 respawn timer and +0x2EE s16 lives remaining per the HUD tokens).
  Score as the HUD's `#local_player_score` (0x180079b20): game-engine globals TLS +0x20, valid while +0x841C is
  1..4, s16 at +0x3270 + index×8 (+2 when TLS +0x48 → +0x1EAB8 is set: round score). Team game: globals +0 bit 3.
  Team score (0x180079f1c): s16 at the same table +0x80 + team×8 (+2 round).
- **Kills / deaths / assists:** the engine's live game results, updated every tick by 0x1800f88d0 (named `UpdateLiveGameResultsPlayers` in Ghidra) and copied into the
  host's game result at the end (BuildGameResultsForHost 0x1800f92e8): player entries 16 × 0xD8 at 0x1828c82a8 (u8 in
  use at +0, u64 xuid at +7, lookup by xuid as 0x1800fed6c does), statistics 16 × 0x708 at 0x1828c9200 (entry k at
  +k×0x708), one s16 per statistic in the order of the statistic definition table at 0x180b016f0 (pairs {name,
  bits}): 0 games-played, 1 games-completed, 2 games-won, 3 games-tied, 4 rounds-completed, 5 rounds-won,
  6 in-round-score, 7 in-game-total-score, **8 kills, 9 assists, 10 deaths**, 11 betrayals, 12 suicides,
  13 most-kills-in-a-row, 14 seconds-alive, 15-18 custom 1-4, 19 firefight-generators-destroyed.
  Verified by memory diffs (`REACH_SB_DUMP`, 3 dumps around two `unit_kill`s): word 10 went 0/1/2, word 14 counted
  seconds alive (1/9/17), word 0 = 1; kills and assists are placed by the table order (no player-on-player kill
  could be scripted). `unit_kill` counts a death (score −1 in Slayer) but not a suicide (word 12 stayed 0).
  A **client's** copy misses deaths it did not see (2-player test: host D1/D2, client D0/D1), so the host sends its
  counts with the ping table (notes/networking.md "Ping") and clients show those.
- **Ping:** ping.rs (RPNG); max players for the title from `--players` (a joiner: the server's max), 16 offline.
- `REACH_SB_DUMP=1` writes, on each press, the TLS block (0x400), ±7 MB of game state around the engine globals and
  the DLL's .data (0x180afa000..0x184e54000) to `sb-dump-N.bin` (segments [u64 address][u64 length][bytes],
  committed memory only; ~85 MB each). `REACH_SB_MENU=1|kick|copy`: in mouse mode, open the first other player's
  menu (and press that item once).
- Verified 2026-10-17 (hidden pair on 127.0.0.1:48850/48851, `--test-input`, captures looked at): team Slayer and FFA
  on both instances show the right deaths, scores, team scores and 0-1 ms pings; right-click mouse mode opened the
  menu, "Copy name" logged `Copied "Player 2"`; a dedicated server sent counts and pings to its client ("(1/4)").
- **Open:** Firefight per-player score is still 0 (handover open item 2; candidates: statistic 7 in-game-total-score
  above, or the survival globals, not checked). Pad users have no mouse mode (no pad navigation of the menu yet).

## Customize screen (2026-10-15)
Main menu entry CUSTOMIZE (`customize.rs`), laid out after Reclaimer's: player card + rows (armour, colours, Spartan
gender / Firefight voice, emblem rows greyed "not yet") on the left, a flat drawn preview in the centre, the picker for
the selected row on the right. Mouse: hover tries a choice on in the preview, click equips (saved to settings.json
`armour`, applied to a running engine through settings_apply). Keyboard: Up/Down row, Left/Right value, Tab
Spartan/Elite, Esc back. No pad navigation (the menu has none yet). Test hook: `REACH_UI_SHOT=customize,
customize-primary,customize-elite` (any row label lower-case with dashes, e.g. `customize-visor-color`).
**3D preview (2026-10-15):** the centre pane shows the real model rendered by a hidden preview engine process
(`preview.rs`, notes/customization.md "3D preview"): drag = rotate, wheel = zoom, RESET VIEW, hover tries the
piece on the live model (~0.1 s), Elite tab respawns the biped as an Elite (~5 s). The flat figure is the fallback
while it loads (~20 s) or if it cannot start. No stance picker (lowering the weapon crashes the engine). The
`REACH_UI_SHOT=customize…` steps wait until the preview shows (`preview::settled`, up to ~5 min of frames;
`REACH_PREVIEW_SETTLE_SECS` holds the screen longer for measurements).
Follow-ups: emblems, real palette RGB from the game's globals, a fixed backdrop spot (the biped stands where Slayer
spawns it, so the background varies).

## Server browser rewrite (2026-10-15, `ui/browser_screen.rs`)
- The browser is its own full-screen screen (`BrowserScreen`, returns `Action::{None,Back,Launch}`); `menu.rs` only
  opens it and launches the args. Layout in points (egui scales by window height / 720, so 1280x720 and 2560x1440
  look the same): title + BACK, tabs ALL SERVERS / FAVORITES with count badges, search (name, address, map, mode,
  playlist), chips ALL / MULTIPLAYER / FIREFIGHT / CAMPAIGN / FORGE (forge always 0 for now) with counts, HIDE FULL,
  HIDE EMPTY, FILTERS popup (max ping, version match, playlist only; password is a disabled placeholder).
- Table: star, status dot, server, map (pretty), game type (variant pretty name, mode icon), players (sort arrow),
  ping (signal bars), version (red when it differs from ours). Elided text shows a tooltip. ALL SERVERS = master list
  + LAN broadcast finds. Side panel: totals, selected server card/details, JOIN GAME (disabled when full or campaign
  in progress, with the reason), DIRECT CONNECT dialog (Connect / Add to favorites), REFRESH ("Refreshing: X of Y
  servers answered so far" via `query::query_many_progress`).
- Map picture: the install has no per-map thumbnails outside packed texture packs, so the card is the install's
  `Data/UI/BackgroundVideos/Images/HaloReach.png` tinted by a colour from the map name (read at runtime, never copied).
- Test hooks: `REACH_UI_SHOT=browser,browser-cat,browser-sel,browser-direct,browser-fav`; `REACH_BROWSER_FAKE=1` shows
  made-up servers (no network). Checked in `work/run-browserui` with a local master (49510) + 3 dedicated (49500-49502)
  and the live master, at 1280x720 and 2560x1440.

## Campaign screen (2026-10-15, `ui/campaign.rs`, `ui/mccart.rs`)
Main menu entry CAMPAIGN (full window, Reclaimer's Halo 3 campaign screen adapted to Reach). Tabs CAMPAIGN / FIREFIGHT.
- Left: missions in story order (m05 … m70, m70_bonus = Lone Wolf; m70_a credits left out; only missions whose
  `maps/<stem>.map` exists), thumbnail + "MISSION NN" + name, "N / 11 FINISHED" (settings `campaign.finished`;
  **TODO:** nothing writes it yet, mission end is not detected). Firefight tab: the 9 Firefight maps.
- Right: banner with number, name and a one-line tagline (our own words); DIFFICULTY cards (icon + one-line
  description); CAMPAIGN MODIFIERS row → skull picker (13 Reach skulls, icons, description, toggle); on the Firefight
  tab a GAME TYPE ◀ ▶ row (`hr_ff_*`) takes its place (launch skulls have no effect in Firefight). PLAY SOLO → `--campaign M --difficulty D --insertion 0 [--skulls a,b]` (Firefight
  `--firefight M --difficulty D --game-variant V`); HOST CO-OP → the same flags through menu.rs
  `listen_args` (Host screen's port/name/master, Players slider 2..4). A campaign host gathers friends first (lobby panel, Enter starts; campaign-firefight.md "Campaign lobby on a listen host"). Last mission, difficulty and skulls saved in
  settings `campaign`. Keys: Up/Down mission, Left/Right difficulty, Enter play, Esc back / close picker.
- Layout in points for 720 lines (ui::draw scales by height); list rows and banner share the space left.
  Checked at 1280x720 and 2560x1440.
- **Art from the player's install at runtime (never in the repo):** `Data\UI\texturepacks\<pack>.perm.bin` = array of
  0x1E0-byte entries: name (ASCII) at +0x44, format at +0x6C (1 = BC1, 3 = BC3), u16 width/height at +0x74/+0x76,
  u64 size at +0x88, u64 offset at +0x90 into `<pack>.temp.bin`; one mip, linear. mccart.rs decodes BC1/BC3 itself.
  Names used: `levelstexturepack` `HREACH_C01_NOBLEACTUAL` … `HREACH_C11_LONEWOLF` (316x160) and `…_LARGE` (668x292),
  `REACHFIREFIGHT_<MAP>` / `REACHFIREFIGHT_<MAP>LARGE`; `mainmenuandcampaigntexturepack` `DIFFICULTYLARGE{EASY,
  NORMAL,HEROIC,LEGENDARY}` (256²); `skullstexturepack` `IRON`, `BLACKEYE`, … `SKULL` (128²). Missing picture = text card.
  Also there: MP map previews (`HREACHMP_*`, `HREACH_*_MINI`), Firefight game-type art (`FF_REACH_*`), for later screens.
- egui-directx11 samples with BORDER addressing and a white border colour: image uvs are inset half a texel, else
  a grey line shows along icon edges.
- Test hooks: `REACH_UI_SHOT=campaign,campaign-skulls,campaign-ff,campaign-start,campaign-host` with `REACH_UI_CAMP="m10 heroic iron"`
  (mission, difficulty, skull list) applied when the screen is first made; `campaign-start` presses PLAY SOLO.

## Forge screen (2026-10-15, `ui/forge.rs`)
Main menu entry FORGE: maps / saved maps with install art, START FORGE / HOST FORGE, play a saved map with a game type,
pause-menu SAVE MAP section, map-variant row on Play / Host. Details and verification: [forge.md](forge.md).

## Menu music (2026-10-17, `music.rs`)
The main menu plays Reach's own menu theme, decoded at runtime from the player's install (nothing shipped). Started in
`main.rs` just before `ui::menu::run`, faded out (about 1 s) and joined when the menu returns (game launch or quit).
Volume = `Settings.audio.menu_music` (Audio page, "Menu music"), re-read live; 0 pauses the stream.
- Audio: `haloreach/fmod/pc/sfx.fsb`, FSB5 v1, codec 15 (Vorbis), 16162 samples. Header 0x3C bytes: u32 numSamples @8,
  sampleHeadersSize @12, nameTableSize @16, dataSize @20. Sample data at 0x3C + shs + nts. Per sample a u64: bit0 has
  chunks, bits 7..33 data offset / 32, bits 34.. sample count; chunks u32 {bit0 more, bits1-24 size, bits25-31 type},
  type 11 = Vorbis, first u32 = setup-header CRC. Packets are `{u16 size, bytes}`; no Vorbis headers in the bank.
- Names in the bank are short; `sfx.fsb.info` (0x118-byte entries, same order, full path at +0x18) gives the real path.
  Menu music = paths under `data\sound\music\reach_menu\track1\`: `in\02_menu_in` (intro, 16 s) + 12 `loop\*`
  (hymn_brass, sad_strings, spacey7, tlr_rhythm, 08_tplse, 14_strings, each plus a `_01`). Stereo 48 kHz. Order: intro,
  then random loops, never the same twice in a row; tracks are joined gaplessly (no crossfade).
- Setup header comes from the player's `haloreach.dll`: FMOD's fsbvorbis codec (0x1805c51c0) searches a table at
  0x180a725c0, 0x20-byte rows {u64 data VA, u32 size, u32 crc, u64 base VA, u32 patch offset, u32 patch length}. Base 0:
  data is the complete header (`\x05vorbis...`). Else header = `size` bytes copied from base, then `patch length` bytes at
  `patch offset` replaced from data (menu CRC 0xad11d38d: base 0x180a57d80, size 3484, patch 31 bytes at 3405). The host
  scans the DLL for rows by CRC (PE sections parsed, no fixed offsets); a different build logs `music: none (...)`.
  Using the base unpatched decodes without errors but as noise bursts: the patch matters.
- Ident header synthesised for lewton: channels 2, 48000 Hz, blocksizes 256/2048 (byte 0xB8).
- Test hooks: `REACH_MUSIC_DUMP=<wav>` writes the first 30 s decoded and logs RMS instead of playing;
  `REACH_MUSIC_TRACKS=1` (with DUMP) logs per-track length, rms and diff/rms (about 0.1 for real music, 1.0 = noise).

## Vote screens (2026-10-17, `ui/vote.rs`)
- **Next-game vote page** (Reclaimer's lobby page in our theme), whenever a next-game round is open and no player
  unit is alive (post-game, pre-game wait, dead, loading): CUSTOM GAME title, tabs SETTINGS (pause overlay's settings)
  and LEAVE LOBBY (the leave path); left GAME column (map, game type + base-mode chip, playlist) and a state banner;
  centre the countdown with a shrinking accent line and 3 cards (map art `HREACHMP_<KEY>` / `REACHFIREFIGHT_<KEY>` /
  `HREACH_<mission key>` from the install, map name, game type · base mode, description, votes, vote-share fill) and
  NONE OF THE ABOVE / KEEP PLAYING; right the players from the engine table (team colour, name, YOU) and team sizes.
  Not shown because we have no data for them: emblems, service tags, rank icons, LAST GAME REPORT, CUSTOMIZE.
  Game type name and description come from the variant file's content header (UTF-16 at 0xC0 / 0x1C0); built-in
  variants carry a `$hr_gvar_…` key there, then a line of our own per base mode is used.
- In play: a compact card at the top centre (options, tallies, countdown); 1-4 vote, Backspace hides it.
- **Call vote card** at the right edge, mid-height (on the players column over the page): kind, seconds, question,
  caller, yes bar with the pass tick, YES/NO counts, needed, [F5] YES / [F6] NO (keys via input.rs raw input, and egui
  while the page has the cursor); then the result for 6 s. Pause menu: CALL A VOTE section (greyed with the reason when
  the host cannot run a kind), Kick opens a player picker. A host without call votes: the old VOTE FOR A NEW GAME item.
- Test hooks: `REACH_VOTE_FAKE=next,call[,page]` (made-up next-game round / call vote; `page` forces the page after 150
  frames), `REACH_VOTE_SHOT=1` (`vote-panel.bmp` frame 90, `vote-page.bmp` frame 240, `vote-result.bmp`); test script
  `vote call <shuffle|kick|skip|restart> [name]`, `vote yes|no`, `vote teams`, `vote members`, `vote team <name> <t>`.
