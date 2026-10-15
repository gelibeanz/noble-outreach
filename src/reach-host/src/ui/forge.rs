//! The FORGE screen: pick a multiplayer map (art from the player's install, mccart.rs) or one of your saved map
//! variants (`reach-data/map_variants`, forge.rs), then START FORGE alone or HOST FORGE as a listen server friends
//! join to build together. A saved variant can also be played: PLAY / HOST with a game type on it.
//! Also here: the pause-menu Forge section (save the map) and the map-variant row of the Play / Host screens.
//! Launch flags: `--forge <map>` / `--forge <file.mvar>`; play: `--map-variant <file> --game-variant <V>`.

use std::path::{Path, PathBuf};

use egui::{Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, TextureHandle, Ui, Vec2};

use super::mccart;
use super::menu::{variant_title, MAPS};
use super::theme::{self, ACCENT, DIM, HOVER, LINE, PANEL, TEXT};
use crate::forge::{self, Saved};

/// What the menu should do after a frame of this screen.
pub enum Action {
    None,
    Back,
    /// Launch these flags as a local game.
    Play(Vec<String>),
    /// Host these flags as a listen server for `players` (menu.rs adds `--listen`, port, master).
    Host { players: u32, args: Vec<String> },
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Maps,
    Saved,
}

pub struct ForgeScreen {
    mcc: PathBuf,
    pub tab: Tab,
    maps: Vec<(&'static str, &'static str)>,
    map: usize,
    saved: Vec<Saved>,
    pub saved_sel: usize,
    variants: Vec<String>,
    variant: usize,
    players: u32,
}

/// Texture-pack key of a map: its English name in capitals without spaces (`Forge World` → `FORGEWORLD`).
fn art_key(name: &str) -> String {
    name.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_uppercase()
}

/// Controls the engine binds for Forge by default (keyboard / pad), shown as a reminder.
const CONTROLS: &[(&str, &str)] = &[
    ("~  /  D-pad up", "Monitor / Spartan"),
    ("1", "Place object menu"),
    ("Left mouse  /  A", "Grab / drop object"),
    ("R  /  F", "Raise / lower"),
    ("Delete", "Delete object"),
    ("2", "Object tools"),
    ("6", "Rotation axes"),
    ("Esc", "Pause menu: save the map"),
];

