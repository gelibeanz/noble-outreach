# Noble Outreach 0.3.1

A standalone community client for Halo: Reach on PC. It runs the engine from your own Steam copy of Halo: The
Master Chief Collection. This release is about the menus, voting, and moving from one game to the next. 0.3.1 adds the vote screens
and in-game votes on top of 0.3.0.

**You need:** MCC on Steam, build **2025.08.16** (`2025.08.16.178512.1`), with Halo: Reach installed. Linux
also needs [umu-launcher](https://github.com/Open-Wine-Components/umu-launcher). The launcher updates itself from
0.2.0 and 0.3.0.

## Downloads
- `NobleOutreach-Launcher.exe` (Windows, unsigned: SmartScreen > More info > Run anyway) or
  `NobleOutreach-Launcher-linux` (`chmod +x`). The launcher installs and updates the game client.
- `NobleOutreach-Server-v0.3.1.tar.gz`: dedicated server (`reach-host.exe --dedicated`) and a static `reach-master`.
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

## Voting
- **Vote for the next game** on a full lobby page between games: the map, game type and playlist on the left,
  three choices with the map's art, the game type's description and live vote counts, plus None of the above,
  and the players with their teams on the right. In a match the vote shows as a compact card. Keys 1-4, the
  mouse, or the d-pad and A.
- **In-game votes** from the pause menu (*CALL A VOTE*): **Shuffle teams**, **Kick player**, **Skip to next game**
  (playlist servers) and **Restart game** (dedicated servers). Everyone gets a card at the right edge: **F5** yes,
  **F6** no. A vote passes with a majority of the players in the game; the caller counts as yes. 30 seconds per
  vote, one at a time, and a minute between two calls from the same player.
- A kicked player goes back to the menu with a notice and cannot rejoin that server until it restarts.

## Scoreboard
- Hold **Tab** (pad: Back): teams with their scores, then each player's score, kills, deaths, assists and ping.
- Right-click while holding Tab to use the mouse: the menu next to a player starts a vote to kick them or
  copies their name.

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
