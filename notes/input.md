# Player input (host slots 36, 39, 98, 100, 116)

How `haloreach.dll` (MCC build 2025.08.16) takes keyboard, mouse and pad input from a host, and how
`src/reach-host/src/input.rs` feeds it. Addresses are for the frozen DLL. Verified 2026-10-15 on Forge World
(Slayer) in our host with scripted input (see "Verification").

## Engine flow

1. `PollInputDevices` (0x1800239e0), once per frame: for each local player 0–3 it pre-fills the state's kind
   with the player's last device (global 0x182dfbe74, one u32 per player: 0 pad, 1 keyboard/mouse) and calls
   host slot 36. A `true` return marks the player as having a device and stores the kind back as the new
   last device. Players for whom the host returns `false` have no device.
2. Keyboard: the engine polls a fixed list of 0x68 virtual keys (table at 0x18098fdf0) and reads them from
   **player 0's** key array only (+0x04). The list holds side-specific modifiers (`VK_LSHIFT` 0xA0,
   `VK_RSHIFT` 0xA1, `VK_LCONTROL` 0xA2, `VK_RCONTROL` 0xA3, `VK_LMENU` 0xA4, `VK_RMENU` 0xA5); generic
   `VK_SHIFT`/`VK_CONTROL` are not polled. A VK→poll-index table sits at 0x18098fbf0 (s16 per VK, −1 = not
   polled).
3. Mouse: player 0's mouse block (+0x104) is converted by `ConvertHostMouseStateToEngine` (0x1800245f0) into
   the engine's mouse record at 0x182dfc1c0.
4. Pad: for a player whose kind is 0 the pad block is copied into the engine's pad record
   (0x182dfc224 + player × 0x3c, via `UpdateGamepadButtonTimers` 0x1800abdbc) and the player's "pad present"
   bit is set (0x182dfc220). Kind 1 clears that bit, so the keyboard/mouse path runs instead.
5. `UpdatePlayerInputActions` (0x1800d1540) turns devices into per-action records per local player
   (0x18287ff94 + player × 0x700; keyboard/mouse action *a* at +0x224 + a × 0x10: u16 held ms, u8 held
   frames, flags, f32 value). The keyboard/mouse branch runs for player 0 only.
6. `UpdatePlayerControlState` (0x180061a38) / `ComputePlayerUnitControl` (0x18005df80) turn actions into
   unit control: player control globals (engine TLS block +0xD8) hold per player (stride 0x180, from +0x80):
   +0x80 unit handle, +0x88 u64 action flags, +0x94 yaw, +0x98 pitch (radians), +0xAC throttle forward,
   +0xB0 strafe. The unit's object (object table TLS +0x10 → +0x50, 0x18-byte entries, pointer at +0x10)
   holds its world position at +0x20 (3 floats).

Keyboard/mouse mode does **not** pause the game: with kind 1 for player 0 the pause word stayed 0 in every
run. Players 1–3 must keep returning `false` (and stay signed out via slot 88), see engine-interface.md.

## input_state (0x130 bytes per player, slot 36)

