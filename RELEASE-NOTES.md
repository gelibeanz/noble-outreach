# Noble Outreach 0.3.0

A standalone community client for Halo: Reach on PC. It runs the engine from your own Steam copy of Halo: The
Master Chief Collection. This release is about the menus and moving from one game to the next.

**You need:** MCC on Steam, build **2025.08.16** (`2025.08.16.178512.1`), with Halo: Reach installed. Linux
also needs [umu-launcher](https://github.com/Open-Wine-Components/umu-launcher). The launcher updates itself from
0.2.0.

## Downloads
- `NobleOutreach-Launcher.exe` (Windows, unsigned: SmartScreen > More info > Run anyway) or
  `NobleOutreach-Launcher-linux` (`chmod +x`). The launcher installs and updates the game client.
- `NobleOutreach-Server-v0.3.0.tar.gz`: dedicated server (`reach-host.exe --dedicated`) and a static `reach-master`.
- `reach-host.exe` and `SHA256SUMS` are what the launcher downloads and checks.

## Main menu
- **Menu music.** The main menu plays Halo: Reach's own menu theme, read from your MCC files. *Settings > Audio >
  Menu music* sets its volume (0 turns it off).
- **Live background.** Your own Spartan stands on Spire behind the menu, in your armour. Still art from your
  install shows for the first 20 seconds or so while it loads. *Settings > Graphics > Live menu background*
  turns it off if your PC is busy (it uses about one CPU core while the menu is open).
- **CUSTOMIZE is ready sooner:** the 3D preview starts with the menu instead of when you open the screen, and
  the camera now turns all the way round your character.

## Game after game in one window
- When a vote picks the next game, when you leave a game, and when a campaign mission rolls on into the next,
  the game stays in its window. A loading card with the map's art shows while the next map loads. Before, the
  game window closed and a new one opened.
- After many games in one session the client starts the next game in a fresh process to free memory, so you
  may see the window reopen then.

## Scoreboard
- Hold **Tab** (pad: Back): teams with their scores, then each player's score, kills, deaths, assists and ping.
- Right-click while holding Tab to use the mouse: the menu next to a player copies their name. Vote to kick
  comes with the in-game votes in the next release.

## Controls
- *Settings > Keyboard & mouse* shows the real key for every action, including Forge and Theater, instead of
  "Default". Forge actions have clearer names (Monitor mode, Create objects, Object properties, ...).

## Fixes
- The game could fail to start its graphics under Wine/Proton on the first try; it now retries.

## Known limits
- **Campaign on a dedicated server can end on its own** soon after the mission starts when two or more players
  are in, and the server then restarts. Host campaign co-op from the menu (listen server) until this is fixed.
- CUSTOMIZE: switching between Spartan and Elite can keep showing the previous character in the 3D preview.
  The change is saved and shows in game.
- The Firefight scoreboard shows 0 for each player. The HUD score is right.
- A listen host behind carrier-grade NAT (CGNAT, common on mobile and some home broadband) cannot be reached
  directly. Let another player host, or use a dedicated server. NAT punching needs a master server with
  rendezvous enabled.
- Works with one exact MCC build only. When Steam updates MCC, wait for a matching release.
- A dedicated server in Docker needs an NVIDIA GPU. AMD and Intel GPUs are untested.

Not affiliated with Microsoft, 343 Industries or Halo Studios. No game files are included. GPL-3.0.