impl ForgeScreen {
    pub fn new(mcc: &Path) -> Self {
        mccart::init(mcc);
        let maps: Vec<_> = MAPS.iter().copied().filter(|m| mcc.join("haloreach").join("maps").join(format!("{}.map", m.0)).is_file()).collect();
        let mut variants: Vec<String> = std::fs::read_dir(mcc.join("haloreach").join("game_variants"))
            .map(|d| d.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.ends_with(".bin") && !n.starts_with("hr_ff_") && n != forge::FORGE_GAME_VARIANT).collect())
            .unwrap_or_default();
        variants.sort_by_key(|v| variant_title(v).to_lowercase());
        let variant = variants.iter().position(|v| v == "slayer_054.bin").unwrap_or(0);
        let saved = forge::list();
        let last = super::settings::get().last_map;
        let map = maps.iter().position(|m| m.0 == last).unwrap_or(0);
        ForgeScreen { mcc: mcc.to_path_buf(), tab: if saved.is_empty() { Tab::Maps } else { Tab::Saved }, maps, map, saved, saved_sel: 0, variants, variant, players: 8 }
    }

    /// `--forge` flags for the current pick (map, or saved variant to keep building).
    pub fn forge_args(&self) -> Option<Vec<String>> {
        Some(match self.tab {
            Tab::Maps => vec!["--forge".into(), self.maps.get(self.map)?.0.into()],
            Tab::Saved => vec!["--forge".into(), self.saved.get(self.saved_sel)?.path.to_string_lossy().into_owned()],
        })
    }

    /// Play the selected saved variant with the chosen game type.
    pub fn play_args(&self) -> Option<Vec<String>> {
        let s = self.saved.get(self.saved_sel).filter(|_| self.tab == Tab::Saved)?;
        let gv = self.mcc.join("haloreach").join("game_variants").join(self.variants.get(self.variant)?);
        let mut a = vec!["--map-variant".into(), s.path.to_string_lossy().into_owned(), "--game-variant".into(), gv.to_string_lossy().into_owned()];
        if !s.map.is_empty() {
            a.splice(0..0, ["--map".to_string(), s.map.clone()]);
        }
        Some(a)
    }

    fn count(&self) -> usize {
        if self.tab == Tab::Maps { self.maps.len() } else { self.saved.len() }
    }

    fn sel(&mut self) -> &mut usize {
        if self.tab == Tab::Maps { &mut self.map } else { &mut self.saved_sel }
    }

    pub fn ui(&mut self, ui: &mut Ui) -> Action {
        let full = ui.max_rect();
        let mut act = self.keys(ui);
        let m = 32.0;
        let top = Rect::from_min_max(full.min + Vec2::new(m, 22.0), Pos2::new(full.right() - m, full.top() + 74.0));
        ui.scope_builder(egui::UiBuilder::new().max_rect(top), |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("FORGE").size(30.0).color(TEXT));
                ui.add_space(24.0);
                for (label, t) in [("Maps", Tab::Maps), ("Saved maps", Tab::Saved)] {
                    let label = if t == Tab::Saved { format!("{label} ({})", self.saved.len()) } else { label.to_string() };
                    if tab_button(ui, &label, self.tab == t).clicked() {
                        self.tab = t;
                    }
                }
            });
        });
        ui.painter().line_segment([Pos2::new(top.left(), top.top() + 42.0), Pos2::new(top.left() + 36.0, top.top() + 42.0)], Stroke::new(2.0, ACCENT));
        let mut back = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_size(Pos2::new(top.right() - 76.0, top.top() + 2.0), Vec2::new(76.0, 36.0))));
        if theme::ghost_button(&mut back, "Back").clicked() {
            act = Action::Back;
        }
        let body = Rect::from_min_max(Pos2::new(full.left() + m, top.bottom() + 8.0), full.max - Vec2::new(m, 28.0));
        let list_w = (body.width() * 0.32).clamp(300.0, 470.0);
        let left = Rect::from_min_size(body.min, Vec2::new(list_w, body.height()));
        let right = Rect::from_min_max(Pos2::new(left.right() + 22.0, body.top()), body.max);
        self.list(ui, left);
        if let Some(a) = self.detail(ui, right) {
            act = a;
        }
        act
    }

    fn keys(&mut self, ui: &mut Ui) -> Action {
        if ui.ctx().egui_wants_keyboard_input() {
            return Action::None;
        }
        let n = self.count();
        let (up, down, tab, enter, esc) = ui.input(|i| {
            use egui::Key::*;
            (i.key_pressed(ArrowUp), i.key_pressed(ArrowDown), i.key_pressed(Tab), i.key_pressed(Enter), i.key_pressed(Escape))
        });
        if n > 0 {
            let s = self.sel();
            if up {
                *s = (*s + n - 1) % n;
            }
            if down {
                *s = (*s + 1) % n;
            }
        }
        if tab {
            self.tab = if self.tab == Tab::Maps { Tab::Saved } else { Tab::Maps };
        }
        if esc {
            return Action::Back;
        }
        if enter {
            if let Some(a) = self.forge_args() {
                return Action::Play(a);
            }
        }
        Action::None
    }

    /// (name, subtitle, thumbnail key, banner key) of list row `i`.
    fn item(&self, i: usize) -> (String, String, String, String) {
        match self.tab {
            Tab::Maps => {
                let m = self.maps[i];
                let k = art_key(m.1);
                (m.1.to_string(), "Multiplayer map".into(), format!("HREACH_{k}_MINI"), format!("HREACHMP_{k}LARGE"))
            }
            Tab::Saved => {
                let s = &self.saved[i];
                let map = if s.map.is_empty() { "Unknown map".to_string() } else { super::menu::map_title(&s.map) };
                let k = art_key(&map);
                (s.name.clone(), map, format!("HREACH_{k}_MINI"), format!("HREACHMP_{k}LARGE"))
            }
        }
    }

    fn list(&mut self, ui: &mut Ui, rect: Rect) {
        let p = ui.painter().clone();
        p.rect_filled(rect, 2.0, PANEL);
        p.rect_stroke(rect, 2.0, Stroke::new(1.0, LINE), StrokeKind::Inside);
        let inner = rect.shrink2(Vec2::new(14.0, 14.0));
        p.text(inner.left_top(), Align2::LEFT_TOP, if self.tab == Tab::Maps { "MAPS" } else { "YOUR SAVED MAPS" }, FontId::proportional(12.0), DIM);
        let area = Rect::from_min_max(inner.min + Vec2::new(0.0, 24.0), inner.max);
        let n = self.count();
        if n == 0 {
            let msg = if self.tab == Tab::Maps { "No multiplayer maps found in your MCC files." } else { "Nothing saved yet. Start Forge on a map, build, then Esc > SAVE MAP." };
            ui.scope_builder(egui::UiBuilder::new().max_rect(area), |ui| ui.label(egui::RichText::new(msg).size(15.0).color(DIM)));
            return;
        }
        let row_h = (area.height() / n as f32).clamp(46.0, 64.0);
        let thumb = Vec2::new((row_h - 10.0) * 16.0 / 9.0, row_h - 10.0);
        let ctx = ui.ctx().clone();
        ui.scope_builder(egui::UiBuilder::new().max_rect(area), |ui| {
            egui::ScrollArea::vertical().id_salt("forge-list").max_height(area.height()).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for i in 0..n {
                    let (name, sub, th, _) = self.item(i);
                    let selected = *self.sel() == i;
                    let (r, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), row_h), Sense::click());
                    let p = ui.painter();
                    if selected {
                        p.rect_filled(r, 0.0, ACCENT.gamma_multiply(0.20));
                        p.rect_filled(Rect::from_min_size(r.min, Vec2::new(3.0, r.height())), 0.0, ACCENT);
                    } else if resp.hovered() {
                        p.rect_filled(r, 0.0, HOVER);
                    }
                    let tr = Rect::from_min_size(r.min + Vec2::new(10.0, 5.0), thumb);
                    match mccart::texture(&ctx, &th) {
                        Some(t) => cover(p, &t, tr, if selected || resp.hovered() { Color32::WHITE } else { Color32::from_gray(190) }),
                        None => placeholder(p, tr),
                    }
                    p.rect_stroke(tr, 0.0, Stroke::new(1.0, if selected { ACCENT } else { LINE }), StrokeKind::Inside);
                    let x = tr.right() + 12.0;
                    p.text(Pos2::new(x, r.center().y - 2.0), Align2::LEFT_BOTTOM, sub.to_uppercase(), FontId::proportional(11.5), if selected { ACCENT } else { DIM });
                    p.text(Pos2::new(x, r.center().y), Align2::LEFT_TOP, name, FontId::proportional(17.0), if selected { Color32::WHITE } else { TEXT });
                    if resp.clicked() {
                        *self.sel() = i;
                    }
                    if resp.double_clicked() {
                        *self.sel() = i;
                    }
                }
            });
        });
    }

    fn detail(&mut self, ui: &mut Ui, rect: Rect) -> Option<Action> {
        if self.count() == 0 {
            return None;
        }
        let i = *self.sel();
        let (name, sub, _, banner) = self.item(i);
        let ctx = ui.ctx().clone();
        let p = ui.painter().clone();
        let bh = (rect.height() * 0.42).min(rect.width() * 292.0 / 668.0);
        let br = Rect::from_min_size(rect.min, Vec2::new(rect.width(), bh));
        match mccart::texture(&ctx, &banner) {
            Some(t) => cover(&p, &t, br, Color32::WHITE),
            None => placeholder(&p, br),
        }
        let mut mesh = egui::Mesh::default();
        let fade = Rect::from_min_max(Pos2::new(br.left(), br.bottom() - 90.0), br.max);
        mesh.colored_vertex(fade.left_top(), Color32::TRANSPARENT);
        mesh.colored_vertex(fade.right_top(), Color32::TRANSPARENT);
        mesh.colored_vertex(fade.right_bottom(), Color32::from_black_alpha(220));
        mesh.colored_vertex(fade.left_bottom(), Color32::from_black_alpha(220));
        mesh.add_triangle(0, 1, 2);
        mesh.add_triangle(0, 2, 3);
        p.add(mesh);
        p.text(Pos2::new(br.left() + 18.0, br.bottom() - 40.0), Align2::LEFT_BOTTOM, sub.to_uppercase(), FontId::proportional(13.0), ACCENT);
        p.text(Pos2::new(br.left() + 18.0, br.bottom() - 12.0), Align2::LEFT_BOTTOM, name.to_uppercase(), FontId::proportional(30.0), Color32::WHITE);
        p.rect_stroke(br, 0.0, Stroke::new(1.0, LINE), StrokeKind::Inside);

        let lower = Rect::from_min_max(Pos2::new(rect.left(), br.bottom() + 14.0), rect.max);
        let mut act = None;
        ui.scope_builder(egui::UiBuilder::new().max_rect(lower), |ui| {
            ui.columns(2, |cols| {
                let ui = &mut cols[0];
                theme::caption(ui, "Build together");
                ui.label(egui::RichText::new(if self.tab == Tab::Maps {
                    "Start Forge on this map. Host it and friends who join build with you; everything placed is shared."
                } else {
                    "Keep building this map, alone or with friends. Save again from the pause menu."
                }).size(13.5).color(DIM));
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    theme::caption(ui, "Players");
                    ui.add(egui::Slider::new(&mut self.players, 2..=16));
                });
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    let a = self.forge_args();
                    if theme::primary_button(ui, "Start Forge", 170.0, a.is_some()).clicked() {
                        act = a.map(Action::Play);
                    }
                    let a = self.forge_args();
                    if big_ghost(ui, "Host Forge", 170.0, a.is_some()).clicked() {
                        act = a.map(|args| Action::Host { players: self.players, args });
                    }
                });
                if self.tab == Tab::Saved {
                    ui.add_space(14.0);
                    theme::caption(ui, "Play on it");
                    let n = self.variants.len().max(1);
                    ui.horizontal(|ui| {
                        if theme::ghost_button(ui, "◀").clicked() {
                            self.variant = (self.variant + n - 1) % n;
                        }
                        let v = self.variants.get(self.variant).map(|v| variant_title(v)).unwrap_or_default();
                        ui.add_sized(Vec2::new(150.0, 30.0), egui::Label::new(egui::RichText::new(v).size(16.0)));
                        if theme::ghost_button(ui, "▶").clicked() {
                            self.variant = (self.variant + 1) % n;
                        }
                    });
                    ui.horizontal(|ui| {
                        let a = self.play_args();
                        if big_ghost(ui, "Play", 170.0, a.is_some()).clicked() {
                            act = a.map(Action::Play);
                        }
                        let a = self.play_args();
                        if big_ghost(ui, "Host game", 170.0, a.is_some()).clicked() {
                            act = a.map(|args| Action::Host { players: self.players, args });
                        }
                    });
                    if let Some(s) = self.saved.get(self.saved_sel) {
                        ui.add_space(6.0);
                        ui.label(egui::RichText::new(s.path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default()).size(12.0).color(DIM));
                    }
                }
                let ui = &mut cols[1];
                theme::caption(ui, "Forge controls");
                for (k, what) in CONTROLS {
                    ui.horizontal(|ui| {
                        ui.add_sized(Vec2::new(150.0, 20.0), egui::Label::new(egui::RichText::new(*k).size(13.5).color(ACCENT)));
                        ui.label(egui::RichText::new(*what).size(13.5));
                    });
                }
                ui.label(egui::RichText::new("Rebind under Settings > Keyboard > Forge.").size(12.0).color(DIM));
            });
        });
        act
    }
}

