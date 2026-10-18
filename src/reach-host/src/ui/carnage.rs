//! End-of-game carnage report (Reclaimer style): standings per team (or free-for-all), each player's score, kills,
//! deaths, assists, K/D, best weapon and top medals; Firefight shows its score. The data is the host's report
//! (stats.rs, sent to every client at game over), else this machine's own. Shown as a page over the post-game for
//! `SHOW_SECS`, and as the first tab of the next-game vote page (ui/vote.rs).

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use egui::{Align2, Color32, FontId, Pos2, Rect, Stroke, StrokeKind, Vec2};

use super::scoreboard::{team_colour, TEAM_NAMES};
use super::theme::{self, ACCENT, DIM, TEXT};
use crate::log;
use crate::stats::{self, Report};

/// How long the page stays up on its own after the game ends (the vote page keeps it as a tab).
const SHOW_SECS: u64 = 20;
pub const PANEL_W: f32 = 980.0;
const ROW_H: f32 = 34.0;

static FRAMES: AtomicU32 = AtomicU32::new(0);

fn fake() -> bool {
    std::env::var_os("REACH_CARNAGE_FAKE").is_some()
}

/// The report to show as a page now (fresh, or the `REACH_CARNAGE_FAKE` sample).
pub fn current() -> Option<Report> {
    if fake() {
        return Some(stats::sample_report());
    }
    let (r, age) = stats::report()?;
    let show = age < Duration::from_secs(SHOW_SECS);
    if !show {
        FRAMES.store(0, Ordering::Relaxed);
    }
    show.then_some(r)
}

/// The report for the vote page's CARNAGE tab (any report from the last few minutes).
pub fn recent() -> Option<Report> {
    if fake() {
        return Some(stats::sample_report());
    }
    stats::report().filter(|(_, age)| *age < Duration::from_secs(300)).map(|(r, _)| r)
}

/// Full page over the post-game: dark backdrop and the panel.
pub fn page(ui: &mut egui::Ui, r: &Report) {
    let full = ui.max_rect();
    ui.painter().rect_filled(full, 0.0, Color32::from_rgba_unmultiplied(8, 10, 13, 210));
    let w = PANEL_W.min(full.width() - 32.0);
    panel(ui, r, egui::pos2(full.center().x, full.top() + 40.0), w);
}

/// Render thread, after drawing the page: `REACH_CARNAGE_SHOT` saves the 60th frame as `carnage-<report id>.bmp`.
pub fn after_draw(swapchain: *mut std::ffi::c_void, r: &Report) {
    let n = FRAMES.fetch_add(1, Ordering::Relaxed) + 1;
    if n != 60 || std::env::var_os("REACH_CARNAGE_SHOT").is_none() {
        return;
    }
    let path = std::env::current_exe().unwrap_or_default().with_file_name(format!("carnage-{}.bmp", r.id));
    match unsafe { crate::host::capture_back_buffer(swapchain, &path) } {
        Ok((w, h)) => log!("carnage: back buffer {w}x{h} saved to {}", path.display()),
        Err(e) => log!("carnage: capture failed: {e}"),
    }
}

fn text(ui: &egui::Ui, pos: Pos2, anchor: Align2, s: impl ToString, size: f32, c: Color32) -> Rect {
    ui.painter().text(pos, anchor, s.to_string(), FontId::proportional(size), c)
}

/// Column x positions from the row's left edge: numbers are centred, weapon and medals left-aligned.
const COL_SCORE: f32 = 250.0;
const COL_KILLS: f32 = 312.0;
const COL_DEATHS: f32 = 362.0;
const COL_ASSISTS: f32 = 412.0;
const COL_KD: f32 = 470.0;
const COL_WEAPON: f32 = 520.0;
const COL_MEDALS: f32 = 655.0;

fn kd(k: i16, d: i16) -> String {
    if d <= 0 {
        format!("{k}.00")
    } else {
        format!("{:.2}", k as f32 / d as f32)
    }
}

