//! Player input for local player 0: Win32 raw keyboard + mouse and an XInput pad, fed to the engine through
//! host slot 36 (get_input_state). Layouts and engine behaviour: notes/input.md.
//!
//! Real devices and the `--test-input <script>` mode go through the same entry points (`key`, `mouse_move`,
//! `mouse_button`, `wheel`, `pad`), so a script proves the same path a player uses.

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Instant;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{ClientToScreen, ScreenToClient};
use windows::Win32::UI::Input::KeyboardAndMouse::GetFocus;
use windows::Win32::UI::Input::XboxController::{XInputGetState, XInputSetState, XINPUT_STATE, XINPUT_VIBRATION};
use windows::Win32::UI::Input::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::log;

// ---- Bindings -------------------------------------------------------------------------------------------

/// MCC "abstract button" indices (the order of the profile's 66 key-mapping entries, profile +0x42C).
#[allow(dead_code)]
mod act {
    pub const JUMP: usize = 0;
    pub const SWITCH_GRENADE: usize = 1;
    pub const ACTION: usize = 2; // interact / pick up (engine "action", reload is separate)
    pub const RELOAD: usize = 3;
    pub const SWITCH_WEAPON: usize = 4;
    pub const MELEE: usize = 5;
    pub const FLASHLIGHT: usize = 6;
    pub const THROW_GRENADE: usize = 7;
    pub const FIRE: usize = 8;
    pub const CROUCH: usize = 9;
    pub const ZOOM: usize = 10;
    pub const MOVE_FORWARD: usize = 16;
    pub const MOVE_BACK: usize = 17;
    pub const STRAFE_LEFT: usize = 18;
    pub const STRAFE_RIGHT: usize = 19;
    pub const SCOREBOARD: usize = 20;
    pub const ARMOR_ABILITY: usize = 23; // MCC "equipment"; Reach sprint is an armor ability
    pub const COUNT: usize = 66;
}

/// Binding codes as MCC writes them into the profile: Win32 VK codes, plus two wheel pseudo-codes.
const WHEEL_UP: u32 = 0xAF;
const WHEEL_DOWN: u32 = 0xAE;
const VK_LBUTTON: u32 = 0x01;
const VK_RBUTTON: u32 = 0x02;
/// Any code >= 0x100 leaves the engine's built-in PC default for that action (InitDefaultKeyboardMouseBindings).
const ENGINE_DEFAULT: u32 = 0xFFFF_FFFF;

/// Keyboard/mouse bindings for player 0: (MCC action, primary, secondary). The engine reads two codes per
/// action; 0 = unbound. Actions not listed keep the engine's built-in PC default (which matches MCC's Reach
/// defaults, see notes/input.md). Esc (pause) is an engine action outside MCC's list and always stays on Esc.
const KBM_BINDINGS: &[(usize, u32, u32)] = &[
    (act::MOVE_FORWARD, b'W' as u32, 0),
    (act::MOVE_BACK, b'S' as u32, 0),
    (act::STRAFE_LEFT, b'A' as u32, 0),
    (act::STRAFE_RIGHT, b'D' as u32, 0),
    (act::JUMP, 0x20, 0),             // Space
    (act::CROUCH, 0xA2, 0),           // Left Ctrl
    (act::FIRE, VK_LBUTTON, 0),
    (act::ZOOM, VK_RBUTTON, 0),
    (act::RELOAD, b'R' as u32, 0),
    (act::ACTION, b'E' as u32, 0),
    (act::MELEE, b'Q' as u32, 0),
    (act::THROW_GRENADE, b'F' as u32, 0),
    (act::SWITCH_GRENADE, b'G' as u32, 0),
    (act::SWITCH_WEAPON, WHEEL_UP, WHEEL_DOWN),
    (act::ARMOR_ABILITY, 0xA0, 0),    // Left Shift
    (act::SCOREBOARD, 0x09, 0),       // Tab
];

/// Writes the key/mouse mapping block (66 entries of {u32 action, i32 codes[5]} at profile +0x42C) that
/// `RebuildPlayerInputBindings` (0x1802c8dfc) turns into the engine's binding tables for player 0.
pub unsafe fn write_profile_bindings(profile: *mut u8, m: &crate::ui::settings::Mouse) {
    for i in 0..act::COUNT {
        let e = profile.add(0x42C + i * 0x18) as *mut u32;
        e.write_unaligned(i as u32);
        let [a, b] = m.keys(i as u32);
        e.add(1).write_unaligned(a);
        e.add(2).write_unaligned(b);
    }
}

/// Our default codes for an MCC action (both slots); `ENGINE_DEFAULT` = the engine's built-in PC binding.
pub fn default_keys(action: usize) -> [u32; 2] {
    KBM_BINDINGS.iter().find(|b| b.0 == action).map_or([ENGINE_DEFAULT; 2], |b| [b.1, b.2])
}

pub fn is_engine_default(code: u32) -> bool {
    code >= 0x100
}

/// Reach's built-in PC layout (`InitDefaultKeyboardMouseBindings` 0x1800d23e4) per MCC action, in our code space
/// (VK / mouse 1,2,4,5,6 / WHEEL_*). The engine only fills slot 0, so slot 1 is unbound. Decoded from the three
/// tables via `MapMccActionToEngineAction`; the full list with engine action numbers is in notes/input.md
/// ("Reach PC defaults"). 0x10 is the generic Shift (poll index 105), not Left/Right Shift. Actions not listed
/// have no default binding. Used by the UI only; untouched slots are still sent as `ENGINE_DEFAULT`.
const ENGINE_DEFAULT_KEYS: &[(usize, u32)] = &[
    (act::JUMP, 0x20),
    (act::SWITCH_GRENADE, b'G' as u32),
    (act::ACTION, b'E' as u32),
    (act::RELOAD, b'R' as u32),
    (act::SWITCH_WEAPON, b'C' as u32),
    (act::MELEE, b'Q' as u32),
    (act::FLASHLIGHT, b'4' as u32),
    (act::THROW_GRENADE, b'F' as u32),
    (act::FIRE, VK_LBUTTON),
    (act::CROUCH, 0xA2),
    (act::ZOOM, VK_RBUTTON),
    (11, b'Z' as u32),
    (12, b'X' as u32),
    (act::MOVE_FORWARD, b'W' as u32),
    (act::MOVE_BACK, b'S' as u32),
    (act::STRAFE_LEFT, b'A' as u32),
    (act::STRAFE_RIGHT, b'D' as u32),
    (act::SCOREBOARD, 0x09),
    (21, 0xDB), // vehicle trick 1: [
    (22, 0xDD), // vehicle trick 2: ]
    (act::ARMOR_ABILITY, 0xA0),
    (24, VK_RBUTTON),
    (25, b'R' as u32),  // Forge: raise
    (26, b'F' as u32),  // Forge: lower
    (27, VK_LBUTTON),   // Forge: grab / drop
    (28, 0x10),         // Forge: move faster (Shift)
    (29, 0xA3),         // Forge: precision (Right Ctrl)
    (30, 0x2E),         // Forge: delete
    (31, b'1' as u32),  // Forge: create objects
    (32, b'2' as u32),  // Forge: object properties
    (33, 0xC0),         // Forge: monitor mode (`)
    (34, VK_RBUTTON),   // Forge: rotate held object
    (35, 0xA2),         // Forge: lock object (Left Ctrl)
    (36, b'X' as u32),  // Theater: toggle panel
    (37, b'Z' as u32),  // Theater: toggle interface
    (38, b'C' as u32),  // Theater: change view
    (39, VK_RBUTTON),
    (40, 0x27),         // Theater: fast forward (Right)
    (41, 0x25),         // Theater: rewind (Left)
    (42, 0x6C),         // Theater: pause / play (Num Enter)
    (43, 0xA4),
    (44, 0x20),         // Theater: free camera (Space)
    (45, 0x10),
    (47, b'R' as u32),  // Theater: camera up
    (48, b'F' as u32),  // Theater: camera down
    (50, VK_RBUTTON),
    (51, b'B' as u32),
    (52, b'G' as u32),
    (54, b'X' as u32),
    (58, b'L' as u32),
    (61, b'P' as u32),
    (62, 0xDB),
    (63, 0xDD),
];

