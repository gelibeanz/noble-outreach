//! In-game pause overlay. Esc reaches the engine as its pause action, which calls host slot 7 and draws nothing
//! itself; we open RESUME / SETTINGS / LEAVE GAME and draw it over the engine's finished frame in host slot 1
//! (end_frame, render thread, right before the engine presents). The game keeps running underneath (it is
//! multiplayer); only player 0's input is withheld while the overlay is up (input.rs).

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Instant;

use egui::{Rect, Vec2};
use windows::core::Interface;
use windows::Win32::Graphics::Dxgi::IDXGISwapChain;

use super::settings_screen::SettingsScreen;
use super::theme::{self, DIM};
use crate::log;

/// Engine object and its post-message function (vtable slot 3), set by main once the game is initialised.
pub static ENGINE: AtomicUsize = AtomicUsize::new(0);
pub static POST: AtomicUsize = AtomicUsize::new(0);
/// Engine message 13: quit (the game thread winds down and exits).
const ENGINE_MSG_QUIT: u32 = 13;

static OPEN: AtomicBool = AtomicBool::new(false);
static LEAVE_AT: Mutex<Option<Instant>> = Mutex::new(None);
static FRAMES: AtomicU32 = AtomicU32::new(0);
static OPENS: AtomicU32 = AtomicU32::new(0);

#[derive(Clone, Copy, PartialEq)]
enum Page {
    Main,
    Settings,
}

struct State {
    page: Page,
    settings: Option<SettingsScreen>,
}

static STATE: Mutex<State> = Mutex::new(State { page: Page::Main, settings: None });

/// Host slot 7 (`pause_game(u32 controller)`): the engine's pause action (Esc / Start) fired.
pub unsafe extern "system" fn slot_pause_game(_this: *mut c_void, controller: u32) {
    log!("host slot 7 pause_game(controller {controller}) -> pause overlay");
    if crate::host::DEDICATED.load(Ordering::Relaxed) || LEAVE_AT.lock().unwrap_or_else(|e| e.into_inner()).is_some() {
        return;
    }
    if !OPEN.swap(true, Ordering::Relaxed) {
        FRAMES.store(0, Ordering::Relaxed);
        OPENS.fetch_add(1, Ordering::Relaxed);
        *STATE.lock().unwrap_or_else(|e| e.into_inner()) = State { page: Page::Main, settings: None };
        super::post_to_window(super::WM_UI_OPEN);
    }
}

fn close() {
    if OPEN.swap(false, Ordering::Relaxed) {
        super::post_to_window(super::WM_UI_CLOSE);
    }
}

/// When the player chose LEAVE GAME (main loop then says bye to the peers and returns to the menu at once).
pub fn leave_requested() -> Option<Instant> {
    *LEAVE_AT.lock().unwrap_or_else(|e| e.into_inner())
}

fn leave() {
    let mut at = LEAVE_AT.lock().unwrap_or_else(|e| e.into_inner());
    if at.is_some() {
        return;
    }
    *at = Some(Instant::now());
    let (engine, post) = (ENGINE.load(Ordering::Relaxed), POST.load(Ordering::Relaxed));
    if engine != 0 && post != 0 {
        let post: unsafe extern "system" fn(*mut c_void, u32, *const u8) -> *mut c_void = unsafe { std::mem::transmute(post) };
        unsafe { post(engine as *mut c_void, ENGINE_MSG_QUIT, std::ptr::null()) };
        log!("pause overlay: leave game, posted engine message {ENGINE_MSG_QUIT} (quit)");
    } else {
        log!("pause overlay: leave game (no engine to message)");
    }
}

