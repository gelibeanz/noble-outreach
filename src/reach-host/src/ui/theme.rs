//! Look of the frontend: dark translucent panels, thin uppercase headings, one orange accent.

use egui::{Align2, Color32, FontId, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2};

pub const ACCENT: Color32 = Color32::from_rgb(242, 140, 40);
pub const BG: Color32 = Color32::from_rgb(9, 11, 14);
pub const PANEL: Color32 = Color32::from_rgba_premultiplied(14, 16, 20, 222);
pub const LINE: Color32 = Color32::from_rgba_premultiplied(18, 18, 18, 18);
pub const TEXT: Color32 = Color32::from_rgb(228, 231, 235);
pub const DIM: Color32 = Color32::from_rgb(138, 145, 155);
pub const HOVER: Color32 = Color32::from_rgba_premultiplied(20, 20, 20, 20);

pub fn install(ctx: &egui::Context) {
    ctx.all_styles_mut(|s| {
        use egui::{FontFamily::Proportional, TextStyle::*};
        s.text_styles = [
            (Heading, FontId::new(30.0, Proportional)),
            (Body, FontId::new(16.0, Proportional)),
            (Button, FontId::new(16.0, Proportional)),
            (Small, FontId::new(12.0, Proportional)),
            (Monospace, FontId::new(14.0, egui::FontFamily::Monospace)),
        ]
        .into();
        s.spacing.item_spacing = egui::vec2(10.0, 8.0);
        s.spacing.interact_size.y = 30.0;
        let v = &mut s.visuals;
        *v = egui::Visuals::dark();
        v.override_text_color = Some(TEXT);
        v.panel_fill = Color32::TRANSPARENT;
        v.window_fill = PANEL;
        v.extreme_bg_color = Color32::from_rgba_premultiplied(0, 0, 0, 120);
        v.selection.bg_fill = ACCENT.gamma_multiply(0.6);
        v.selection.stroke = Stroke::new(1.0, ACCENT);
        v.text_cursor.stroke = Stroke::new(2.0, ACCENT);
        for w in [&mut v.widgets.inactive, &mut v.widgets.noninteractive] {
            w.corner_radius = 0.0.into();
        }
        v.widgets.inactive.weak_bg_fill = HOVER;
        v.widgets.inactive.bg_fill = Color32::from_rgba_premultiplied(30, 30, 30, 30);
        v.widgets.hovered.weak_bg_fill = Color32::from_rgba_premultiplied(40, 40, 40, 40);
        v.widgets.hovered.bg_fill = Color32::from_rgba_premultiplied(40, 40, 40, 40);
        v.widgets.hovered.bg_stroke = Stroke::new(1.0, ACCENT);
        v.widgets.hovered.corner_radius = 0.0.into();
        v.widgets.active.bg_fill = ACCENT;
        v.widgets.active.weak_bg_fill = ACCENT;
        v.widgets.active.corner_radius = 0.0.into();
        v.widgets.inactive.fg_stroke = Stroke::new(1.5, TEXT);
        v.slider_trailing_fill = true;
    });
}

/// Full-window background for the menu (no game scene behind it yet): dark gradient, faint grid, accent glow.
pub fn backdrop(ui: &Ui) {
    let r = ui.max_rect();
    let p = ui.painter();
    p.rect_filled(r, 0.0, BG);
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(r.left_top(), Color32::from_rgb(20, 24, 30));
    mesh.colored_vertex(r.right_top(), Color32::from_rgb(12, 14, 18));
    mesh.colored_vertex(r.right_bottom(), Color32::from_rgb(6, 7, 9));
    mesh.colored_vertex(r.left_bottom(), Color32::from_rgb(26, 18, 12));
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    p.add(mesh);
    let grid = Color32::from_rgba_premultiplied(5, 5, 5, 5);
    let mut x = r.left();
    while x < r.right() {
        p.line_segment([egui::pos2(x, r.top()), egui::pos2(x + r.height() * 0.4, r.bottom())], Stroke::new(1.0, grid));
        x += 48.0;
    }
    p.rect_filled(Rect::from_min_size(r.left_bottom() - Vec2::new(0.0, 3.0), Vec2::new(r.width(), 3.0)), 0.0, ACCENT.gamma_multiply(0.7));
}

/// Translucent panel filling `rect`; `add` lays out inside it with a margin.
pub fn panel<R>(ui: &mut Ui, rect: Rect, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.painter().rect_filled(rect, 2.0, PANEL);
    ui.painter().rect_stroke(rect, 2.0, Stroke::new(1.0, LINE), StrokeKind::Inside);
    ui.scope_builder(egui::UiBuilder::new().max_rect(rect.shrink(22.0)), add).inner
}