/// What a binding row really holds: slots still at `ENGINE_DEFAULT` resolved to the engine's real codes
/// (slot 0 = Reach's default key, slot 1 = unbound), so the UI can show names and detect clashes.
pub fn effective_keys(action: usize, raw: [u32; 2]) -> [u32; 2] {
    let def = ENGINE_DEFAULT_KEYS.iter().find(|d| d.0 == action).map_or(0, |d| d.1);
    [if is_engine_default(raw[0]) { def } else { raw[0] }, if is_engine_default(raw[1]) { 0 } else { raw[1] }]
}

/// The 0x68 virtual keys the engine polls (table at 0x18098fdf0); any other key cannot be bound.
const POLLED_VKS: &[u8] = &[
    0x1B, 0x70, 0x71, 0x72, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7A, 0x7B, 0x7C, 0x91, 0x7E, 0xC0, 0x31, 0x32,
    0x33, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x30, 0xBD, 0xBB, 0x08, 0x09, 0x51, 0x57, 0x45, 0x52, 0x54, 0x59, 0x55,
    0x49, 0x4F, 0x50, 0xDB, 0xDD, 0xDC, 0x14, 0x41, 0x53, 0x44, 0x46, 0x47, 0x48, 0x4A, 0x4B, 0x4C, 0xBA, 0xDE, 0x0D,
    0xA0, 0x5A, 0x58, 0x43, 0x56, 0x42, 0x4E, 0x4D, 0xBC, 0xBE, 0xBF, 0xA1, 0xA2, 0x5B, 0xA4, 0x20, 0xA5, 0x5C, 0x5D,
    0xA3, 0x26, 0x28, 0x25, 0x27, 0x2D, 0x24, 0x21, 0x2E, 0x23, 0x22, 0x90, 0x6F, 0x6A, 0x60, 0x61, 0x62, 0x63, 0x64,
    0x65, 0x66, 0x67, 0x68, 0x69, 0x6D, 0x6B, 0x6C, 0x6E,
];

/// A code the engine can use as a binding (Esc is its own pause key and is not offered).
pub fn bindable(code: u32) -> bool {
    matches!(code, 1 | 2 | 4 | 5 | 6 | WHEEL_UP | WHEEL_DOWN) || (code != 0x1B && code < 0x100 && POLLED_VKS.contains(&(code as u8)))
}

pub fn key_name(code: u32) -> String {
    let s = match code {
        0 => "—",
        1 => "Left mouse",
        2 => "Right mouse",
        4 => "Middle mouse",
        5 => "Mouse 4",
        6 => "Mouse 5",
        WHEEL_UP => "Wheel up",
        WHEEL_DOWN => "Wheel down",
        0x08 => "Backspace",
        0x09 => "Tab",
        0x0D => "Enter",
        0x10 => "Shift",
        0x14 => "Caps Lock",
        0x1B => "Esc",
        0x20 => "Space",
        0x21 => "Page Up",
        0x22 => "Page Down",
        0x23 => "End",
        0x24 => "Home",
        0x25 => "Left",
        0x26 => "Up",
        0x27 => "Right",
        0x28 => "Down",
        0x2D => "Insert",
        0x2E => "Delete",
        0x5B => "Left Win",
        0x5C => "Right Win",
        0x5D => "Menu",
        0x6A => "Num *",
        0x6B => "Num +",
        0x6C => "Num Enter",
        0x6D => "Num -",
        0x6E => "Num .",
        0x6F => "Num /",
        0x90 => "Num Lock",
        0x91 => "Scroll Lock",
        0xA0 => "Left Shift",
        0xA1 => "Right Shift",
        0xA2 => "Left Ctrl",
        0xA3 => "Right Ctrl",
        0xA4 => "Left Alt",
        0xA5 => "Right Alt",
        0xBA => ";",
        0xBB => "=",
        0xBC => ",",
        0xBD => "-",
        0xBE => ".",
        0xBF => "/",
        0xC0 => "` ~",
        0xDB => "[",
        0xDC => "\\",
        0xDD => "]",
        0xDE => "'",
        c if is_engine_default(c) => "—",
        c @ 0x30..=0x39 | c @ 0x41..=0x5A => return char::from(c as u8).to_string(),
        c @ 0x60..=0x69 => return format!("Num {}", c - 0x60),
        c @ 0x70..=0x87 => return format!("F{}", c - 0x6F),
        c => return format!("Key {c:#04x}"),
    };
    s.to_string()
}

static INVERT_LOOK: AtomicBool = AtomicBool::new(false);
static VIBRATION: AtomicBool = AtomicBool::new(true);
static ACCEL_BITS: AtomicU64 = AtomicU64::new(0);
static SMOOTH_BITS: AtomicU64 = AtomicU64::new(0);

/// Settings that the host applies itself (mouse sensitivity is set separately, `set_mouse_sens`).
pub fn configure(s: &crate::ui::settings::Settings) {
    INVERT_LOOK.store(s.mouse.invert_look, Ordering::Relaxed);
    VIBRATION.store(s.pad.vibration, Ordering::Relaxed);
    ACCEL_BITS.store((s.mouse.acceleration.clamp(0.0, 1.0) as f64).to_bits(), Ordering::Relaxed);
    SMOOTH_BITS.store((s.mouse.smoothing.clamp(0.0, 1.0) as f64).to_bits(), Ordering::Relaxed);
}

pub fn pad_connected() -> bool {
    state().pad_index.is_some()
}

/// Mouse look: radians of turn per mouse count at sensitivity 1 (`--mouse-sens` multiplies it). With a host
/// the engine applies the slot-36 mouse value as a turn in radians, unscaled (notes/input.md).
const RAD_PER_COUNT: f32 = 0.0015;
const PAD_STICK_DEADZONE: i16 = 7849; // XInput's left-stick recommendation, used to detect "pad in use"
const PAD_TRIGGER_THRESHOLD: u8 = 30;

// ---- State -----------------------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Device {
    Pad,
    Kbm,
}

#[derive(Clone, Copy, Default)]
struct PadState {
    buttons: u16,
    lt: u8,
    rt: u8,
    lx: i16,
    ly: i16,
    rx: i16,
    ry: i16,
}

impl PadState {
    fn active(&self) -> bool {
        self.buttons != 0
            || self.lt > PAD_TRIGGER_THRESHOLD
            || self.rt > PAD_TRIGGER_THRESHOLD
            || [self.lx, self.ly, self.rx, self.ry].iter().any(|v| v.unsigned_abs() > PAD_STICK_DEADZONE as u16)
    }
}

