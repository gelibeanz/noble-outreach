# haloreach.dll engine interface (MCC build 2025.08.16.178512)

Facts about how a host drives `haloreach.dll`, in our own words. Addresses are for the frozen DLL in
`work/mcc/haloreach/haloreach.dll` (sha256 `738dd2d2…0894`, PE timestamp `0x68a0efe1`, image base
`0x180000000`; RVA = address − `0x180000000`). Sources: Ghidra on that DLL (project
`work/ghidra/reach.gpr`), libmcc / HaloX / Opus headers as hints (read only, `work/ref/`).

## Exports

| Export | RVA | What it does |
|---|---|---|
| `CreateDataAccess(out**)` | `0x10558` | Allocates a 0x28-byte data-access object (vtable `0x180994e60`, 8 slots), reserves a 0x4B000 buffer, returns **0x46** |
| `CreateGameEngine(out**)` | `0x10468` | `_aligned_malloc(0xC60, 0x20)` engine object (vtable `0x180985e70`), three SLISTs (free list at +0xC40 of 32 nodes, queue at +0xC30), stores it in global `0x180c1a0c8`, returns **0x46** |
| `SetLibrarySettings(wchar_t[3][85])` | `0x103f4` | Copies three 85-wchar language strings (audio, text, text2, e.g. `en-US`) into globals. Returns nothing useful (void) |

0x46 (70) looks like the interface version both factories report.

## Engine vtable (`0x180985e70`, 11 slots)

| Slot | Address | Name (ours) | Notes |
|---|---|---|---|
| 0 | `0x18000fff0` | EngineDestroy | `_aligned_free(this)`, clears the global |
| 1 | `0x18000f0b4` | EngineInitializeGraphics(device, context, swapchain, swapchain2) | Stores device, swapchain, swapchain2 in globals (context unused). Finds the process's own top-level window with `EnumWindows` + PID |
| 2 | `0x18000f118` | EngineInitializeGame(host, options) → thread handle | `memcpy` 0x2BF30 bytes of options into global `0x1824971a0`; host pointer into global `0x180c1a100`; starts `GameThreadMain` (`0x18000dc78`) |
| 3 | `0x18000f1bc` | EnginePostMessage(id, param) | Queues a message. Param bytes copied: none for id < 0x10; 8 for 0x10, 0x11, 0x13, 0x18; 16 for 0x12; 48 for 0x19 |
| 4–8 | `0x18000c380` | — | `_guard_check_icall` (empty). Preload calls are no-ops in Reach |
| 9 | `0x180010008` | EnginePostCommand(const char*) | Queues a copy of the string (id −1). MCC sends script commands prefixed `HS: ` |
| 10 | `0x18000d560` | — | returns 0 |

## Launch options (0x2BF30 bytes)

Size confirmed by the `memcpy` in slot 2. Fields read directly in slot 2:

- byte 0 flags: bit 3 (0x08) = multiplayer → copies options+0x50 into `0x180cfb5e8` and sets `0x184e38ce1`;
  `flags & 0x48 == 0x48` sets `0x180c1a0bd`.
- options+0x58 (qword) → `0x180d1ade8` (libmcc calls it host_address).

Verified MP layout, data-access object and variant copy methods: [launch-options.md](launch-options.md).
Rest of the layout per libmcc (unverified on this build): +0x0C game mode (1 campaign, 3 MP, 5
firefight), +0x10 legacy map id, +0x14 map id, +0x48 skulls, +0x60 player options (17 peer
addresses, player count, 16 × {xuid, address, …}, peer count), then game variant (117760 bytes), an
unknown 1032-byte block, map variant, another 1032-byte block, saved-game pointer, film path.

## Host interface (the object MCC passes to slot 2)

Stored in global `0x180c1a100`; the engine calls it as `(*host)->vtable[slot](host, …)`. A scan of
every load of that global followed by an indirect call finds **55 distinct slots** in use:

0–7, 9–15, 17, 18, 23, 24, 26–28, 30, 32, 33, 35, 36, 39, 41, 44, 45, 47, 48, 51, 52, 55, 57, 58,
62, 65, 67, 73–75, 88, 89, 94, 95, 98–100, 104, 106, 107, 114.

Highest slot used: 114 (byte offset 0x390). libmcc's list has 121 slots. Per-slot signatures and
meaning: [host-slots.md](host-slots.md).