| Offset | Type | Meaning |
|---|---|---|
| +0x000 | u32 | device kind, in/out: 0 pad, 1 keyboard/mouse |
| +0x004 | u8[256] | key down (non-zero), indexed by Win32 VK; read for player 0 only |
| +0x104 | f32 | mouse X this poll; with a host it is **radians of yaw** (positive turns right) |
| +0x108 | f32 | mouse Y this poll, radians of pitch (positive looks down) |
| +0x10C | f32 | cursor X in client pixels (libmcc; not read by the poll) |
| +0x110 | f32 | cursor Y |
| +0x114 | f32 | wheel delta (Windows units, 120 per notch, positive = up) |
| +0x118 | f32 | wheel divisor (engine uses max(this, its own default)); we send 120 |
| +0x11C | u32 | mouse buttons: bit 0 left, **bit 1 middle, bit 2 right**, bit 3 X1, bit 4 X2 |
| +0x120 | u16 | pad buttons, XInput bit layout (engine mask table at 0x1809d7470 is exactly XInput's) |
| +0x122 | u16 | unused |
| +0x124 | u8 | left trigger |
| +0x125 | u8 | right trigger |
| +0x126 | s16 ×4 | left X, left Y, right X, right Y (XInput ranges) |

Mouse button order: right is bit 2, not bit 1 as libmcc's enum says. Verified: holding bit 2 scoped the
DMR. The binding code for `VK_RBUTTON` maps to mouse index 2 and `VK_MBUTTON` to index 1.

Mouse look units: without a host the engine scales raw counts by π/1000 and the profile sensitivity; with a
host, `ComputePlayerUnitControl` uses the value as-is (divided by the tick time to a rate and applied over the
tick). So the host owns sensitivity. Ours sends `counts × 0.0015 × --mouse-sens`. Verified: 500 counts
turned yaw by exactly 43.0° (0.75 rad).

## Bindings

Keyboard/mouse bindings come from the **profile** (slot 34), not from a slot of their own.
`RebuildPlayerInputBindings` (0x1802c8dfc) runs for player 0 on settings sync. It starts from the engine's
built-in PC layout (`InitDefaultKeyboardMouseBindings` 0x1800d23e4) and then, for each of MCC's 66 actions,
reads two codes from the profile's mapping block (+0x42C, entries of 0x18 bytes: u32 action, then codes, of
which the engine reads the first two at entry +4 and +8). MCC action → engine action:
`MapMccActionToEngineAction` (0x18003c724). Per code:

- 0 → that binding slot is unbound
- 1 / 2 / 4 / 5 / 6 (`VK_LBUTTON`, `VK_RBUTTON`, `VK_MBUTTON`, X1, X2) → mouse buttons 0 / 2 / 1 / 3 / 4
- 0xAF / 0xAE → wheel up / wheel down
- other values below 0x100 → keyboard VK, kept only if the engine polls it
- **0x100 or more → keep the engine's default** for that slot

A zeroed profile therefore unbinds every key (the reason players could not move before). Three tables
result (mouse button, axis, key; 81 engine actions × 2 slots each) at 0x18287f7e0 / 0x18287fa68 /
0x18287fcf0. Engine actions outside MCC's list keep their defaults: Esc = Start/pause, mouse axes = look.

Our table (`KBM_BINDINGS` in `input.rs`; everything else keeps the engine default):

| Action | Binding |
|---|---|
| Move | W A S D |
| Look | mouse |
| Jump | Space |
| Crouch | Left Ctrl |
| Fire | Left mouse |
| Zoom | Right mouse |
| Reload | R |
| Action / interact | E |
| Melee | Q |
| Throw grenade | F |
| Switch grenade | G |
| Switch weapon | wheel up / wheel down |
| Armor ability (sprint is an ability in Reach) | Left Shift |
| Scoreboard | Tab |
| Pause | Esc (engine default; also frees the mouse) |
| Flashlight, vehicle tricks, Forge/Theater keys | engine defaults (Forge: `~` monitor, `1` place, `2` tools, LMB grab, R/F raise/lower, Delete; see [forge.md](forge.md)) |

The engine's own defaults decode to the same layout as MCC's Reach PC defaults (Space jump, G switch
grenade, C switch weapon, E action, Q melee, LShift ability, F grenade, LMB fire, LCtrl crouch, RMB zoom,
Tab scores, R reload, 4 flashlight, WASD). We swap C for the wheel on switch weapon: the engine reads only
two codes per action and Reach has no "weapon 1 / weapon 2" action, so 1/2 cannot be bound.