struct State {
    keys: [bool; 256],
    dx: f32,
    dy: f32,
    wheel: f32,
    mouse_buttons: u32, // engine bit order: 0 left, 1 middle, 2 right, 3 X1, 4 X2
    device: Device,
    pad: PadState,
    pad_override: Option<PadState>, // set by the test script
    pad_index: Option<u32>,
    pad_rescan: Option<Instant>,
    rumble: (u16, u16),
    smooth: (f32, f32), // last smoothed mouse delta (counts per poll)
}

static STATE: Mutex<State> = Mutex::new(State {
    keys: [false; 256],
    dx: 0.0,
    dy: 0.0,
    wheel: 0.0,
    mouse_buttons: 0,
    device: Device::Kbm,
    pad: PadState { buttons: 0, lt: 0, rt: 0, lx: 0, ly: 0, rx: 0, ry: 0 },
    pad_override: None,
    pad_index: None,
    pad_rescan: None,
    rumble: (0, 0),
    smooth: (0.0, 0.0),
});

static WINDOW: AtomicUsize = AtomicUsize::new(0);
static CAPTURED: AtomicBool = AtomicBool::new(false);
static TEST_MODE: AtomicBool = AtomicBool::new(false);
static MOUSE_SENS_BITS: AtomicU64 = AtomicU64::new(0);
/// The frontend (menu or pause overlay, ui/) owns keyboard and mouse: no capture, nothing reaches the engine.
static UI_ACTIVE: AtomicBool = AtomicBool::new(false);

fn state() -> std::sync::MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

fn mouse_sens() -> f32 {
    let b = MOUSE_SENS_BITS.load(Ordering::Relaxed);
    if b == 0 { 1.0 } else { f64::from_bits(b) as f32 }
}

// ---- Event entry points (raw input and test script) -----------------------------------------------------

pub fn key(vk: u8, down: bool) {
    let mut s = state();
    s.keys[vk as usize] = down;
    if down {
        s.device = Device::Kbm;
    }
}

/// Relative mouse motion in counts. Ignored unless the cursor is captured (or a test script runs).
pub fn mouse_move(dx: i32, dy: i32) {
    if !CAPTURED.load(Ordering::Relaxed) && !TEST_MODE.load(Ordering::Relaxed) {
        return;
    }
    let mut s = state();
    s.dx += dx as f32;
    s.dy += dy as f32;
    if dx != 0 || dy != 0 {
        s.device = Device::Kbm;
    }
}

/// `bit`: 0 left, 1 middle, 2 right, 3 X1, 4 X2 (the engine's order, not Windows').
pub fn mouse_button(bit: u32, down: bool) {
    let mut s = state();
    if down {
        s.mouse_buttons |= 1 << bit;
        s.device = Device::Kbm;
    } else {
        s.mouse_buttons &= !(1 << bit);
    }
}

/// Mouse buttons held for the game (bits as in `mouse_button`; raw input or test script). Scoreboard right-click.
pub fn mouse_buttons() -> u32 {
    state().mouse_buttons
}

/// Wheel delta in Windows units (120 per notch, positive = away from the user).
pub fn wheel(delta: i32) {
    let mut s = state();
    s.wheel += delta as f32;
    s.device = Device::Kbm;
}

/// Window thread: hand keyboard and mouse to the UI (frees the cursor) or back to the game (captures it).
pub unsafe fn set_ui_active(on: bool) {
    UI_ACTIVE.store(on, Ordering::Relaxed);
    release_all();
    set_capture(!on);
}

/// Settings value; `--mouse-sens` on the command line wins (main only calls this without it).
pub fn set_mouse_sens(v: f64) {
    MOUSE_SENS_BITS.store(v.to_bits(), Ordering::Relaxed);
}

fn release_all() {
    let mut s = state();
    s.keys = [false; 256];
    s.mouse_buttons = 0;
    s.dx = 0.0;
    s.dy = 0.0;
}

// ---- Window side ----------------------------------------------------------------------------------------

/// Call once after the window exists. Parses our own command-line options (`--test-input`, `--mouse-sens`).
pub unsafe fn init(hwnd: HWND) {
    configure(&crate::ui::settings::get());
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--test-input" => {
                if let Some(p) = args.next() {
                    load_script(&p);
                }
            }
            "--mouse-sens" => {
                if let Some(v) = args.next().and_then(|v| v.parse::<f64>().ok()) {
                    MOUSE_SENS_BITS.store(v.to_bits(), Ordering::Relaxed);
                }
            }
            _ => {}
        }
    }
    let devs = [
        RAWINPUTDEVICE { usUsagePage: 1, usUsage: 2, dwFlags: RAWINPUTDEVICE_FLAGS(0), hwndTarget: hwnd }, // mouse
        RAWINPUTDEVICE { usUsagePage: 1, usUsage: 6, dwFlags: RAWINPUTDEVICE_FLAGS(0), hwndTarget: hwnd }, // keyboard
    ];
    match RegisterRawInputDevices(&devs, std::mem::size_of::<RAWINPUTDEVICE>() as u32) {
        Ok(()) => log!("input: raw mouse + keyboard registered; click the window to capture the mouse, F10 toggles, Esc releases"),
        Err(e) => log!("input: RegisterRawInputDevices failed: {e}"),
    }
    log!("input: mouse sensitivity x{} ({} rad/count)", mouse_sens(), RAD_PER_COUNT * mouse_sens());
    WINDOW.store(hwnd.0 as usize, Ordering::Relaxed);
    crate::ui::settings_apply::startup(hwnd);
    if GetFocus() == hwnd {
        set_capture(true);
    }
}

/// The main menu is running (seamless.rs: the menu comes back in the same process after a game). A UI-close
/// message left over from the game's pause overlay must not capture the mouse again under the menu.
static MENU: AtomicBool = AtomicBool::new(false);

/// Window thread: entering (true) or leaving the main menu. Entering frees and shows the cursor.
pub unsafe fn set_menu(on: bool) {
    MENU.store(on, Ordering::Relaxed);
    if on {
        release_all();
        set_capture(false);
    }
}

unsafe fn set_capture(on: bool) {
    // Not before init (focus arrives during CreateWindow), and a scripted run must not grab the desktop's mouse.
    if on && (WINDOW.load(Ordering::Relaxed) == 0 || TEST_MODE.load(Ordering::Relaxed) || UI_ACTIVE.load(Ordering::Relaxed) || MENU.load(Ordering::Relaxed)) {
        return;
    }
    let hwnd = HWND(WINDOW.load(Ordering::Relaxed) as _);
    if CAPTURED.swap(on, Ordering::Relaxed) == on {
        if on {
            clip_to_client(hwnd);
        }
        return;
    }
    if on {
        clip_to_client(hwnd);
    } else {
        let _ = ClipCursor(None);
        let mut s = state();
        s.mouse_buttons = 0;
        s.dx = 0.0;
        s.dy = 0.0;
    }
    sync_cursor();
    log!("input: mouse {}", if on { "captured" } else { "released" });
}

unsafe fn clip_to_client(hwnd: HWND) {
    let mut r = RECT::default();
    if GetClientRect(hwnd, &mut r).is_err() {
        return;
    }
    let mut tl = POINT { x: r.left, y: r.top };
    let mut br = POINT { x: r.right, y: r.bottom };
    let _ = ClientToScreen(hwnd, &mut tl);
    let _ = ClientToScreen(hwnd, &mut br);
    let _ = ClipCursor(Some(&RECT { left: tl.x, top: tl.y, right: br.x, bottom: br.y }));
}

