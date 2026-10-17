# Host interface slots (haloreach.dll, MCC build 2025.08.16)

The engine calls the host object's vtable (global 0x180c1a100); slots 0–34 mapped from host-slots-a, 35–116 from host-slots-b. The register-tracking rescan output in the b notes found extra slots 16, 25, 29, 34 that are not covered in a; they are listed under "Not yet mapped" below.

Confidence (Conf): **C** confirmed from code (arguments and use of the result); **M** arguments confirmed,
meaning partly inferred; **L** arguments confirmed, meaning a guess; **G** guess. The scan only finds calls
through the host global, so a slot with no listed caller may still be called through a register copy.

| Slot | Off | Our name | Signature | Called from | When | Minimal host | Conf |
|---|---|---|---|---|---|---|---|
| 0 | 0x000 | begin_frame | `void ()` | `BeginRenderFrame` 0x18025103c | start of every rendered frame, after the engine's own pending ResizeBuffers | no-op | C |
| 1 | 0x008 | end_frame | `void (IDXGISwapChain* sc, u32* present_flags)` | `PresentRenderFrame` 0x18025113c | every frame, right before Present | no-op (leave `*flags` = 0). Engine then calls `Present(vsync, *flags)` itself | C |
| 2 | 0x010 | resize | `void ()` | `ProcessEngineMessageQueue`, msg 0x0E | when the host posts msg 0x0E | resize the swap chain buffers if the window changed, else no-op. Engine re-reads the back-buffer size right after | C |
| 3 | 0x018 | set_game_state | `void (u32 state)` | 7+ sites, see notes | state changes | no-op; but treat state 10 as "engine is ending the game" | C (values) / M (meaning) |
| 4 | 0x020 | signal_exit | `void (u32 reason, const char* debug_msg)` | `RunGameThreadBody` 0x18000db08 | once, as the game thread exits | no-op (log it) | C |
| 5 | 0x028 | game_auto_save | `void (const u8* data, u32 len)` | `FlushPendingSavesToHost` 0x18000ce9c, `WriteCoreSaveToHost` 0x180035538 | checkpoint / core save | no-op (saves are lost). To persist: copy `len` bytes | C |
| 6 | 0x030 | set_game_result | `void (void* result)` | `BuildGameResultsForHost` 0x1800f92e8 | end of a game (post-game stats) | no-op | M |
| 7 | 0x038 | pause_game | `void (u32 controller)` | `HandleStartButtonPause` 0x1802b6bc0 | Start pressed in game | no-op means Start does nothing (see notes) | C |
| 9 | 0x048 | set_game_objectives | `void (const wchar_t* primary, const wchar_t* secondary)` | `HandleStartButtonPause` 0x1802b6bc0 | campaign only, right before slot 7 | no-op | C |
| 10 | 0x050 | get_game_event_manager | `i_event_manager* ()` | 20 sites (stats/achievement code) | many, in game | **must** return a valid object (see notes) | C |
| 11 | 0x058 | set_game_variant | `void (obj* variant)` | `ProcessEngineMessageQueue`, msg 0x0A | when host posts msg 0x0A | not called unless we post 0x0A; object (0x15238 bytes, engine heap) is handed over | M |
| 12 | 0x060 | set_map_variant | `void (obj* variant)` | `ProcessEngineMessageQueue`, msg 0x0B | when host posts msg 0x0B: snapshot of the live map variant (Forge save) | ours: serialise via the object's vtable slot 8 and write `.mvar` (forge.rs, [forge.md](forge.md)) | C |
| 13 | 0x068 | save_content_item | `void (void* a, void* b, void* profile_data, u32 d)` | `FUN_18035b1e4` (UI screen) | UI content save (host replaces engine storage) | no-op | L |
| 14 | 0x070 | set_player_look_control | `void (u32 controller, bool inverted)` | `SyncPlayerLookInversionToHost` 0x180065ce0 | look-inversion setting changes | no-op | C |
| 15 | 0x078 | set_player_game_specific | `void (u32 controller, u8 data[0x100])` | `SendPlayerGameSpecificDataToHost` 0x18003a9e4, `…Alt` 0x18003ab1c | player profile settings saved | no-op | M |
| 17 | 0x088 | get_campaign_map_info | `void (const map_id* id, const u8** out_data, u32* out_size)` | `PreloadCustomMapInfo` 0x18000dccc | game-thread startup, campaign, non-built-in map id only | not reached for built-in ids; otherwise must return a valid map-info blob (see notes) | C |
| 18 | 0x090 | get_multiplayer_map_info | `void (const map_id* id, const u8** out_data, u32* out_size)` | `LoadMultiplayerMapInfoFromHost` 0x18003cf88 | startup, MP/Firefight non-built-in map id (and maps listed by slot 17's blob) | as slot 17 | C |
| 23 | 0x0b8 | update_launch_progress | `void (u32 stage, float progress /*xmm2*/)` | launch state machine 0x18000e0xx–0x18000efxx, 0x180250a30 | during launch/loading | no-op | C (args) / M |
| 24 | 0x0c0 | set_player_muted | `void (u64 xuid, bool muted)` | `FUN_180323404` | scoreboard/roster mute toggle | no-op | L |
| 26 | 0x0d0 | get_next_game_options | `bool (launch_options* out, bool flag)` | `LoadNextGameOptionsFromHost` 0x18040bde0 | between MP games | return false | M |
| 27 | 0x0d8 | get_memory_budget? | `i32 ()` | `FUN_180446c34` | resource budget calculation | return 0 (same as no host) | L |
| 28 | 0x0e0 | get_player_voice_state | `u32 (u64 xuid)` | `FUN_1803cf0fc` (8 callers) | roster/UI/voice | return 0 | L |
| 30 | 0x0f0 | update_session_roster | `void (roster* r)` | `PublishSessionRosterToHost` 0x18000d6b8 (21 network callers) | MP session changes, launch state 0x0B | no-op | M |
| 32 | 0x100 | get_video_settings | `bool (video_settings* io)` | `LoadHostVideoSettings` 0x18000d564, msg 5 in `ProcessEngineMessageQueue` | startup and when host posts msg 5 | return false (engine keeps defaults) | C |
| 33 | 0x108 | get_audio_settings | `void (audio_settings* out)` | `CreateAudioMasteringVoice` 0x180175c60 | audio init | no-op (zeroed = default device) | M |
| 35 | 0x118 | profiles_unavailable | `bool ()` | `GetLocalPlayerProfileFromHost` 0x18003accc | UI/player setup, often | `true` → engine skips slots 88/34 and uses defaults (null profile is handled) | L |
| 36 | 0x120 | get_input_state | `bool (int player 0–3, input_state* s)` | `PollInputDevices` 0x1800239e0 | every input tick, 4× | player 0: `true` + filled state (kind 1 = keyboard/mouse or 0 = pad, layout in [input.md](input.md)); 1–3: `false`. If player 0 has no device, game time stays paused (see engine-interface.md). Implemented in `input.rs` | C |
| 39 | 0x138 | set_rumble | `void (int player, const u16 r[4])` | `UpdateControllerRumble` 0x1800240c0 | every input tick, 4× | no-op; ours forwards motors 0/1 to the XInput pad while it is player 0's device | C |
| 41 | 0x148 | net_send | `int (u64 network_id, const u8* buf, u32 len, u32 port)` | `TransportEndpointSendTo` 0x18043ab88 | network | see Networking | C |
| 42 | 0x150 | (send reliable) | — | never called | — | stub | C |
| 43 | 0x158 | net_recv | `int (u8* buf, u32 cap, u64* network_id_out, u32 port)` | `TransportEndpointRecvFrom` 0x18043aa78 | network, polled | return 0 (nothing) | C |
| 44 | 0x160 | command_output | `void (const char* text)` | game-thread command dispatcher (0x18000fe49) | when an `HS:` script command can't run | logged ("engine command output: …") | L |
| 45 | 0x168 | set_skulls | `void (u64 host_skull_bits, float score_multiplier)` | `UpdateActiveSkulls` 0x18010da70 | when active skulls change (not in mode 4) | no-op | C |
| 47 | 0x178 | get_folder_path | `bool (int folder, wchar_t* buf, u32 cap=260)` | `ResolveHostFolderPath` 0x1800abae4 | startup (folder 1 → `preferences.dat`), file IO (folder 2) | return a writable dir; `false` leaves the path empty (CWD) | C |
| 48 | 0x180 | redirect_path | `bool (int kind, char* path_inout, u32 cap=256)` | `BuildMapFilePath`, `BuildBinkMoviePath`, 0x18002eaf4, 0x180231630, 0x1800ac8f4 | map/movie/sound loads | `false` (path unchanged) | C (args), L (kinds) |
| 50 | 0x190 | get_ugc_info | `bool (map_id, char* buf, u32 cap=256)` | `LoadMultiplayerMapInfoFromHost` 0x18003cf88 | custom-map preload | `false` | L |
| 51 | 0x198 | get_setting | `bool (int id, bool* out, u64* out2_or_null)` | 13 sites | UI, fonts, strings, game start | `false` (every caller has a default) | C |
| 52 | 0x1a0 | accept_failed_validation | `bool (int kind)` | 0x18002f228, 0x18002fd0a, 0x18009e569, 0x1801ba56c, 0x18024c723 | only after the engine's own hash/signature check fails | `false` (reject) | C |
| 53 | 0x1a8 | filter_text | `const char* (const char* text)` | 0x1801fa3ba | player-entered text | return the input pointer; **never null** (engine `strlen`s it) | L |
| 55 | 0x1b8 | use_custom_strings | `bool ()` | string-table load 0x1800313de | map load | `false` | C |
| 56 | 0x1c0 | insert_string | `void (int unic_index, int string_id, const char* tag, const char* name)` | same, only if 55 true | map load | not called if 55 = false | L |
| 57 | 0x1c8 | get_string | `bool (int unic_index, int string_id, wchar_t* buf, size_t cap=0x400)` | 0x18009dcb4 … 0x18009e277 (5 sites) | every localized string lookup | `false` (engine text) | C |
| 58 | 0x1d0 | get_subtitle | `bool (const char* sound_tag, const char* prefix(null), int index, wchar_t* buf, size_t cap=0x400)` | 0x18002d3b8, 0x1802cae18, `ShowSoundSubtitleViaHost` | dialogue | `false` | C |
| 61 | 0x1e8 | font_precache_string | `bool (const wchar_t* chars, int point_size, float scale /*xmm3*/, const char* face [stack])` | `PrecacheHostGlyphsForString` 0x1801cf6e4 | every text draw, setting 6 (chars de-duplicated, icons U+E000..E2FF removed) | rasterise the chars, return true | C |
| 62 | 0x1f0 | font_get_glyph | `bool (u16 ch, glyph* out, int point_size, float scale [stack], const char* face [stack])` | `GetHostGlyphMetrics` 0x1801cfa64 | text layout and draw, setting 6 | fill the 26-byte glyph (see Fonts) | C |
| 63 | 0x1f8 | font_get_texture | `ID3D11ShaderResourceView* (int page)` | `DrawQueuedGlyphBatchWithHostTextures` 0x1802683d0, 0x1802e6588 | text draw, per atlas page | 1024x1024 RGBA atlas page | C |
| 64 | 0x200 | font_has_icon | `bool (u16 ch, int point_size, float scale /*xmm3*/, const char* face [stack])` | `PrecacheHostGlyphsForString` | icon glyphs U+E000..E2FF | true once slot 66 delivered it | C |
| 65 | 0x208 | font_get_kerning | `int (u16 ch, u16 next (0 if next >= 0xE000), int point_size, float scale [stack], const char* face [stack])` | `DrawTextRunGlyphs` 0x1801d04a4 | setting 6 | 0 | C (args) |
| 66 | 0x210 | font_add_icon | `bool (u16 ch, int point_size, float scale /*xmm3*/, const char* face, int w, int h, int ascent, const u8* rgba, int pitch=w*4)` (all after scale on stack) | `UploadIconGlyphToHost` 0x1801cf4d0 | first use of each icon glyph | store w x h RGBA8 (decoded from font_package_icon 4444) | C |
| 67 | 0x218 | font_get_line_metrics | `void (int point_size, float scale /*xmm2*/, const char* face, u16* ascent [stack], u16* descent [stack])` | `SetFontScaleFromHostMetrics` 0x1801ce944, `SetFontIdAndHostLineMetrics` 0x1801cecb8 | font setup, setting 6 | both positive, texels | C |
| 72 | 0x240 | draw_player_emblem | `void (u64 xuid, int -1, float x0 /*xmm3*/, float x1, float y0, float y1, float alpha [stack])`, coords 0..1 of the screen | `DrawPlayerEmblemViaHost` 0x1802f65fc | at spawn (MCC: 14 calls); the engine sets up 2D state and the host draws into the bound target | no-op (nothing drawn) | M |
| 73 | 0x248 | get_player_emblem | `ID3D11ShaderResourceView* (u64 xuid)` | `BindPlayerEmblemTextures` 0x180261b00 | emblem widgets (MCC: 321 calls). With a host the engine never builds emblems itself: the SRV is bound to texture slots 4-9 | `null` (no emblem; texture layout MCC uses unknown) | C |
| 74 | 0x250 | get_player_emblem_colors | `void (u64 xuid, u32* c1, u32* c2, u32* c3 [stack])` | same | same | write 0s | C |
| 75 | 0x258 | get_weapon_offset | `bool (int local_player, int weapon_tag_index, const char* weapon_tag, float out[3] [stack], bool [stack]=0)` | 0x1802b38ac | first-person weapon draw | `false` (out is in 1/1000 units when true) | C |
| 88 | 0x2c0 | get_player_xuid | `bool (u64* xuid_out, wchar_t* name_or_null, u32 name_cap, int local_player [stack])` | `GetLocalPlayerXuidFromHost` 0x18003aabc | profile lookups (cached at 0x184e1b290) | `true` + xuid for player 0 only, `false` for 1–3 (each extra signed-in user without input pauses game time) | C |
| 89 | 0x2c8 | show_subtitle | `bool (const float rgba[4], const float rgba2[4], void* (0 or ptr), const wchar_t* text [stack], float seconds [stack], int style [stack])` | 0x1800d5710, `ShowSoundSubtitleViaHost` | dialogue/cinematics | `false` (engine draws) | L |
| 91 | 0x2d8 | (UI reset, 2 args) | `void (0, 0)` | 0x180033bac, 0x180035464, 0x1800d3767 | after 94 at level/state end | no-op | G |
| 92 | 0x2e0 | subtitle_text | `void (const wchar_t* text)` | 0x18002d3e0 (after 58 succeeds) | dialogue | no-op | L |
| 93 | 0x2e8 | (set float) | `void (float v)` — 0.55 or 1.0 | 0x180048040 | map load | no-op | G |
| 94 | 0x2f0 | (UI reset) | `void ()` | 0x180035464, 0x1800d3240, 0x1802c2ef3, 0x180033b9b … | state changes (game end, cinematic start, HUD reset) | no-op | G |
| 95 | 0x2f8 | get_binding_device | `bool (int action, int* device_out)` | `GetHostActionBindingGlyph`, 0x1802d94bc, 0x1802db54c | button-prompt draw | `false` | L |
| 96 | 0x300 | get_action_binding | `bool (int action, u32* a, u32* b, u32* c [stack])` | same | button-prompt draw | `false` | L |
| 97 | 0x308 | transform_hud_color | `u32 (int mode, u32 argb)` | `TransformHudColorViaHost` 0x1802ddf3c (6× per chud draw) | every HUD draw | **return `argb` unchanged**; 0 draws the HUD black | C |
| 98 | 0x310 | mouse_blend_with_pad | `bool ()` | `UpdatePlayerInputActions` 0x1800d1540 | every frame, per local player | **`false` in keyboard/mouse mode** (true makes the engine skip every key and mouse-button binding, only mouse look is read); true while on the pad blends mouse look in (bigger of pad/mouse wins). See [input.md](input.md) | C |
| 99 | 0x318 | (force refresh) | `bool ()` | 0x1802c78a0, 0x1802c8dfc, 0x18031c1f8 | per-player setting change | `false` | G |
| 100 | 0x320 | set_control_context | `void (int local_player, u8 context)` | `UpdateVehicleControlContext` 0x1800652b4, 0x1800d3240 | entering vehicles (ghost etc.), cinematics (ctx 3); context 1 seen at map start on foot | no-op (ours logs it) | C (args) |
| 104 | 0x340 | set_ui_flag | `void (const char* name, bool value, bool, int [stack])` | `UpdateSingleInteractPrompt` (`/enableSingleInteract`), 0x1800d3240 / 0x1801af484 (`/showCinematicSkipButton`) | gameplay prompts, cinematics | no-op | C (args) |
| 106 | 0x350 | begin_event | `void (int 0, const char* name)` | `OpenBinkMovie` 0x1800d8e70 ("bink_setup") | movie open | no-op | L |
| 107 | 0x358 | end_event | `void ()` | `OpenBinkMovie` and 3 more in the Bink player | pairs with 106 | no-op | L |
| 108 | 0x360 | (per-entry query) | `void (const void* entry36, struct* out)` | 0x18047d418 | init, once per entry of a globals list | leave `out` as given | G |
| 112 | 0x380 | (player event) | `void (u64 xuid or 0)` | 0x18032308c | on a specific game event | no-op | G |
| 114 | 0x390 | command_result | `void (u8 status)` | game-thread command dispatcher (0x18000fec2) | after a host command that loads a `.mglo` file | no-op | L |
| 116 | 0x3a0 | get_gamepad_mapping | `const u8* (u64 xuid)` → 66 bytes, one per MCC action, value = abstract pad button 0–15 | `RebuildPlayerInputBindings` 0x1802c8dfc | control setup (5× in the traced match) | `null` keeps the engine's default pad layout; a table is applied over it, bytes ≥ 0x10 are skipped. See [input.md](input.md) | C |

## Not yet mapped

- Slot 16 = **get_next_campaign_level** `bool (this, u32* out_map_id?, u16* out_insertion?, u64* out)`. Two callers:
  `HandleLevelCompleted` 0x1800a3314 (logs "level completed"; false/none → back to the main menu) and a next-mission
  name lookup 0x1800a9e70 (from 0x1800aa090 / 0x1800d359c, about 14 s after every campaign map load: harmless).
  2026-10-17: the dedicated m35 early end is NOT this slot (probes on 0x1800a3314 and game_won never hit).
- Slot 25 (found by the register-tracking rescan, not in host-slots-a)
- Slot 29 (found by the register-tracking rescan, not in host-slots-a)
- Slot 34 (found by the register-tracking rescan, not in host-slots-a; other agent's range)

## Notes (slots 0–34)

**Slot 1.** `sc` is the engine's swap chain global (`0x184e38868`). After the call the engine presents
the host-overridable swap chain at `0x180c1a0e8` if non-null, otherwise `sc`, with sync interval = vsync
byte from slot 32 and flags = whatever the host wrote (e.g. 0x200 allow-tearing).

**Slot 2.** The engine releases its back-buffer views before the call and after it reads the back buffer's
width/height (GetBuffer + texture GetDesc), so the host is expected to call ResizeBuffers here.

**Slot 3 state values seen** (arg in edx):
1 map loaded (`LoadGameLevel`); 3 / 4 when msgs 6 / 7 are processed (pause / resume of simulation);
5 shutdown (`ShutdownGameSystems`); 7 loading ≥ 90 % (`CheckLoadScreenFinished`); 8 / 9 scoreboard
shown / hidden (end of `ProcessEngineMessageQueue`); 10 game over / exit requested (`RequestGameExit`
0x18000d648 and end-of-game in 0x18007d714); 13 level load starting (0x1800a2fc4); 0x10 / 0x11 from
0x18010e008 (toggles a per-game byte, meaning unknown).

**Slot 4.** `RequestGameExit(reason, msg)` records the first reason/message and sends state 10; the game
thread later passes them to slot 4. Seen reasons include 4 with "film file does not exist", 7, 9.

**Slot 5.** Two sources: queued save buffers (data = node+0x18, len = node+0x10), and a 0xA60000-byte
core save at the global `0x1824fb710`. Without a host the engine writes the save itself.

**Slot 6.** Pointer to a 0x5D138-byte stack struct the engine fills from its post-game data (global
`0x1828b83b0`). Layout not mapped.

**Slot 7 / 9: pause.** On Start (UI event 0x0D, button 1 in campaign, 0x0C otherwise) the engine calls
slot 9 (campaign only, two 1024-wchar buffers) and then slot 7 with the controller index, and shows
**nothing itself**. The engine's own pause menu (`FUN_1802b6d3c`) runs only when no host is set. A
standalone host therefore has to provide its own pause UI, or start/stop the sim with msgs 6 / 7.
Slot 7 is skipped while `0x180c1a0e2` is set (msg 0 sets it, msg 1 clears it after a countdown).

**Slot 10: event manager.** Most callers do not null-check. Methods called on the returned object (byte
offsets into its vtable): 0x460, 0x470, 0x478 and 0x480 (float in xmm1/xmm2), 0x488, 0x490 (returns
bool), 0x498, 0x4a8, 0x4b0, plus 0xb0 (record a per-player event with a 16-byte id and a FILETIME). So
the object needs a vtable of at least 0x4b8/8 = 151 entries. Minimal host: every entry is a stub
returning 0, except **+0x498 must return a pointer to 16 readable bytes** (the caller copies 16 bytes
from it; zeros are fine).

**Slot 11 / 12.** Only reached when the host posts msg 0x0A / 0x0B. The object comes from the engine's
`operator new`; the host owns it afterwards (a no-op leaks it).

**Slot 13.** Args: rdx = screen+0x2bb0, r8 = screen+0x2c30 (0x80 apart), r9 = profile data + 0xff8,
5th (stack, 32-bit) = screen+0x11b0. Without a host the engine saves the item itself. Meaning is a guess.

**Slot 15.** The 0x100 bytes are the player's game-specific profile block (profile+0x310) with its first
0x20 bytes replaced by the caller's data.

**Slots 17 / 18: map info.** Map ids are 16 bytes. Built-in ids are skipped: campaign
`{u32 index < 7, u16 1, u16 0x1111, 8 zero bytes}`, MP/Firefight `{…, u16 at +6 = 0x8888, 8 zero bytes}`.
So with stock maps neither slot is called. For other ids the engine reads launch options +0x24 (campaign)
or +0x14 (MP/FF), then **does not check the result**: a null or bad blob crashes (null deref in the
campaign path, uninitialised pointer in the MP path). The blob is validated as a BLF-style file with a
0x111 chunk version, minimum size 0xA20 (campaign) / 0xCBA0 (MP); it looks like a `.mapinfo` file
(scenario path at +0x20, names at +0x130, descriptions at +0x430). The campaign blob holds 64 map ids at
+0x20 and slot 18 is called for each valid one.

**Slot 23 stages.** 1 (progress 0), 2, 4 (progress 0) from the launch state machine; 5 with progress =
global loading fraction clamped to 0..1.

**Slot 26.** `out` is a full launch-options buffer (0x2BF30 bytes, prepared by `FUN_180010f20`): game
variant at +0x300, a 1032-byte string at +0x1CF00, map variant at +0x1D308, a 1064-byte string at
+0x2BB08. On true the engine loads the variants from it. flag = true when `FUN_180451154` returned false.

**Slot 30 roster** (≥ 0x220 bytes): +0x00 u64 = launch options +0x50; +0x08 u64 host machine id;
+0x10 16 × 0x20-byte entries {+0x00 u64 player id, +0x08 u64 machine id, +0x10 i32, +0x14 u8, +0x18 i32
peer index}; +0x210 u32 entry count; +0x214 i32 (default −1); +0x218 i32 (default −1); +0x21c u8.

**Slot 32 video settings** (0x170 bytes; `InitVideoSettingsDefaults` 0x18000d3dc zeroes it and sets defaults
before the call): +0x00 u32 width (1280), +0x04 u32 height (720), +0x08..+0x0F eight bytes of 2, +0x10 u8 1,
+0x11..+0x13 bytes of 1, **+0x14 u8 vsync** (copied to 0x180b43ab8, the Present sync interval), **+0x15 u8 frame
cap**: 0 → 60 fps, 1 → 120, 2 → 180, 3 → 240, 4 or other → uncapped (0x180afc094 fps, 0x180afc090 = 1/fps),
+0x16 u8 0, +0x18..+0xB7 zero, +0xB8 a render-settings block (floats 1.0 and −1s). libmcc names the first 24 bytes
(texture resolution/filtering, lighting, effects, details, post-processing, water, AA, motion blur, blood, vsync,
fps lock, FSR) and +0xB8 (LOD/fade/light/shadow scales). **Correction 2026-10-17:** the result lands in the global
0x1829f5900 (defaults set by 0x1800062c9: +0x08..+0x0F = 2, +0x10..+0x13 = 1), and `ApplyQualityToRenderThrottles`
0x18003aef8 reads +0x08..+0x12 every frame (from 0x180270258, which copies the render throttle row (scenario tag or
built-in table 0x180b43e40, 0x50 bytes per split-screen count) into the live block 0x180ca0240). Each quality byte:
0 low (scales x0.5, water x0.3, lights x0, and low bits set in the live flags), 1 medium (x1), 2 high (x3, lights
x10). Byte map from the code: +0x09 anisotropy (live +0x48), +0x0A cpu/gpu lights (+0x28..+0x3C), +0x0B effect
(+0x0C; 0 sets flag 0x10), +0x0C shadows (+0x40/+0x44; 0 sets 6), +0x0D detail: decorator/instance/object_fade/
+0x18/+0x1C/structure_lod (+0x08..+0x24), +0x0E 0 sets 0x28, +0x0F water (+0x04), +0x11 0 sets 0x400, +0x12 0 sets
0x800. We used to zero the block → Low preset (pixelated far objects as flat imposters, checkerboard fades). Now
Settings > Video > Graphics quality (default High). The hs globals `render_debug_*imposter*` have null storage.

**Engine message 5** (`ProcessEngineMessageQueue` 0x18000f26c): calls `MarkLocalPlayerSettingsDirty`
(0x1802a6a58, sets flag 0x400 on each local player's settings record 0x182a03c58 + i × 0xab8), then re-reads
slot 32 (vsync/frame cap live). The dirty flag makes `FUN_1802c7360` call `ApplyProfileLookSettingsToPlayer`
0x1802eda08, which fetches the profile again (slot 34). Any compared field that changed sets flag 0x800 →
`RebuildPlayerInputBindings` (key codes are not compared, so the host flips +0x1D2 to force a rebuild).
Verified 2026-10-15: FOV 78 → 110 and Jump rebound Space → J both took effect in a running game after msg 5.

**Slot 34 profile fields read by `ApplyProfileLookSettingsToPlayer`** (names from libmcc `s_player_profile`):
+0x00 (→ record +0xa30), +0x1C crosshair location, +0x1D look inverted (pad), +0x1F vibration disabled, +0x20
impulse triggers disabled, +0x21 aircraft inverted, +0x23 auto-centre, +0x24 crouch lock, +0x26 clench protection,
+0x27 female voice, +0x38/+0xA4/+0xA8 model and armour, +0x40..+0x9C customisation (MCC item ids, see
[customization.md](customization.md)), +0x1AC service tag, +0x1B5/
+0x1B6 look sensitivity (< 19), +0x1B7 acceleration (< 10), +0x1B8/+0x1BC dead zones (0..0.2), +0x1C8 button
preset (MCC numbering; 0,2,3,4,7..13 → engine 0..11, others → 0), +0x1C9 stick preset (0..3), +0x1CA lefty (0..2),
+0x1CF..+0x1D2 theater settings, +0x1D7 swap triggers/bumpers, +0x1D8/+0x1D9 modern aim / double-jump jetpack.
FOV +0x14/+0x18 are read elsewhere (FOV verified live); volumes +0xA5C.. see "Volumes" below. Button preset ids (string table at
0x180b4b4f0, MCC numbering): 0 standard, 1 community, 2 boxer, 3 green_thumb, 4 professional, 5 evolved,
7 universal_recon, 8 universal_reclaimer, 9 universal_zoom_and_shoot, 10 universal_bumper_jumper, 11 universal_green_fingers.

**Slot 33 audio settings** (global `0x180b87740`): +0x20E is a wide string passed as the device id to
the audio API's CreateMasteringVoice (48 kHz); empty string → default device. Other fields not mapped. Our no-op
(struct left zeroed) is fine: verified 2026-10-15 the engine then opens the default device (6 ch, 48 kHz) and
plays sound. Silence was not this slot but a missing `xaudio2_9redist.dll` (engine-interface.md). Volume does
not come from this struct: the volume globals' only writer reads the profile (see "Volumes").

**Volumes** (verified 2026-10-15 by reading back the engine's state, not by ear). The engine's effective volumes
are three f32 globals, 0..1: master `0x180afc108`, music `0x180afc10c`, sfx `0x180afc110`.
- Only writer: `RebuildPlayerInputBindings` 0x1802c8dfc (at 0x1802c97ac), for local player index 0 only, copying
  slot-34 profile +0xA5C/+0xA60/+0xA64. That function runs only when the settings sync sees a compared control
  field change (flag 0x800, see engine message 5 above), so **msg 5 after a volume-only change does nothing**.
  It also queues master for the mastering voice: an SList item (free list `0x1827a3d30`, pending `0x1827a3d20`,
  value at item +0x10; only if `0x1827a3d40` != 0), which an audio pump (0x180066678, reached from file/IO wait
  paths) drains with `IXAudio2Voice::SetVolume`. Measured in Firefight with master 0: global 0 at 13 s but the
  mastering voice still at 1.0 until the pump ran (~15–30 s).
- Readers: mastering voice `*(0x180b88730)` gets `SetVolume(master)` when created (0x1800670ce) and when the queue
  drains (0x180066c99). A second queue (`0x1827a3be0`, 0x180066d15) sets the voice to 0 or master by a flag byte at
  item +0x10 (probably a pause/focus mute). Music / sfx multiply each source voice's volume on
  every audio update (0x180178385 music if the sound's flag bit 12 is set, else 0x18017838f sfx), so they act live.
- 0 is a real value everywhere (no "unset" check). Mastering voice vtable: SetVolume +0x60 `(this, f32, u32 opset)`,
  GetVolume +0x68 `(this, f32* out)`.
- Host fix (`audio.rs`): `watch` polls until the mastering voice exists, then `apply_volume` writes the three
  globals and calls SetVolume directly; `settings_apply::apply` calls it again on any audio change. The profile is
  still written so a later rebuild copies the same values. Log line: `audio: volume (<when>): engine master ..,
  mastering voice ..` (GetVolume). Test hooks: `REACH_TEST_VOLUME=<v>` (live change at ~20 s), `REACH_VOLUME_WATCH=1`.

**Slots 24 / 28.** Arg is a u64 from the local-user table (record +0xd0, probably the XUID). Slot 28
returns 0–4: 4 marks the player muted, 2 sets a "talking" style flag, 0/3 also tested. Guess: voice
mute state.

## Notes (slots 35–116)

### Notes on structs and ids

- **input_state (slot 36)**, stride 0x130 per player: +0x00 u32 device kind (in/out; engine pre-fills
  1 if the player last used keyboard/mouse, 0 = gamepad); +0x04 256-byte key-down array indexed by
  Windows VK code (read for player 0 only); +0x104 mouse block; +0x120 pad block (u16 XInput buttons,
  2 unused bytes, +0x124 u8 LT, +0x125 u8 RT, +0x126 s16 LX, LY, RX, RY). Full layout and verification:
  [input.md](input.md).
- **rumble (slot 39)**: four u16: left motor, right motor, left trigger, right trigger.
- **slot 48 kinds**: 0 absolute map path (contains `:\`), 1 `<maps>\<name>.map`, 3 `.fsb` sound bank,
  4 `bink\<name>_<n>.bik`, 5 streamed file with `_x<N>` resolution variants, 6 a data file read into a
  vector. True = host rewrote the path.
- **slot 51 ids**: 0 pause-menu option (feeds menu flag 0x20); 1 campaign player-0 appearance allowed
  (false → 0xFF defaults); 2 checked by the main update; 4 bool + u16 limit (filters a list); 5 strings
  use `\a`-tagged names instead of hex indexes; **6 host renders fonts** (`font_package_icon`, gates
  slots 62/63/65/67). Meanings of 0/2/4 are guesses.
- **slot 52 kinds** 1–4: different file types (1 is the map/cache header check, "failed actual
  validation"). Only reached on a failed check.
- **slot 45**: skull bits are remapped from the engine's order to MCC's (`ConvertSkullMaskToHostBits`);
  the float is the summed score multiplier.
- **slot 51 id 6 must be true.** MCC ships no `maps\fonts\font_package.bin`, only `font_package_icon*.bin`.
  With id 6 false the engine loads the missing package and every string draws empty (HUD numbers, score
  widget, loadout names); weapon and grenade HUD pictures are icon-font glyphs (U+E000..) and vanish too.
  With id 6 true it loads `font_package_icon.bin` (slot 48 kind 5) and uses slots 61-67 (verified 2026-10-15).
- **slot 88 name**: callers pass a name buffer (0x184e1b3b0 + player × 0x40, cap 0x40) when they want the
  gamertag; without a host the engine writes "Weasel %04X". Empty = blank names.
- Player records (stride 0x490) hold the xuid at +0xD0 and packed emblem data at +0xE0; slots 73/74/112
  get that xuid.

## Networking

Verified from `TransportEndpointSendTo` (0x18043ab88), `TransportEndpointRecvFrom` (0x18043aa78) and
their callers `SendLinkPacketOnEndpoint` (0x180423a90), `ReadLinkPacketFromEndpoint` (0x180423cb4),
`PollTransportSocketsIntoQueue` (0x18043a314).

- **Address format.** The engine's transport address is 20 bytes: +0x00 8–16 bytes of address data,
  +0x10 u16 port, +0x12 u16 address type. Type **8** means "host-routed": the first 8 bytes are the
  peer's **network_id (u64)**. Only type-8 addresses go to the host; anything else goes to real
  WinSock (`sendto`/`recvfrom` by ordinal 20/17, `WSAGetLastError` 111).
- **Send (slot 41).** Called only when the transport is up (bytes 0x184e1b4b0 and 0x184e1b4b1 both
  non-zero) and the address type is 8. Args: `network_id` = address bytes 0–7, `buf`, `len`,
  `port` = address +0x10. The engine uses two link endpoints: endpoint 0 sends with **port 1000**,
  endpoint 1 with **port 1001**. Payload is 1–1192 bytes (larger is dropped before the call).
  Endpoint 0 frames as u16 length + data + trailer; endpoint 1 sends raw. Return: bytes sent; −1 on a
  failure the caller treats as "peer unreachable" (marks the peer after repeated failures), −2 would
  block, −3 other error.
- **Receive (slot 43).** If a host exists, receive **always** goes through it (no flag check, WinSock
  never used). Args: `buf`, `cap` (0x1000 for link reads, 0x800 in the socket poll loop), pointer to a
  u64 that the host fills with the sender's network_id, and `port` passed **by value** = the local
  socket's bound port (u16 at socket+8), so the host must hand back only packets for that port. A
  return > 0 is the packet length; the engine then builds a type-8 address {network_id, port = local
  port}. Return ≤ 0 means nothing pending; the poll loop stops on < 1, so it must not block. The poll
  loop drains both sockets every tick into a ring buffer.
- **Reliable send (slot 42)** is never called. The engine does its own reliability on top of slot 41,
  so our transport only needs unreliable datagrams.
- **Peer identity.** A peer is its u64 secure address (launch options +0x58 on that machine). Verified
  in a 2-instance run: the joiner sends to the host's +0x58 value. Session layout and roles: [networking.md](networking.md).
- **Minimal transport for our host:** map network_id → (IP, UDP port) ourselves; slot 41 = `sendto`
  with a small header carrying the engine port (1000/1001) and return `len`; slot 43 = non-blocking
  read from a per-port queue that fills `*network_id_out` and returns the length, or 0 when empty.

## Fonts (slots 51 id 6, 61-67)

`GetFontFaceName` 0x1801ce748 maps font ids to faces (0 Fixedsys, 1/5/8+ Eurostile or TVNordCond, 2-4
TVNordCondBold; pl-PL BlenderProEditWide); `GetFontPointSize` 0x1801ce6e4 gives sizes 9-42. The scale
argument is the font package's resolution factor (font header +0x160, 1 for the x1 package); all metrics are
texels of the scaled font and the engine multiplies by 1/scale.

Glyph record (slot 62, 26 bytes, packed): +0x00 u16 char, +0x04 f32 scale, +0x08 i16 atlas x, +0x0A atlas y,
+0x0C width, +0x0E height, +0x10 bearing y (top above baseline), +0x12 bearing x, +0x14 advance, +0x18 i8
atlas page, +0x19 u8 valid. UVs = (x, y) / 1024 (globals 0x180bb1b48/4c), so pages are 1024x1024.
Glyph top = baseline − bearing y; icons (U+E000..U+F8FF) are placed with the line ascent instead and drawn in
their own colours (tint forced to white), text glyphs are tinted, so text glyphs are white with coverage in
alpha. Icon bitmaps come from the engine through slot 66 after slot 64 says the host lacks them.

Ours (`text.rs`): GDI `GetGlyphOutlineW` (GGO_GRAY8) into RGBA atlas pages, uploaded in slot 63 (render
thread). Halo faces are not installed, so: TVNordCond → Bahnschrift Condensed / Arial Narrow / Liberation
Sans Narrow, Eurostile → Bahnschrift / Liberation Sans, Fixedsys → Consolas / Liberation Mono.
