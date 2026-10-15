# Spartan / Elite customisation (MCC build 2025.08.16)

## How it reaches the game
- The host owns the appearance: slot 34's profile (libmcc `s_player_profile`) holds it; there is no per-player field in
  the launch options. `ApplyProfileLookSettingsToPlayer` 0x1802eda08 reads it into the local player's settings record
  (0x182a03c58 + i × 0xab8) when the player is set up and after engine message 5.
- Every piece is stored as **MCC's own customisation item id** (a 32-bit value from one global list shared by all MCC
  games), not Reach's index. `ConvertHostCustomizationItemToEngineIndex` 0x18003b2f4 (renamed in Ghidra) is a big
  switch from MCC id to Reach's per-category index; unknown ids become 0 (the default piece).
- The engine's own player data carries the appearance to the other machines. Verified 2026-10-15: a 2-instance online
  Firefight (ff20_courtyard, ports 48600/48601): instance B saw instance A in Operator helmet, gold visor, coral primary
  and cyan secondary while B itself had defaults.

## Profile fields read (all u32 MCC ids unless noted)
| Offset | Meaning | Engine setter | MCC ids |
|---|---|---|---|
| +0x27 (u8) | female voice | flags bit 0 | — |
| +0x38 (u8) | play as Elite | record flag | — |
| +0x40 | helmet (the attachment is part of the id) | `FUN_1802c807c` category 0 slot 0 | 0x00–0x59 (base + variants per helmet; 0x40–0x59 are MCC's later helmets) |
| +0x44 / +0x48 | left / right shoulder | category 0 slots 1 / 2 | 0x5A–0x6B / 0x6C–0x7D |
| +0x4C | chest | category 0 slot 3 | 0x7E–0x8F, 0xE8–0xEF |
| +0x50 | wrist | category 0 slot 4 | 0x90–0x95 |
| +0x54 | utility | category 0 slot 5 | 0x96–0x9C |
| +0x64 | Spartan body: 0xC1 female (flags bit 1), anything else male | flags | 0xC0 / 0xC1 |
| +0x68 | Spartan armour effect | category 1 slot 0 | 0xA6–0xAC |
| +0x6C | knee guards | category 0 slot 6 | 0xA2–0xA5 |
| +0x70 | visor colour | `FUN_1802c7f10` | 0x9D–0xA1 (default, silver, blue, black, gold) |
| +0x8C | Elite armour type | category 0 slot 7 | 0xC2–0xC9 (minor … officer) |
| +0x90 | Elite armour effect | category 1 slot 1 | 0xAD–0xB3 |
| +0x94 | Firefight voice | category 1 slot 2 | 0xB4–0xBF |
| +0x98 / +0x9C | primary / secondary colour | `FUN_1802c7cfc` / `FUN_1802c7d64` | 0xCA–0xE7 → palette index 0–29 |
| +0xA4 / +0xA8 | Spartan / Elite pose (−1 = none, picked by +0x38), u16 into the flags block | `FUN_1802c7bd8` | 0xF2–0x11B |

Not read by Reach: +0x2C..+0x34 (palette indices, other MCC games), +0x58/+0x5C/+0x60 (arms, legs, backpack),
+0x74..+0x88 (Elite pieces), +0xA0 tertiary colour, +0xAC skins. Reach has only primary and secondary colours.

## Palette (engine index = id − 0xCA)
Steel, Silver, White, Brown, Tan, Khaki, Sage, Olive, Drab, Forest, Green, Seafoam, Teal, Aqua, Cyan, Blue, Cobalt,
Ice, Violet, Orchid, Lavender, Maroon, Brick, Rose, Rust, Coral, Peach, Gold, Yellow, Pale. The RGB values come from
the map's globals, not the DLL; our swatches are hand-picked approximations.

## Names
The DLL has no piece names (only the string "leftshoulder"); the display names live in the maps' string tables.
`customize.rs` uses our own English names keyed by MCC id. Reading the real names (and palette RGB) from the
player's map files at runtime is a follow-up.

## Emblems
Not done. With a host the engine never builds emblems itself: slots 72/73/74 are asked for a drawn emblem, an SRV and
three colours by xuid; we return nothing. Needs our own emblem renderer (foreground/background art is game data).

## Ours
- `ui/settings.rs` `Armour` (settings.json `armour`), defaults Mark V [B] / steel.
- `profile.rs` `write_armour` writes the fields above (called from `write_settings` at profile init and on change).
- `ui/customize.rs` the screen; `ui/settings_apply.rs` posts message 5 when `armour` changes in a running game
  (live change in a running match not verified).
- `preview.rs` the engine-rendered 3D preview (below).

## 3D preview (2026-10-15)
MCC has no Reach menu scene (no mainmenu map; MCC's own armoury is its UE frontend), so the preview is the engine
itself in a **hidden child process**: the CUSTOMIZE screen runs `reach-host --mcc <dir> --preview <mapping> --size
720x900`, which rewrites itself into a solo Slayer (`20_sword_slayer`, `slayer_054.bin`, `--hidden`, own log
`reach-host-preview.log`, no XAudio2 preload = silent), puts an orbit camera on its own biped and copies its frames into
shared memory the menu draws as an egui texture. Menu and engine stay separate processes: the menu keeps running at
full speed while the engine loads (~20 s to the first frame; the flat figure shows meanwhile, "LOADING 3D PREVIEW"),
and a crash or missing engine just leaves the flat figure.

- **Script commands work.** Engine slot 9 queues a string; the game thread (`ProcessEngineMessageQueue`, around
  0x18000fdb3) skips `MPBOT: ` lines, runs `HS: <expr>` through the Halo script compiler while a scenario is loaded,
  and sends "Skipping command ..." to host slot 44 otherwise. The function table is at 0x180a28d40 (1893 entries:
  +0 return type, +8 name, +0x20 evaluate, +0x38 u16 parameter count then u16 types; evaluate 0x180086cbc = stripped
  stub). Type ids seen: 4 void, 5 boolean, 6 real, 7 short, 8 long, 11 string_id, 15 cutscene camera point, 71 object,
  72 unit. Used: `(chud_show 0)`, `(player_disable_movement 1)`, `(unit_kill_silent (player_get 0))`.
  `camera_place_*`, `debug_camera_*`, `director_*` are stubs; the HS globals `director_*` have a null address.
  `(unit_lower_weapon (player_get 0) 15)` **crashes** the engine (animation code, haloreach+0x213f45) on an MP biped.
- **Why `camera_set_mode` alone does nothing:** each director (game thread TLS +0x188, 4 × 0x1AC bytes, camera object
  at +8) is updated every tick; the game director (vtable 0x1809d9e88, `UpdateGameDirectorCamera` 0x180134260) picks the
  camera mode itself: mode 3 (first person, camera vtable 0x1809d8440) while `CheckUnitWantsThirdPersonCamera`
  0x1800c6948 says no, so a scripted mode lasts one tick.
- **The fix:** swap director 0's vtable for the observer director's (0x180a2df08, `UpdateObserverDirectorCamera`
  0x1801dc744; the engine's spectator director, which orbits the player's own unit and follows respawns), set
  +0x190 = 0 (index into the mode table 0x180a2df60, entry 0 = mode 1), +0x194 = 1, +0x198 = 0, then
  `SetDirectorCameraMode(director, 1, 0.0 in xmm2, 0 in r9b)` 0x1800c5834 (same call the engine's own director
  restore `RestoreDirectorVtablesAfterLoad` 0x1800c4050 makes). Done from host slot 36 (game thread) 10 ticks after
  player 0 has a unit.
- **Orbiting camera** (mode 1, vtable 0x180a2dff0, update `UpdateOrbitingCamera` 0x1801de658), fields relative to
  the camera object (director +8): +0x08 target object, +0x18 yaw, +0x1C pitch (clamped by globals 0x180b42c90/94),
  +0x20 distance (clamped to the target's size), +0x24 height added to the eye point, +0x28 marker offset scale. The
  observer director copies its +0x198 / +0x19C into +0x24 / +0x28 every tick (they are not yaw/pitch). The camera
  looks at the target's centre from (yaw, pitch, distance); yaw = biped facing (player control +0x94 at install) +
  π faces the model. Field of view = profile +0x14 (40° in the child, re-read on msg 5).
- **Live changes:** armour/colours change on the standing biped through `write_armour` + engine message 5 (verified:
  helmet and both colours changed between captures without a respawn). Spartan ↔ Elite needs a new biped:
  `unit_kill_silent`, Slayer respawns in ~5 s and the observer director re-targets the new unit.

### Child protocol (`preview.rs`)
- stdin lines from the menu: `view <yaw> <pitch> <dist>`, `armour <Armour as JSON>`; end of input = exit.
  (umu-run does not forward stdin, so standalone tests use `REACH_PREVIEW_VIEW="yaw pitch dist"` and the test-script
  steps `set armour helmet|primary|secondary|elite <hex>` and `hs <expr>`; `hs ?dir` logs director 0.)
- Shared memory `Local\ReachPreview<menu pid>` (menu creates, child opens): 64-byte header {magic "RP3D", width,
  height, seq, state, child pid, menu tick} + two RGBA buffers of 1024×1024. The child (host slot 1, render thread)
  copies the back buffer through two staging textures into buffer (seq+1)%2, then bumps seq.
- Pacing: the child's render thread sleeps to ~30 fps while the menu draws the pane (it bumps header word 6 each
  frame), ~4 fps otherwise (4 fps path unverified).
- The child exits when its game thread ends (Slayer time limit); the menu restarts it (at most once a minute), keeping
  the last frame on screen.
- Cost (2026-10-15, GE-Proton, desktop PC): child ~1.55 GB RSS, ~32 % of one core steady at 30 fps (~75-80 %
  unpaced, ~145 % while loading); menu process ~340 MB. Started on first CUSTOMIZE open, kept until the menu
  launches a game or quits.

