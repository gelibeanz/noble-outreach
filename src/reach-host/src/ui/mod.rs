//! Our frontend: egui drawn with Direct3D 11 (egui-directx11) into the host's own swap chain.
//!
//! Two places draw it: the main menu (menu.rs) runs its own loop on the window before the engine exists, and the
//! pause overlay (overlay.rs) draws on top of the engine's finished frame in host slot 1 (end_frame), right before
//! the engine presents. While either is up, `active()` is true and input.rs hands the keyboard and mouse to us.
//! Structure and how to add a screen: notes/frontend.md.

pub mod browser_screen;
pub mod campaign;
pub mod carnage;
pub mod customize;
pub mod forge;
pub mod hostinfo;
pub mod loading;
pub mod mccart;
pub mod menu;
pub mod menu_scene;
pub mod overlay;
pub mod play_screen;
pub mod scoreboard;
pub mod settings;
pub mod settings_apply;
pub mod settings_screen;
pub mod theme;
pub mod vote;
pub mod widgets;

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Mutex;
use std::time::Instant;

use windows::core::Interface;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::UI::Input::KeyboardAndMouse::GetKeyState;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::log;

/// Posted to the window by other threads: an owner (wParam, `OWN_*`) wants keyboard and mouse / is done with them
/// (cursor and capture are per window thread).
pub const WM_UI_OPEN: u32 = WM_APP + 0x51;
pub const WM_UI_CLOSE: u32 = WM_APP + 0x52;

/// Who holds the UI. Each owner opens and closes only its own bit, so one closing (the pause overlay on LEAVE GAME,
/// a message that lands after the menu is back) cannot switch off another (the menu, the call-vote page).
pub const OWN_MENU: u32 = 1;
pub const OWN_OVERLAY: u32 = 2;
pub const OWN_VOTE: u32 = 4;
pub const OWN_SCOREBOARD: u32 = 8;
static OWNERS: AtomicU32 = AtomicU32::new(0);

static ACTIVE: AtomicBool = AtomicBool::new(false);
/// Window events in client pixels; converted to points when a frame drains them.
static EVENTS: Mutex<Vec<egui::Event>> = Mutex::new(Vec::new());

struct Gui {
    ctx: egui::Context,
    renderer: egui_directx11::Renderer,
    start: Instant,
    /// Fresh pipeline state swapped in while we draw over the engine's frame, so its own state survives.
    state: Option<ID3DDeviceContextState>,
}

// COM pointers of one device, only ever used under the GUI lock.
unsafe impl Send for Gui {}

static GUI: Mutex<Option<Gui>> = Mutex::new(None);

pub fn active() -> bool {
    ACTIVE.load(Ordering::Relaxed)
}

/// Window thread only: the menu takes (or gives back) keyboard and mouse. Entering or leaving the menu also drops
/// every in-game owner, so a game's late open/close cannot leave the UI on (or off) for what comes next.
pub fn set_active(on: bool) {
    let old = OWNERS.swap(if on { OWN_MENU } else { 0 }, Ordering::Relaxed);
    if old & !OWN_MENU != 0 {
        log!("ui: dropped in-game owners {:#x} ({})", old & !OWN_MENU, if on { "menu" } else { "game" });
    }
    apply_owners();
}

/// Window thread only: `owner` (`OWN_*`) takes or gives back keyboard and mouse.
fn set_owner(owner: u32, on: bool) {
    let old = if on { OWNERS.fetch_or(owner, Ordering::Relaxed) } else { OWNERS.fetch_and(!owner, Ordering::Relaxed) };
    if (old & owner != 0) != on {
        log!("ui: owner {owner:#x} {} (owners now {:#x})", if on { "open" } else { "closed" }, OWNERS.load(Ordering::Relaxed));
    }
    apply_owners();
}

fn apply_owners() {
    let on = OWNERS.load(Ordering::Relaxed) != 0;
    if ACTIVE.swap(on, Ordering::Relaxed) != on {
        EVENTS.lock().unwrap_or_else(|e| e.into_inner()).clear();
        unsafe { crate::input::set_ui_active(on) };
    }
}

/// Any thread: `owner` (`OWN_*`) opens or closes its hold on keyboard and mouse.
pub fn post_ui(owner: u32, open: bool) {
    let hwnd = HWND(crate::host::WINDOW.load(Ordering::Relaxed) as _);
    unsafe {
        let _ = PostMessageW(Some(hwnd), if open { WM_UI_OPEN } else { WM_UI_CLOSE }, WPARAM(owner as usize), LPARAM(0));
    }
}