### Reach PC defaults (decoded 2026-10-17)
Static decode of `InitDefaultKeyboardMouseBindings` + `MapMccActionToEngineAction`; the engine fills **slot 0 only**
(slot 1 stays unbound). Mouse table values 0/2/1/3/4 = LMB/RMB/MMB/X1/X2; key values are poll indices into the list at
0x18098fdf0 (VK → index map at 0x18098fbf0, 16-bit entries; VK 0x10 = generic Shift maps to index 105, 0x11 Ctrl to
106). There is **no default for the wheel** (axes table only holds the look axes). `ENGINE_DEFAULT_KEYS` in
`input.rs` mirrors this table; the settings screen resolves untouched slots through `effective_keys` (we still send
0xFFFFFFFF for them, so engine behaviour is unchanged).

| MCC | Engine | Default | Meaning |
|---|---|---|---|
| 0 | 0x00 | Space | Jump |
| 1 | 0x01 | G | Switch grenade |
| 2 | 0x03 | E | Action |
| 3 | 0x3E | R | Reload |
| 4 | 0x02 | C | Switch weapon (we bind the wheel) |
| 5 | 0x04 | Q | Melee |
| 6 | 0x16 | 4 | Flashlight |
| 7 | 0x06 | F | Throw grenade |
| 8 | 0x07 | Left mouse | Fire |
| 9 | 0x08 | Left Ctrl | Crouch |
| 10 | 0x09 | Right mouse | Zoom |
| 11 / 12 | 0x3C / 0x3D | Z / X | not labelled yet |
| 16-19 | 0x49-0x4C | W S A D | Move |
| 20 | 0x3B | Tab | Scoreboard |
| 21 / 22 | 0x0C / 0x0D | `[` / `]` | Vehicle tricks |
| 23 | 0x05 | Left Shift | Armor ability |
| 24 | 0x0A | Right mouse | not labelled yet |
| 25 / 26 | 0x1E / 0x1F | R / F | Forge raise / lower |
| 27 | 0x22 | Left mouse | Forge grab / drop |
| 28 | 0x26 | Shift (generic) | Forge move faster |
| 29 | 0x27 | Right Ctrl | Forge precision |
| 30 | 0x21 | Delete | Forge delete |
| 31 | 0x20 | 1 | Forge create objects |
| 32 | 0x23 | 2 | Forge object properties |
| 33 | 0x1D | `` ` `` | Forge monitor mode |
| 34 | 0x28 | Right mouse | Forge rotate held object (inferred from the Halo 3 layout) |
| 35 | 0x29 | Left Ctrl | Forge lock object |
| 36 / 37 / 38 | 0x2F / 0x3F / 0x43 | X / Z / C | Theater panel / interface / view |
| 39, 50 | 0x42, 0x31 | Right mouse | not labelled yet |
| 40 / 41 | 0x2D / 0x2C | Right / Left arrow | Theater fast forward / rewind |
| 42 | 0x2E | Num Enter | Theater pause / play |
| 43 / 45 | 0x2B / 0x40 | Left Alt / Shift | not labelled yet |
| 44 | 0x41 | Space | Theater free camera |
| 47 / 48 | 0x44 / 0x45 | R / F | Theater camera up / down |
| 51 / 52 / 54 | 0x2A / 0x25 / 0x24 | B / G / X | not labelled yet |
| 58 / 61 | 0x3A / 0x46 | L / P | not labelled yet |
| 62 / 63 | 0x0E / 0x0F | `[` / `]` | not labelled yet (pair with the vehicle tricks) |

MCC actions with no default: 13-15, 46, 49, 53, 55-57, 59, 60, 64, 65 (and 46, 56, 65 map to engine actions with
no binding). Engine-only: Esc = pause (engine 0x18), look axes 0x4D-0x50.

Pad: slot 116 may return a 66-byte table (MCC action → abstract pad button 0–15: LT, RT, d-pad up, down,
left, right, Start, Back, LS, RS, LB, RB, A, B, X, Y), applied over the engine default layout
(`InitDefaultGamepadSettings` 0x1800d22ec); bytes ≥ 0x10 are skipped. `null` keeps the default (Reach
"Default" preset). Look sensitivity, inversion, southpaw/lefty and similar come from per-player settings the
engine builds from the profile; ours are profile zeros (lowest stick sensitivity index), not tuned yet.

## Other input slots

- **98** `UpdatePlayerInputActions` calls it per local player. True: the keyboard and mouse-button bindings
  are skipped and only the mouse axes are read, blended with the pad (larger magnitude wins). False: full
  keyboard/mouse processing when the player's kind is 1, nothing from them when on the pad. So: false on
  keyboard/mouse, true on pad (lets the mouse nudge aim). The earlier note "true else mouse look is zeroed"
  was wrong.
- **39** rumble: four u16 (left, right, left trigger, right trigger) per player each poll; the engine zeroes
  them while paused or when the player is not on a pad. Ours forwards left/right to `XInputSetState`.
- **100** control context: (player, u8). Context 1 arrives at map start; vehicles and cinematics change it.
  Logged only.
- **7** pause: Esc (and pad Start) reach host slot 7; the engine draws nothing itself (see host-slots.md).

## Mouse in engine menus (loadout picker), 2026-10-15

The engine's own menus (the spawn-time loadout picker; its button bar shows "Select Loadout" with a mouse icon
in keyboard/mouse mode) support the mouse natively. They do **not** use the cursor fields of slot 36
(+0x10C/+0x110 are never read); they call Win32 directly:

- **Hover:** `UpdateScreenCursorHover` 0x1802d2018 (a screen's per-frame update, for screens owned by local
  player 0) calls `GetForegroundWindow`, and only if that equals the engine window global **0x180c1a188**
  does it call `GetCursorPos` + `ScreenToClient` + `GetClientRect`, scales client pixels to the render size
  (0x180b43a90 / 0x180b43a94) and, when the position changed, hit-tests and focuses the widget under it.
- **Click:** `TranslateInputToUiEvents` 0x1802ada60, while player 0's device kind is 1 (0x182dfbe74), turns a
  fresh press (held frames == 1) of mouse button 0/1/2 in the engine mouse record (0x182dfc1cc..ce) into a UI
  event via `PostMouseButtonUiEvent` 0x1802aed20: {type 0x0D, player 0, button 0x1E left / 0x1F middle /
  0x20 right}. Wheel up/down become events {0x11, 0x21/0x22}. List widgets handle 0x1E in
  `HandleListMouseClickEvent` 0x18034e774 (cursor read again, child under it selected and activated);
  `HandleScreenButtonMouseEvent` 0x180305b44 does the same for button-bar items.
- `GetForegroundWindow` is imported only for these three functions. Import slots: `GetForegroundWindow`
  haloreach+0x91e4b0, `GetCursorPos` +0x91e4b8, `ScreenToClient` +0x91e4c0.
- **The catch:** the window global 0x180c1a188 is set once in `EngineInitializeGraphics` 0x18000f0b4 by
  `EnumWindows` + `FindVisibleProcessWindowCallback` 0x18000f054: the first *visible*, unowned top-level window
  of the process. With `--hidden` (or a window not yet shown) it stays 0, so hover and clicks never happen.

Host side (`input.rs`):
- `ensure_engine_window` (each slot-36 poll) writes our HWND into 0x180c1a188 when it differs (logged once).
- `install_cursor_hooks` (main.rs, right after the DLL loads) replaces the two import slots after checking they
  hold the real user32 functions: `GetForegroundWindow` returns our window in scripted runs (hidden window),
  else the real one; `GetCursorPos` returns the script's virtual cursor when set, else the real one.
- **Spawn screen** = player control exists but player 0's unit is `0xFFFFFFFF` (map loaded, waiting to spawn or
  dead). While it lasts the OS cursor is shown (still clipped to the window while captured); it is hidden again
  on spawn (`WM_INPUT_CURSOR` posted to the window thread, `sync_cursor`). Mouse buttons already reach the
  engine while captured, so a left click picks the loadout. Picking closes the list ("Press [L] to access
  Loadout Menu"); the spawn itself still waits for the engine's spawn timer.
- Test script additions: a `from map` line times the script from the map load instead of the spawn;
  `cursor <x> <y>` (client pixels) / `cursor off` sets the cursor the engine reads.

Verified (`scripts/test-input-loadout-mouse.txt`, Forge World Slayer, `--hidden`, 1280x720): before the window
fix the hook saw ~120 `GetForegroundWindow` calls/s and no `GetCursorPos`; after it, cursor on row 3 → Jet Pack
row highlighted (pulled out like the focused row); left click → list closed; after spawn the HUD ability icon is
the jet pack and holding LShift lifted the player (z 45.029 → 46.091), where the default (Sprint) loadout spawns
otherwise. Not verified: a real physical mouse in a visible window, the respawn picker after a death, the
button-bar items (Cancel), and pad users (cursor is shown on the spawn screen even when on the pad).

## Host behaviour (input.rs)

- Raw input (`WM_INPUT`) for mouse and keyboard; generic Shift/Ctrl/Alt are split into the left/right VKs
  from the scan code / E0 flag. Legacy `WM_KEYDOWN/UP/CHAR` are swallowed (system keys still work).
- The mouse is captured (clipped to the client area, cursor hidden) on focus or a click; Esc and focus loss
  release it, F10 toggles. Mouse motion and buttons only count while captured. Focus loss releases every key.
- XInput: first connected pad (slots probed once a second while none is connected). The device used last
  (a key, mouse motion/button/wheel, or pad input past the dead zone / trigger threshold) decides kind 0/1.
- `--mouse-sens <x>` multiplies 0.0015 rad/count. No config file yet.
- `--test-input <script>` replays a timed script (seconds after player 0 spawns) into the same state the
  devices write: `key <name> down|up`, `button left|middle|right down|up`, `wheel <n>`, `turn <dx> <dy> <secs>`,
  `pad <buttons hex> <lt> <rt> <lx> <ly> <rx> <ry>` / `pad off`, `cursor <x> <y>` / `cursor off`,
  `capture <file.bmp>`, `log <text>`, `exit`; a `from map` line times it from the map load instead.
  Scripted runs ignore the real keyboard, mouse and pad and never capture the mouse. Every 0.5 s (30 s in
  normal runs) the log shows player 0's unit, position, yaw, pitch, throttle and held actions.

## Verification (2026-10-15, `scripts/run-host-input.sh --map forge_world --test-input …`)

`scripts/test-input-move.txt`, `test-input-actions.txt`, `test-input-keys.txt`; logs in
`work/run-input/reach-host.log`, captures next to the exe.

- W held 2 s: throttle 1.00, position (63.498, 236.430, 45.029) → (63.487, 232.126, 45.029), 4.3 units.
- Turn 500 counts right over 0.5 s: yaw 269.9° → 226.9°; capture shows the hangar side wall, compass SW.
- Space: z 45.029 → 45.552 mid-jump → 45.029 landed.
- Right mouse: DMR scope view in the capture. Left mouse 1 s: ammo counter 32 → 21.
- Wheel up: DMR → magnum. F: grenade in flight. Q: melee swing. LCtrl: camera lower, "crouch" held.
- Tab, LShift, E, G, R: the matching engine actions read as held (scores, ability, action,
  grenade-switch, reload). Esc: host slot 7 called.
- Synthetic pad (kind 0): left stick up → throttle 1.00; right stick right 1 s → yaw 226.9° → 99.0°.
- Pause bits stayed 0 throughout, keyboard/mouse and pad.

Not verified: the real raw-input and XInput paths with physical devices (the script feeds the state after
decoding); rumble; Tab shows no scoreboard on screen even though the action is held (no host slot is called
either; MCC may draw it itself); Shift's ability effect was not visible in captures; pad look sensitivity
(profile index 0) is untuned.
