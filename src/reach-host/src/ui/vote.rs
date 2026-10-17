//! Playlist vote panel (client side of vote.rs), drawn as a passive overlay layer in host slot 1. Reclaimer-like: the
//! 3 offered games plus "None of the above", each with its live tally, and the countdown. Keys 1-4 vote at any time
//! (input.rs hands them here while a vote is open); when no player unit exists (pre-game, loading, dead) the panel
//! also takes the mouse (click a row) and the pad (d-pad up/down + A).

use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU32, Ordering};

use egui::{Align2, Color32, FontId, Rect, Sense, Vec2};

use super::theme::{self, ACCENT, DIM, TEXT};
use crate::log;
use crate::vote::{self, View, NONE};

/// We made the UI active (cursor shown) for the vote; give it back when the vote ends.
static INTERACTIVE: AtomicBool = AtomicBool::new(false);
/// Pad selection (row index) and last pad buttons, for edge detection.
static SEL: AtomicU32 = AtomicU32::new(0);
static PAD_PREV: AtomicU16 = AtomicU16::new(0);
static FRAMES: AtomicU32 = AtomicU32::new(0);
/// Round id the player hid with Backspace (0 = none): the panel stays hidden until the next round or the result.
static HIDDEN: AtomicU16 = AtomicU16::new(0);

/// input.rs: Backspace while a vote is open hides the panel for this round (voting with 1-4 still works).
/// True = consumed.
pub fn hide_key() -> bool {
    let Some(v) = vote::view() else { return false };
    let Some(o) = &v.open else { return false };
    HIDDEN.store(o.id, Ordering::Relaxed);
    log!("vote: panel hidden for round {}", o.id);
    true
}

/// input.rs: digit key `n` (0 = key 1) pressed while the game has the keyboard. True = consumed (a vote is open).
pub fn key(n: u8) -> bool {
    vote::cast(n)
}

fn mode_title(o: &vote::Opt) -> String {
    match o.mode {
        crate::query::MODE_FIREFIGHT => "Firefight".into(),
        crate::query::MODE_CAMPAIGN => "Campaign".into(),
        _ => super::menu::variant_title(&o.game_variant),
    }
}

/// Is a player unit alive under us? (No unit = pre-game / loading / dead: the panel may take the mouse and pad.)
fn in_game() -> bool {
    crate::input::player0_alive() // render thread: player0_view only works on the game thread
}

/// Passive layer (overlay.rs): current vote view, if any; also switches the cursor on/off for the panel.
pub fn current() -> Option<View> {
    let v = vote::view();
    let want = v.as_ref().is_some_and(|v| v.open.is_some()) && !in_game();
    if INTERACTIVE.swap(want, Ordering::Relaxed) != want {
        super::post_to_window(if want { super::WM_UI_OPEN } else { super::WM_UI_CLOSE });
    }
    if v.is_none() {
        FRAMES.store(0, Ordering::Relaxed);
    }
    v
}

/// Render-thread: after drawing, the 90th frame of a vote panel is captured when `REACH_VOTE_SHOT` is set.
pub fn after_draw(swapchain: *mut std::ffi::c_void, v: &View) {
    let n = FRAMES.fetch_add(1, Ordering::Relaxed) + 1;
    if std::env::var_os("REACH_VOTE_SHOT").is_none() {
        return;
    }
    let name = if v.open.is_some() && n == 90 {
        "vote-panel.bmp"
    } else if v.open.is_none() && v.result.is_some() && n % 1000 == 30 {
        "vote-result.bmp"
    } else {
        return;
    };
    let path = std::env::current_exe().unwrap_or_default().with_file_name(name);
    match unsafe { crate::host::capture_back_buffer(swapchain, &path) } {
        Ok((w, h)) => log!("vote: back buffer {w}x{h} saved to {}", path.display()),
        Err(e) => log!("vote: capture failed: {e}"),
    }
}