Networking goes through the host: slot 41 (+0x148) send and slot 43 (+0x158) receive, for every
type-8 (network-id) address; slot 42 (send reliable in libmcc) is never called. Details in
[host-slots.md](host-slots.md#networking).

## HaloX offsets match this build

HaloX (`work/ref/HaloX`, May 2026) Reach RVAs line up with this DLL, e.g. the polled virtual-key
byte table at RVA `0x98FDF0` (ESC, F1–F12, 1–0, Q–P …). So its documented RVAs are usable leads,
still to be checked one by one before relying on them.

## Host requirements found by running (2026-10-15)

- The swap chain passed to `initialize_graphics` must allow `DXGI_USAGE_SHADER_INPUT` as well as render
  target output: the engine builds a shader view of the back buffer (render target type 1). Without it the
  first frame faults in the screen clear (`haloreach+0x2744f2`).
- Rasterizer init (`0x18024fb2c`) adopts our device and swap chain (QueryInterface for `ID3D11Device1`,
  `ID3D11DeviceContext1`, `IDXGIDevice1`; creates its own `IDXGIFactory1`) and builds render targets itself.
- Host slot 34 returns the local player's profile (0xACC bytes) and must not be null.
- With a zeroed options block the engine starts, then exits cleanly: `signal_exit(4, "no map selected")`.

### Captured Slayer launch (Forge World, `work/traces/2026-10-15-slayer/game-options-0.bin`), 2026-10-15

Our host now loads and plays the match MCC recorded: map loaded at ~8.4 s, spawn ~18 s, 60 fps steady, first-person
view with HUD. What it needed:

- **Map info slots 17/18 are not needed** for a stock map (id `…0x8888` form). Slots 11/12 are not needed either:
  MCC only gets them because it posts msgs 0x0A/0x0B (snapshot the game/map variant back to the host).
- **Loading-screen "hang" was game time paused for controller loss.** After `set_game_state(1)` the engine kept
  drawing the black screen with the load bar while the game ticked. Game time's pause word (engine TLS block
  +0xa0, u16 at +2) read `0x78`: pause reasons 3–6 = one per local user slot (table `0x180994ac0`). The per-frame
  check `0x18005ccd4` pauses for local user *i* when the user is signed in (slot 88 gave a xuid) but has no input
  device, in a game with one machine and ≤ 1 player. Two fixes, both required:
  - **slot 36 (get_input_state)** must return **true for player 0** with a filled 0x130-byte state (kind 0 =
    gamepad, all zero = at rest). Returning false (or leaving it to XInput, which finds no pad under Wine) means
    "controller disconnected".
  - **slot 88 (get_player_xuid)** must return **false for players 1–3**. Returning a xuid for every index signs in
    four local users, and the three without input each hold a pause bit.
  With both, the pause word is 0, the scoreboard shows (state 8), the player spawns (state 9, slot 75
  first-person weapon offset starts), and the world renders.
- Engine messages 0/1 (seen in MCC's trace) are the host's own pause/resume (pause reason 2, plus a 5-tick
  hold of `0x180c1a0e2`); posting msg 1 did nothing for the hang. Not needed to start a match.
- The engine's own load screen flag (`0x180c519c8`) clears 2 s after progress reaches 90 % (`CheckLoadScreenFinished`);
  it was already 0 during the "hang", so the black screen was the game view with game time paused.
- **HUD drawn black: slot 97 is a HUD colour transform.** `SetupChudDrawShaderConstants` (0x1802ddd8c) passes each
  of its six chud colours through `TransformHudColorViaHost` (0x1802ddf3c): host slot 97 (+0x308) gets
  `(this, int mode = *0x180d1f71c, u32 argb)` and returns the ARGB to use (MCC applies colour-blind filters here;
  ~1.5 M calls in the traced match). A stub returning 0 makes the whole HUD black and transparent (dark tracker,
  empty shield segments). Returning the input unchanged gives the normal blue HUD (verified on a back-buffer
  capture).
- Volumes: profile (slot 34) +0xA5C/+0xA60/+0xA64 (master/music/sfx, 0..1) are copied into engine globals
  `0x180afc108/10c/110` only by `RebuildPlayerInputBindings`, so the engine applies them late and not on a
  volume-only change. The host sets the globals and the mastering voice itself (`audio.rs`, notes/host-slots.md
  "Volumes").
- **No game audio = `xaudio2_9redist.dll` not found.** The engine loads it itself (`LoadXAudio2RedistAndX3DAudio`
  0x180176030: `LoadLibraryExA(name, 0, LOAD_LIBRARY_SEARCH_DEFAULT_DIRS)`, HRESULT stored at `0x180b019e0`) and
  delay-imports it for XAudio2Create. Default dirs are the exe's folder and System32, not SetDllDirectory or the
  cwd, and the DLL lives in `MCC/Binaries/Win64`. Our host preloads it by full path (`audio.rs`). With it: load
  hr 0, audio system `0x180b88718` and mastering voice `0x180b88730` non-null, 6 ch / 48 kHz, and the game's
  PulseAudio stream is non-silent (confirmed by ear). FMOD is statically linked (no fmod DLL); slot 33 can stay a no-op.
- Runs can be checked without the window: the host logs a heartbeat every 10 s (fps, load-screen globals, game
  time pause bits) and saves the back buffer as `frame-NNNs.bmp` next to the exe for the first minute.

### HUD text, icons and zoomed look (2026-10-15)

- **All text and the HUD weapon/grenade pictures need host fonts.** MCC's files have no
  `font_package.bin`; with slot 51 setting 6 false the engine draws every string empty and loses the
  icon-font glyphs (weapon picture, grenade icons). Setting 6 true + slots 61-67 (`text.rs`, GDI glyph atlas)
  brings back the weapon icon, grenade count, score widget (time, mode, score), map/range labels and the
  loadout picker (names, abilities). Details: host-slots.md "Fonts".
- **Zoomed mouse look comes from the profile.** `GetZoomLookSensitivityScale` 0x1800618f0: while zoomed and on
  keyboard/mouse, look is scaled by profile +0x1C0 / magnification (+0x1C4 in a vehicle seat). A zero profile
  froze the crosshair when scoped. Ours sets both to 1.0 plus the engine defaults for pad look (sensitivity index
  4 at +0x1B5/+0x1B6, dead zones 0.2) in `profile.rs`. Verified: 300 counts while DMR-scoped turned 8.6° (25.8°
  unscoped / 3x).
- **Player names come from slot 88's name buffer** (see host-slots.md); ours writes `REACH_NAME` or "Player <n>".
- Back-buffer captures by wall clock for screens before spawn: `REACH_CAPTURE_AT=14,16` → `at-014s.bmp` ….
