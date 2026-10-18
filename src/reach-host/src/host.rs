//! The host object handed to `initialize_game`: first field is a pointer to a 192-entry vtable.
//! Every slot starts as a generated stub that logs its first call and returns 0; real implementations
//! go in `host_overrides()`.

use std::ffi::c_void;
use std::ffi::CStr;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use windows::core::Interface;
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_UNKNOWN;
use windows::Win32::Graphics::Dxgi::IDXGISwapChain1;
use windows::Win32::UI::WindowsAndMessaging::GetClientRect;

use crate::log;

core::arch::global_asm!(include_str!(concat!(env!("OUT_DIR"), "/stubs.s")));

pub const HOST_SLOTS: usize = 192; // build.rs generates the same number

extern "C" {
    static HOST_STUBS: [usize; HOST_SLOTS];
    static EVT_STUBS: [usize; HOST_SLOTS];
}

/// Our window (HWND) and swap chain (IDXGISwapChain1*), set by main before the engine starts.
pub static WINDOW: AtomicUsize = AtomicUsize::new(0);
pub static SWAPCHAIN: AtomicUsize = AtomicUsize::new(0);

/// Xuid of our one local player (slot 88); per instance in a networked game (options::xuid).
pub static LOCAL_XUID: AtomicU64 = AtomicU64::new(0x0009_0000_0000_0001);

/// Launch options handed back by slot 26 when a game ends (dedicated server: play the same map/variant again).
pub static NEXT_GAME_OPTIONS: std::sync::Mutex<Option<Vec<u8>>> = std::sync::Mutex::new(None);

/// Dedicated server: no local user is signed in (slot 88) and no local input exists (slot 36).
pub static DEDICATED: AtomicBool = AtomicBool::new(false);

/// Listen server (`--listen`, menu HOST): local player plus late join; slot 26 replays the same game.
pub static LISTEN: AtomicBool = AtomicBool::new(false);

static SLOT_CALLS: [AtomicU64; HOST_SLOTS] = [const { AtomicU64::new(0) }; HOST_SLOTS];
static TOTAL_CALLS: AtomicU64 = AtomicU64::new(0);
static OVERRIDE_CALLS: [AtomicU64; HOST_SLOTS] = [const { AtomicU64::new(0) }; HOST_SLOTS];

#[repr(C)]
pub struct Host {
    pub vtable: *const usize,
}

static mut VTABLE: [usize; HOST_SLOTS] = [0; HOST_SLOTS];
static mut HOST: Host = Host { vtable: std::ptr::null() };

/// Entered from every default stub with the slot index in rcx (arguments of the original call are gone,
/// which is fine: default slots only report and return 0).
#[no_mangle]
unsafe extern "system" fn host_stub_common(slot: usize) -> usize {
    let total = TOTAL_CALLS.fetch_add(1, Ordering::Relaxed) + 1;
    if SLOT_CALLS[slot].fetch_add(1, Ordering::Relaxed) == 0 {
        log!("host slot {slot} called (first call; total host calls {total})");
    }
    0
}

/// Log the first call to an overridden slot; returns the call count.
fn note_override(slot: usize, what: &str) -> u64 {
    let n = OVERRIDE_CALLS[slot].fetch_add(1, Ordering::Relaxed) + 1;
    TOTAL_CALLS.fetch_add(1, Ordering::Relaxed);
    if n == 1 {
        log!("host slot {slot} ({what}) called (first call)");
    }
    n
}

/// Entered from every event-manager stub with the slot index in rcx; logs the first call per slot, returns 0.
#[no_mangle]
unsafe extern "system" fn evt_stub_common(slot: usize) -> usize {
    static SEEN: [AtomicBool; HOST_SLOTS] = [const { AtomicBool::new(false) }; HOST_SLOTS];
    if !SEEN[slot].swap(true, Ordering::Relaxed) {
        log!("event slot {slot} first call");
    }
    0
}

#[repr(C)]
struct EventManager {
    vtable: *const usize,
}

static mut EVT_VTABLE: [usize; HOST_SLOTS] = [0; HOST_SLOTS];
static mut EVT_MANAGER: EventManager = EventManager { vtable: std::ptr::null() };
static EVT_ZEROS: [u8; 16] = [0; 16];

