//! Applying settings: to input.rs at once, to the window (window thread), and to a running engine by rewriting the
//! slot 34 profile and posting engine message 5, which re-reads the video settings (slot 32: vsync, frame cap) and
//! marks every local player's settings dirty so the engine re-applies the profile (FOV, controls, bindings).

use std::sync::atomic::{AtomicBool, Ordering};

use windows::core::Interface;
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Dxgi::IDXGISwapChain1;
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST};
use windows::Win32::UI::WindowsAndMessaging::*;

use super::settings::Settings;
use crate::log;

/// Engine message 5: re-read video settings and re-apply local players' profile settings.
const ENGINE_MSG_SETTINGS: u32 = 5;
/// Engine message 14: the window size changed (engine calls slot 2, then rebuilds its render targets).
pub const ENGINE_MSG_RESIZE: u32 = 14;

/// Posted to the window: apply the display settings (style, size, fullscreen) on the window thread.
pub const WM_UI_DISPLAY: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 0x53;

static FULLSCREEN: AtomicBool = AtomicBool::new(false);

pub fn engine_running() -> bool {
    super::overlay::ENGINE.load(Ordering::Relaxed) != 0
}

pub fn post_engine(msg: u32) -> bool {
    let (engine, post) = (super::overlay::ENGINE.load(Ordering::Relaxed), super::overlay::POST.load(Ordering::Relaxed));
    if engine == 0 || post == 0 {
        return false;
    }
    let post: unsafe extern "system" fn(*mut std::ffi::c_void, u32, *const u8) -> *mut std::ffi::c_void = unsafe { std::mem::transmute(post) };
    unsafe { post(engine as *mut _, msg, std::ptr::null()) };
    true
}

/// Apply everything that changed between `old` and `new`.
pub fn apply(old: &Settings, new: &Settings) {
    if old.mouse_sens != new.mouse_sens {
        crate::input::set_mouse_sens(new.mouse_sens as f64);
    }
    crate::input::configure(new);
    let display = (old.graphics.display_mode, old.graphics.window_w, old.graphics.window_h, old.graphics.render_scale)
        != (new.graphics.display_mode, new.graphics.window_w, new.graphics.window_h, new.graphics.render_scale);
    if display {
        super::post_to_window(WM_UI_DISPLAY);
    }
    if !engine_running() {
        return;
    }
    let profile = (&old.game, &old.audio, &old.pad, &old.mouse, &old.armour) != (&new.game, &new.audio, &new.pad, &new.mouse, &new.armour);
    let video = (old.graphics.vsync, old.graphics.fps_cap) != (new.graphics.vsync, new.graphics.fps_cap);
    if profile {
        let p = crate::host::profile_ptr();
        unsafe {
            crate::profile::write_settings(p, new);
            if old.mouse.bindings != new.mouse.bindings {
                crate::profile::nudge_binding_rebuild(p);
            }
        }
    }
    if old.audio != new.audio {
        // Msg 5 alone does not re-read the profile volumes (only a changed control field does): set them directly.
        crate::audio::apply_volume(&new.audio);
    }
    if profile || video {
        post_engine(ENGINE_MSG_SETTINGS);
        log!("settings: applied to the running game (engine message {ENGINE_MSG_SETTINGS})");
    }
}

/// At start-up (window thread, before the menu or engine): bring the window to the saved display mode. Skipped
/// for scripted, hidden and dedicated runs and when `--size` is given.
pub fn startup(hwnd: HWND) {
    let skip = std::env::args().any(|a| matches!(a.as_str(), "--size" | "--hidden" | "--dedicated" | "--test-input" | "--no-launch"));
    if !skip && unsafe { IsWindowVisible(hwnd) }.as_bool() {
        // Posted: the swap chain (needed for exclusive fullscreen) is created after this.
        unsafe {
            let _ = PostMessageW(Some(hwnd), WM_UI_DISPLAY, Default::default(), Default::default());
        }
    }
}

/// Window thread: window style and size for the display mode, exclusive fullscreen on the swap chain.
pub unsafe fn apply_display(hwnd: HWND) {
    let g = super::settings::get().graphics;
    let sc_raw = crate::host::SWAPCHAIN.load(Ordering::Relaxed) as *mut std::ffi::c_void;
    let sc = IDXGISwapChain1::from_raw_borrowed(&sc_raw);
    if g.display_mode != 2 && FULLSCREEN.swap(false, Ordering::Relaxed) {
        if let Some(sc) = sc {
            let _ = sc.SetFullscreenState(false, None);
        }
    }
    let mon = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
    let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
    let _ = GetMonitorInfoW(mon, &mut mi);
    let m = mi.rcMonitor;
    if g.display_mode == 0 {
        let style = WS_OVERLAPPEDWINDOW | WS_VISIBLE;
        SetWindowLongPtrW(hwnd, GWL_STYLE, style.0 as isize);
        let mut r = RECT { left: 0, top: 0, right: g.window_w as i32, bottom: g.window_h as i32 };
        let _ = AdjustWindowRect(&mut r, style, false);
        let (w, h) = (r.right - r.left, r.bottom - r.top);
        let x = m.left + ((m.right - m.left - w) / 2).max(0);
        let y = m.top + ((m.bottom - m.top - h) / 2).max(0);
        let _ = SetWindowPos(hwnd, None, x, y, w, h, SWP_FRAMECHANGED | SWP_NOZORDER);
    } else {
        SetWindowLongPtrW(hwnd, GWL_STYLE, (WS_POPUP | WS_VISIBLE).0 as isize);
        let _ = SetWindowPos(hwnd, Some(HWND_TOP), m.left, m.top, m.right - m.left, m.bottom - m.top, SWP_FRAMECHANGED);
        if g.display_mode == 2 {
            if let Some(sc) = sc {
                match sc.SetFullscreenState(true, None) {
                    Ok(()) => FULLSCREEN.store(true, Ordering::Relaxed),
                    Err(e) => log!("settings: exclusive fullscreen refused ({e}); staying borderless"),
                }
            }
        }
    }
    log!("settings: display mode {} ({}x{} window, {} % rendering)", g.display_mode, g.window_w, g.window_h, g.render_scale);
    if engine_running() {
        post_engine(ENGINE_MSG_RESIZE);
    }
}
