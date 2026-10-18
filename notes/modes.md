# Game modes on dedicated servers (2026-10-18)

Every stock Reach game variant family was launched on a dedicated server (`scripts/run-dedicated.sh`, 1 hidden
client, ~100-130 s each, silent: `WINEDLLOVERRIDES="winepulse.drv=d;winealsa.drv=d"`). "Works" = the server log
reached `set_game_state` 0x7 -> 0x1 (map loaded) and the client's capture (`REACH_CAPTURE_AT`) showed the in-game
HUD on that mode (objective text bottom right, e.g. "Capture the Flag", "Infection", "Juggernaut", Skeeball course,
Race timer; Territories shows numbered zone markers; Firefight shows wave/score HUD). With one client no
second player exists, so scoring/win rules were not played out. Test harness: copy of `run-dedicated.sh` calls,
one server + one client per run, results in the scratchpad (not kept). Playlists built from the results:
`playlists/objective.json`, `party.json`, `invasion.json`, `firefight.json` (added Score Attack).

## Teams

Team modes need nothing extra: the single client lands on the Red team (scoreboard "RED TEAM" in the
`assault` capture) and the engine assigns later joiners itself; `options.rs::team()` only writes the table entry
for the host's own player (alternating 0/1) and a dedicated server has no table, so the engine balances.
Callvote shuffle (callvote.rs) rebalances red/blue for 2+ players. FFA variants show no team banner.

## Variant results (map = what we tested on; any map of the right type works the same way)

Map ids are mapinfo ids (`maps/info/*.mapinfo` at +0x3C, big endian): 1000 Sword Base, 1020 Countdown, 1035 Boardwalk,
1040 Zealot, 1055 Powerhouse, 1080 Boneyard, 1150 Reflection, 1200 Spire, 1500 Condemned, 1510 Highlands,
2001 Anchor 9, 2002 Breakpoint, 2004 Tempest, 3006 Forge World, 10010 Penance, 10020 Battle Canyon,
10030 Ridgeline, 10050 Breakneck, 10060 High Noon, 10070 Solitary.

| Variant file (`game_variants/`) | Family | Teams | Map used | Works | Evidence |
|---|---|---|---|---|---|
| ctf_054, ctf_1flag, ctf_multiteam, ctf_neutralflag | CTF | team (multiteam: 4) | Zealot / Boardwalk / Sword Base / Zealot | yes | map loaded + HUD "Capture the Flag" (ctf, ctf_multiteam, ctf_neutralflag spawned; ctf_1flag loaded, spawn slower) |
| assault, assault_one_bomb, assault_neutral_bomb | Assault | team | Sword Base / Powerhouse / Zealot | yes | loaded + spawned, red team scoreboard |
| oddball | Oddball | FFA | Sword Base | yes | loaded + spawned |
| koth, koth_crazyking | King of the Hill | team / FFA | Powerhouse / Countdown | yes | loaded + spawned |
| stockpile | Stockpile | team | Zealot | yes | loaded + spawned |
| territories, territories-3plot, territories-landgrab | Territories | team | Sword Base / Powerhouse / Reflection | yes | zone markers on HUD; territories-3plot's server later exited with signal_exit reason 0 (round end) at 100 s, normal end-of-game |
| headhunter, headhunter_pro | Headhunter | FFA | Forge World / Powerhouse | yes | loaded (headhunter spawned later than the capture window) |
| juggernaut | Juggernaut | FFA | Forge World | yes | HUD "Juggernaut" popup |
| infection, infection_safehavens | Infection | team (zombies) | Forge World / Boneyard | yes | loaded + spawned, shotgun/zombie loadout |
| race | Race | FFA | Forge World (plain) | loads, no track | HUD "Race" timer; needs a race map variant (see below) |
| rally | Rally | FFA | Forge World, Reflection | NO | `signal_exit(4, "map and game incompatible")` in 23 s: needs a map variant with race points |
| SWAT | SWAT | FFA | Sword Base | yes | loaded + spawned, no-shield HUD |
| slayer_covy | Slayer | FFA | Zealot | yes | loaded + spawned |
| skeeball_default | Skeeball | FFA | Forge World + `skeeball_court_031.mvar` (hopper) | yes | capture shows the skeeball lanes and golf club |
| 3nvasion, 3nvasion_dlc | Invasion | team (6v6 attackers/defenders) | Breakpoint (`dlc_invasion`) | yes | loaded + spawned |
| 3nvasion_spire | Invasion | team | Spire (`35_island`) | yes | loaded + spawned |
| 3nvasion_boneyard | Invasion | team | Boneyard (`70_boneyard`) | yes | loaded + spawned |
| 2nvasion_slayer | Invasion Slayer | team | Boneyard | yes | loaded + spawned |
| hr_4v4_team_{3plots, assault, crazyKing, headhunter, multiFlag, oddball, oneFlag, slayer, stockpile, territories}_* | MM 4v4 team | team | Powerhouse / Zealot / Countdown / Sword Base | yes | all loaded; stockpile spawned twice (round) |
| hr_4v4_team_oneBomb | MM 4v4 | team | Countdown | loads | map loaded; spawn not seen within 120 s on a loaded machine |
| hr_ffa_escalationSlayer_mixed_tu / _vanilla, hr_ffa_juggernaut_ar_mag_150points_tu | MM FFA | FFA | Forge World | yes | loaded + spawned |
| hopper `tu1_team_gun_game_054` | Gun Game (stock, Title Update) | team | Powerhouse | yes | loaded + spawned (it does exist as a stock matchmaking variant, in `hopper_game_variants`) |
| hopper `grifball_vanilla_054` | Grifball | team | Forge World + `forgeworld_grif_the_vault_031.mvar` | yes | loaded + spawned |
| 00_basic_editing_054 | Forge | n/a | Forge World | server hosts it as the forge game (forge.rs); not a playlist mode | |
| hr_ff_gruntpocalypse_1round | Firefight | co-op | Glacier `ff60_icecave` | yes | loaded + spawned (2 spawns = new round) |
| hr_ff_generatorDefense_1set | Firefight | co-op | Holdout `ff70_holdout` | yes | HUD with generator markers |
| hr_ff_rocketfight_1set | Firefight | co-op | Outpost `ff60_airview` | yes | rocket launcher HUD |
| hr_ff_scoreAttack_1round | Firefight | co-op | Overlook `ff10_prototype` | yes | loaded + spawned |
| hr_ff_standard_1set | Firefight | co-op | all ff maps | yes | already in firefight.json |
| hr_ff_versus_2turns | Firefight Versus | 2 teams | Courtyard | NO (untested properly) | client leaves with `_map_state_none` at 62 s, no map loaded on the server; Versus needs 2 players on opposing teams |
| hr_ff_classic_1set | Firefight | co-op | not tested | ? | |