/// Pad: d-pad up/down moves the selection, A votes for it. Only while the panel is interactive.
fn poll_pad(rows: u32) {
    use windows::Win32::UI::Input::XboxController::{XInputGetState, XINPUT_STATE};
    if !INTERACTIVE.load(Ordering::Relaxed) || rows == 0 {
        return;
    }
    let mut b = 0u16;
    for i in 0..4 {
        let mut s = XINPUT_STATE::default();
        if unsafe { XInputGetState(i, &mut s) } == 0 {
            b = s.Gamepad.wButtons.0;
            break;
        }
    }
    let pressed = b & !PAD_PREV.swap(b, Ordering::Relaxed);
    let sel = SEL.load(Ordering::Relaxed).min(rows - 1);
    if pressed & 0x0001 != 0 {
        SEL.store(sel.saturating_sub(1), Ordering::Relaxed);
    } else if pressed & 0x0002 != 0 {
        SEL.store((sel + 1).min(rows - 1), Ordering::Relaxed);
    } else if pressed & 0x1000 != 0 {
        vote::cast(if sel == 3 { NONE } else { sel as u8 });
    }
}

pub fn draw(ui: &mut egui::Ui, v: &View) {
    let full = ui.max_rect();
    let w = 560.0f32.min(full.width() - 32.0);
    if let Some(o) = v.open.as_ref().filter(|o| HIDDEN.load(Ordering::Relaxed) == o.id) {
        let _ = o; // hidden with Backspace: nothing until the result
    } else if let Some(o) = v.open.as_ref().filter(|_| v.mine.is_some() && in_game()) {
        // Voted mid-game: a small bar so the match stays visible (1-4 still change the vote).
        let voted: u32 = o.tally.iter().map(|&t| t as u32).sum();
        let mine = v.mine.unwrap_or(0);
        let pick = if mine == NONE { if o.keep { "Keep playing".to_string() } else { "None of the above".to_string() } } else { o.options.get(mine as usize).map(|p| p.name.clone()).unwrap_or_default() };
        theme::panel_fit(ui, egui::pos2(full.center().x, full.top() + 16.0), egui::Align::Center, w, |ui| {
            ui.label(egui::RichText::new(format!("VOTE  ·  {} s  ·  your pick: {pick}  ·  {voted} of {} voted", o.secs_left, o.voters.max(voted as u8))).size(14.0).color(TEXT));
            ui.label(egui::RichText::new("1-4 change your vote  ·  Backspace hides").size(11.5).color(DIM));
        });
    } else if let Some(o) = &v.open {
        let rows = o.options.len() + o.none_allowed as usize;
        poll_pad(rows as u32);
        for k in 0..4u8 {
            let key = [egui::Key::Num1, egui::Key::Num2, egui::Key::Num3, egui::Key::Num4][k as usize];
            if ui.input(|i| i.key_pressed(key)) {
                vote::cast(k);
            }
        }
        // Rows shrink on small windows so the whole panel fits.
        // Heading, subtitle, hint and padding take ~170 points; the panel itself grows to fit (theme::panel_fit).
        let row_h = ((full.height() - 190.0) / rows as f32 - 6.0).clamp(30.0, 52.0);
        let h = 170.0 + (row_h + 6.0) * rows as f32;
        let top = (full.height() - h).clamp(0.0, 80.0) / 2.0;
        theme::panel_fit(ui, egui::pos2(full.center().x, full.top() + top), egui::Align::Center, w, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("VOTE FOR THE NEXT GAME").size(19.0).color(ACCENT).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(egui::RichText::new(format!("{} s", o.secs_left)).size(19.0).color(TEXT).strong());
                });
            });
            let voted: u32 = o.tally.iter().map(|&t| t as u32).sum();
            let sub = if o.playlist.is_empty() { String::new() } else { format!("{} playlist  ·  ", o.playlist) };
            ui.label(egui::RichText::new(format!("{sub}{voted} of {} voted", o.voters.max(voted as u8))).size(13.0).color(DIM));
            ui.add_space(8.0);
            let sel = SEL.load(Ordering::Relaxed);
            for k in 0..rows {
                let choice = if k == 3 { NONE } else { k as u8 };
                let (title, detail) = match o.options.get(k) {
                    Some(opt) if k < 3 => (opt.name.clone(), format!("{}  ·  {}", super::menu::map_title(&opt.map), mode_title(opt))),
                    _ if o.keep => ("Keep playing this game".to_string(), "Cancel the vote and carry on".to_string()),
                    _ => ("None of the above".to_string(), "Re-roll three new games (once)".to_string()),
                };
                let count = o.tally[choice as usize] as f32;
                let mine = v.mine == Some(choice);
                let (row, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), row_h), Sense::click());
                if resp.clicked() {
                    vote::cast(choice);
                }
                let p = ui.painter();
                let hl = resp.hovered() || (INTERACTIVE.load(Ordering::Relaxed) && sel as usize == k);
                p.rect_filled(row, 4.0, if mine { Color32::from_rgba_unmultiplied(242, 140, 40, 60) } else if hl { theme::HOVER } else { Color32::from_black_alpha(60) });
                // Tally bar along the bottom of the row.
                let frac = if voted > 0 { count / voted as f32 } else { 0.0 };
                p.rect_filled(Rect::from_min_size(egui::pos2(row.left(), row.bottom() - 4.0), Vec2::new(row.width() * frac, 4.0)), 0.0, ACCENT);
                p.text(egui::pos2(row.left() + 14.0, row.center().y), Align2::LEFT_CENTER, format!("{}", k + 1), FontId::proportional(22.0), if mine { ACCENT } else { DIM });
                let f = row_h / 52.0;
                p.text(egui::pos2(row.left() + 46.0, row.top() + 15.0 * f), Align2::LEFT_CENTER, &title, FontId::proportional(17.0 * f.max(0.8)), TEXT);
                p.text(egui::pos2(row.left() + 46.0, row.top() + 36.0 * f), Align2::LEFT_CENTER, &detail, FontId::proportional(13.0 * f.max(0.8)), DIM);
                p.text(egui::pos2(row.right() - 14.0, row.center().y), Align2::RIGHT_CENTER, format!("{}", count as u32), FontId::proportional(22.0), if count > 0.0 { TEXT } else { DIM });
                ui.add_space(6.0);
            }
            ui.label(egui::RichText::new(if INTERACTIVE.load(Ordering::Relaxed) { "Keys 1-4, click, or d-pad + A" } else { "Press 1-4 to vote  ·  Backspace hides" }).size(12.0).color(DIM));
        });
    } else if let Some((res, _)) = &v.result {
        theme::panel_fit(ui, egui::pos2(full.center().x, full.top() + 40.0), egui::Align::Center, w, |ui| {
            ui.label(egui::RichText::new("NEXT GAME").size(15.0).color(DIM));
            ui.label(egui::RichText::new(&res.opt.name).size(22.0).color(ACCENT).strong());
            ui.label(egui::RichText::new(format!("{}  ·  {}", super::menu::map_title(&res.opt.map), mode_title(&res.opt))).size(14.0).color(TEXT));
            let note = if vote::following() { "Server restarting, rejoining when it is back ..." } else if res.restart { "Server restarting ..." } else if res.winner == NONE { "Vote cancelled, carrying on" } else { "Already playing it, carrying on" };
            ui.label(egui::RichText::new(note).size(13.0).color(DIM));
        });
    } else if let Some(text) = vote::call_notice() {
        theme::panel_fit(ui, egui::pos2(full.center().x, full.top() + 40.0), egui::Align::Center, w, |ui| {
            ui.label(egui::RichText::new(text).size(15.0).color(TEXT));
        });
    }
}