pub fn post_to_window(msg: u32) {
    let hwnd = HWND(crate::host::WINDOW.load(Ordering::Relaxed) as _);
    unsafe {
        let _ = PostMessageW(Some(hwnd), msg, WPARAM(0), LPARAM(0));
    }
}

fn push(e: egui::Event) {
    EVENTS.lock().unwrap_or_else(|e| e.into_inner()).push(e);
}

fn modifiers() -> egui::Modifiers {
    let down = |vk: i32| unsafe { GetKeyState(vk) } < 0;
    let ctrl = down(0x11);
    egui::Modifiers { alt: down(0x12), ctrl, shift: down(0x10), mac_cmd: false, command: ctrl }
}

fn vk_to_key(vk: u32) -> Option<egui::Key> {
    use egui::Key::*;
    Some(match vk {
        0x08 => Backspace,
        0x09 => Tab,
        0x0D => Enter,
        0x1B => Escape,
        0x20 => Space,
        0x21 => PageUp,
        0x22 => PageDown,
        0x23 => End,
        0x24 => Home,
        0x25 => ArrowLeft,
        0x26 => ArrowUp,
        0x27 => ArrowRight,
        0x28 => ArrowDown,
        0x2E => Delete,
        0x31 => Num1, // 1-4: playlist vote (ui/vote.rs)
        0x32 => Num2,
        0x33 => Num3,
        0x34 => Num4,
        0x41 => A,
        0x43 => C,
        0x56 => V,
        0x58 => X,
        0x5A => Z,
        _ => return None,
    })
}

fn lparam_pos(lp: LPARAM) -> egui::Pos2 {
    egui::pos2((lp.0 & 0xFFFF) as i16 as f32, ((lp.0 >> 16) & 0xFFFF) as i16 as f32)
}

/// Window-procedure hook, called before input.rs. `Some` = handled.
pub unsafe fn window_message(_hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
    match msg {
        WM_UI_OPEN | WM_UI_CLOSE => {
            set_owner(wp.0 as u32, msg == WM_UI_OPEN);
            return Some(LRESULT(0));
        }
        settings_apply::WM_UI_DISPLAY => {
            settings_apply::apply_display(_hwnd);
            return Some(LRESULT(0));
        }
        WM_SIZE => {
            // The engine only rebuilds its render targets on message 14 (it then calls slot 2 to resize the buffers).
            if settings_apply::engine_running() && (lp.0 & 0xFFFF) != 0 {
                settings_apply::post_engine(settings_apply::ENGINE_MSG_RESIZE);
            }
            return None;
        }
        _ => {}
    }
    if !active() {
        return None;
    }
    if settings_screen::capturing() {
        if let Some(code) = binding_code(msg, wp, lp) {
            settings_screen::captured(code);
            return Some(LRESULT(0));
        }
    }
    let button = |b: egui::PointerButton, pressed: bool| {
        push(egui::Event::PointerButton { pos: lparam_pos(lp), button: b, pressed, modifiers: modifiers() });
    };
    match msg {
        WM_MOUSEMOVE => push(egui::Event::PointerMoved(lparam_pos(lp))),
        WM_LBUTTONDOWN => button(egui::PointerButton::Primary, true),
        WM_LBUTTONUP => button(egui::PointerButton::Primary, false),
        WM_RBUTTONDOWN => button(egui::PointerButton::Secondary, true),
        WM_RBUTTONUP => button(egui::PointerButton::Secondary, false),
        WM_MOUSEWHEEL => {
            let notches = ((wp.0 >> 16) & 0xFFFF) as i16 as f32 / 120.0;
            push(egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Line,
                delta: egui::vec2(0.0, notches),
                phase: egui::TouchPhase::Move,
                modifiers: modifiers(),
            });
            return Some(LRESULT(0));
        }
        WM_KEYDOWN | WM_KEYUP => {
            if let Some(key) = vk_to_key(wp.0 as u32) {
                let pressed = msg == WM_KEYDOWN;
                let m = modifiers();
                if pressed && m.ctrl && key == egui::Key::C {
                    push(egui::Event::Copy);
                } else if pressed && m.ctrl && key == egui::Key::X {
                    push(egui::Event::Cut);
                }
                push(egui::Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: m });
            }
            return Some(LRESULT(0));
        }
        WM_CHAR => {
            if let Some(c) = char::from_u32(wp.0 as u32).filter(|c| !c.is_control()) {
                push(egui::Event::Text(c.to_string()));
            }
            return Some(LRESULT(0));
        }
        _ => {}
    }
    None
}