/// Event manager slot 147 (+0x498): caller copies 16 bytes from the returned pointer.
unsafe extern "system" fn evt_slot_147(_this: *mut c_void) -> *const u8 {
    static SEEN: AtomicBool = AtomicBool::new(false);
    if !SEEN.swap(true, Ordering::Relaxed) {
        log!("event slot 147 (+0x498, returns 16 zero bytes) first call");
    }
    EVT_ZEROS.as_ptr()
}

/// True the first time `id` is seen for this mask (ids >= 64 always count as new).
fn first_id(mask: &AtomicU64, id: i32) -> bool {
    id as u32 >= 64 || mask.fetch_or(1 << id, Ordering::Relaxed) & (1 << id) == 0
}

// Slot 0 / 1: frame begin / end. Argument meaning to be refined; raw values are logged on first call.
unsafe extern "system" fn slot_begin_frame(this: *mut c_void) -> usize {
    note_override(0, &format!("begin_frame this={this:?}"));
    0
}

unsafe extern "system" fn slot_end_frame(this: *mut c_void, swapchain: *mut c_void, flags: *mut u32) -> usize {
    let n = note_override(
        1,
        &format!("end_frame this={this:?} swapchain={swapchain:?} flags={flags:?}"),
    );
    // Every 10 s: frame rate and a few engine globals; for the first minute also a copy of the back buffer
    // (frame-NNNs.bmp next to the exe), so a run can be checked without looking at the window.
    static FIRST: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    static LAST_BEAT: AtomicU64 = AtomicU64::new(0);
    static LAST_N: AtomicU64 = AtomicU64::new(0);
    crate::ui::overlay::end_frame(swapchain); // pause overlay over the finished frame (before captures)
    crate::preview::end_frame(swapchain); // preview child: publish the frame to the menu
    let secs = FIRST.get_or_init(std::time::Instant::now).elapsed().as_secs();
    let beat = secs / 10;
    if beat > LAST_BEAT.load(Ordering::Relaxed) {
        LAST_BEAT.store(beat, Ordering::Relaxed);
        let frames = n - LAST_N.swap(n, Ordering::Relaxed);
        log!("heartbeat {secs}s after first frame: {n} frames, {:.1} fps", frames as f64 / 10.0);
        crate::crash::dump_heartbeat();
        if beat <= 6 {
            let path = exe_dir().join(format!("frame-{secs:03}s.bmp"));
            match capture_back_buffer(swapchain, &path) {
                Ok((w, h)) => log!("back buffer {w}x{h} saved to {}", path.display()),
                Err(e) => log!("back buffer capture failed: {e}"),
            }
        }
    }
    if let Some(name) = crate::input::take_capture_request().or_else(|| crate::text::timed_capture(secs)) {
        let path = exe_dir().join(name);
        match capture_back_buffer(swapchain, &path) {
            Ok((w, h)) => log!("back buffer {w}x{h} saved to {}", path.display()),
            Err(e) => log!("back buffer capture failed: {e}"),
        }
    }
    0
}

fn exe_dir() -> std::path::PathBuf {
    std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())).unwrap_or_default()
}

