# Launch options: building a multiplayer custom game ourselves

How `reach-host --map <name|id> [--game-variant <.bin>] [--map-variant <.mvar>]` builds the 0x2BF30-byte
block that `initialize_game` copies to the global at `0x1824971a0`. Frozen DLL, image base `0x180000000`.
Checked against an MCC capture `work/traces/2026-10-15-slayer/game-options-0.bin` (Forge World Slayer).

## Data-access object (`CreateDataAccess`, vtable `0x180994e60`)

All calls are MS x64 with `this` in rcx.

| Slot | Address | Our name | Signature / result |
|---|---|---|---|
| 0 | `0x180069e94` | DestroyDataAccess | `void (this)`; frees the object and a cached helper global |
| 1 | `0x180069f14` | CreateMapVariantFromFile | `map_variant* (this, const u8* file, u32 len)`. Takes a whole `.mvar` (BLF `_blf`/`chdr`/`mvar`/`_eof`). Null if it does not parse. A `chdr` (version 10, sub-version 2) is used when the file is longer than 0x7328 |
| 2 | `0x18006a0d4` | CreateMapVariantFromMapId | `map_variant* (this, const map_id* id)`, 16-byte map id. Builds the map's default (empty) variant |
| 3 | `0x18006a168` | CreateMapVariantFromRawVariant | `map_variant* (this, const void* raw)`: wraps a 0xD9BC-byte raw variant; null in → null |
| 4 | `0x18006a548` | CreateGameVariantFromFile | `game_variant* (this, const u8* file, u32 len)`. Takes a `.bin` game variant (BLF with `chdr`, `mpvr`); needs len > 0x5328 for the BLF path. Null on failure |
| 5 | `0x18006a6d8` | CreateGameVariantFromRawVariant | `game_variant* (this, const void* raw)`: wraps a 0xFC04-byte raw variant |
| 6 | `0x18006a1d4` | LoadMapInfoForMapVariant | `bool (this, ?, map_variant*, host*)`: loads the `.mapinfo` for the variant's map (built-in id → `maps\info\<name>.mapinfo`, other ids → host slot 18) into a cache at `0x184e3ac70`, then validates the variant. Not needed to build options |
| 7 | `0x18006a730` | ReadFilmHeaderFromBuffer | `(this, buf, u32 len)` film header parse (len ≥ 0x1F8A8); always returns 0 |

Slot 1/4 first run the input through a helper (`0x1803c5ca0`, uses data-access +0x08/+0x10) that may
substitute a decoded buffer.

## Variant objects