/// Captured with the cursor hidden: put it back in the middle of the window after each mouse move. ClipCursor alone
/// is not enough (Wine on Wayland ignores it, and other programs can reset it), and in borderless fullscreen the
/// hidden cursor then drifts onto another monitor, where a click leaves the game. Aiming uses raw input deltas, so
/// warping the cursor does not move the view. Not while the cursor is shown (loadout picker reads it).
unsafe fn recentre_cursor() {
    if SPAWN_UI.load(Ordering::Relaxed) {
        return;
    }
    let hwnd = HWND(WINDOW.load(Ordering::Relaxed) as _);
    let mut r = RECT::default();
    if hwnd.0.is_null() || GetClientRect(hwnd, &mut r).is_err() {
        return;
    }
    let mut c = POINT { x: (r.left + r.right) / 2, y: (r.top + r.bottom) / 2 };
    let _ = ClientToScreen(hwnd, &mut c);
    let mut p = POINT::default();
    if GetCursorPos(&mut p).is_ok() && ((p.x - c.x).abs() > 64 || (p.y - c.y).abs() > 64) {
        let _ = SetCursorPos(c.x, c.y);
    }
}

/// Window-procedure hook; `Some` means the message is handled and should not reach DefWindowProc.
pub unsafe fn window_message(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
    match msg {
        WM_INPUT => {
            raw_input(HRAWINPUT(lp.0 as _));
            None
        }
        WM_LBUTTONDOWN if !CAPTURED.load(Ordering::Relaxed) && !UI_ACTIVE.load(Ordering::Relaxed) => {
            set_capture(true);
            Some(LRESULT(0))
        }
        WM_SETFOCUS => {
            set_capture(true);
            None
        }
        WM_KILLFOCUS => {
            set_capture(false);
            if !TEST_MODE.load(Ordering::Relaxed) {
                release_all();
            }
            None
        }
        WM_SIZE | WM_MOVE if CAPTURED.load(Ordering::Relaxed) => {
            clip_to_client(hwnd);
            None
        }
        WM_INPUT_CURSOR => {
            sync_cursor();
            Some(LRESULT(0))
        }
        WM_SETCURSOR if CURSOR_HIDDEN.load(Ordering::Relaxed) && (lp.0 & 0xFFFF) as u32 == HTCLIENT => {
            SetCursor(None);
            Some(LRESULT(1))
        }
        // Keys come from raw input; swallow the legacy ones except system combos (Alt+F4 etc.).
        WM_KEYDOWN | WM_KEYUP | WM_CHAR => Some(LRESULT(0)),
        _ => {
            let _ = wp;
            None
        }
    }
}

unsafe fn raw_input(h: HRAWINPUT) {
    if TEST_MODE.load(Ordering::Relaxed) || UI_ACTIVE.load(Ordering::Relaxed) {
        return; // scripted runs ignore the real keyboard and mouse; the UI reads window messages instead
    }
    let mut buf = [0u64; 8]; // RAWINPUT for mouse/keyboard is 48 bytes on x64
    let mut size = std::mem::size_of_val(&buf) as u32;
    let n = GetRawInputData(h, RID_INPUT, Some(buf.as_mut_ptr() as _), &mut size, std::mem::size_of::<RAWINPUTHEADER>() as u32);
    if n == u32::MAX || n == 0 {
        return;
    }
    let ri = &*(buf.as_ptr() as *const RAWINPUT);
    if ri.header.dwType == RIM_TYPEMOUSE.0 {
        let m = ri.data.mouse;
        if m.usFlags.0 & MOUSE_MOVE_ABSOLUTE.0 == 0 {
            mouse_move(m.lLastX, m.lLastY);
        }
        if !CAPTURED.load(Ordering::Relaxed) {
            return;
        }
        recentre_cursor();
        let f = m.Anonymous.Anonymous.usButtonFlags;
        // (down flag, up flag, engine bit)
        for (d, u, bit) in [(0x1, 0x2, 0), (0x4, 0x8, 2), (0x10, 0x20, 1), (0x40, 0x80, 3), (0x100, 0x200, 4)] {
            if f & d != 0 {
                mouse_button(bit, true);
            }
            if f & u != 0 {
                mouse_button(bit, false);
            }
        }
        if f & 0x400 != 0 {
            wheel(m.Anonymous.Anonymous.usButtonData as i16 as i32);
        }
    } else if ri.header.dwType == RIM_TYPEKEYBOARD.0 {
        let k = ri.data.keyboard;
        let down = k.Flags & 1 == 0; // RI_KEY_BREAK
        let e0 = k.Flags & 2 != 0;
        let mut vk = k.VKey as u8;
        if k.VKey == 0xFF {
            return; // fake key from an escaped sequence
        }
        // The engine polls left/right-specific modifiers (VK_LSHIFT etc.), not the generic ones.
        let side = match vk {
            0x10 => Some(if k.MakeCode == 0x36 { 0xA1 } else { 0xA0 }),
            0x11 => Some(if e0 { 0xA3 } else { 0xA2 }),
            0x12 => Some(if e0 { 0xA5 } else { 0xA4 }),
            _ => None,
        };
        if let Some(s) = side {
            key(vk, down);
            vk = s;
        }
        // 1-4 vote while a playlist vote is open (ui/vote.rs); otherwise they reach the engine as usual.
        if down && (0x31..=0x34).contains(&vk) && crate::ui::vote::key(vk - 0x31) {
            return;
        }
        // F5 / F6: yes / no on an open call vote (callvote.rs); otherwise they reach the engine as usual.
        if down && (vk == 0x74 || vk == 0x75) && crate::callvote::key(vk == 0x74) {
            return;
        }
        if down && vk == 0x08 && crate::ui::vote::hide_key() {
            return; // Backspace: hide the vote panel for this round
        }
        // Enter on a listen host's campaign lobby: start the mission now (lobby.rs).
        if down && vk == 0x0D && crate::lobby::start_now() {
            log!("lobby: host pressed Enter, starting now");
            return;
        }
        if down && vk == 0x79 {
            // F10: toggle mouse capture
            set_capture(!CAPTURED.load(Ordering::Relaxed));
            return;
        }
        if down && vk == 0x1B {
            set_capture(false); // Esc goes to the engine (pause) and frees the mouse
            // Joining (no unit yet): the engine has no pause screen to open, so open ours (LEAVE GAME).
            if crate::net::host_addr().is_some() && !player0_alive() {
                crate::ui::overlay::open();
            }
        }
        key(vk, down);
    }
}

// ---- Engine side (host slots) ---------------------------------------------------------------------------

unsafe fn poll_pad(s: &mut State) {
    if let Some(p) = s.pad_override {
        s.pad = p;
        return;
    }
    if TEST_MODE.load(Ordering::Relaxed) {
        return;
    }
    let mut xs = XINPUT_STATE::default();
    if let Some(i) = s.pad_index {
        if XInputGetState(i, &mut xs) == 0 {
            let g = xs.Gamepad;
            s.pad = PadState { buttons: g.wButtons.0, lt: g.bLeftTrigger, rt: g.bRightTrigger, lx: g.sThumbLX, ly: g.sThumbLY, rx: g.sThumbRX, ry: g.sThumbRY };
            if s.pad.active() {
                s.device = Device::Pad;
            }
            return;
        }
        log!("input: XInput pad {i} disconnected");
        s.pad_index = None;
        s.pad = PadState::default();
        if s.device == Device::Pad {
            s.device = Device::Kbm;
        }
    }
    // Probing empty XInput slots is slow on Windows; look for a new pad once a second.
    let now = Instant::now();
    if s.pad_rescan.is_some_and(|t| now < t) {
        return;
    }
    s.pad_rescan = Some(now + std::time::Duration::from_secs(1));
    for i in 0..4 {
        if XInputGetState(i, &mut xs) == 0 {
            log!("input: XInput pad found in slot {i}");
            s.pad_index = Some(i);
            return;
        }
    }
}