/// Copy the current back buffer to a 24-bit BMP. Runs on the engine's render thread right before Present,
/// so using the immediate context here does not race the engine.
pub unsafe fn capture_back_buffer(sc: *mut c_void, path: &std::path::Path) -> windows::core::Result<(u32, u32)> {
    use windows::Win32::Graphics::Direct3D11::*;
    use windows::Win32::Graphics::Dxgi::IDXGISwapChain;
    let Some(sc) = IDXGISwapChain::from_raw_borrowed(&sc) else { return Err(windows::core::Error::empty()) };
    let tex: ID3D11Texture2D = sc.GetBuffer(0)?;
    let mut desc = D3D11_TEXTURE2D_DESC::default();
    tex.GetDesc(&mut desc);
    let dev = tex.GetDevice()?;
    let ctx = dev.GetImmediateContext()?;
    let sdesc = D3D11_TEXTURE2D_DESC {
        Usage: D3D11_USAGE_STAGING,
        BindFlags: 0,
        CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
        MiscFlags: 0,
        ..desc
    };
    let mut staging = None;
    dev.CreateTexture2D(&sdesc, None, Some(&mut staging))?;
    let staging = staging.unwrap();
    ctx.CopyResource(&staging, &tex);
    let mut m = D3D11_MAPPED_SUBRESOURCE::default();
    ctx.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut m))?;
    let (w, h) = (desc.Width as usize, desc.Height as usize);
    let bgra = desc.Format.0 == 87 || desc.Format.0 == 91; // B8G8R8A8_UNORM(_SRGB)
    let row = (w * 3 + 3) & !3;
    let mut bmp = Vec::with_capacity(54 + row * h);
    let size = (54 + row * h) as u32;
    bmp.extend_from_slice(b"BM");
    bmp.extend_from_slice(&size.to_le_bytes());
    bmp.extend_from_slice(&[0; 4]);
    bmp.extend_from_slice(&54u32.to_le_bytes());
    bmp.extend_from_slice(&40u32.to_le_bytes());
    bmp.extend_from_slice(&(w as i32).to_le_bytes());
    bmp.extend_from_slice(&(h as i32).to_le_bytes());
    bmp.extend_from_slice(&1u16.to_le_bytes());
    bmp.extend_from_slice(&24u16.to_le_bytes());
    bmp.extend_from_slice(&[0; 24]);
    for y in (0..h).rev() {
        let src = (m.pData as *const u8).add(y * m.RowPitch as usize);
        for x in 0..w {
            let p = src.add(x * 4);
            let (r, g, b) = (*p, *p.add(1), *p.add(2));
            if bgra { bmp.extend_from_slice(&[r, g, b]) } else { bmp.extend_from_slice(&[b, g, r]) }
        }
        bmp.resize(bmp.len() + (row - w * 3), 0);
    }
    ctx.Unmap(&staging, 0);
    std::fs::write(path, &bmp).map_err(|_| windows::core::Error::empty())?;
    Ok((desc.Width, desc.Height))
}

// Slot 44: text from the script command dispatcher (a `HS:` command that could not run). Logged (preview.rs).
unsafe extern "system" fn slot_command_output(_this: *mut c_void, text: *const i8) {
    if !text.is_null() {
        log!("engine command output: {}", CStr::from_ptr(text).to_string_lossy());
    }
}

// Slot 2: the engine released its back-buffer views and re-reads the size after we return.
unsafe extern "system" fn slot_resize(_this: *mut c_void) {
    note_override(2, "resize");
    let sc = SWAPCHAIN.load(Ordering::Relaxed) as *mut c_void;
    if let Some(sc) = IDXGISwapChain1::from_raw_borrowed(&sc) {
        let (w, h) = render_size(&crate::ui::settings::get().graphics);
        if let Err(e) = sc.ResizeBuffers(0, w, h, DXGI_FORMAT_UNKNOWN, Default::default()) {
            log!("ResizeBuffers failed: {e}");
        }
    }
}

fn state_name(state: u32) -> &'static str {
    match state {
        1 => "map loaded",
        3 => "sim paused",
        4 => "sim resumed",
        5 => "shutdown",
        7 => "loading >= 90%",
        8 => "scoreboard shown",
        9 => "scoreboard hidden",
        10 => "game over / exit requested",
        13 => "level load starting",
        0x10 | 0x11 => "per-game byte toggle",
        _ => "unknown",
    }
}

