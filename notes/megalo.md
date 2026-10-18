# Megalo: our own game types (experiment, 2026-10-18)

Question: can we write our own Reach game types (Gun Game-style, like Project Reclaimer's built-in modes) from
Megalo script source and play them on our host? **Yes.** ReachVariantTool compiles our source headlessly under
Wine onto a stock variant, and our dedicated server loads and runs the result. The test showed the script running
(starting weapon); the kill → next weapon step is untested.

## Tool: ReachVariantTool (RVT)
- **v2.3.2** (2026-05-04), by DavidJCobb, GPL-3 (help manual CC0, Qt 5.15 LGPL-3). The repo was renamed:
  github.com/DavidJCobb/ReachVariantEditor (the old `ReachVariantTool` URL is gone). Release asset
  `ReachVariantTool.v2.3.2.zip` (sha256 `1342e4ce…637aa9`), unpacked at `work/tools/rvt/v2.3.2/` in the main
  checkout (git-ignored, never committed). Get it with
  `gh release download v2.3.2 -R DavidJCobb/ReachVariantEditor -p 'ReachVariantTool.v2.3.2.zip'`.
- **It has a headless command line** (since 2.3.x, a console-subsystem exe):
  `ReachVariantTool --headless <in-variant> --recompile <source> [--dst <out-variant>]`. It loads the variant,
  compiles the source, replaces **only the script** (plus new strings it needs), and saves only if compilation
  succeeds. Errors print as `[FATAL]/[ERROR]/[WARN:] Line L Col C: …`. No other operations: name, description,
  settings, traits and loadouts can't be set from the command line (GUI only).
- **Runs under Wine**: plain Proton wine (`GE-Proton11-5/files/bin/wine`) with its own prefix
  (`work/prefix-megalo-rvt/pfx`, made by one umu-run), `WINEDLLOVERRIDES="winepulse.drv=d;winealsa.drv=d"`
  (silent), no window at all in headless mode, about 2 s per compile. umu-run swallows its stdout; plain wine
  prints it. Output is byte-identical between runs.
- Docs: `work/tools/rvt/v2.3.2/help/` (`cli.html`, `script/*.html`, `script/api/`). Language: Lua-like RVT
  dialect, not 343's MegaloEdit language. Points that matter: no expressions (`a += b` only), fixed variable pools
  (`allocate player.number` etc., 8 numbers / 4 objects per player), top-level code runs every tick,
  `on init:` / `on object death:` events, `if … altif … alt … end`, `for each player/object/team`.

## How to build
`scripts/build-modes.sh [modes/x.megalo …]` compiles every `modes/*.megalo` onto its base variant
(`-- base: <file>` line, from `<MCC>/haloreach/game_variants`) into `work/modes/<name>.bin`.
Test run (hidden, silent): `scripts/run-dedicated.sh --game-variant 'Z:\…\work\modes\gun-game.bin'`.

## The test mode: `modes/gun-game.megalo`
Our own code from the RVT docs. Base: stock `slayer_054.bin`. Weapons 1 Magnum, 2 Assault Rifle, 3 Plasma
Repeater, 4 Shotgun, 5 DMR, 6 Energy Sword. Each tick it strips and re-arms a biped that is new or whose player
levelled up (`remove_weapon` both slots, `add_weapon(type, force)`, grenades 0); a kill (`killer_type_is(kill)`)
adds a level to the killer and sets score = level; finishing level 6 ends the round; round timer ends it too.

## Result (dedicated server + 1 hidden client, Sword Base, port 48700)
- Server log: `data access slot 4 (game variant from file …\gun_game.bin, 21289 bytes)`, `game variant copied
  by its slot 7`, the client joined and spawned. The engine accepts the RVT-written variant (RVT rewrites the
  mpvr hash; our host does no signature check).
- Captures (worktree `work/run-megalo/c1/gg-0-spawn.png`, `gg-2-hud.png`, `gg-3-scoreboard.png`, 960x540): the player
  spawns with **only a Magnum** (16 rounds, weapon icon top right, no second weapon, no grenades), where stock
  Slayer gives Assault Rifle + Magnum + grenades. So our script runs on the server and drives the client.
- Not seen: the round card text (it shows before/at spawn, missed by the captures). HUD and scoreboard still
  say **SLAYER**: the name comes from the base variant's metadata, which the command line can't change.
- Untested: a kill (needs two players aiming at each other), so the weapon step, scoring and win are compiled
  but unverified. Weapons dropped on death can be picked up (nothing removes them yet).
- One client start with `--size 640x360` died before its D3D11 device with a Wine GL assertion
  (`gl_glGenTextures: is_wine_reserved_texture`); 320x180 and 960x540 worked. Probably unrelated (VRAM was
  7.2/8 GB with other agents' games running).

## What a "custom modes" feature needs
1. Sources in `modes/*.megalo` (ours), compiled at build/release time by `scripts/build-modes.sh`; ship the
   `.bin` files we made, not RVT. The base variant is MCC's (`slayer_054.bin`), so either compile on the player's
   machine from their own files (needs RVT + Wine on the player side: heavy) or ship the compiled `.bin`. A
   compiled variant still holds Bungie's base settings blob around our script; shipping it is the same grey zone
   as a shared file-share game type, so prefer compiling against the player's own base at first run if we can,
   or write our own minimal base variant later.
2. Name and description: the CLI can't set them. Options: set once in the RVT GUI and commit that as our own base
   variant (a human: open slayer_054.bin, Metadata → Name "Gun Game", description, Save As, then compile onto it),
   or add a `--set-name` operation to RVT upstream (GPL-3, small change in `cli.cpp`/`headless.cpp`), or write
   the string table ourselves (bit-packed, needs the mpvr layout).
3. Catalog: `catalog.rs` reads game types from `game_variants`/`hopper_game_variants`; a third source
   (`<data>/modes/` or a `Noble` category) and the menu category, vote/playlist entries by file name.
4. Settings worth setting in the base: weapon pickup off (base player traits) so dropped guns don't break the
   ladder, loadout palettes none, score to win = weapon count.
5. Licence: RVT is GPL-3 (fine to use as a build tool; if shipped, ship its source offer with it). Our scripts
   are ours (add them under the project's GPL-3). Project Reclaimer / ReclaimerForge files are not ours: never
   ship, copy or derive from them (their catalogue needs an API key; we did not touch it).
6. Note: MCC already ships "TU Escalation Slayer" (`game_variants/hr_ffa_escalationSlayer_mixed_tu.bin`,
   weapon ladder per kill), playable today with `--game-variant` and no new code.
