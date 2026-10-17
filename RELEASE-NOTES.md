# Noble Outreach 0.2.0

A standalone community client for Halo: Reach on PC. It runs the engine from your own Steam copy of Halo: The
Master Chief Collection. This release comes from the first long online sessions with players on Windows and Linux.

**You need:** MCC on Steam, build **2025.08.16** (`2025.08.16.178512.1`), with Halo: Reach installed. Linux
also needs [umu-launcher](https://github.com/Open-Wine-Components/umu-launcher). The launcher updates itself from
0.1.0.

## Downloads
- `NobleOutreach-Launcher.exe` (Windows, unsigned: SmartScreen > More info > Run anyway) or
  `NobleOutreach-Launcher-linux` (`chmod +x`). The launcher installs and updates the game client.
- `NobleOutreach-Server-v0.2.0.tar.gz`: dedicated server (`reach-host.exe --dedicated`) and a static `reach-master`.
- `reach-host.exe` and `SHA256SUMS` are what the launcher downloads and checks.

## Graphics
- **Graphics quality is now High.** 0.1.0 ran the engine's Low preset by mistake, with halved detail distances
  and some lighting, shadow and effect features switched off. Distant objects looked flat or blocky, and fades
  showed as checkerboards. *Settings > Video > Graphics quality* picks Low, Medium or High while you play.
- **Sharp picture on Windows with display scaling** (125 %, 150 % ...). The game used to be drawn smaller and
  stretched by Windows.
- *Settings > Video > Menu size* (60-125 %). Menus and panels also fit narrower windows and screens now.

## Playlist voting
- After a vote picks a new game, every player rejoins the restarted server. Before, players who had joined
  from the server browser came back alone in a local copy of the map, and the old game window could stay open.
- *Pause menu > Vote for a new game*: any player on a playlist server can start a vote mid-match. The current
  game is left out of the options, and option 4 is **Keep playing this game**. If nobody votes, the game carries on.
- Once you have voted in a match, the vote panel shrinks to a bar. Backspace hides it for the round.

## Campaign
- A finished mission rolls on into the next one on a listen server, and the players who joined follow.
- Dedicated campaign servers show joined players the player count and the countdown while they gather players.

## Joining and controls
- Esc while joining a game opens the pause menu, so *Leave game* always works.
- If the host does not answer within 20 seconds, the game goes back to the menu and says why, instead of
  showing a black screen.
- In borderless fullscreen the mouse can no longer drift onto another monitor.
- Settings sliders take a typed value (click the number).
- Fixed: Esc in a match flashed the pause menu and closed it again.
- Fixed: the scoreboard and vote panels could cut off their last rows.
- Fixed: the Elite customisation preview no longer shows the character dying when you switch species.

## Known limits
- **Campaign on a dedicated server can end on its own** soon after the mission starts when two or more players
  are in, and the server then restarts. Host campaign co-op from the menu (listen server) until this is fixed.
- The Firefight scoreboard shows 0 for each player. The HUD score is right.
- A listen host behind carrier-grade NAT (CGNAT, common on mobile and some home broadband) cannot be reached
  directly. Let another player host, or use a dedicated server. NAT punching needs a master server with
  rendezvous enabled.
- Works with one exact MCC build only. When Steam updates MCC, wait for a matching release.
- A dedicated server in Docker needs an NVIDIA GPU. AMD and Intel GPUs are untested.

Not affiliated with Microsoft, 343 Industries or Halo Studios. No game files are included. GPL-3.0.