// Slot 3 / 4: progress markers, every call logged.
unsafe extern "system" fn slot_set_game_state(_this: *mut c_void, state: u32) {
    note_override(3, "set_game_state");
    log!("host slot 3 set_game_state({state:#x}) = {}", state_name(state));
    if state == 10 {
        crate::mission::on_game_over();
    }
    let base = crate::net::ENGINE_BASE.load(Ordering::Relaxed);
    if state == 10 && base != 0 {
        // RequestGameExit records the first reason/message at 0x180c1a110 / 0x180c1a108 before calling us.
        let reason = ((base + 0xc1a110) as *const i32).read_volatile();
        let msg = ((base + 0xc1a108) as *const *const i8).read_volatile();
        let msg = if msg.is_null() { "<null>".into() } else { CStr::from_ptr(msg).to_string_lossy() };
        log!("  exit reason {reason}, \"{msg}\"; {}", crate::net::session_state());
        let mut frames = [std::ptr::null_mut::<c_void>(); 16];
        let n = windows::Win32::System::Diagnostics::Debug::RtlCaptureStackBackTrace(0, &mut frames, None) as usize;
        let trace: Vec<String> = frames[1..n].iter().map(|f| crate::crash::describe(*f as usize)).collect();
        log!("  called from {}", trace.join(" <- "));
    }
}

unsafe extern "system" fn slot_signal_exit(_this: *mut c_void, reason: u32, msg: *const u8) {
    note_override(4, "signal_exit");
    let msg = if msg.is_null() { "<null>".into() } else { CStr::from_ptr(msg as _).to_string_lossy() };
    log!("host slot 4 signal_exit(reason {reason}, \"{msg}\")");
}

unsafe extern "system" fn slot_get_event_manager(_this: *mut c_void) -> *mut EventManager {
    note_override(10, "get_event_manager");
    std::ptr::addr_of_mut!(EVT_MANAGER)
}

// Slot 32: 0x170-byte video settings, read at start-up and on engine message 5 (notes/host-slots.md). Width/height:
// our swap-chain size. +0x08..+0x0F are quality levels (0 low, 1 medium, 2 high: detail/LOD distances, lights,
// shadows, effects, water, anisotropy) and +0x10..+0x13 feature switches (1 = on), applied every frame by
// 0x18003aef8; all zero = the Low preset with features off (the engine's own defaults are 2 and 1).
unsafe extern "system" fn slot_get_video_settings(_this: *mut c_void, io: *mut [u8; 0x170]) -> bool {
    note_override(32, "get_video_settings");
    let g = crate::ui::settings::get().graphics;
    let (w, h) = render_size(&g);
    let io = &mut *io;
    io.fill(0);
    io[0..4].copy_from_slice(&w.to_le_bytes());
    io[4..8].copy_from_slice(&h.to_le_bytes());
    io[0x08..0x10].fill(g.quality.min(2));
    io[0x10..0x14].fill(1);
    io[0x14] = g.vsync as u8;
    io[0x15] = g.fps_cap.min(4);
    log!("video settings: {w}x{h}, quality {}, vsync {}, frame cap code {}", g.quality.min(2), g.vsync as u8, g.fps_cap);
    true
}

/// Swap-chain size: the window's client area times the rendering-resolution percentage.
pub fn render_size(g: &crate::ui::settings::Graphics) -> (u32, u32) {
    let (mut w, mut h) = (1280u32, 720u32);
    let mut r = RECT::default();
    if unsafe { GetClientRect(HWND(WINDOW.load(Ordering::Relaxed) as _), &mut r) }.is_ok() && r.right > 0 && r.bottom > 0 {
        (w, h) = (r.right as u32, r.bottom as u32);
    }
    let pct = g.render_scale.clamp(25, 200);
    ((w * pct / 100).max(64), (h * pct / 100).max(64))
}

// Slot 33: audio settings; left zeroed = default output device (verified with sound).
unsafe extern "system" fn slot_get_audio_settings(_this: *mut c_void, _out: *mut c_void) -> bool {
    note_override(33, "get_audio_settings");
    false
}

unsafe extern "system" fn slot_profiles_unavailable(_this: *mut c_void) -> bool {
    note_override(35, "profiles_unavailable");
    true
}

// Slot 41 / 43: peer transport (net.rs). Without --net-role nothing is reachable.
unsafe extern "system" fn slot_net_send(_this: *mut c_void, id: u64, buf: *const u8, len: u32, port: u32) -> i32 {
    note_override(41, &format!("net_send id={id:#x} len={len} port={port}"));
    if buf.is_null() {
        return -3;
    }
    crate::net::send(id, std::slice::from_raw_parts(buf, len as usize), port)
}

