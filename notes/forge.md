# Forge (2026-10-15)

Forge works under our host with no engine patching: the editor, the monitor, the engine's own PLACE OBJECT menu,
budget and object sync between machines are all the engine's. Frozen DLL, image base `0x180000000`.

## How the engine enters Forge
- Forge is a plain multiplayer launch (options +0x0C = 3) whose **game variant has engine type 1** (sandbox).
  Raw game variant +0x00 = engine type (1 sandbox/Forge, 2 megalo = Slayer etc.); in a `.bin` file the same value
  sits in the `chdr` chunk at file +0x6A. MCC ships one: `game_variants/00_basic_editing_054.bin` ("Basic Editing").
- `reach-host --forge <map>` = `--map <map> --game-variant <mcc>/haloreach/game_variants/00_basic_editing_054.bin`.
  `--forge <file.mvar>` loads a saved map variant to keep building (map from its sidecar). Works with `--listen`,
  `--dedicated` and joiners (`--net-role client ... --forge forge_halo`, or `--join-server`: the listed variant
  is the stock basic-editing file). Any MP map works the same way (only Forge World verified).
- The player spawns as a Spartan; the monitor toggle switches the player's unit (a new unit handle, z rises).

## Controls (engine defaults, no host work needed)
`InitDefaultKeyboardMouseBindings` 0x1800d23e4 fills three tables (mouse buttons +0x000, axes +0x288, keys +0x510;
81 engine actions × 2 slots × u32). Key values are **poll indices** into the VK list at 0x18098fdf0, not VKs.
MCC action → engine action: `MapMccActionToEngineAction` 0x18003c724. Forge defaults decoded from it (MCC numbering
as libmcc `e_game_abstract_button`):

| MCC action | Engine | Default |
|---|---|---|
| 33 switch player mode (monitor ⇄ Spartan) | 0x1D | `~` (VK 0xC0); pad d-pad up |
| 31 create object (place-object menu) | 0x20 | `1` |
| 32 object tools menu | 0x23 | `2` |
| 27 grab / drop object | 0x22 | left mouse; pad A |
| 25 / 26 raise / lower | 0x1E / 0x1F | `R` / `F` |
| 30 delete object | 0x21 | Delete |

The engine's monitor HUD prints its own prompts (`[2] Tools`, `[1] Place Object`, `[6] Toggle rotation axes`,
`[DELETE] Delete Object`). Rebinding goes through the existing profile +0x42C path (settings screen "Forge" group).

## Engine UI screens
The Forge PLACE OBJECT / CHOOSE ITEM menus are engine UI screens and they **do draw** under our host, like the
loadout picker (unlike the scoreboard). Pad d-pad + A navigate them; mouse hover/click should work through the
cursor hooks in input.rs (not tried). No overlay of our own was needed.

## Saving a map variant
- **Engine message 0x0B** (`ProcessEngineMessageQueue` 0x18000f26c): if a game runs in MP mode (TLS +0x48 → +0x10
  == 3) the game thread snapshots the live map variant into a 0xD9BC-byte raw variant, wraps it in a new map-variant
  object (`ConstructMapVariantObjectFromRaw` 0x18006abd0: 0x14CF0 bytes, vtable 0x180994fc8) and calls **host slot
  12** with it. The object is the host's afterwards (we leak it, ~85 KB per save). Works on a client too (it saves
  its synced copy).
- Map-variant vtable slot 8 (0x1800692c4) / slot 9 (0x1800692bc) → shared body 0x1800692cc:
  `bool (this, u8** data, u32* len)` builds a BLF file inside the object (+0xD9C4) and returns 0x7329 bytes.
  Slot 8 first resets the raw variant as a new item (0x18006c754: clears the built-in flag +0x2E9 and re-ids it),
  slot 9 keeps it as is. We use slot 8. The result loads with data-access slot 1 (`--map-variant`).
- Raw variant content header (+0x00..+0x2B0, copied to the file's `chdr` at file +0x40): name UTF-16 at **+0x80**
  (file +0xC0), description at **+0x180** (file +0x1C0), 0x100 bytes each. We write the save name and
  "Made in Forge ..." there before serialising.
- No "save content" host slot is involved: slot 13 (`save_content_item`) is never called; the engine's own
  "Save map" lives in its pause menu, which never runs with a host (slot 7).

## Our side
- `src/reach-host/src/forge.rs`: `--forge` support helpers, slot 12 handler, `request_save(name)` (posts msg 0x0B),
  library in `<exe dir>/reach-data/map_variants/<name>.mvar` + `<name>.json` sidecar `{map, name}` (the map is not
  readable from the mvar without the engine), `is_forge_file`, test-script commands `forge save <name>` and
  `forge count` (object data array at TLS +0x10: +0x20 entry size 0x18, +0x28 `d@t@`, +0x2C capacity 0x800,
  +0x40/+0x44/+0x48 allocation counters, +0x50 entries, u16 salt at entry +0).
- `src/reach-host/src/ui/forge.rs`: FORGE screen (MAPS / SAVED MAPS tabs, art `HREACH_<NAME>_MINI` and
  `HREACHMP_<NAME>LARGE` from the install, START FORGE / HOST FORGE, PLAY / HOST GAME with a game type on a saved
  map, controls list), pause-menu section (name + SAVE MAP), "Saved '<name>'" toast, and the "Map variant" row
  on the Play / Host screens (adds `--map-variant`).
- Browser: `query::MODE_FORGE` (3) when a host runs a Forge game variant; master entries whose game variant is
  `00_basic_editing_054.bin` count as Forge too; FORGE chip counts them.
- Test hooks: `REACH_UI_SHOT=forge,forge-saved,forge-start,forge-host` (`REACH_UI_FORGE=<map>` for the start steps).

## Verified (work/run-forge, prefixes prefix-forge / prefix-forge-b)
- Solo `--forge forge_halo`: `~` → monitor; `1` → PLACE OBJECT; pad down×3 + A → VEHICLES → A → Banshee held
  (budget $30000 → $29900); left mouse dropped it; `forge save Test Forge 1` → slot 12 → 29481-byte
  `Test_Forge_1.mvar` with the name in its header. Relaunch `--map-variant Test_Forge_1.mvar --game-variant
  slayer_054.bin`: Slayer HUD, Banshee standing in the hangar (captures f2-*, f3-*).
- Listen host + joiner (both `--forge forge_halo`): the host placed a Banshee; the joiner's capture before shows
  none, after shows it in the hangar; object-table counters rose on both (0x4b → 0x50).
- Listen host on the saved `.mvar` + joiner on the default variant: the joiner sees the Banshee at once (the
  session syncs the host's map variant).
- Dedicated `--dedicated --forge forge_halo` + joiner: the joiner went to monitor, placed a Banshee and saved
  it locally; server answers the query as "forge".
- Menu: FORGE screen captures, START FORGE launched `--forge forge_halo`, pause overlay shows the save section.

## Not done / unverified
- Other MP maps in Forge; mouse in the engine's Forge menus; the joiner's raise/lower/rotate; pad X/Y meanings
  (monitor HUD says X = object options, Y = delete while holding).
- A host-side "save on the dedicated server" (no UI there; any player can save their synced copy instead).
- Map variants from outside our library without a sidecar: the listing then shows the file name as the map and
  `--join-server` would fail to resolve it.
- Variant objects are never freed (slot 0 of the vtable looks like a destructor; signature unchecked).
