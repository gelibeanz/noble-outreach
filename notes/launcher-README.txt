NOBLE OUTREACH - launcher
=========================

Halo: Reach community client for community servers. It uses YOUR copy of Halo: The Master Chief
Collection from Steam (Reach must be installed). It never touches official matchmaking or EAC.

WINDOWS
  1. Run NobleOutreach-Launcher.exe (if SmartScreen complains: More info > Run anyway).
  2. It finds MCC in your Steam libraries. If not: Browse... and pick the
     "Halo The Master Chief Collection" folder (Steam: right-click MCC > Manage > Browse local files).
  3. Type your player name, press PLAY. The launcher downloads/updates the game client by itself.

LINUX
  1. Install umu-launcher (Fedora/Nobara: sudo dnf install umu-launcher; Arch: sudo pacman -S umu-launcher;
     Ubuntu/Debian/others: the .deb or zipapp from github.com/Open-Wine-Components/umu-launcher/releases).
  2. chmod +x NobleOutreach-Launcher-linux, then run it.
  3. Proton: leave "Automatic" (umu downloads the latest GE-Proton on first start) or pick a GE-Proton
     you already have. The first PLAY sets up a Wine prefix and can take a minute or two.
  4. Name, PLAY, same as Windows.

The launcher updates itself: on start it checks for updates and, if there is a newer launcher,
downloads it and restarts (Windows leaves a short-lived .old copy next to it). "Launcher update failed: ..."
in red means it could not, and the old one keeps working; then download the launcher again.

"Your MCC build is X; Noble Outreach needs build Y"
  Noble Outreach is tied to one exact MCC build. If Steam updated MCC, don't play until there is a
  client for your build (ask your server host, or check the project's releases).

Logs: "Open logs folder" in the launcher (reach-host.log). Attach it when you report a problem.
Where things live: Windows %LOCALAPPDATA%\NobleOutreach, Linux ~/.local/share/NobleOutreach
(client, Wine prefix, launcher.json). A folder from the old name (ReachReclaimer) is moved there on the
first start.