unsafe extern "system" fn slot_net_recv(_this: *mut c_void, buf: *mut u8, cap: u32, id: *mut u64, port: u32) -> i32 {
    note_override(43, &format!("net_recv cap={cap} port={port}"));
    if buf.is_null() || id.is_null() {
        return 0;
    }
    crate::net::recv(std::slice::from_raw_parts_mut(buf, cap as usize), &mut *id, port)
}

// Slot 30: session roster (>= 0x220 bytes): +0x00 game instance id, +0x08 host machine id, +0x10 16 x 0x20
// {u64 player id, u64 machine id, i32, u8, i32 peer index}, +0x210 u32 count. Logged when it changes.
unsafe extern "system" fn slot_update_session_roster(_this: *mut c_void, r: *const u8) {
    note_override(30, "update_session_roster");
    if r.is_null() {
        return;
    }
    let rd64 = |o: usize| (r.add(o) as *const u64).read_unaligned();
    let count = (r.add(0x210) as *const u32).read_unaligned();
    let prev = crate::net::ROSTER_COUNT.swap(count as u64 + 1, Ordering::Relaxed);
    if prev != count as u64 + 1 {
        let mut line = String::new();
        for i in 0..(count.min(16) as usize) {
            let e = 0x10 + 0x20 * i;
            line += &format!(" [{i}: player {:#x} machine {:#x} peer {}]", rd64(e), rd64(e + 8), (r.add(e + 0x18) as *const i32).read_unaligned());
        }
        log!("host slot 30 roster: instance {:#x} host {:#x}, {count} players{line}", rd64(0), rd64(8));
    }
}

// Slot 47: writable dir per folder id, `<exe dir>\reach-data\folder<id>\` (trailing backslash).
unsafe extern "system" fn slot_get_folder_path(_this: *mut c_void, folder: i32, buf: *mut u16, cap: u32) -> bool {
    note_override(47, "get_folder_path");
    let mut dir = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())).unwrap_or_default();
    dir.push("reach-data");
    dir.push(format!("folder{folder}"));
    let _ = std::fs::create_dir_all(&dir);
    let wide: Vec<u16> = format!("{}\\", dir.display()).encode_utf16().collect();
    if wide.len() + 1 > cap as usize {
        return false;
    }
    std::ptr::copy_nonoverlapping(wide.as_ptr(), buf, wide.len());
    *buf.add(wide.len()) = 0;
    static SEEN: AtomicU64 = AtomicU64::new(0);
    if first_id(&SEEN, folder) {
        log!("host slot 47 folder {folder} -> {}", String::from_utf16_lossy(&wide));
    }
    true
}

// Slot 48: false = path unchanged.
unsafe extern "system" fn slot_redirect_path(_this: *mut c_void, kind: i32, path: *mut u8, _cap: u32) -> bool {
    note_override(48, "redirect_path");
    static SEEN: AtomicU64 = AtomicU64::new(0);
    if first_id(&SEEN, kind) {
        log!("host slot 48 redirect kind {kind}: {}", CStr::from_ptr(path as _).to_string_lossy());
    }
    false
}

unsafe extern "system" fn slot_filter_text(_this: *mut c_void, text: *const u8) -> *const u8 {
    note_override(53, "filter_text");
    text
}

// Slot 88: one local player.
unsafe extern "system" fn slot_get_player_xuid(_this: *mut c_void, xuid: *mut u64, name: *mut u16, name_cap: u32, player: i32) -> bool {
    note_override(88, &format!("get_player_xuid player={player}"));
    static SEEN: AtomicU64 = AtomicU64::new(0);
    let dedicated = DEDICATED.load(Ordering::Relaxed);
    if first_id(&SEEN, player) {
        log!("host slot 88 player {player} -> {}", if player == 0 && !dedicated { "signed in" } else { "none" });
    }
    if player != 0 || dedicated {
        return false;
    }
    *xuid = LOCAL_XUID.load(Ordering::Relaxed);
    crate::profile::write_player_name(name, name_cap, *xuid);
    true
}