/// Host slot 1 hook: draw the overlay over the engine's frame if it is open.
pub unsafe fn end_frame(swapchain: *mut c_void) {
    if !OPEN.load(Ordering::Relaxed) {
        passive_frame(swapchain);
        return;
    }
    let Some(sc) = IDXGISwapChain::from_raw_borrowed(&swapchain) else { return };
    let (rtv, size) = match super::back_buffer_rtv(sc) {
        Ok(v) => v,
        Err(e) => {
            log!("pause overlay: no back buffer: {e}");
            return;
        }
    };
    let Ok(dc) = rtv.GetDevice().and_then(|d| d.GetImmediateContext()) else { return };
    let mut st = STATE.lock().unwrap_or_else(|e| e.into_inner());
    if FRAMES.load(Ordering::Relaxed) == 0 && super::shot_steps().iter().any(|s| s == "pause-settings") {
        st.page = Page::Settings; // REACH_UI_SHOT: capture the in-game settings screen
    }
    if let Err(e) = super::draw(&dc, &rtv, size, true, |ui| draw_ui(ui, &mut st)) {
        log!("pause overlay: draw failed: {e}");
    }
    drop(st);
    drop(rtv);

    // Unattended check (REACH_UI_SHOT=...,pause[,resume][,leave]): capture the overlay; the n-th opening then
    // takes the n-th of the listed resume/leave actions.
    let n = FRAMES.fetch_add(1, Ordering::Relaxed) + 1;
    let steps = super::shot_steps();
    if n == 20 && steps.iter().any(|s| s == "pause") {
        let path = std::env::current_exe().unwrap_or_default().with_file_name("ui-pause.bmp");
        match crate::host::capture_back_buffer(swapchain, &path) {
            Ok((w, h)) => log!("pause overlay: back buffer {w}x{h} saved to {}", path.display()),
            Err(e) => log!("pause overlay: capture failed: {e}"),
        }
    }
    if n == 30 {
        let actions: Vec<&String> = steps.iter().filter(|s| *s == "resume" || *s == "leave").collect();
        match actions.get(OPENS.load(Ordering::Relaxed) as usize - 1).map(|s| s.as_str()) {
            Some("resume") => {
                log!("pause overlay: resume (REACH_UI_SHOT)");
                close();
            }
            Some("leave") => leave(),
            _ => {}
        }
    }
}

/// Overlay closed: draw the non-interactive layers (listen-server address banner / join toast, Tab scoreboard).
unsafe fn passive_frame(swapchain: *mut c_void) {
    let hosting = super::hostinfo::hosting();
    if hosting {
        let names: Vec<String> = super::scoreboard::rows().into_iter().map(|r| r.name).collect();
        super::hostinfo::note_players(&names);
    }
    let banner = if hosting { super::hostinfo::banner_alpha() } else { 0.0 };
    let toast = if hosting { super::hostinfo::toast() } else { None };
    let scores = super::scoreboard::visible();
    if !scores {
        super::scoreboard::hidden();
    }
    let vote = super::vote::current();
    let nat = crate::nat::client_status(); // joiner: "Connected to the host (NAT punch)" etc.
    let lobby = crate::lobby::waiting_view(); // listen campaign host gathering friends
    let forge_saved = crate::forge::last_status().is_some(); // "Saved '<name>'" toast after a Forge save
    if banner <= 0.0 && toast.is_none() && !scores && vote.is_none() && nat.is_none() && lobby.is_none() && !forge_saved {
        return;
    }
    let Some(sc) = IDXGISwapChain::from_raw_borrowed(&swapchain) else { return };
    let Ok((rtv, size)) = super::back_buffer_rtv(sc) else { return };
    let Ok(dc) = rtv.GetDevice().and_then(|d| d.GetImmediateContext()) else { return };
    if let Err(e) = super::draw(&dc, &rtv, size, true, |ui| {
        if banner > 0.0 {
            host_banner(ui, banner);
        }
        if let Some((name, a)) = &toast {
            join_toast(ui, name, *a, if banner > 0.0 { 184.0 } else { 16.0 });
        }
        if scores {
            super::scoreboard::draw(ui);
        }
        if let Some(v) = &vote {
            super::vote::draw(ui, v);
        }
        if let Some((text, a)) = &nat {
            nat_status(ui, text, *a);
        }
        if let Some((n, max, secs)) = lobby {
            super::campaign::lobby_panel(ui, n, max, secs);
        }
        if forge_saved {
            super::forge::save_toast(ui);
        }
    }) {
        log!("overlay: passive draw failed: {e}");
    }
    if let Some(v) = &vote {
        super::vote::after_draw(swapchain, v);
    }
}