/// Headline under the title: the winning team or player.
fn headline(r: &Report) -> String {
    if r.firefight {
        let total: i32 = r.players.iter().map(|p| p.score).sum();
        return format!("FIREFIGHT  ·  TEAM SCORE {total}");
    }
    if r.teams {
        let mut ts: Vec<i8> = r.players.iter().map(|p| p.team).filter(|&t| (0..8).contains(&t)).collect();
        ts.sort();
        ts.dedup();
        ts.sort_by_key(|&t| std::cmp::Reverse(r.team_scores[t as usize]));
        return match ts.as_slice() {
            [a, b, ..] if r.team_scores[*a as usize] == r.team_scores[*b as usize] => "TIE GAME".into(),
            [a, ..] => format!("{} TEAM WINS", TEAM_NAMES[*a as usize]),
            [] => String::new(),
        };
    }
    match r.players.as_slice() {
        [a, b, ..] if a.score == b.score => "TIE GAME".into(),
        [a, ..] => format!("{} WINS", a.name.to_uppercase()),
        [] => String::new(),
    }
}

/// The report panel, `width` wide, centred under `top`.
pub fn panel(ui: &mut egui::Ui, r: &Report, top: Pos2, width: f32) {
    let me = crate::host::LOCAL_XUID.load(Ordering::Relaxed);
    super::scoreboard::fit_centered(ui, top, width, |ui| {
        theme::heading(ui, "Carnage report");
        // Columns are laid out for the full panel; a narrower one (the vote page tab) squeezes them evenly.
        let sx = (ui.available_width() / (PANEL_W - 44.0)).min(1.0);
        let (x_score, x_kills, x_deaths, x_assists, x_kd, x_weapon, x_medals) =
            (COL_SCORE * sx, COL_KILLS * sx, COL_DEATHS * sx, COL_ASSISTS * sx, COL_KD * sx, COL_WEAPON * sx, COL_MEDALS * sx);
        let game = super::vote::current_game();
        let sub = game.as_ref().map(|g| {
            let map = super::menu::map_title(&g.map).to_uppercase();
            match super::vote::variant_name(g) {
                v if v.is_empty() => map,
                v => format!("{map}  ·  {}", v.to_uppercase()),
            }
        });
        let (hr, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 26.0), egui::Sense::hover());
        let hl = text(ui, egui::pos2(hr.left(), hr.center().y), Align2::LEFT_CENTER, headline(r), 20.0, ACCENT);
        if let Some(s) = sub {
            text(ui, egui::pos2(hl.right() + 18.0, hr.center().y), Align2::LEFT_CENTER, s, 13.0, DIM);
        }
        ui.add_space(8.0);
        // Column headers.
        let (hr, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 20.0), egui::Sense::hover());
        let small = 12.5;
        text(ui, egui::pos2(hr.left() + 14.0, hr.center().y), Align2::LEFT_CENTER, "PLAYER", small, DIM);
        for (x, h) in [(x_score, "SCORE"), (x_kills, "K"), (x_deaths, "D"), (x_assists, "A"), (x_kd, "K/D")] {
            text(ui, egui::pos2(hr.left() + x, hr.center().y), Align2::CENTER_CENTER, h, small, DIM);
        }
        text(ui, egui::pos2(hr.left() + x_weapon, hr.center().y), Align2::LEFT_CENTER, "BEST WEAPON", small, DIM);
        text(ui, egui::pos2(hr.left() + x_medals, hr.center().y), Align2::LEFT_CENTER, "MEDALS (TOTAL)", small, DIM);
        ui.add_space(4.0);
        if r.players.is_empty() {
            ui.label(egui::RichText::new("No players").color(DIM));
        }
        // Groups: one per team in a team game (best team first), else everyone.
        let mut groups: Vec<(Option<i8>, Vec<&stats::Line>)> = Vec::new();
        if r.teams && !r.firefight {
            let mut ts: Vec<i8> = r.players.iter().map(|p| p.team).filter(|&t| (0..8).contains(&t)).collect();
            ts.sort();
            ts.dedup();
            ts.sort_by_key(|&t| std::cmp::Reverse(r.team_scores[t as usize]));
            for t in ts {
                groups.push((Some(t), r.players.iter().filter(|p| p.team == t).collect()));
            }
            let rest: Vec<&stats::Line> = r.players.iter().filter(|p| !(0..8).contains(&p.team)).collect();
            if !rest.is_empty() {
                groups.push((None, rest));
            }
        } else {
            groups.push((None, r.players.iter().collect()));
        }
        for (gi, (team, rows)) in groups.iter().enumerate() {
            if gi > 0 {
                ui.add_space(10.0);
            }
            if let Some(t) = *team {
                let (hr, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 26.0), egui::Sense::hover());
                let c = team_colour(t);
                let place = ["1ST", "2ND", "3RD", "4TH", "5TH", "6TH", "7TH", "8TH"][gi.min(7)];
                text(ui, egui::pos2(hr.left() + 14.0, hr.center().y), Align2::LEFT_CENTER, format!("{place}  {} TEAM", TEAM_NAMES[t as usize]), 17.0, c);
                text(ui, egui::pos2(hr.left() + x_score, hr.center().y), Align2::CENTER_CENTER, r.team_scores[t as usize], 17.0, c);
                ui.add_space(2.0);
            }
            for (k, p) in rows.iter().enumerate() {
                let (row, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW_H), egui::Sense::hover());
                let fill = match team {
                    Some(t) => team_colour(*t).gamma_multiply(0.85),
                    None => Color32::from_rgba_unmultiplied(60, 66, 76, 200),
                };
                let pt = ui.painter();
                pt.rect_filled(row, 2.0, fill);
                if p.xuid == me {
                    pt.rect_stroke(row.shrink(1.0), 2.0, Stroke::new(2.0, Color32::from_rgb(232, 190, 70)), StrokeKind::Inside);
                }
                let y = row.center().y;
                let x0 = row.left();
                if team.is_none() && !r.firefight {
                    text(ui, egui::pos2(x0 + 14.0, y), Align2::LEFT_CENTER, k + 1, 14.0, DIM);
                }
                let nx = if team.is_none() && !r.firefight { 40.0 } else { 14.0 };
                text(ui, egui::pos2(x0 + nx, y), Align2::LEFT_CENTER, &p.name, 18.0, TEXT);
                let num = |x: f32, v: String| text(ui, egui::pos2(x0 + x, y), Align2::CENTER_CENTER, v, 18.0, TEXT);
                num(x_score, p.score.to_string());
                num(x_kills, p.kills.to_string());
                num(x_deaths, p.deaths.to_string());
                num(x_assists, p.assists.to_string());
                num(x_kd, kd(p.kills, p.deaths));
                let weapon = p.weapon.map_or("—".to_string(), |(i, n)| format!("{} ({n})", stats::WEAPON_NAMES.get(i as usize).unwrap_or(&"?")));
                text(ui, egui::pos2(x0 + x_weapon, y), Align2::LEFT_CENTER, weapon, 13.5, TEXT);
                let medals = if p.medals.is_empty() {
                    "—".to_string()
                } else {
                    let list: Vec<String> = p.medals.iter().take(2).map(|(i, c)| {
                        let n = stats::medal_name(*i as usize);
                        if *c > 1 { format!("{n} ×{c}") } else { n }
                    }).collect();
                    let more = p.medals.len().saturating_sub(2);
                    let list = if more > 0 { format!("{}  +{more}", list.join(" · ")) } else { list.join(" · ") };
                    format!("({})  {list}", p.medal_total)
                };
                let clip = Rect::from_min_max(egui::pos2(x0 + x_medals, row.top()), egui::pos2(row.right() - 10.0, row.bottom()));
                let g = ui.painter().layout_no_wrap(medals, FontId::proportional(13.0), TEXT);
                let at = egui::pos2(clip.left(), y - g.size().y / 2.0);
                ui.painter().with_clip_rect(clip).galley(at, g, TEXT);
                ui.add_space(2.0);
            }
        }
        ui.add_space(8.0);
        let note = if r.firefight { "Score from the Firefight scoring" } else { "Medal counts from the host's game results" };
        ui.label(egui::RichText::new(note).size(12.0).color(DIM));
    });
}
