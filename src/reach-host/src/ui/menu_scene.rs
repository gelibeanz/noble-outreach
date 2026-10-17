//! Main-menu background, like Reclaimer's live menu scene. `Settings.graphics.live_menu_scene` on: the preview
//! engine (preview.rs, started with the menu) shows the player's own Spartan on Spire with a slow cinematic camera,
//! full screen behind the menu. Until its first frame arrives (~20 s after launch), and with the setting off: still
//! art from the player's install (ui/mccart.rs) with a slow pan, crossfading to the live frames when they come.
//! The left side is darkened so the menu column stays readable. On CUSTOMIZE the same engine switches to the
//! customize camera, so the change is instant.

use std::path::Path;

use egui::{Color32, Rect, Ui};

use super::mccart;
use super::settings::Settings;

/// Still art, in order of preference: wide (668x292) loading-screen shots in the levels pack.
const ART: &[&str] = &["HREACHMP_SPIRELARGE", "HREACHMP_FORGEWORLDLARGE", "HREACH_C05_TIPOFTHESPEAR_LARGE"];

/// Cinematic camera: wider than the customize view, low, swinging slowly ±0.5 rad around the front over 90 s.
const FOV: u32 = 68;
const PITCH: f32 = 0.04;
const DIST: f32 = 3.4;
const YAW: f32 = 1.0;
const SWING: f32 = 0.5;
const SWING_SECS: f64 = 90.0;
/// Scale of the live frame on screen (its centre lands at ZOOM / 2 of the width).
const ZOOM: f32 = 1.2;
/// Seconds the still art takes to fade into the live frames.
const FADE_SECS: f64 = 1.5;

/// When the live frames started showing (egui time), for the crossfade.
static LIVE_SINCE: std::sync::Mutex<Option<f64>> = std::sync::Mutex::new(None);

/// Draw the background over `rect` (call right after `theme::backdrop`).
pub fn draw(ui: &Ui, rect: Rect, mcc: &Path, s: &Settings) {
    let ctx = ui.ctx().clone();
    let t = ui.input(|i| i.time);
    let p = ui.painter();
    mccart::init(mcc);
    if let Some(tex) = ART.iter().find_map(|n| mccart::texture(&ctx, n)) {
        // Cover the screen, panning slowly across the art's extra width and back (2 minutes).
        let pan = 0.5 + 0.5 * (t * std::f64::consts::TAU / 120.0).sin() as f32;
        p.image(tex.id(), rect, cover_uv(tex.size_vec2(), rect, pan), Color32::from_gray(200));
    }
    let mut live = None;
    if s.graphics.live_menu_scene {
        crate::preview::set_armour(&s.armour);
        let yaw = YAW + SWING * (t * std::f64::consts::TAU / SWING_SECS).sin() as f32;
        crate::preview::set_shot(yaw, PITCH, if s.armour.elite { DIST * 1.25 } else { DIST }, FOV);
        if let crate::preview::Frame::Live(tex, size) = crate::preview::frame(&ctx) {
            if !crate::preview::switching() {
                live = Some((tex, size));
            }
        }
    }
    let mut since = LIVE_SINCE.lock().unwrap_or_else(|e| e.into_inner());
    match live {
        Some((tex, size)) => {
            let a = ((t - *since.get_or_insert(t)) / FADE_SECS).clamp(0.0, 1.0) as f32;
            // Enlarged and anchored left, so the Spartan (centre of the frame) stands right of the menu column.
            let big = Rect::from_min_size(egui::pos2(rect.left(), rect.top() - rect.height() * (ZOOM - 1.0) / 2.0), rect.size() * ZOOM);
            p.image(tex, big, cover_uv(size, big, 0.5), Color32::from_white_alpha((a * 255.0) as u8));
        }
        None => *since = None,
    }
    // Readability: a light overall dim, a dark band behind the menu column fading out to the right, and a
    // darker floor for the bottom text.
    p.rect_filled(rect, 0.0, Color32::from_black_alpha(40));
    let band = Rect::from_min_max(rect.min, egui::pos2(rect.left() + rect.width() * 0.62, rect.bottom()));
    gradient(p, band, Color32::from_rgba_unmultiplied(6, 9, 14, 225), Color32::TRANSPARENT, true);
    let floor = Rect::from_min_max(egui::pos2(rect.left(), rect.bottom() - rect.height() * 0.3), rect.max);
    gradient(p, floor, Color32::TRANSPARENT, Color32::from_black_alpha(150), false);
    ui.ctx().request_repaint(); // the pan and the live frames move
}

/// UV rectangle that covers `rect` with an image of `size` (crop the wider side); `pan` 0..1 picks the horizontal
/// position of a wider image.
fn cover_uv(size: egui::Vec2, rect: Rect, pan: f32) -> Rect {
    let (ia, ra) = (size.x / size.y, rect.width() / rect.height());
    if ia > ra {
        let w = ra / ia;
        let x = (1.0 - w) * pan;
        Rect::from_min_max(egui::pos2(x, 0.0), egui::pos2(x + w, 1.0))
    } else {
        let h = ia / ra;
        Rect::from_min_max(egui::pos2(0.0, 0.5 - h / 2.0), egui::pos2(1.0, 0.5 + h / 2.0))
    }
}

/// Two-colour gradient over `r`: left to right when `horizontal`, else top to bottom.
fn gradient(p: &egui::Painter, r: Rect, a: Color32, b: Color32, horizontal: bool) {
    let mut m = egui::Mesh::default();
    let (tl, tr, br, bl) = if horizontal { (a, b, b, a) } else { (a, a, b, b) };
    m.colored_vertex(r.left_top(), tl);
    m.colored_vertex(r.right_top(), tr);
    m.colored_vertex(r.right_bottom(), br);
    m.colored_vertex(r.left_bottom(), bl);
    m.add_triangle(0, 1, 2);
    m.add_triangle(0, 2, 3);
    p.add(m);
}