/// Smoothing (average with the previous poll), acceleration (faster motion turns more per count) and look
/// inversion, applied to this poll's raw counts.
fn shape_mouse(s: &mut State) -> (f32, f32) {
    let smooth = f64::from_bits(SMOOTH_BITS.load(Ordering::Relaxed)) as f32 * 0.85;
    let accel = f64::from_bits(ACCEL_BITS.load(Ordering::Relaxed)) as f32;
    let (mut dx, mut dy) = (s.dx, s.dy);
    if smooth > 0.0 {
        dx = s.smooth.0 * smooth + dx * (1.0 - smooth);
        dy = s.smooth.1 * smooth + dy * (1.0 - smooth);
        s.smooth = (dx, dy);
    }
    if accel > 0.0 {
        let gain = 1.0 + accel * ((dx * dx + dy * dy).sqrt() / 20.0).min(3.0);
        dx *= gain;
        dy *= gain;
    }
    if INVERT_LOOK.load(Ordering::Relaxed) {
        dy = -dy;
    }
    (dx, dy)
}

/// Host slot 36: fill the 0x130-byte input state for local player 0 (layout in notes/input.md).
pub unsafe fn fill_input_state(st: *mut u8) {
    let mut s = state();
    run_script(&mut s);
    poll_pad(&mut s);
    std::ptr::write_bytes(st, 0, 0x130);
    let kbm = s.device == Device::Kbm;
    (st as *mut u32).write_unaligned(if kbm { 1 } else { 0 });
    for (i, &k) in s.keys.iter().enumerate() {
        *st.add(4 + i) = k as u8;
    }
    let sens = RAD_PER_COUNT * mouse_sens();
    let f = |off: usize, v: f32| (st.add(off) as *mut f32).write_unaligned(v);
    let (dx, dy) = shape_mouse(&mut s);
    f(0x104, dx * sens);
    f(0x108, dy * sens);
    let hwnd = HWND(WINDOW.load(Ordering::Relaxed) as _);
    let mut pt = POINT::default();
    if GetCursorPos(&mut pt).is_ok() && ScreenToClient(hwnd, &mut pt).as_bool() {
        f(0x10C, pt.x as f32);
        f(0x110, pt.y as f32);
    }
    f(0x114, s.wheel);
    f(0x118, 120.0);
    // While Tab (show scores) is held, right-click belongs to the scoreboard (mouse mode), not the engine's zoom.
    let buttons = if crate::ui::scoreboard::held() { s.mouse_buttons & !4 } else { s.mouse_buttons };
    (st.add(0x11C) as *mut u32).write_unaligned(buttons);
    s.dx = 0.0;
    s.dy = 0.0;
    s.wheel = 0.0;
    if UI_ACTIVE.load(Ordering::Relaxed) {
        std::ptr::write_bytes(st.add(4), 0, 0x130 - 4); // device stays present (no pause), but idle
        return;
    }
    if !kbm {
        let p = s.pad;
        (st.add(0x120) as *mut u16).write_unaligned(p.buttons);
        *st.add(0x124) = p.lt;
        *st.add(0x125) = p.rt;
        for (i, v) in [p.lx, p.ly, p.rx, p.ry].into_iter().enumerate() {
            (st.add(0x126 + i * 2) as *mut i16).write_unaligned(v);
        }
    }
    static LAST: Mutex<Option<Device>> = Mutex::new(None);
    let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
    if *last != Some(s.device) {
        log!("input: player 0 device -> {:?} (kind {})", s.device, if kbm { 1 } else { 0 });
        *last = Some(s.device);
    }
    drop(s);
    update_spawn_screen();
    log_player_state();
}

/// Host slot 98: true lets mouse look blend in while the pad is the active device. In keyboard/mouse mode it
/// must be false: the engine skips every keyboard and mouse-button binding when it is true.
pub fn mouse_blend_with_pad() -> bool {
    state().device == Device::Pad
}

/// Host slot 39: motor speeds for a local player; forwarded to the XInput pad while it is player 0's device.
pub unsafe fn set_rumble(player: i32, r: *const u16) {
    if player != 0 || r.is_null() {
        return;
    }
    let (l, rr) = (*r, *r.add(1));
    let mut s = state();
    let want = if s.device == Device::Pad && VIBRATION.load(Ordering::Relaxed) { (l, rr) } else { (0, 0) };
    if want == s.rumble {
        return;
    }
    s.rumble = want;
    if let Some(i) = s.pad_index {
        let v = XINPUT_VIBRATION { wLeftMotorSpeed: want.0, wRightMotorSpeed: want.1 };
        XInputSetState(i, &v);
    }
}

// ---- Engine UI cursor (loadout picker) -------------------------------------------------------------------

/// Window message: the spawn screen came or went; the window thread shows or hides the cursor.
pub const WM_INPUT_CURSOR: u32 = WM_APP + 0x61;
/// Player 0 is on the spawn screen (map loaded, no unit: loadout picker / respawn wait).
static SPAWN_UI: AtomicBool = AtomicBool::new(false);
/// Player 0 has a live unit (set on the game thread in slot 36). `player0_view` reads the game thread's TLS block,
/// so the window and render threads must use this instead.
static PLAYER0_ALIVE: AtomicBool = AtomicBool::new(false);

/// Any thread: does player 0 have a unit with a position (spawned, not loading/dead)?
pub fn player0_alive() -> bool {
    PLAYER0_ALIVE.load(Ordering::Relaxed)
}
/// Window thread: we hold one ShowCursor(false) (captured and not on the spawn screen).
static CURSOR_HIDDEN: AtomicBool = AtomicBool::new(false);
/// Test-script cursor in client pixels (x in the low half, y in the high half); u64::MAX = use the real one.
static VCURSOR: AtomicU64 = AtomicU64::new(u64::MAX);

/// Window thread: hide the cursor while the mouse is captured for play, show it on the spawn screen (the
/// engine's loadout picker reads the OS cursor itself, see notes/input.md "Mouse in engine menus").
unsafe fn sync_cursor() {
    let hide = CAPTURED.load(Ordering::Relaxed) && !SPAWN_UI.load(Ordering::Relaxed);
    if CURSOR_HIDDEN.swap(hide, Ordering::Relaxed) != hide {
        ShowCursor(!hide);
        if !hide {
            let _ = SetCursor(LoadCursorW(None, IDC_ARROW).ok());
        }
    }
}

/// The engine's window global (haloreach+0xc1a188): `EngineInitializeGraphics` picks the process's first
/// *visible* top-level window, so with `--hidden` (or a window shown later) it stays null and the engine's
/// menu-cursor code never reads the cursor. Point it at our window.
fn ensure_engine_window() {
    let base = crate::crash::ENGINE_BASE.load(Ordering::Relaxed);
    let ours = WINDOW.load(Ordering::Relaxed);
    if base == 0 || ours == 0 {
        return;
    }
    let g = (base + 0xc1a188) as *mut usize;
    unsafe {
        if g.read() != ours {
            log!("input: engine window global was {:#x}, set to ours {ours:#x} (menu cursor)", g.read());
            g.write(ours);
        }
    }
}