/// Binding code (input.rs `key_name`) of a key or mouse press, for key rebinding.
fn binding_code(msg: u32, wp: WPARAM, lp: LPARAM) -> Option<u32> {
    Some(match msg {
        WM_KEYDOWN | WM_SYSKEYDOWN => {
            let ext = (lp.0 >> 24) & 1 != 0;
            match wp.0 as u32 {
                0x10 => if (lp.0 >> 16) & 0xFF == 0x36 { 0xA1 } else { 0xA0 },
                0x11 => if ext { 0xA3 } else { 0xA2 },
                0x12 => if ext { 0xA5 } else { 0xA4 },
                0x0D if ext => 0x6C, // numpad Enter
                vk => vk,
            }
        }
        WM_LBUTTONDOWN => 1,
        WM_RBUTTONDOWN => 2,
        WM_MBUTTONDOWN => 4,
        WM_XBUTTONDOWN => if (wp.0 >> 16) & 0xFFFF == 1 { 5 } else { 6 },
        WM_MOUSEWHEEL => if ((wp.0 >> 16) & 0xFFFF) as i16 > 0 { 0xAF } else { 0xAE },
        _ => return None,
    })
}

/// Pixels per point: the layout is designed for 720 lines and scales with the window height.
/// Points to pixels: the 1280 x 720 layout fitted to the back buffer (the narrower of width and height decides, so
/// 16:10 / 4:3 / windowed sizes do not cut screens off), times the player's menu size setting.
fn scale_for(w: u32, h: u32) -> f32 {
    let fit = (h as f32 / 720.0).min(w as f32 / 1280.0);
    let user = settings::get().graphics.ui_scale.clamp(50, 150) as f32 / 100.0;
    (fit * user).clamp(0.5, 3.0)
}

/// Run one egui frame and draw it into `rtv` (size `w`x`h`). `preserve_state`: swap in our own pipeline state
/// around the draw (drawing over the engine's frame).
pub unsafe fn draw(
    dc: &ID3D11DeviceContext,
    rtv: &ID3D11RenderTargetView,
    (w, h): (u32, u32),
    preserve_state: bool,
    mut build: impl FnMut(&mut egui::Ui),
) -> windows::core::Result<()> {
    let mut g = GUI.lock().unwrap_or_else(|e| e.into_inner());
    if g.is_none() {
        let device = dc.GetDevice()?;
        let renderer = egui_directx11::Renderer::new(&device)?;
        let ctx = egui::Context::default();
        theme::install(&ctx);
        log!("ui: egui renderer created on device {:?}", device.as_raw());
        settings_screen::note_gpu(&device);
        *g = Some(Gui { ctx, renderer, start: Instant::now(), state: None });
    }
    let gui = g.as_mut().unwrap();
    let ppp = scale_for(w, h);
    // Window events are in client pixels; the back buffer can be smaller or larger (rendering resolution).
    let mut cr = RECT::default();
    let to_buf = if GetClientRect(HWND(crate::host::WINDOW.load(Ordering::Relaxed) as _), &mut cr).is_ok() && cr.right > 0 {
        w as f32 / cr.right as f32
    } else {
        1.0
    };
    let ppp_in = ppp / to_buf;
    let events: Vec<egui::Event> = std::mem::take(&mut *EVENTS.lock().unwrap_or_else(|e| e.into_inner()))
        .into_iter()
        .map(|e| match e {
            egui::Event::PointerMoved(p) => egui::Event::PointerMoved(p / ppp_in),
            egui::Event::PointerButton { pos, button, pressed, modifiers } => {
                egui::Event::PointerButton { pos: pos / ppp_in, button, pressed, modifiers }
            }
            e => e,
        })
        .collect();
    let mut raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(w as f32 / ppp, h as f32 / ppp))),
        time: Some(gui.start.elapsed().as_secs_f64()),
        modifiers: modifiers(),
        events,
        focused: true,
        ..Default::default()
    };
    raw.viewports.entry(egui::ViewportId::ROOT).or_default().native_pixels_per_point = Some(ppp);
    let out = gui.ctx.run_ui(raw, |ui| build(ui));
    let (out, _, _) = egui_directx11::split_output(out);

    let mut prev = None;
    if preserve_state {
        if gui.state.is_none() {
            gui.state = new_context_state(dc);
        }
        if let (Some(s), Ok(dc1)) = (&gui.state, dc.cast::<ID3D11DeviceContext1>()) {
            dc1.SwapDeviceContextState(s, Some(&mut prev));
        }
    }
    let r = gui.renderer.render(dc, rtv, &gui.ctx, out);
    if let (Some(p), Ok(dc1)) = (prev, dc.cast::<ID3D11DeviceContext1>()) {
        dc1.SwapDeviceContextState(&p, None);
    }
    r
}