/// Listen server: where friends can reach us (top right, ~10 s after spawning, fading out).
fn host_banner(ui: &mut egui::Ui, alpha: f32) {
    let Some((lines, hint)) = super::hostinfo::lines() else { return };
    let full = ui.max_rect();
    let r = Rect::from_min_size(egui::pos2(full.right() - 440.0, full.top() + 16.0), Vec2::new(424.0, 76.0 + 20.0 * lines.len() as f32));
    ui.scope(|ui| {
        ui.set_opacity(alpha);
        theme::panel(ui, r, |ui| host_lines(ui, &lines, &hint));
    });
}

/// "<name> joined" for 5 s (top right, below the banner while that is up).
fn join_toast(ui: &mut egui::Ui, name: &str, alpha: f32, top: f32) {
    let full = ui.max_rect();
    let r = Rect::from_min_size(egui::pos2(full.right() - 340.0, full.top() + top), Vec2::new(324.0, 64.0));
    ui.scope(|ui| {
        ui.set_opacity(alpha);
        theme::panel(ui, r, |ui| {
            ui.label(egui::RichText::new(format!("{name} joined")).size(17.0).color(theme::ACCENT));
        });
    });
}

/// Joiner connection status (nat.rs), bottom left, while connecting and 5 s after.
fn nat_status(ui: &mut egui::Ui, text: &str, alpha: f32) {
    let full = ui.max_rect();
    let r = Rect::from_min_size(egui::pos2(full.left() + 16.0, full.bottom() - 64.0), Vec2::new(460.0, 48.0));
    ui.scope(|ui| {
        ui.set_opacity(alpha);
        theme::panel(ui, r, |ui| {
            ui.label(egui::RichText::new(text).size(14.0));
        });
    });
}

fn host_lines(ui: &mut egui::Ui, lines: &[String], hint: &str) {
    for (i, l) in lines.iter().enumerate() {
        let t = egui::RichText::new(l).size(if i == 0 { 15.0 } else { 14.0 });
        ui.label(if i == 0 { t.color(theme::ACCENT) } else { t });
    }
    ui.add_space(4.0);
    ui.label(egui::RichText::new(hint).size(11.5).color(DIM));
}

fn draw_ui(ui: &mut egui::Ui, st: &mut State) {
    let full = ui.max_rect();
    ui.painter().rect_filled(full, 0.0, egui::Color32::from_black_alpha(150));
    if st.page == Page::Settings && leave_requested().is_none() {
        ui.painter().rect_filled(full, 0.0, egui::Color32::from_black_alpha(90));
        if st.settings.get_or_insert_with(|| SettingsScreen::new(true)).ui(ui) {
            st.settings = None;
            st.page = Page::Main;
        }
        return;
    }
    let esc = ui.input(|i| i.key_pressed(egui::Key::Escape));
    let leaving = leave_requested().is_some();
    let left = Rect::from_min_size(full.min + Vec2::new(48.0, 48.0), Vec2::new(340.0, full.height() - 96.0));
    theme::panel(ui, left, |ui| {
        theme::heading(ui, "Paused");
        ui.add_space(16.0);
        if leaving {
            ui.label(egui::RichText::new("LEAVING GAME ...").size(20.0).color(DIM));
            return;
        }
        if theme::nav_item(ui, "Resume", false).clicked() || (esc && st.page == Page::Main) {
            close();
        }
        if theme::nav_item(ui, "Settings", st.page == Page::Settings).clicked() {
            st.page = Page::Settings;
        }
        ui.add_space(14.0);
        if theme::nav_item(ui, "Leave game", false).clicked() {
            leave();
        }
        super::forge::pause_section(ui); // Forge game: name + SAVE MAP
        if let Some((lines, hint)) = super::hostinfo::lines() {
            ui.add_space(24.0);
            host_lines(ui, &lines, &hint);
        }
        ui.add_space((ui.available_height() - 20.0).max(0.0));
        ui.label(egui::RichText::new("Esc to resume").size(13.0).color(DIM));
    });
}