/// Game thread (slot 36): track the spawn screen from player 0's unit handle (all ones = none yet / dead).
fn update_spawn_screen() {
    ensure_engine_window();
    let view = unsafe { player0_view() };
    PLAYER0_ALIVE.store(view.as_ref().is_some_and(|v| v.pos.is_some()), Ordering::Relaxed);
    let on = view.is_some_and(|v| v.unit == u32::MAX);
    if SPAWN_UI.swap(on, Ordering::Relaxed) != on {
        log!("input: spawn screen {} (loadout picker takes the mouse cursor)", if on { "up, cursor shown" } else { "gone" });
        let hwnd = HWND(WINDOW.load(Ordering::Relaxed) as _);
        unsafe {
            let _ = PostMessageW(Some(hwnd), WM_INPUT_CURSOR, WPARAM(0), LPARAM(0));
        }
    }
}

type GetCursorPosFn = unsafe extern "system" fn(*mut POINT) -> i32;
type GetForegroundWindowFn = unsafe extern "system" fn() -> HWND;
static REAL_GET_CURSOR_POS: AtomicUsize = AtomicUsize::new(0);
static REAL_GET_FOREGROUND: AtomicUsize = AtomicUsize::new(0);

/// IAT hook for the engine's `GetCursorPos`: the test script's cursor when set, else the real one.
unsafe extern "system" fn hook_get_cursor_pos(p: *mut POINT) -> i32 {
    let v = VCURSOR.load(Ordering::Relaxed);
    if v != u64::MAX && !p.is_null() {
        let mut pt = POINT { x: v as u32 as i32, y: (v >> 32) as u32 as i32 };
        let _ = ClientToScreen(HWND(WINDOW.load(Ordering::Relaxed) as _), &mut pt);
        *p = pt;
        return 1;
    }
    let real: GetCursorPosFn = std::mem::transmute(REAL_GET_CURSOR_POS.load(Ordering::Relaxed));
    real(p)
}

/// IAT hook for the engine's `GetForegroundWindow` (only its menu-cursor code calls it): a scripted run counts
/// as foreground, so a hidden window still gets hover and clicks.
unsafe extern "system" fn hook_get_foreground_window() -> HWND {
    if TEST_MODE.load(Ordering::Relaxed) {
        return HWND(WINDOW.load(Ordering::Relaxed) as _);
    }
    let real: GetForegroundWindowFn = std::mem::transmute(REAL_GET_FOREGROUND.load(Ordering::Relaxed));
    real()
}

/// Patch haloreach.dll's import slots for GetForegroundWindow (+0x91e4b0) and GetCursorPos (+0x91e4b8),
/// frozen build only: each slot must hold the real user32 function before it is replaced.
pub unsafe fn install_cursor_hooks(base: usize) {
    use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
    use windows::Win32::System::Memory::{VirtualProtect, PAGE_PROTECTION_FLAGS, PAGE_READWRITE};
    let Ok(user32) = GetModuleHandleW(windows::core::w!("user32.dll")) else { return };
    let real = |n: &[u8]| GetProcAddress(user32, windows::core::PCSTR(n.as_ptr())).map(|f| f as usize).unwrap_or(0);
    for (rva, name, hook, store) in [
        (0x91e4b0usize, &b"GetForegroundWindow\0"[..], hook_get_foreground_window as *const () as usize, &REAL_GET_FOREGROUND),
        (0x91e4b8, &b"GetCursorPos\0"[..], hook_get_cursor_pos as *const () as usize, &REAL_GET_CURSOR_POS),
    ] {
        let slot = (base + rva) as *mut usize;
        let want = real(name);
        if want == 0 || slot.read() != want {
            log!("input: import slot +{rva:#x} is not {} (other DLL build?): menu cursor hook skipped",
                String::from_utf8_lossy(&name[..name.len() - 1]));
            continue;
        }
        store.store(want, Ordering::Relaxed);
        let mut old = PAGE_PROTECTION_FLAGS(0);
        if VirtualProtect(slot as _, 8, PAGE_READWRITE, &mut old).is_ok() {
            slot.write(hook);
            let _ = VirtualProtect(slot as _, 8, old, &mut old);
        }
    }
    log!("input: engine menu cursor hooks installed (GetForegroundWindow, GetCursorPos)");
}

// ---- Engine state readout (verification) ----------------------------------------------------------------

/// Player 0's control record in the engine's TLS game state ("player control globals", TLS +0xD8):
/// +0x80 unit handle, +0x94 yaw, +0x98 pitch (radians), +0xAC throttle forward, +0xB0 strafe.
pub struct PlayerView {
    pub unit: u32,
    pub yaw: f32,
    pub pitch: f32,
    pub throttle: (f32, f32),
    pub pos: Option<[f32; 3]>,
}

/// The engine's per-thread game-state block (TLS index at 0x180c17b18); only valid on the game thread.
pub(crate) unsafe fn engine_tls_block() -> Option<*const u8> {
    let base = crate::crash::ENGINE_BASE.load(Ordering::Relaxed);
    if base == 0 {
        return None;
    }
    let idx = ((base + 0xc17b18) as *const u32).read() as usize;
    let tls: *const *const u8;
    core::arch::asm!("mov {}, gs:[0x58]", out(reg) tls);
    if tls.is_null() {
        return None;
    }
    let block = *tls.add(idx);
    (!block.is_null()).then_some(block)
}

/// Object table: TLS +0x10 -> +0x50 -> entries of 0x18 bytes, object pointer at +0x10.
unsafe fn object_ptr(block: *const u8, handle: u32) -> Option<*const f32> {
    let objs = *(block.add(0x10) as *const *const u8);
    if objs.is_null() {
        return None;
    }
    let table = *(objs.add(0x50) as *const *const u8);
    if table.is_null() {
        return None;
    }
    let obj = *(table.add((handle as usize & 0xffff) * 0x18 + 0x10) as *const *const f32);
    (!obj.is_null()).then_some(obj)
}

pub unsafe fn player0_view() -> Option<PlayerView> {
    let block = engine_tls_block()?;
    let pc = *(block.add(0xd8) as *const *const u8);
    if pc.is_null() {
        return None;
    }
    let rd = |o: usize| (pc.add(o) as *const f32).read_unaligned();
    let unit = (pc.add(0x80) as *const u32).read_unaligned();
    let pos = if has_unit(unit) {
        object_ptr(block, unit).map(|o| [*o.add(OBJ_POS / 4), *o.add(OBJ_POS / 4 + 1), *o.add(OBJ_POS / 4 + 2)])
    } else {
        None
    };
    Some(PlayerView { unit, yaw: rd(0x94), pitch: rd(0x98), throttle: (rd(0xac), rd(0xb0)), pos })
}

/// One-off dump of the start of player 0's unit object (used to locate the position field).
unsafe fn dump_unit_object(unit: u32) {
    let Some(block) = engine_tls_block() else { return };
    let Some(obj) = object_ptr(block, unit) else { return };
    let mut line = String::new();
    for i in 0..48 {
        line += &format!(" +{:#x}={:.3}", i * 4, *obj.add(i));
    }
    log!("unit object {obj:?}:{line}");
}

/// Datum handles carry a non-zero salt in the high half; 0 is the zeroed record before the game starts.
fn has_unit(h: u32) -> bool {
    h != u32::MAX && h >> 16 != 0
}