// Slot 36: input for local player 0..3 (0x130-byte input_state, notes/input.md). Player 0 always reports a
// device (keyboard/mouse or pad): without one the engine holds game time paused (notes/engine-interface.md).
// Players 1..3 return false and stay signed out.
unsafe extern "system" fn slot_get_input_state(_this: *mut c_void, player: i32, s: *mut u8) -> bool {
    note_override(36, "get_input_state");
    if player != 0 || DEDICATED.load(Ordering::Relaxed) {
        return false;
    }
    crate::input::fill_input_state(s);
    crate::ui::scoreboard::game_tick(); // game thread: remember its state block for the Tab scoreboard
    crate::ui::scoreboard::maybe_dump();
    crate::preview::game_tick(); // preview child: orbit camera on the player's biped
    true
}

// Slot 39: rumble for a local player, four u16 (left motor, right motor, left/right trigger).
unsafe extern "system" fn slot_set_rumble(_this: *mut c_void, player: i32, r: *const u16) {
    note_override(39, "set_rumble");
    crate::input::set_rumble(player, r);
}

// Slot 100: input context for a local player (vehicles, cinematics = 3). Logged only.
unsafe extern "system" fn slot_set_control_context(_this: *mut c_void, player: i32, ctx: u8) {
    note_override(100, "set_control_context");
    log!("host slot 100 set_control_context(player {player}, context {ctx})");
}

// Slot 116: per-xuid gamepad remap (66 MCC actions -> abstract pad button). Null keeps the engine's default
// layout; a table would only be applied over it (entries >= 0x10 are ignored).
unsafe extern "system" fn slot_get_gamepad_mapping(_this: *mut c_void, xuid: u64) -> *const u8 {
    note_override(116, &format!("get_gamepad_mapping xuid={xuid:#x} -> null (engine default layout)"));
    std::ptr::null()
}

// Slot 97: HUD colour transform. The chud draw (`SetupChudDrawShaderConstants` 0x1802ddd8c) passes each of
// its six colours as packed ARGB (plus a mode int from 0x180d1f71c) and uses whatever comes back; MCC applies
// its colour-blind filter here. Returning 0 draws the whole HUD black and transparent: hand the colour back.
unsafe extern "system" fn slot_transform_hud_color(_this: *mut c_void, mode: i32, argb: u32) -> u32 {
    note_override(97, &format!("transform_hud_color mode={mode} argb={argb:#010x}"));
    argb
}

// Slot 98: true = blend mouse look into pad input; false is required for keyboard/mouse (see input.rs).
unsafe extern "system" fn slot_mouse_input_active(_this: *mut c_void) -> bool {
    note_override(98, "mouse_input_active");
    crate::input::mouse_blend_with_pad()
}

/// (slot, function pointer) pairs applied over the default stubs.
/// Player profile handed out by slot 34 (layout per libmcc `s_player_profile`, 0xACC bytes; the engine
/// reads fields up to +0x1d9 when it syncs a local player's settings).
const PROFILE_SIZE: usize = 0xACC;
static mut PROFILE: [u8; PROFILE_SIZE] = [0; PROFILE_SIZE];

unsafe fn init_profile() {
    let p = std::ptr::addr_of_mut!(PROFILE) as *mut u8;
    let put = |off: usize, v: i32| (p.add(off) as *mut i32).write_unaligned(v);
    put(0x14, 78); // field of view
    put(0x18, 78); // vehicle field of view
    put(0xa4, -1); // armour/model selections: -1 = none
    put(0xa8, -1);
    // master / music / sfx volume (floats 0..1), read when local player 0's settings are applied.
    for off in [0xa5c, 0xa60, 0xa64] {
        (p.add(off) as *mut f32).write_unaligned(1.0);
    }
    crate::profile::write_look_defaults(p);
    crate::profile::write_settings(p, &crate::ui::settings::get()); // FOV, volumes, controls, key bindings
}

/// The slot 34 profile (settings are written into it live, then engine message 5 makes the engine re-read it).
pub fn profile_ptr() -> *mut u8 {
    std::ptr::addr_of_mut!(PROFILE) as *mut u8
}

// Slot 34: profile for a local player's xuid. Must not be null.
unsafe extern "system" fn slot_get_player_profile(_this: *mut c_void, xuid: u64) -> *mut u8 {
    note_override(34, &format!("get_player_profile xuid={xuid:#x}"));
    std::ptr::addr_of_mut!(PROFILE) as *mut u8
}