Firefight must be started with `--firefight` (playlist mode `firefight`), not `--map`; a plain `--map ff..` with an ff game
variant gives `signal_exit(4, "no game selected")`.

## Map variants

The map id is the i32 at **file offset 108** of a `.mvar` (the `chdr` chunk), little endian; it equals the base map's
mapinfo id (raw +0x2B4 inside the `mvar` chunk, as in launch-options.md). Every file in `map_variants` and
`hopper_map_variants` was classified this way:

- `map_variants` (11 files): `hr_forgeWorld_{asylum,hemorrhage,paradiso,pinnacle,theCage}` = Forge World (3006);
  `beaver_creek_cl` 10020 Battle Canyon, `damnation_cl` 10010 Penance, `hang_em_high_cl` 10060 High Noon,
  `headlong_cl` 10050 Breakneck, `prisoner_cl` 10070 Solitary, `timberland_cl` 10030 Ridgeline.
- `hopper_map_variants` (266 files) by base map: Forge World (3006) all `forgeworld_*` (incl. `forgeworld_grif_*`
  Grifball, 30+), `hr_forgeWorld_*` (btb, inv, miniGame, mlg), `coastline_*`, `enclosed_*`, `synapse_*`, `treasury_*`,
  `atom*`, `cliffhanger*`, `affinity*`, `hockey_rink`, `skeeball_court*`, `mlg_android/element/nexus/oasis/redemption/
  retroactive/sanctuary/the_pit/veridian/warlock`; Zealot 1040 (`aftship_*`, `zealot_*`, `hr_zealot_*`, `mlg_zealot_*`);
  Countdown 1020 (`countdown_swat`, `launch_*`, `hr_countdown_*`, `mlg_countdown_*`); Powerhouse 1055 (`settlement_*`,
  `powerhouse_swat`, `hr_powerhouse_v6`); Sword Base 1000 (`sword_base_*`); Boardwalk 1035 (`panoptical_*`);
  Reflection 1150 (`ivory_tower_*`, `reflection_swat`); Boneyard 1080; Spire 1200 (`spire*`, `hr_spire_*`);
  Breakpoint 2002; Tempest 2004 (`tempest_*`, `temptation`, `hr_tempest_*`); Anchor 9 2001 (`anchor9_*`, `unanchored_*`);
  Condemned 1500; Highlands 1510; Penance, Battle Canyon, High Noon, Breakneck, Solitary, Ridgeline (`*_default_031`,
  `mlg_*`, `hr_*`).
- Tested loading (`--map-variant` + a Slayer/other game variant, map loaded and spawned, HUD seen):
  `hr_forgeWorld_asylum.mvar` (+ slayer), `hr_forgeWorld_pinnacle.mvar` (+ CTF), `prisoner_cl_031.mvar` on
  `cex_prisoner`, `hr_forgeWorld_btb_brick.mvar` (hopper), `skeeball_court_031.mvar` (hopper), `forgeworld_grif_the_vault_031.mvar`
  (hopper, Grifball). A hopper file also works from `hopper_map_variants` by file name in a playlist entry.
- The base `--map` must be the variant's map: `hr_forgeWorld_miniGame_run.mvar` + Race on Forge World failed with
  `signal_exit(4, "map and game incompatible")` (the minigame course does not provide Race's checkpoints), and
  `hr_forgeworld_inv_calamity.mvar` (Forge World) with `3nvasion_054` on Breakpoint failed to load. A Forge World mvar
  given with `--map dlc_invasion` and Slayer did load (the engine takes the variant's map), so keep `map` equal to the variant's base map anyway.
- Invasion on Forge World variants (`hr_forgeworld_inv_*`) was not tested with the proper `3nvasion`/`hr_6v6_*` variant on `forge_world`.

## Not tested / open

`hr_ff_versus_2turns`, `hr_ff_classic_1set`, `rally`/`race` with a real race map variant, 2+ player scoring in any
mode, other hopper game variants (hopper has 420; tested families only). The hopper game variants load from
`hopper_game_variants` in the playlist (`playlist.rs` checks both folders).

## Custom modes (Gun Game etc.)

The stock files contain no extra modes beyond the lists above (the stock Title Update Gun Game
`tu1_team_gun_game_054.bin` works). A genuinely new mode needs a Megalo game variant (`.bin`, BLF with `mpvr`): author it with
ReachVariantTool (GPL, can compile Megalo scripts) and drop it in `game_variants` or point a playlist entry at it.
Note only; nothing built here.
