# Community content (2026-10-18)

Community Reach maps (`.mvar`) and game types (`.bin`) are listed in PLAY / HOST GAME next to the shipped ones.
Code: `catalog.rs` (`Roots`, `scan_with`, `workshop_dirs`), chip and list group in `ui/play_screen.rs`.

| Where | What |
|-------|------|
| `<exe dir>\reach-data\custom\` | made on first menu start with `maps\`, `gametypes\` and a `README.txt`; the whole tree is walked (depth 5), loose files count |
| `<steam library>\steamapps\workshop\content\976730\<item id>\...` | MCC's Workshop subscriptions, walked the same way. Libraries: the one MCC sits in (up from the MCC path to `steamapps`), every `"path"` in that `libraryfolders.vdf`, the default Steam folders (`C:\Program Files (x86)\Steam`, `~/.local/share/Steam`), and `NOBLE_STEAM_LIBRARY` (`;` list of library roots, for tests) |

- Only Reach files are kept: BLF `_blf` + `chdr` (0x2B0 content header) + `mvar` / `mpvr` chunk; game types need engine
  1, 2 or 4 (content +0x2A), map variants a mapinfo id (+0x2C) of a built-in multiplayer map. Everything else
  (junk, Halo 3 / other headers, unknown base map) is skipped with one `catalog: skipped <path> (reason)` log line.
- Byte-identical copies are listed once (a community file whose body equals a shipped / hopper / earlier one is
  dropped; compared from the `chdr` chunk for game types, from the `mvar` chunk for maps, so a renamed copy of a
  shipped file is dropped too).
- Source tags: `Source::Custom` / `Source::Workshop` (map rows show CUSTOM / WORKSHOP in accent colour, game type
  facts line "Community (custom folder / Steam Workshop)"). Game types sort after the matchmaking ones under a
  "COMMUNITY GAME TYPES" header; the COMMUNITY chip (default on) hides them. It does not hide community map
  variants in the map step (they stay under their base map).
- `game_variant_path` / `map_variant_path` find community files by bare file name through the cached catalog
  (votes, playlists).
- Test hook: `REACH_UI_MP="community=0"`; data for `REACH_UI_SHOT` runs: fake Steam library via `NOBLE_STEAM_LIBRARY`.
  Unit test `catalog::tests::community_folders` builds a temp tree (custom + Workshop, junk, damaged-header, unknown
  base map) from work/mcc files.

## Joiners get the host's custom map and game type
Checked 2026-10-18: dedicated `--map forge_halo --map-variant <path only the server has>.mvar --game-variant
<path only the server has>.bin` (a renamed copy of Pinnacle and of `ctf_054.bin`), one hidden client with no such
files and its own launch default (`slayer_054.bin`, no map variant). The client joined, spawned, and its captures
show the CTF HUD (flag marker, "Capture the Flag" score text, flag standing at its base): the session carries the
game type and the map variant to joiners; they need no copy of either file (same as the Forge map sync in forge.md).