// Slot 26: the post-game life-cycle state asks for the next game's launch options (0x2BF30 bytes; the engine reads
// the variants at +0x300 / +0x1D308 and their signature blocks). False = no next game: the session falls back to
// the pre-game state, whose exit loads the main menu, which a hosted engine turns into an exit.
unsafe extern "system" fn slot_get_next_game_options(_this: *mut c_void, out: *mut u8, flag: bool) -> bool {
    let n = note_override(26, "get_next_game_options");
    crate::vote::game_ended(); // playlist server: a game ended, open the vote for the next one
    let guard = NEXT_GAME_OPTIONS.lock().unwrap_or_else(|e| e.into_inner());
    let Some(opts) = guard.as_ref() else {
        log!("host slot 26 get_next_game_options(flag {flag}) -> false");
        return false;
    };
    std::ptr::copy_nonoverlapping(opts.as_ptr(), out, opts.len());
    log!("host slot 26 get_next_game_options(flag {flag}) -> same map and game variant again (game {})", n + 1);
    true
}

// Slot 118: per-player boolean the profile sync asks for alongside slot 34 (meaning unknown).
unsafe extern "system" fn slot_118(_this: *mut c_void, arg: u64) -> bool {
    note_override(118, &format!("unknown bool query arg={arg:#x}"));
    false
}

pub fn host_overrides() -> Vec<(usize, usize)> {
    vec![
        (0, slot_begin_frame as *const () as usize),
        (1, slot_end_frame as *const () as usize),
        (7, crate::ui::overlay::slot_pause_game as *const () as usize),
        (2, slot_resize as *const () as usize),
        (3, slot_set_game_state as *const () as usize),
        (4, slot_signal_exit as *const () as usize),
        (10, slot_get_event_manager as *const () as usize),
        (32, slot_get_video_settings as *const () as usize),
        (33, slot_get_audio_settings as *const () as usize),
        (34, slot_get_player_profile as *const () as usize),
        (35, slot_profiles_unavailable as *const () as usize),
        (41, slot_net_send as *const () as usize),
        (43, slot_net_recv as *const () as usize),
        (30, slot_update_session_roster as *const () as usize),
        (44, slot_command_output as *const () as usize),
        (47, slot_get_folder_path as *const () as usize),
        (48, slot_redirect_path as *const () as usize),
        (53, slot_filter_text as *const () as usize),
        (36, slot_get_input_state as *const () as usize),
        (88, slot_get_player_xuid as *const () as usize),
        (97, slot_transform_hud_color as *const () as usize),
        (98, slot_mouse_input_active as *const () as usize),
        (39, slot_set_rumble as *const () as usize),
        (100, slot_set_control_context as *const () as usize),
        (116, slot_get_gamepad_mapping as *const () as usize),
        (118, slot_118 as *const () as usize),
        (26, slot_get_next_game_options as *const () as usize),
        (5, crate::checkpoint::slot_auto_save as *const () as usize),
    ]
    .into_iter()
    .chain(crate::text::overrides()) // 51 settings, 61-67 fonts
    .chain(crate::forge::overrides()) // 12 map variant (Forge save)
    .collect()
}

/// Build the vtable (stubs + overrides) and return the host object pointer.
pub unsafe fn build_host() -> *mut Host {
    init_profile();
    let vt = std::ptr::addr_of_mut!(VTABLE) as *mut usize;
    for i in 0..HOST_SLOTS {
        *vt.add(i) = HOST_STUBS[i];
    }
    for (slot, f) in host_overrides() {
        *vt.add(slot) = f;
    }
    let evt = std::ptr::addr_of_mut!(EVT_VTABLE) as *mut usize;
    for i in 0..HOST_SLOTS {
        *evt.add(i) = EVT_STUBS[i];
    }
    *evt.add(147) = evt_slot_147 as *const () as usize;
    (*std::ptr::addr_of_mut!(EVT_MANAGER)).vtable = evt;
    let h = std::ptr::addr_of_mut!(HOST);
    (*h).vtable = vt;
    h
}