/// Panel `width` wide hanging from `at` (`anchor_x`: LEFT/CENTER/RIGHT of `at`) that grows to fit its content: the
/// background is painted behind the content once it is laid out, so a guessed height never cuts off the last row.
pub fn panel_fit<R>(ui: &mut Ui, at: egui::Pos2, anchor_x: egui::Align, width: f32, add: impl FnOnce(&mut Ui) -> R) -> R {
    const PAD: f32 = 22.0;
    let bg = ui.painter().add(egui::Shape::Noop);
    let left = at.x - width * (anchor_x.to_factor() + 1.0) / 2.0;
    let max = Rect::from_min_size(egui::pos2(left + PAD, at.y + PAD), Vec2::new(width - 2.0 * PAD, 10_000.0));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(max));
    let inner = add(&mut child);
    let r = Rect::from_min_size(egui::pos2(left, at.y), Vec2::new(width, child.min_rect().height() + 2.0 * PAD));
    ui.painter().set(bg, egui::Shape::Vec(vec![
        egui::Shape::rect_filled(r, 2.0, PANEL),
        egui::Shape::rect_stroke(r, 2.0, Stroke::new(1.0, LINE), StrokeKind::Inside),
    ]));
    inner
}

/// Thin uppercase screen heading with a short accent rule under it.
pub fn heading(ui: &mut Ui, text: &str) {
    let r = ui.label(egui::RichText::new(text.to_uppercase()).size(30.0).color(TEXT));
    let y = r.rect.bottom() + 3.0;
    ui.painter().line_segment([egui::pos2(r.rect.left(), y), egui::pos2(r.rect.left() + 36.0, y)], Stroke::new(2.0, ACCENT));
    ui.add_space(12.0);
}

pub fn caption(ui: &mut Ui, text: &str) {
    ui.label(egui::RichText::new(text.to_uppercase()).size(12.0).color(DIM));
}

/// Big left-column menu entry: accent bar and white text when hovered or selected.
pub fn nav_item(ui: &mut Ui, label: &str, selected: bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 46.0), Sense::click());
    let hot = selected || resp.hovered();
    if hot {
        ui.painter().rect_filled(rect, 0.0, HOVER);
        ui.painter().rect_filled(Rect::from_min_size(rect.min, Vec2::new(4.0, rect.height())), 0.0, ACCENT);
    }
    ui.painter().text(
        rect.left_center() + Vec2::new(18.0, 0.0),
        Align2::LEFT_CENTER,
        label.to_uppercase(),
        FontId::proportional(24.0),
        if hot { Color32::WHITE } else { DIM },
    );
    resp
}

/// Filled accent button (the one primary action on a screen).
pub fn primary_button(ui: &mut Ui, label: &str, width: f32, enabled: bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, 44.0), if enabled { Sense::click() } else { Sense::hover() });
    let fill = if !enabled {
        Color32::from_rgb(60, 52, 44)
    } else if resp.hovered() {
        Color32::from_rgb(255, 166, 74)
    } else {
        ACCENT
    };
    ui.painter().rect_filled(rect, 2.0, fill);
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, label.to_uppercase(), FontId::proportional(19.0), Color32::from_rgb(16, 12, 8));
    resp
}

/// Outlined secondary button.
pub fn ghost_button(ui: &mut Ui, label: &str) -> Response {
    let font = FontId::proportional(15.0);
    let galley = ui.painter().layout_no_wrap(label.to_uppercase(), font, TEXT);
    let (rect, resp) = ui.allocate_exact_size(galley.size() + Vec2::new(28.0, 14.0), Sense::click());
    ui.painter().rect_filled(rect, 2.0, if resp.hovered() { HOVER } else { Color32::TRANSPARENT });
    ui.painter().rect_stroke(rect, 2.0, Stroke::new(1.0, if resp.hovered() { ACCENT } else { DIM.gamma_multiply(0.6) }), StrokeKind::Inside);
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, TEXT);
    resp
}

/// One row of a dense list: `cols` = (x offset in points, text). Selected rows get the accent bar.
pub fn list_row(ui: &mut Ui, cols: &[(f32, String)], selected: bool, height: f32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::click());
    if selected {
        ui.painter().rect_filled(rect, 0.0, ACCENT.gamma_multiply(0.22));
        ui.painter().rect_filled(Rect::from_min_size(rect.min, Vec2::new(3.0, rect.height())), 0.0, ACCENT);
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, 0.0, HOVER);
    }
    for (i, (x, t)) in cols.iter().enumerate() {
        let right = cols.get(i + 1).map_or(rect.right(), |c| rect.left() + c.0) - 8.0;
        let clip = Rect::from_min_max(egui::pos2(rect.left() + x, rect.top()), egui::pos2(right.max(rect.left() + x), rect.bottom()));
        ui.painter().with_clip_rect(clip).text(
            egui::pos2(rect.left() + x + 10.0, rect.center().y),
            Align2::LEFT_CENTER,
            t,
            FontId::proportional(15.0),
            if selected { Color32::WHITE } else { TEXT },
        );
    }
    resp
}

/// Column captions above a `list_row` table.
pub fn list_header(ui: &mut Ui, cols: &[(f32, &str)]) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 22.0), Sense::hover());
    for (x, t) in cols {
        ui.painter().text(egui::pos2(rect.left() + x + 10.0, rect.center().y), Align2::LEFT_CENTER, t.to_uppercase(), FontId::proportional(11.5), DIM);
    }
    ui.painter().line_segment([rect.left_bottom(), rect.right_bottom()], Stroke::new(1.0, LINE));
}