unsafe fn new_context_state(dc: &ID3D11DeviceContext) -> Option<ID3DDeviceContextState> {
    let dev: ID3D11Device1 = dc.GetDevice().ok()?.cast().ok()?;
    let level = dev.GetFeatureLevel();
    let mut state = None;
    match dev.CreateDeviceContextState(0, &[level], D3D11_SDK_VERSION, &ID3D11Device::IID, None, Some(&mut state)) {
        Ok(()) => log!("ui: own device context state created (feature level {:#x})", level.0),
        Err(e) => log!("ui: CreateDeviceContextState failed ({e}); overlay draws without saving the engine's state"),
    }
    state
}

/// Back-buffer render target view of a swap chain (made per frame, so ResizeBuffers never sees a live view).
pub unsafe fn back_buffer_rtv(
    sc: &windows::Win32::Graphics::Dxgi::IDXGISwapChain,
) -> windows::core::Result<(ID3D11RenderTargetView, (u32, u32))> {
    let tex: ID3D11Texture2D = sc.GetBuffer(0)?;
    let mut desc = D3D11_TEXTURE2D_DESC::default();
    tex.GetDesc(&mut desc);
    let dev = tex.GetDevice()?;
    let mut rtv = None;
    dev.CreateRenderTargetView(&tex, None, Some(&mut rtv))?;
    Ok((rtv.unwrap(), (desc.Width, desc.Height)))
}

/// `REACH_UI_SHOT`: comma list of steps for unattended checks (menu screens to capture, `pause`, `leave`, `play`).
pub fn shot_steps() -> Vec<String> {
    std::env::var("REACH_UI_SHOT").map(|v| v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()).unwrap_or_default()
}

/// Start a fresh reach-host showing the main menu, then exit this one. Only used with `REACH_RELAUNCH=1` or when the
/// engine could not be ended in place; normally the menu comes back in this process (seamless.rs).
pub fn relaunch_to_menu(mcc: &std::path::Path) -> ! {
    relaunch_to_menu_with(mcc, None)
}

/// As `relaunch_to_menu`, with a message the new menu shows (server browser), e.g. "The host left the game."
/// This process is terminated, not exited: a wedged engine thread must not keep the player stuck in DLL detach.
pub fn relaunch_to_menu_with(mcc: &std::path::Path, notice: Option<&str>) -> ! {
    let exe = std::env::current_exe().unwrap_or_default();
    // The new process starts a fresh reach-host.log; keep this game's log next to it.
    let _ = std::fs::copy(exe.with_file_name("reach-host.log"), exe.with_file_name("reach-host-game.log"));
    let mut cmd = std::process::Command::new(&exe);
    cmd.arg("--mcc").arg(mcc).env("REACH_UI_RELAUNCHED", "1");
    match notice {
        Some(n) => cmd.env("REACH_UI_NOTICE", n),
        None => cmd.env_remove("REACH_UI_NOTICE"),
    };
    match cmd.spawn() {
        Ok(c) => log!("ui: back to the main menu (new process {})", c.id()),
        Err(e) => log!("ui: could not start {} for the menu: {e}", exe.display()),
    }
    hard_exit(0);
}

/// End this process at once (TerminateProcess): no DLL detach, no waiting on engine threads.
pub fn hard_exit(code: u32) -> ! {
    unsafe {
        let _ = windows::Win32::System::Threading::TerminateProcess(windows::Win32::System::Threading::GetCurrentProcess(), code);
    }
    std::process::exit(code as i32);
}
