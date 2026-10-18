# Noble Outreach 0.4.3

A standalone community client for Halo: Reach on PC. It runs the engine from your own Steam copy of Halo: The
Master Chief Collection. 0.4.3 lets the dedicated server run **without a GPU** (see *Dedicated server* below).
The rest of these notes cover 0.4.x as a whole: new PLAY and HOST screens with every game type and map variant in
your files, campaign progress and checkpoints, a carnage report after each game, co-op for up to 8 players, and
community game types and maps.

**You need:** MCC on Steam, build **2025.08.16** (`2025.08.16.178512.1`), with Halo: Reach installed. Linux
also needs [umu-launcher](https://github.com/Open-Wine-Components/umu-launcher). The launcher updates itself from
0.2.0, 0.3.x and 0.4.x.

## Downloads
- `NobleOutreach-Launcher.exe` (Windows, unsigned: SmartScreen > More info > Run anyway) or
  `NobleOutreach-Launcher-linux` (`chmod +x`). The launcher installs and updates the game client.
- `NobleOutreach-Server-v0.4.3.tar.gz`: dedicated server (`reach-host.exe --dedicated`) and a static `reach-master`.
- `reach-host.exe` and `SHA256SUMS` are what the launcher downloads and checks.

## Dedicated server (0.4.3)
- **No GPU needed.** The Docker server now runs on software Vulkan (Mesa's lavapipe) and draws nothing
  (`--no-render`), so it runs on any x86-64 Linux box or cloud server: about a third of one CPU core and about
  2 GB of RAM with players in. The container's memory limit is now 4 GB (`REACH_MEM_LIMIT`).
- **NVIDIA is optional:** set `COMPOSE_FILE=compose.yml:compose.nvidia.yml` and `NVIDIA_VERSION` in `.env` to
  render on an NVIDIA GPU as before (less RAM: the textures sit on the GPU). Existing NVIDIA setups need those two
  lines, or they switch to the software path on the next build.

## PLAY and HOST
- **Every game type in your files.** PLAY and HOST list all of Reach's game types (custom games, 2010
  matchmaking, Title Update, Firefight Arcade and MCC's own), sorted into Slayer, Objective, Infection, Party,
  Invasion, Firefight and Forge, with search and an FFA / Teams filter.
- **One row per mode.** Reach ships many modes in several editions. Each mode is one row ("3 VERSIONS"); the
  chips in the detail panel pick the edition (MCC, Arcade, Original matchmaking, Title Update).
- **Map variants.** Pick a map, then any of its variants, including the Forge World remixes.
- **Community content.** Game types and maps you put in `reach-data\custom\` next to the game, and MCC
  Steam Workshop items you are subscribed to, show up with a COMMUNITY tag.
- **Join without the server's files.** Joining a server that runs a game type or map variant you do not have
  now works: the server sends it to you when you join. Before, the join was refused.

## Campaign
- **CONTINUE** picks up your next mission, and the campaign screen shows which missions you finished on each
  difficulty.
- **Checkpoints (solo).** The last checkpoint of each mission is kept; playing that mission solo again can
  resume from it.
- **Up to 8 players** in campaign and Firefight co-op (Reach's own limit is 4). Hosts choose the player count on
  the co-op screens; dedicated servers use `--coop-max N`.

## After a game
- **Carnage report:** kills, deaths, assists, medals and score for every player when a game ends. On a server
  with a playlist vote it is the first tab of the vote page.

## Voting
- A running call vote (shuffle, kick, skip, restart) can now be answered from the Esc menu with the mouse, as
  well as with F5 / F6. In game the card's YES / NO show the keys only, since the mouse belongs to the game.

## Fixes
- **Campaign on a dedicated server** no longer ends on its own after the first cutscene.
- **Mouse in the menus:** after leaving a game, and on the call-vote options, clicks could stop working. Each
  screen that needs the mouse now holds it on its own, so one closing no longer takes it from another.

## Known limits
- CUSTOMIZE: switching between Spartan and Elite can keep showing the previous character in the 3D preview.
  The change is saved and shows in game.
- The Firefight scoreboard shows 0 for each player. The HUD score is right.
- A listen host behind carrier-grade NAT (CGNAT, common on mobile and some home broadband) cannot be reached
  directly. Let another player host, or use a dedicated server. NAT punching needs a master server with
  rendezvous enabled.
- More than 4 players in campaign or Firefight is new: it is checked on our own test runs, not yet in long games.
- Works with one exact MCC build only. When Steam updates MCC, wait for a matching release.
- The Docker server's NVIDIA option (`compose.nvidia.yml`) was not rebuilt for this release; the default, GPU-free
  setup is the tested one.

Not affiliated with Microsoft, 343 Industries or Halo Studios. No game files are included. GPL-3.0.