- **Map variant** object: 0x14CF0 bytes, vtable `0x180994fc8` (15 slots). Raw variant (0xD9BC bytes) at
  object +0x08. Slot 7 `0x180069258` CopyMapVariantToGameOptions(obj, options): copies the raw variant to
  **options +0x1D308**, nothing else. Slot 13 `0x180036944` GetMapVariantMapId returns obj +0x2C0 (raw +0x2B8,
  the 16-byte map id). Slot 14 `0x180069478` SetMapVariantMapId. Slots 10/11 `0x180069160` (same function)
  read raw +0x2E9/+0x2EA/+0x2E0 flags (libmcc's "is built-in").
- **Game variant** object: 0x15238 bytes, vtable `0x180995048`, built by `ConstructGameVariantObject`
  (`0x180067b20`). Raw variant (0xFC04 bytes) at object +0x08. Slot 7 `0x180067f34`
  CopyGameVariantToGameOptions(obj, options): copies the raw variant to **options +0x300**, nothing else.
  Slot 8 `0x180068e78` serialises back to a BLF file (0x5329 bytes).
- So copy_to_game_options writes only the variant body; every other field is ours to fill.
- Raw map variant: +0x2B4 i32 mapinfo id (e.g. 3006), +0x2B8 16-byte map id, +0x2E9 byte "built-in".
  `InitializeMapVariantForMapId` (`0x18006c244`) fills those from a map id.
- Raw game variant: +0x00 u32 engine type (2 for the Slayer files).

## Map ids

- Built-in MP/Firefight map id (16 bytes): `{i32 map index, u16 0, u16 0x8888, 8 zero bytes}`.
- `.mapinfo` id (u32 big-endian at file +0x3C, in the `levl` chunk) ↔ map index: two switch functions,
  `0x180038c30` (mapinfo id → index, −1 if unknown, pure, safe to call before the game starts) and
  `0x180038970` (index → mapinfo id). Examples: Forge World `forge_halo` 3006 → 0xD1, Boneyard 1080 →
  0xBF, Sword Base 1000 → 0xC4, Firefight Beachhead `ff10_prototype` 7000 → 0xD8. Indices run 0xB2–0xDB
  (campaign m05..m70_bonus are 0xB2–0xBD, MP 0xBE–0xD1, Firefight 0xD2–0xDB). m40 (5040) has no index.
- reach-host resolves `--map` by reading `maps/info/<name>.mapinfo` and calling `0x180038c30`; a number
  < 1000 is taken as an index, ≥ 1000 as a mapinfo id; `forge_world` is an alias of `forge_halo`.

## Launch-options fields (MP custom game)

Values in brackets are what MCC sent in the capture; "ours" is what reach-host writes.

| Offset | Size | Meaning | Capture | Ours |
|---|---|---|---|---|
| +0x00 | u8 | flags, bit 3 = multiplayer (engine checks `& 8`, `& 0x48`) | 0x08 | same |
| +0x01 | u8 | more flags; bit 6 (0x40) makes the MP setup apply +0x0B and +0x03 | 0x03 | same |
| +0x02 | u8 | unknown | 0x18 | same |
| +0x03 | u8 | 1..16 passed to the session when +0x01 bit 6 is set (max players?) | 0x10 | same |
| +0x04 | u8 | 4 → team lookup from the player table is skipped | 0x03 | same |
| +0x0C | i32 | game mode (3 = MP → launch state 6) | 3 | same |
| +0x10 | i32 | map index ("legacy map id") | 0xD1 | index |
| +0x14 | 16 | map id | `{0xD1,0,0x8888}` | same form |
| +0x34 | i32 | difficulty | 1 | same |
| +0x44 | u16 | unknown | 0xFFFF | same |
| +0x50 | u64 | game instance id; copied to `0x180cfb5e8` and into the slot 30 roster | random | time-based |
| +0x58 | u64 | this machine's own secure address / network id (→ `0x180d1ade8`; not the session host's, see networking.md) | MCC machine id | `0x5245414348300001` |
| +0x60 | 17×u64 | peer addresses; [0] = host | host | same as host |
| +0xE8 | i32 | player count; < 2 → skip "wait for peers" (state 8) and go straight to state 10 | 1 | 1 |
| +0xF0 | 16×0x20 | players: {u64 xuid, u64 peer address, i32 team, i32, i32, i32} | p0 = {xuid, host, 0, 1, 0, −1}; empty = {0, 0, −1, 0, −1, −1} | p0 xuid 0x0009000000000001 (= host slot 88) |
| +0x2F0 | i32 | entries used by the team-assignment loop (`0x1803915a8` matches xuid → team) | 1 | 1 |
| +0x2F8 | u64 | joiner only: secure address of the host to join (launch state 9) | 0 | see [networking.md](networking.md) |
| +0x300 | 0xFC04 | game variant (libmcc reserves 0x1CC00) | | slot 7 copy |
| +0x1CF00 | 0x408 | u32 1 + a `_fsm` BLF chunk (hash + signature of the game variant file) | present | zero |
| +0x1D308 | 0xD9BC | map variant (libmcc reserves 0xE800) | | slot 7 copy |
| +0x2BB08 | 0x408 | same as +0x1CF00 for the map variant | present | zero |
| +0x2BF10 | u32 | saved-game size (0xA60000 = core save in memory; campaign-firefight.md "Saves") | 0 | 0 |
| +0x2BF18 | ptr | saved-game state (non-zero → campaign resume path, launch state 5) | 0 | 0 |
| +0x2BF20 | ptr | film path (non-zero → theater path) | 0 | 0 |

The engine reads the two 0x408 blocks only as an optional pointer (passed when their first byte is non-zero),
so zero means "no signature"; the build still loads with them zero.

## Who reads it

- `TickExternalLaunchStateMachine` `0x18000eea4`, state global `0x180c1a114`. State 2
  `LaunchStateSelectGameModePath` `0x18000de8c` picks the path from +0x0C and the film/saved-game pointers:
  MP → 6, campaign → 4/5, film → 3.
- State 6 `LaunchStateSetupMultiplayerSession` `0x18000e6fc`: parses the game variant at +0x300, sets the
  session UI mode, then `SetSessionMapVariant` (`0x1802a4598`, options +0x1D308) and `SetSessionGameVariant`
  (`0x1802a4c60`), then state 8 or 10 by +0xE8.
- State 10 `LaunchStateWaitForSessionStart` `0x18000ed68`: if the session reports status 7, the error code
  goes through `GetSessionStartErrorString` (`0x1803a2dcc`, 73 strings) to `signal_exit(code − 3, text)`.
  "no map selected" is code 0x11: the session had no valid map variant (a zeroed block gives exactly this).
  Other codes worth knowing: 0x12 no game selected, 0x13 map load failure, 0x48 map and game incompatible.

## Result (2026-10-15)

- `--map forge_world --game-variant game_variants/slayer_054.bin`, `--map 70_boneyard` (default Slayer)
  and `--map-variant map_variants/hr_forgeWorld_paradiso.mvar --game-variant game_variants/slayer_pro_054.bin`
  all reach `set_game_state` 0 → 7 (loading ≥ 90 %) → 1 (map loaded) in about 9 s, no exception, no
  signal_exit. The captured MCC block behaves the same in our host.
- After "map loaded" the window stays on the engine's loading bar (screenshot at 40 s); getting from the
  loading screen into gameplay is a host-side problem, not an options problem.
- Diff against the capture (`built-options.bin` vs `game-options-0.bin`): header and player table identical
  except the instance id, host/peer address and xuid; the variant bodies differ (different variants: the
  capture's map variant is a saved Forge World variant, ours the default one) and the two `_fsm` blocks are
  zero in ours.

## Unverified

- Meaning of +0x01..+0x04, +0x34, +0x44, player fields +0x14/+0x1C; we copy MCC's values.
- Whether an unsigned variant (zero `_fsm` blocks) matters for networked games.
- Variant objects are never freed (slot 0 of the variant vtables looks like a destructor, not checked).