/// Offset of the world position (3 floats) in an object.
const OBJ_POS: usize = 0x20;

fn log_player_state() {
    static LAST: Mutex<Option<Instant>> = Mutex::new(None);
    let period = if TEST_MODE.load(Ordering::Relaxed) { 0.5 } else { 30.0 };
    let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
    if last.is_some_and(|t| t.elapsed().as_secs_f32() < period) {
        return;
    }
    *last = Some(Instant::now());
    drop(last);
    if let Some(v) = unsafe { player0_view() } {
        log!("{}", describe(&v));
    }
}

/// Engine actions whose keyboard/mouse state is logged: (name, engine action index).
const WATCHED_ACTIONS: &[(&str, usize)] =
    &[("jump", 0), ("grenade-switch", 1), ("action", 3), ("melee", 4), ("ability", 5), ("grenade", 6), ("fire", 7), ("crouch", 8), ("zoom", 9), ("scores", 0x3b), ("reload", 0x3e)];

/// Names of engine actions held right now in player 0's keyboard/mouse action records
/// (0x18287ff94 + 0x224 + action * 0x10: u16 held ms, u8 held frames).
fn held_actions() -> String {
    let base = crate::crash::ENGINE_BASE.load(Ordering::Relaxed);
    if base == 0 {
        return String::new();
    }
    let mut out = Vec::new();
    for &(name, a) in WATCHED_ACTIONS {
        let frames = unsafe { *((base + 0x287ff94 + 0x224 + a * 0x10 + 2) as *const u8) };
        if frames != 0 {
            out.push(name);
        }
    }
    out.join(",")
}

fn describe(v: &PlayerView) -> String {
    let pos = v.pos.map(|p| format!("({:.3}, {:.3}, {:.3})", p[0], p[1], p[2])).unwrap_or_else(|| "-".into());
    format!(
        "player0: unit {:#x} pos {pos} yaw {:.1} deg pitch {:.1} deg throttle ({:.2}, {:.2}) held [{}]",
        v.unit,
        v.yaw.to_degrees(),
        v.pitch.to_degrees(),
        v.throttle.0,
        v.throttle.1,
        held_actions()
    )
}

// ---- Test script ----------------------------------------------------------------------------------------

/// One timed step; `t` is seconds after player 0 first has a unit (spawn).
struct Step {
    t: f32,
    cmd: Cmd,
}

enum Cmd {
    Key(u8, bool),
    Button(u32, bool),
    Wheel(i32),
    /// Spread `dx, dy` counts evenly over `secs` (like a real mouse sending many small reports).
    Turn(f32, f32, f32),
    /// `cursor <x> <y>`: the OS cursor the engine's menus read, in client pixels (`cursor off` = real one).
    Cursor(Option<(i32, i32)>),
    Pad(Option<PadState>),
    Capture(String),
    Log(String),
    /// `set fov <deg>` / `set fps <cap code>` / `set vsync 0|1` / `set bind <action> <key>`: change a setting the
    /// way the settings screen does (saved, then applied to the running game).
    Set(Vec<String>),
    /// `hs <expression>`: a Halo script command through engine slot 9 (preview.rs).
    Hs(String),
    /// `forge save <name>`: save the live Forge map variant (forge.rs).
    Forge(Vec<String>),
    /// `vote yes|no|call <kind> [target]|...`: call votes (callvote.rs `script_command`).
    Vote(Vec<String>),
    Exit,
}

struct Script {
    steps: Vec<Step>,
    next: usize,
    spawn: Option<Instant>,
    /// `from map` line: times count from the map load (player control exists, no unit yet), not the spawn.
    from_map: bool,
    turns: Vec<(f32, f32, f32, f32, f32)>, // start, secs, dx, dy, applied fraction
}

static SCRIPT: Mutex<Option<Script>> = Mutex::new(None);
static CAPTURE_REQ: Mutex<Option<String>> = Mutex::new(None);

/// Between two games in one process (seamless.rs): no unit yet, and a test script runs again from the next spawn.
pub fn reset_game() {
    PLAYER0_ALIVE.store(false, Ordering::Relaxed);
    SPAWN_UI.store(false, Ordering::Relaxed);
    if let Some(sc) = SCRIPT.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        sc.spawn = None;
        sc.next = 0;
        sc.turns.clear();
    }
}

/// Back-buffer capture requested by the test script (file name), taken by the frame-end slot.
pub fn take_capture_request() -> Option<String> {
    CAPTURE_REQ.lock().unwrap_or_else(|e| e.into_inner()).take()
}

fn vk_from_name(n: &str) -> Option<u8> {
    let u = n.to_ascii_uppercase();
    Some(match u.as_str() {
        "SPACE" => 0x20,
        "LCTRL" | "CTRL" => 0xA2,
        "LSHIFT" | "SHIFT" => 0xA0,
        "TAB" => 0x09,
        "ESC" => 0x1B,
        "ENTER" => 0x0D,
        s if s.len() == 1 && s.as_bytes()[0].is_ascii_alphanumeric() => s.as_bytes()[0],
        s if s.starts_with("0X") => u8::from_str_radix(&s[2..], 16).ok()?,
        _ => return None,
    })
}

fn parse_step(line: &str) -> Result<Option<Step>, String> {
    let line = line.split('#').next().unwrap_or("").trim();
    if line.is_empty() {
        return Ok(None);
    }
    let w: Vec<&str> = line.split_whitespace().collect();
    let t: f32 = w[0].parse().map_err(|_| format!("bad time in '{line}'"))?;
    let arg = |i: usize| w.get(i).copied().ok_or_else(|| format!("missing argument in '{line}'"));
    let num = |i: usize| -> Result<f32, String> { arg(i)?.parse::<f32>().map_err(|_| format!("bad number in '{line}'")) };
    let updown = |i: usize| -> Result<bool, String> {
        match arg(i)? {
            "down" => Ok(true),
            "up" => Ok(false),
            _ => Err(format!("expected down|up in '{line}'")),
        }
    };
    let cmd = match arg(1)? {
        "key" => Cmd::Key(vk_from_name(arg(2)?).ok_or_else(|| format!("unknown key in '{line}'"))?, updown(3)?),
        "button" => {
            let bit = match arg(2)? {
                "left" => 0,
                "middle" => 1,
                "right" => 2,
                _ => return Err(format!("button left|middle|right in '{line}'")),
            };
            Cmd::Button(bit, updown(3)?)
        }
        "wheel" => Cmd::Wheel(num(2)? as i32),
        "turn" => Cmd::Turn(num(2)?, num(3)?, num(4)?.max(0.0)),
        "cursor" if arg(2)? == "off" => Cmd::Cursor(None),
        "cursor" => Cmd::Cursor(Some((num(2)? as i32, num(3)? as i32))),
        "pad" if arg(2)? == "off" => Cmd::Pad(None),
        "pad" => Cmd::Pad(Some(PadState {
            buttons: u16::from_str_radix(arg(2)?.trim_start_matches("0x"), 16).map_err(|_| format!("bad buttons in '{line}'"))?,
            lt: num(3)? as u8,
            rt: num(4)? as u8,
            lx: num(5)? as i16,
            ly: num(6)? as i16,
            rx: num(7)? as i16,
            ry: num(8)? as i16,
        })),
        "capture" => Cmd::Capture(arg(2)?.to_string()),
        "log" => Cmd::Log(w[2..].join(" ")),
        "hs" => Cmd::Hs(w[2..].join(" ")),
        "forge" => Cmd::Forge(w[2..].iter().map(|x| x.to_string()).collect()),
        "vote" => Cmd::Vote(w[2..].iter().map(|x| x.to_string()).collect()),
        "set" if w.len() >= 4 => Cmd::Set(w[2..].iter().map(|x| x.to_string()).collect()),
        "exit" => Cmd::Exit,
        other => return Err(format!("unknown command {other}")),
    };
    Ok(Some(Step { t, cmd }))
}