// ---- Pause menu (in a Forge game) ------------------------------------------------------------------------------

/// Name typed in the pause menu's SAVE MAP field (kept between openings).
static SAVE_NAME: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// Pause overlay section in a Forge game: name field + SAVE MAP, and the last result.
pub fn pause_section(ui: &mut Ui) {
    if !forge::ACTIVE.load(std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    ui.add_space(18.0);
    theme::caption(ui, "Forge: save this map");
    let mut g = SAVE_NAME.lock().unwrap_or_else(|e| e.into_inner());
    let name = g.get_or_insert_with(forge::suggest_name);
    ui.add(egui::TextEdit::singleline(name).desired_width(ui.available_width()).char_limit(48));
    if theme::primary_button(ui, "Save map", ui.available_width(), !name.trim().is_empty()).clicked() {
        forge::request_save(name.trim());
    }
    if let Some((msg, ok, _)) = forge::last_status() {
        ui.label(egui::RichText::new(msg).size(13.0).color(if ok { ACCENT } else { Color32::from_rgb(230, 90, 80) }));
    }
    ui.label(egui::RichText::new("Saved maps appear under FORGE > Saved maps, ready to play or keep building.").size(11.5).color(DIM));
}

/// Passive overlay toast for a save made from the test script or after the pause menu closed.
pub fn save_toast(ui: &mut Ui) {
    let Some((msg, ok, a)) = forge::last_status() else { return };
    let full = ui.max_rect();
    let r = Rect::from_min_size(Pos2::new(full.center().x - 200.0, full.top() + 90.0), Vec2::new(400.0, 46.0));
    ui.scope(|ui| {
        ui.set_opacity(a);
        theme::panel(ui, r, |ui| {
            ui.label(egui::RichText::new(msg).size(16.0).color(if ok { ACCENT } else { Color32::from_rgb(230, 90, 80) }));
        });
    });
}

// ---- Map-variant row for the Play / Host screens ----------------------------------------------------------------

/// "Map variant: Default ◀ ▶" row: cycles through the saved variants of `map` (stem). Returns the chosen file.
pub fn variant_row(ui: &mut Ui, map: &str, sel: &mut Option<PathBuf>) {
    let list: Vec<Saved> = forge::list().into_iter().filter(|s| s.map.is_empty() || s.map == map).collect();
    if sel.as_ref().is_some_and(|p| !list.iter().any(|s| &s.path == p)) {
        *sel = None;
    }
    let pos = sel.as_ref().and_then(|p| list.iter().position(|s| &s.path == p));
    ui.horizontal(|ui| {
        theme::caption(ui, "Map variant");
        if list.is_empty() {
            ui.label(egui::RichText::new("Default (no saved Forge maps for this map)").size(14.0).color(DIM));
            return;
        }
        let n = list.len() + 1; // 0 = default
        let cur = pos.map_or(0, |i| i + 1);
        let mut next = cur;
        if theme::ghost_button(ui, "◀").clicked() {
            next = (cur + n - 1) % n;
        }
        let label = pos.map_or_else(|| "Default".to_string(), |i| list[i].name.clone());
        ui.add_sized(Vec2::new(200.0, 30.0), egui::Label::new(egui::RichText::new(label).size(15.0)).truncate());
        if theme::ghost_button(ui, "▶").clicked() {
            next = (cur + 1) % n;
        }
        if next != cur {
            *sel = if next == 0 { None } else { Some(list[next - 1].path.clone()) };
        }
    });
}

// ---- Drawing helpers ---------------------------------------------------------------------------------------------

fn cover(p: &egui::Painter, t: &TextureHandle, r: Rect, tint: Color32) {
    let [tw, th] = t.size();
    let (ta, ra) = (tw as f32 / th as f32, r.width() / r.height());
    let uv = if ta > ra {
        let w = ra / ta;
        Rect::from_min_max(Pos2::new((1.0 - w) / 2.0, 0.0), Pos2::new((1.0 + w) / 2.0, 1.0))
    } else {
        let h = ta / ra;
        Rect::from_min_max(Pos2::new(0.0, (1.0 - h) / 2.0), Pos2::new(1.0, (1.0 + h) / 2.0))
    };
    // Half a texel in from each edge (egui-directx11 samples with a white BORDER colour).
    let (dx, dy) = (0.5 / tw as f32, 0.5 / th as f32);
    let uv = Rect::from_min_max(Pos2::new(uv.min.x.max(dx), uv.min.y.max(dy)), Pos2::new(uv.max.x.min(1.0 - dx), uv.max.y.min(1.0 - dy)));
    p.image(t.id(), r, uv, tint);
}

fn placeholder(p: &egui::Painter, r: Rect) {
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(r.left_top(), Color32::from_rgb(34, 40, 50));
    mesh.colored_vertex(r.right_top(), Color32::from_rgb(20, 24, 30));
    mesh.colored_vertex(r.right_bottom(), Color32::from_rgb(12, 14, 18));
    mesh.colored_vertex(r.left_bottom(), Color32::from_rgb(40, 28, 18));
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    p.add(mesh);
}

fn tab_button(ui: &mut Ui, label: &str, on: bool) -> egui::Response {
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

fn big_ghost(ui: &mut Ui, label: &str, width: f32, enabled: bool) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(Vec2::new(width, 44.0), if enabled { Sense::click() } else { Sense::hover() });
    let hot = enabled && resp.hovered();
    ui.painter().rect_filled(r, 2.0, if hot { HOVER } else { Color32::TRANSPARENT });
    ui.painter().rect_stroke(r, 2.0, Stroke::new(1.5, if hot { ACCENT } else { ACCENT.gamma_multiply(0.55) }), StrokeKind::Inside);
    ui.painter().text(r.center(), Align2::CENTER_CENTER, label.to_uppercase(), FontId::proportional(19.0), if enabled { TEXT } else { DIM });
    resp
}
