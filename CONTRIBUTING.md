# Contributing

Issues and pull requests are welcome. Two hard rules keep this project shareable:

- **No game files.** Nothing from MCC or any other Halo release: no maps, variants (`.bin`, `.mvar`), videos,
  DLLs, sounds, textures, screenshots of copyrighted UI, or extracted assets. Players bring their own install.
- **No decompiled code.** Don't paste decompiler output or code copied from closed-source projects. Write what
  you learned in your own words in `notes/` (addresses, calling conventions, behaviour) and implement it fresh.

Also: keep to the pinned MCC build (addresses in `notes/` are for it), don't add anything that talks to
official Halo services or anti-cheat, and run `cargo test -p reach-master` plus a Windows build
(`cargo zigbuild --release --target x86_64-pc-windows-gnu -p reach-host`) before opening a PR.
By contributing you agree your work is licensed under GPL-3.0.