fn script_set(a: &[String]) {
    let old = crate::ui::settings::get();
    let mut new = old.clone();
    let n = |i: usize| a.get(i).and_then(|v| v.parse::<i64>().ok()).unwrap_or(0);
    match a[0].as_str() {
        "fov" => new.game.fov = n(1) as i32,
        "fps" => new.graphics.fps_cap = n(1) as u8,
        "vsync" => new.graphics.vsync = n(1) != 0,
        // `set view <yaw> <pitch> <dist>` (preview child: orbit camera, radians / world units).
        "view" => {
            let f = |i: usize| a.get(i).and_then(|v| v.parse::<f32>().ok()).unwrap_or(0.0);
            crate::preview::set_view(f(1), f(2), f(3));
        }
        "spot" => {
            let f = |i: usize| a.get(i).and_then(|v| v.parse::<f32>().ok()).unwrap_or(0.0);
            crate::preview::set_spot([f(1), f(2), f(3), f(4)]);
        }
        // `set armour <field> <hex id>` (preview checks): helmet, primary, secondary, elite (0/1).
        "armour" => {
            let v = a.get(2).and_then(|v| u32::from_str_radix(v.trim_start_matches("0x"), 16).ok()).unwrap_or(0);
            match a.get(1).map(|f| f.as_str()) {
                Some("helmet") => new.armour.helmet = v,
                Some("primary") => new.armour.primary = v,
                Some("secondary") => new.armour.secondary = v,
                Some("elite") => new.armour.elite = v != 0,
                _ => {}
            }
            if crate::preview::is_child() {
                crate::preview::queue_armour(&new.armour);
            }
        }
        "bind" => {
            if let Some(vk) = a.get(2).and_then(|k| vk_from_name(k)) {
                let action = n(1) as u32;
                let mut k = new.mouse.keys(action);
                k[0] = vk as u32;
                new.mouse.set_keys(action, k);
            }
        }
        other => log!("input script: unknown setting {other}"),
    }
    crate::ui::settings::save(&new);
    crate::ui::settings_apply::apply(&old, &new);
    log!("input script: set {}", a.join(" "));
}

fn load_script(path: &str) {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            log!("input: cannot read test script {path}: {e}");
            return;
        }
    };
    let mut steps = Vec::new();
    let mut from_map = false;
    for line in text.lines() {
        if line.trim() == "from map" {
            from_map = true;
            continue;
        }
        match parse_step(line) {
            Ok(Some(s)) => steps.push(s),
            Ok(None) => {}
            Err(e) => {
                log!("input: test script {path}: {e}");
                return;
            }
        }
    }
    steps.sort_by(|a, b| a.t.total_cmp(&b.t));
    log!("input: test script {path}: {} steps, timed from {}", steps.len(), if from_map { "the map load" } else { "player 0's spawn" });
    TEST_MODE.store(true, Ordering::Relaxed);
    *SCRIPT.lock().unwrap_or_else(|e| e.into_inner()) = Some(Script { steps, next: 0, spawn: None, from_map, turns: Vec::new() });
}

/// Runs due script steps; called with the state lock held, from the engine's input poll (game thread).
fn run_script(s: &mut State) {
    let mut guard = SCRIPT.lock().unwrap_or_else(|e| e.into_inner());
    let Some(sc) = guard.as_mut() else { return };
    let spawn = match sc.spawn {
        Some(t) => t,
        None => {
            match unsafe { player0_view() } {
                Some(v) if sc.from_map && v.unit == u32::MAX => {
                    log!("input: test script armed at map load ({})", describe(&v));
                    sc.spawn = Some(Instant::now());
                }
                Some(v) if has_unit(v.unit) => {
                    log!("input: test script armed, player 0 spawned ({})", describe(&v));
                    unsafe { dump_unit_object(v.unit) };
                    sc.spawn = Some(Instant::now());
                }
                _ => {}
            }
            return;
        }
    };
    let now = spawn.elapsed().as_secs_f32();
    // Steps call the public entry points, which take the state lock; apply them on a released copy.
    let mut due = Vec::new();
    while sc.next < sc.steps.len() && sc.steps[sc.next].t <= now {
        due.push(sc.next);
        sc.next += 1;
    }
    let mut turn_dx = 0.0;
    let mut turn_dy = 0.0;
    for &i in &due {
        let st = &sc.steps[i];
        match &st.cmd {
            Cmd::Key(vk, d) => {
                s.keys[*vk as usize] = *d;
                if *d {
                    s.device = Device::Kbm;
                }
            }
            Cmd::Button(b, d) => {
                if *d {
                    s.mouse_buttons |= 1 << b;
                    s.device = Device::Kbm;
                } else {
                    s.mouse_buttons &= !(1 << b);
                }
            }
            Cmd::Wheel(n) => {
                s.wheel += *n as f32;
                s.device = Device::Kbm;
            }
            Cmd::Turn(dx, dy, secs) => sc.turns.push((st.t, *secs, *dx, *dy, 0.0)),
            Cmd::Cursor(c) => {
                VCURSOR.store(c.map_or(u64::MAX, |(x, y)| (x as u32 as u64) | ((y as u32 as u64) << 32)), Ordering::Relaxed);
                s.device = Device::Kbm;
            }
            Cmd::Pad(p) => {
                s.pad_override = *p;
                if p.is_some() {
                    s.device = Device::Pad;
                } else {
                    s.device = Device::Kbm;
                }
            }
            Cmd::Capture(name) => *CAPTURE_REQ.lock().unwrap_or_else(|e| e.into_inner()) = Some(name.clone()),
            Cmd::Log(text) => {
                let v = unsafe { player0_view() };
                log!("input script t={:.2}: {text} | {}", now, v.map(|v| describe(&v)).unwrap_or_default());
            }
            Cmd::Set(a) => script_set(a),
            Cmd::Forge(a) => crate::forge::script_command(a),
            Cmd::Vote(a) => crate::callvote::script_command(a),
            Cmd::Hs(e) if e == "?dir" => unsafe { crate::preview::director_dump() },
            Cmd::Hs(e) => _ = crate::preview::hs(e),
            Cmd::Exit => {
                log!("input script: exit");
                std::process::exit(0);
            }
        }
        if !matches!(st.cmd, Cmd::Log(_)) {
            log!("input script t={:.2}: step {} applied", now, i);
        }
    }
    for tr in sc.turns.iter_mut() {
        let frac = if tr.1 <= 0.0 { 1.0 } else { ((now - tr.0) / tr.1).clamp(0.0, 1.0) };
        turn_dx += tr.2 * (frac - tr.4);
        turn_dy += tr.3 * (frac - tr.4);
        tr.4 = frac;
    }
    sc.turns.retain(|t| t.4 < 1.0);
    if turn_dx != 0.0 || turn_dy != 0.0 {
        s.dx += turn_dx;
        s.dy += turn_dy;
        s.device = Device::Kbm;
    }
}
