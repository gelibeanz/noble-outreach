//! Small widgets shared by the full-window menu screens (CAMPAIGN, PLAY / HOST): pictures from the player's install
//! cropped to fit, text-only stand-ins, header tabs, tall outlined buttons, a type-to-filter search box and chips.

use egui::{Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, TextureHandle, Ui, Vec2};

use super::theme::{ACCENT, DIM, HOVER, LINE, PANEL, TEXT};

/// Paint `t` filling `r` (cropped to keep its aspect).
pub fn cover(p: &egui::Painter, t: &TextureHandle, r: Rect, tint: Color32) {
    let [tw, th] = t.size();
    let (ta, ra) = (tw as f32 / th as f32, r.width() / r.height());
    let uv = if ta > ra {
        let w = ra / ta;
        Rect::from_min_max(Pos2::new((1.0 - w) / 2.0, 0.0), Pos2::new((1.0 + w) / 2.0, 1.0))
    } else {
        let h = ta / ra;
        Rect::from_min_max(Pos2::new(0.0, (1.0 - h) / 2.0), Pos2::new(1.0, (1.0 + h) / 2.0))
    };
    p.image(t.id(), r, inset(t, uv), tint);
}

/// The whole texture, half a texel in from each edge: egui-directx11 samples with BORDER addressing and a white
/// border colour, so linear filtering at uv 0 / 1 bleeds a grey line along the image edge.
pub fn full_uv(t: &TextureHandle) -> Rect {
    inset(t, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)))
}

fn inset(t: &TextureHandle, uv: Rect) -> Rect {
    let [w, h] = t.size();
    let (dx, dy) = (0.5 / w as f32, 0.5 / h as f32);
    Rect::from_min_max(Pos2::new(uv.min.x.max(dx), uv.min.y.max(dy)), Pos2::new(uv.max.x.min(1.0 - dx), uv.max.y.min(1.0 - dy)))
}

/// Text-only stand-in when the player's install has no picture.
pub fn placeholder(p: &egui::Painter, r: Rect, label: &str) {
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(r.left_top(), Color32::from_rgb(34, 40, 50));
    mesh.colored_vertex(r.right_top(), Color32::from_rgb(20, 24, 30));
    mesh.colored_vertex(r.right_bottom(), Color32::from_rgb(12, 14, 18));
    mesh.colored_vertex(r.left_bottom(), Color32::from_rgb(40, 28, 18));
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    p.add(mesh);
    if !label.is_empty() {
        let digits: String = label.chars().filter(|c| c.is_ascii_digit()).collect();
        p.text(r.center(), Align2::CENTER_CENTER, if digits.is_empty() { "—".into() } else { digits }, FontId::proportional(r.height() * 0.5), ACCENT.gamma_multiply(0.6));
    }
}

pub fn tab_button(ui: &mut Ui, label: &str, on: bool) -> egui::Response {
    let font = FontId::proportional(16.0);
    let g = ui.painter().layout_no_wrap(label.to_uppercase(), font, TEXT);
    let (r, resp) = ui.allocate_exact_size(g.size() + Vec2::new(26.0, 18.0), Sense::click());
    let col = if on { Color32::WHITE } else if resp.hovered() { TEXT } else { DIM };
    ui.painter().galley(r.center() - g.size() / 2.0, g, col);
    if on || resp.hovered() {
        ui.painter().rect_filled(Rect::from_min_max(Pos2::new(r.left() + 8.0, r.bottom() - 3.0), Pos2::new(r.right() - 8.0, r.bottom())), 0.0, if on { ACCENT } else { DIM });
    }
    resp
}

/// Outlined button as tall as `primary_button`.
pub fn big_ghost(ui: &mut Ui, label: &str, width: f32, enabled: bool) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(Vec2::new(width, 44.0), if enabled { Sense::click() } else { Sense::hover() });
    let hot = enabled && resp.hovered();
    ui.painter().rect_filled(r, 2.0, if hot { HOVER } else { Color32::TRANSPARENT });
    ui.painter().rect_stroke(r, 2.0, Stroke::new(1.5, if hot { ACCENT } else { ACCENT.gamma_multiply(0.55) }), StrokeKind::Inside);
    ui.painter().text(r.center(), Align2::CENTER_CENTER, label.to_uppercase(), FontId::proportional(19.0), if enabled { TEXT } else { DIM });
    resp
}


/// Search field filling `r` that takes typing without being clicked first: while `active` and no text field has
/// the keyboard, typed characters go into `q` and Backspace removes the last one. True when `q` changed.
pub fn search_box(ui: &mut Ui, r: Rect, q: &mut String, hint: &str, active: bool) -> bool {
    let before = q.len();
    if active && ui.ctx().memory(|m| m.focused().is_none()) {
        ui.input(|i| {
            for e in &i.events {
                match e {
                    egui::Event::Text(t) if q.chars().count() < 40 => q.push_str(t),
                    egui::Event::Key { key: egui::Key::Backspace, pressed: true, modifiers, .. } => {
                        if modifiers.ctrl {
                            q.clear();
                        } else {
                            q.pop();
                        }
                    }
                    _ => {}
                }
            }
        });
    }
    let p = ui.painter();
    p.rect_filled(r, 2.0, Color32::from_black_alpha(110));
    p.rect_stroke(r, 2.0, Stroke::new(1.0, if q.is_empty() { LINE } else { ACCENT.gamma_multiply(0.7) }), StrokeKind::Inside);
    let x = r.left() + 12.0;
    p.text(Pos2::new(x, r.center().y), Align2::LEFT_CENTER, "🔍", FontId::proportional(17.0), DIM);
    let tx = x + 22.0;
    if q.is_empty() {
        p.text(Pos2::new(tx, r.center().y), Align2::LEFT_CENTER, hint, FontId::proportional(14.5), DIM);
    } else {
        let t = p.text(Pos2::new(tx, r.center().y), Align2::LEFT_CENTER, q.as_str(), FontId::proportional(15.5), Color32::WHITE);
        // Caret, blinking.
        if active && (ui.input(|i| i.time) * 2.0) as i64 % 2 == 0 {
            p.rect_filled(Rect::from_min_size(Pos2::new(t.right() + 2.0, r.center().y - 9.0), Vec2::new(2.0, 18.0)), 0.0, ACCENT);
        }
        p.text(Pos2::new(r.right() - 10.0, r.center().y), Align2::RIGHT_CENTER, "ESC CLEARS", FontId::proportional(11.0), DIM);
    }
    if active {
        ui.ctx().request_repaint();
    }
    q.len() != before
}

/// Small toggle chip ("FFA", "TEAM", ...).
pub fn chip(ui: &mut Ui, label: &str, on: bool) -> egui::Response {
    let g = ui.painter().layout_no_wrap(label.to_uppercase(), FontId::proportional(12.5), TEXT);
    let (r, resp) = ui.allocate_exact_size(g.size() + Vec2::new(20.0, 12.0), Sense::click());
    let p = ui.painter();
    p.rect_filled(r, 2.0, if on { ACCENT.gamma_multiply(0.22) } else if resp.hovered() { HOVER } else { PANEL });
    p.rect_stroke(r, 2.0, Stroke::new(1.0, if on { ACCENT } else if resp.hovered() { ACCENT.gamma_multiply(0.6) } else { LINE }), StrokeKind::Inside);
    p.galley(r.center() - g.size() / 2.0, g, if on { Color32::WHITE } else { DIM });
    resp
}
