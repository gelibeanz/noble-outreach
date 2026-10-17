//! The CAMPAIGN screen, laid out after Project Reclaimer's: missions with thumbnails on the left; on the right a
//! banner of the selected mission, difficulty cards, the campaign modifiers (skulls) row that opens a picker, and
//! PLAY SOLO / HOST CO-OP. A FIREFIGHT tab shows the Firefight maps the same way (plus the game type).
//! Pictures come from the player's own MCC install at runtime (mccart.rs); text-only cards when one is missing.
//! Launch flags: `--campaign M --difficulty D --insertion 0 [--skulls a,b]` (Firefight: `--firefight M ...
//! --game-variant V`); HOST CO-OP hands the same flags to the menu's listen-server path. Designed in points for a
//! 720-line layout (ui::draw scales with the window height), widths follow the window's aspect.

use std::path::{Path, PathBuf};

use egui::{Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, TextureHandle, Ui, Vec2};

use super::mccart;
use super::settings;
use super::theme::{self, ACCENT, DIM, HOVER, LINE, PANEL, TEXT};
use crate::options::SKULLS;

/// Campaign missions in story order: (map stem, name, texture-pack key, tagline in our own words).
/// m70_a (credits) is left out; m40 has no map. Thumbnail `HREACH_<key>`, banner `HREACH_<key>_LARGE`.
const MISSIONS: &[(&str, &str, &str, &str)] = &[
    ("m05", "Noble Actual", "C01_NOBLEACTUAL", "A relay outpost has gone quiet. Noble Team is sent to find out why."),
    ("m10", "Winter Contingency", "C02_WINTERCONTINGENCY", "Something is out there in the snow, and it is not insurrectionists."),
    ("m20", "ONI: Sword Base", "C03_ONISWORDBASE", "Hold ONI's Sword Base against a full Covenant assault."),
    ("m30", "Nightfall", "C04_NIGHTFALL", "Slip behind enemy lines after dark and see what the Covenant are building."),
    ("m35", "Tip of the Spear", "C05_TIPOFTHESPEAR", "Spearhead the UNSC counter-attack across the Viery Territory."),
    ("m45", "Long Night of Solace", "C06_LONGNIGHTOFSOLACE", "Take the fight into orbit and bring down a Covenant super-carrier."),
    ("m50", "Exodus", "C07_EXODUS", "New Alexandria is evacuating. Get the civilians out while the city burns."),
    ("m52", "New Alexandria", "C08_NEWALEXANDRIA", "Fly a Falcon over the burning city and knock out the Covenant jammers."),
    ("m60", "The Package", "C09_THEPACKAGE", "Return to Sword Base and recover what Dr. Halsey has been guarding."),
    ("m70", "The Pillar of Autumn", "C10_THEPILLAROFAUTUMN", "Get the package to the Pillar of Autumn, whatever it costs."),
    ("m70_bonus", "Lone Wolf", "C11_LONEWOLF", "The last stand of Noble Six."),
];

/// Texture-pack thumbnail of a mission (`HREACH_<key>`), for the vote cards (ui/vote.rs).
pub fn mission_art(stem: &str) -> Option<String> {
    MISSIONS.iter().find(|m| m.0 == stem).map(|m| format!("HREACH_{}", m.2))
}

/// The mission after `stem` in story order whose map is installed (None after Lone Wolf).
pub fn next_mission(stem: &str, mcc: &std::path::Path) -> Option<&'static str> {
    let i = MISSIONS.iter().position(|m| m.0 == stem)?;
    MISSIONS[i + 1..].iter().map(|m| m.0).find(|s| mcc.join("haloreach").join("maps").join(format!("{s}.map")).is_file())
}

/// Display name of a mission ("Winter Contingency"), or the stem.
pub fn mission_name(stem: &str) -> &str {
    MISSIONS.iter().find(|m| m.0 == stem).map_or(stem, |m| m.1)
}

/// Firefight maps: (map stem, name, texture-pack key, tagline). Thumbnail `REACHFIREFIGHT_<key>`, banner `…<key>LARGE`.
const FF_MAPS: &[(&str, &str, &str, &str)] = &[
    ("ff50_park", "Beachhead", "BEACHHEAD", "A seaside plaza in New Alexandria's parkland."),
    ("ff10_prototype", "Overlook", "OVERLOOK", "A cliffside lookout above the valley."),
    ("ff20_courtyard", "Courtyard", "COURTYARD", "Tight walkways around an old stone courtyard."),
    ("ff30_waterfront", "Waterfront", "WATERFRONT", "Docks and warehouses on the shore."),
    ("ff45_corvette", "Corvette", "CORVETTE", "Fight inside a Covenant corvette."),
    ("ff60_icecave", "Glacier", "GLACIER", "Ice caves high in the mountains."),
    ("ff70_holdout", "Holdout", "HOLDOUT", "Dig in at a besieged outpost."),
    ("ff60_airview", "Outpost", "OUTPOST", "A wide open base with long sight lines."),
    ("ff_unearthed", "Unearthed", "UNEARTHED", "An excavation site gone wrong."),
];

/// (CLI value, name, one-line description, large icon).
const DIFFICULTIES: &[(&str, &str, &str, &str)] = &[
    ("easy", "Easy", "Enemies go down fast and hit softly. Story first.", "DIFFICULTYLARGEEASY"),
    ("normal", "Normal", "A fair fight: the way most first play it.", "DIFFICULTYLARGENORMAL"),
    ("heroic", "Heroic", "Tougher, smarter enemies. You will need cover.", "DIFFICULTYLARGEHEROIC"),
    ("legendary", "Legendary", "Every enemy is deadly. Expect to die a lot.", "DIFFICULTYLARGELEGENDARY"),
];

/// Per `options::SKULLS` key: (pack texture, what it does).
fn skull_info(key: &str) -> (&'static str, &'static str) {
    match key {
        "iron" => ("IRON", "Dying sends everyone back to the last checkpoint."),
        "blackeye" => ("BLACKEYE", "Shields only recharge when you melee enemies."),
        "toughluck" => ("TOUGHLUCK", "Enemies always dodge grenades and dive away."),
        "catch" => ("CATCH", "Enemies throw far more grenades."),
        "cloud" => ("CLOUD", "No motion tracker."),
        "famine" => ("FAMINE", "Dropped weapons carry half the ammo."),
        "thunderstorm" => ("THUNDERSTORM", "Enemies are promoted to higher ranks."),
        "tilt" => ("TILT", "Enemy strengths and weaknesses are doubled."),
        "mythic" => ("MYTHIC", "Enemies have twice the health."),
        "blind" => ("BLIND", "No HUD and no weapon on screen."),
        "cowbell" => ("COWBELL", "Explosions throw things much further."),
        "gruntbirthday" => ("GRUNTBIRTHDAYPARTY", "Grunt headshots end in confetti."),
        "iwhbyd" => ("IWHBYD", "Rare combat dialogue becomes common."),
        _ => ("", ""),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Campaign,
    Firefight,
}

/// What the menu should do after a frame of this screen.
pub enum Action {
    None,
    Back,
    /// Launch these mode flags as a local game.
    Play(Vec<String>),
    /// Host these mode flags as a listen server for `players` (menu.rs adds `--listen`, port, master).
    Host { players: u32, args: Vec<String> },
}

pub struct CampaignScreen {
    mcc: PathBuf,
    pub tab: Tab,
    missions: Vec<usize>,
    ff_maps: Vec<usize>,
    ff_variants: Vec<String>,
    ff_variant: usize,
    pub mission: usize,
    ff_map: usize,
    pub difficulty: usize,
    pub skulls: u64,
    pub picker: bool,
    players: u32,
    finished: Vec<String>,
}

impl CampaignScreen {
    pub fn new(mcc: &Path) -> Self {
        mccart::init(mcc);
        let has_map = |stem: &str| mcc.join("haloreach").join("maps").join(format!("{stem}.map")).is_file();
        let missions: Vec<usize> = (0..MISSIONS.len()).filter(|&i| has_map(MISSIONS[i].0)).collect();
        let ff_maps: Vec<usize> = (0..FF_MAPS.len()).filter(|&i| has_map(FF_MAPS[i].0)).collect();
        let mut ff_variants: Vec<String> = std::fs::read_dir(mcc.join("haloreach").join("game_variants"))
            .map(|d| d.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.starts_with("hr_ff_") && n.ends_with(".bin")).collect())
            .unwrap_or_default();
        ff_variants.sort_by_key(|v| super::menu::ff_variant_title(v).to_lowercase());
        let ff_variant = ff_variants.iter().position(|v| v == "hr_ff_standard_1set.bin").unwrap_or(0);
        let s = settings::get().campaign;
        let mission = missions.iter().position(|&i| MISSIONS[i].0 == s.mission).unwrap_or(0);
        let skulls = SKULLS.iter().filter(|k| s.skulls.iter().any(|x| x == k.0)).fold(0u64, |b, k| b | 1 << k.2);
        CampaignScreen {
            mcc: mcc.to_path_buf(),
            tab: Tab::Campaign,
            missions,
            ff_maps,
            ff_variants,
            ff_variant,
            mission,
            ff_map: 0,
            difficulty: (s.difficulty as usize).min(3),
            skulls,
            picker: false,
            players: 2,
            finished: s.finished,
        }
    }

    /// Select a mission by map stem (test hook `campaign-m10`).
    pub fn select(&mut self, stem: &str) {
        if let Some(i) = self.missions.iter().position(|&i| MISSIONS[i].0 == stem) {
            self.mission = i;
            self.tab = Tab::Campaign;
        } else if let Some(i) = self.ff_maps.iter().position(|&i| FF_MAPS[i].0 == stem) {
            self.ff_map = i;
            self.tab = Tab::Firefight;
        }
    }

    pub fn set_difficulty(&mut self, name: &str) {
        if let Some(i) = DIFFICULTIES.iter().position(|d| d.0 == name) {
            self.difficulty = i;
        }
    }

    /// Mode flags for the current pick.
    pub fn args(&self) -> Option<Vec<String>> {
        let d = DIFFICULTIES[self.difficulty].0.to_string();
        let mut a: Vec<String> = match self.tab {
            Tab::Campaign => {
                let m = MISSIONS[*self.missions.get(self.mission)?];
                vec!["--campaign".into(), m.0.into(), "--difficulty".into(), d, "--insertion".into(), "0".into()]
            }
            Tab::Firefight => {
                let m = FF_MAPS[*self.ff_maps.get(self.ff_map)?];
                let v = self.mcc.join("haloreach").join("game_variants").join(self.ff_variants.get(self.ff_variant)?);
                vec!["--firefight".into(), m.0.into(), "--difficulty".into(), d, "--game-variant".into(), v.to_string_lossy().into_owned()]
            }
        };
        // Firefight ignores launch skulls (its game variant's own skull settings win; notes/campaign-firefight.md).
        if self.skulls != 0 && self.tab == Tab::Campaign {
            let keys: Vec<&str> = SKULLS.iter().filter(|s| self.skulls >> s.2 & 1 != 0).map(|s| s.0).collect();
            a.extend(["--skulls".into(), keys.join(",")]);
        }
        Some(a)
    }

    fn save(&self) {
        let mut s = settings::get();
        if let Some(&i) = self.missions.get(self.mission) {
            s.campaign.mission = MISSIONS[i].0.into();
        }
        s.campaign.difficulty = self.difficulty as u8;
        s.campaign.skulls = SKULLS.iter().filter(|k| self.skulls >> k.2 & 1 != 0).map(|k| k.0.to_string()).collect();
        settings::save(&s);
    }

    /// (stem, name, number label, tagline, thumbnail, banner) of list row `i` in the current tab.
    fn item(&self, i: usize) -> (&'static str, &'static str, String, &'static str, String, String) {
        match self.tab {
            Tab::Campaign => {
                let m = MISSIONS[self.missions[i]];
                (m.0, m.1, format!("Mission {:02}", self.missions[i] + 1), m.3, format!("HREACH_{}", m.2), format!("HREACH_{}_LARGE", m.2))
            }
            Tab::Firefight => {
                let m = FF_MAPS[self.ff_maps[i]];
                (m.0, m.1, "Firefight".into(), m.3, format!("REACHFIREFIGHT_{}", m.2), format!("REACHFIREFIGHT_{}LARGE", m.2))
            }
        }
    }

    fn count(&self) -> usize {
        if self.tab == Tab::Campaign { self.missions.len() } else { self.ff_maps.len() }
    }

    fn sel(&mut self) -> &mut usize {
        if self.tab == Tab::Campaign { &mut self.mission } else { &mut self.ff_map }
    }

    pub fn ui(&mut self, ui: &mut Ui) -> Action {
        let full = ui.max_rect();
        let mut act = Action::None;
        if !self.picker {
            act = self.keys(ui);
        }
        let m = 32.0;
        let top = Rect::from_min_max(full.min + Vec2::new(m, 22.0), Pos2::new(full.right() - m, full.top() + 74.0));
        // Header: title, tabs, BACK.
        ui.scope_builder(egui::UiBuilder::new().max_rect(top), |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(if self.tab == Tab::Campaign { "CAMPAIGN" } else { "FIREFIGHT" }).size(30.0).color(TEXT));
                ui.add_space(24.0);
                for (label, t) in [("Campaign", Tab::Campaign), ("Firefight", Tab::Firefight)] {
                    if tab_button(ui, label, self.tab == t).clicked() {
                        self.tab = t;
                    }
                }
            });
        });
        let p = ui.painter();
        p.line_segment([Pos2::new(top.left(), top.top() + 42.0), Pos2::new(top.left() + 36.0, top.top() + 42.0)], Stroke::new(2.0, ACCENT));
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
        if self.picker {
            self.skull_picker(ui, full);
        }
        if let Action::Play(_) | Action::Host { .. } = act {
            self.save();
        }
        act
    }

    fn keys(&mut self, ui: &mut Ui) -> Action {
        let n = self.count();
        let (up, down, l, r, enter, esc) = ui.input(|i| {
            use egui::Key::*;
            (i.key_pressed(ArrowUp), i.key_pressed(ArrowDown), i.key_pressed(ArrowLeft), i.key_pressed(ArrowRight), i.key_pressed(Enter), i.key_pressed(Escape))
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
        if l {
            self.difficulty = self.difficulty.saturating_sub(1);
        }
        if r {
            self.difficulty = (self.difficulty + 1).min(3);
        }
        if esc {
            return Action::Back;
        }
        if enter {
            if let Some(a) = self.args() {
                return Action::Play(a);
            }
        }
        Action::None
    }

    fn list(&mut self, ui: &mut Ui, rect: Rect) {
        let p = ui.painter().clone();
        p.rect_filled(rect, 2.0, PANEL);
        p.rect_stroke(rect, 2.0, Stroke::new(1.0, LINE), StrokeKind::Inside);
        let inner = rect.shrink2(Vec2::new(14.0, 14.0));
        let cap = if self.tab == Tab::Campaign { "MISSIONS" } else { "MAPS" };
        p.text(inner.left_top(), Align2::LEFT_TOP, cap, FontId::proportional(12.0), DIM);
        if self.tab == Tab::Campaign {
            let done = self.missions.iter().filter(|&&i| self.finished.iter().any(|f| f == MISSIONS[i].0)).count();
            p.text(inner.right_top(), Align2::RIGHT_TOP, format!("{done} / {} FINISHED", self.missions.len()), FontId::proportional(12.0), DIM);
        }
        let area = Rect::from_min_max(inner.min + Vec2::new(0.0, 24.0), inner.max);
        let n = self.count();
        if n == 0 {
            p.text(area.left_top(), Align2::LEFT_TOP, "No maps found in your MCC files.", FontId::proportional(15.0), DIM);
            return;
        }
        // Rows share the height when there is room, else scroll.
        let row_h = (area.height() / n as f32).clamp(46.0, 76.0);
        let thumb = Vec2::new((row_h - 10.0) * 316.0 / 160.0, row_h - 10.0);
        let ctx = ui.ctx().clone();
        ui.scope_builder(egui::UiBuilder::new().max_rect(area), |ui| {
            egui::ScrollArea::vertical().id_salt("camp-list").max_height(area.height()).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for i in 0..n {
                    let (_, name, num, _, th, _) = self.item(i);
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
                        None => placeholder(p, tr, &num),
                    }
                    p.rect_stroke(tr, 0.0, Stroke::new(1.0, if selected { ACCENT } else { LINE }), StrokeKind::Inside);
                    let x = tr.right() + 12.0;
                    p.text(Pos2::new(x, r.center().y - 2.0), Align2::LEFT_BOTTOM, num.to_uppercase(), FontId::proportional(11.5), if selected { ACCENT } else { DIM });
                    p.text(Pos2::new(x, r.center().y), Align2::LEFT_TOP, name, FontId::proportional(17.0), if selected { Color32::WHITE } else { TEXT });
                    if self.tab == Tab::Campaign && self.finished.iter().any(|f| f == self.item(i).0) {
                        p.text(Pos2::new(r.right() - 10.0, r.center().y), Align2::RIGHT_CENTER, "✔", FontId::proportional(15.0), ACCENT);
                    }
                    if resp.clicked() {
                        *self.sel() = i;
                    }
                }
            });
        });
    }

    fn detail(&mut self, ui: &mut Ui, rect: Rect) -> Option<Action> {
        let ctx = ui.ctx().clone();
        let n = self.count();
        if n == 0 {
            return None;
        }
        let i = (*self.sel()).min(n - 1);
        let (_, name, num, tag, _, banner) = self.item(i);
        let p = ui.painter().clone();

        // Banner: picture (or a plain card), darkened towards the bottom, number / name / tagline on it.
        let bh = (rect.width() / (668.0 / 292.0)).min(rect.height() - 330.0).max(120.0);
        let br = Rect::from_min_size(rect.min, Vec2::new(rect.width(), bh));
        match mccart::texture(&ctx, &banner) {
            Some(t) => cover(&p, &t, br, Color32::WHITE),
            None => placeholder(&p, br, ""),
        }
        let mut mesh = egui::Mesh::default();
        let fade_top = br.top() + bh * 0.35;
        let clear = Color32::from_black_alpha(0);
        let dark = Color32::from_black_alpha(215);
        mesh.colored_vertex(Pos2::new(br.left(), fade_top), clear);
        mesh.colored_vertex(Pos2::new(br.right(), fade_top), clear);
        mesh.colored_vertex(br.right_bottom(), dark);
        mesh.colored_vertex(br.left_bottom(), dark);
        mesh.add_triangle(0, 1, 2);
        mesh.add_triangle(0, 2, 3);
        p.add(mesh);
        p.rect_stroke(br, 0.0, Stroke::new(1.0, LINE), StrokeKind::Inside);
        let tx = br.left() + 22.0;
        let tag_r = p.text(Pos2::new(tx, br.bottom() - 16.0), Align2::LEFT_BOTTOM, tag, FontId::proportional(15.0), Color32::from_gray(210));
        let name_r = p.text(Pos2::new(tx, tag_r.top() - 2.0), Align2::LEFT_BOTTOM, name.to_uppercase(), FontId::proportional(34.0), Color32::WHITE);
        p.text(Pos2::new(tx, name_r.top()), Align2::LEFT_BOTTOM, num.to_uppercase(), FontId::proportional(14.0), ACCENT);

        // Difficulty cards.
        let mut y = br.bottom() + 16.0;
        p.text(Pos2::new(rect.left(), y), Align2::LEFT_TOP, "DIFFICULTY", FontId::proportional(12.0), DIM);
        y += 20.0;
        let gap = 12.0;
        let cw = (rect.width() - gap * 3.0) / 4.0;
        let ch = 112.0;
        for (k, d) in DIFFICULTIES.iter().enumerate() {
            let r = Rect::from_min_size(Pos2::new(rect.left() + k as f32 * (cw + gap), y), Vec2::new(cw, ch));
            let resp = ui.interact(r, ui.id().with(("diff", k)), Sense::click());
            let on = self.difficulty == k;
            p.rect_filled(r, 2.0, if on { ACCENT.gamma_multiply(0.18) } else if resp.hovered() { HOVER } else { PANEL });
            p.rect_stroke(r, 2.0, Stroke::new(if on { 2.0 } else { 1.0 }, if on { ACCENT } else if resp.hovered() { ACCENT.gamma_multiply(0.6) } else { LINE }), StrokeKind::Inside);
            let icon = Rect::from_min_size(r.min + Vec2::new(12.0, 12.0), Vec2::splat(46.0));
            match mccart::texture(&ctx, d.3) {
                Some(t) => {
                    p.image(t.id(), icon, full_uv(&t), if on { Color32::WHITE } else { Color32::from_gray(170) });
                }
                None => {
                    p.text(icon.center(), Align2::CENTER_CENTER, (k + 1).to_string(), FontId::proportional(30.0), if on { ACCENT } else { DIM });
                }
            }
            p.text(Pos2::new(icon.right() + 10.0, icon.center().y), Align2::LEFT_CENTER, d.1.to_uppercase(), FontId::proportional(18.0), if on { Color32::WHITE } else { TEXT });
            let g = p.layout(d.2.to_string(), FontId::proportional(12.5), DIM, r.width() - 24.0);
            p.galley(Pos2::new(r.left() + 12.0, icon.bottom() + 8.0), g, DIM);
            if resp.clicked() {
                self.difficulty = k;
            }
        }
        y += ch + 16.0;

        // Firefight game type (◀ name ▶) and the modifiers row.
        let row_h = 54.0;
        let mut mods = Rect::from_min_size(Pos2::new(rect.left(), y + 20.0), Vec2::new(rect.width(), row_h));
        if self.tab == Tab::Firefight {
            let gt = mods;
            p.text(Pos2::new(gt.left(), y), Align2::LEFT_TOP, "GAME TYPE", FontId::proportional(12.0), DIM);
            p.rect_filled(gt, 2.0, PANEL);
            p.rect_stroke(gt, 2.0, Stroke::new(1.0, LINE), StrokeKind::Inside);
            let n = self.ff_variants.len().max(1);
            for (dir, a) in [(-1i32, Align2::LEFT_CENTER), (1, Align2::RIGHT_CENTER)] {
                let ar = if dir < 0 { Rect::from_min_size(gt.min, Vec2::new(44.0, row_h)) } else { Rect::from_min_size(Pos2::new(gt.right() - 44.0, gt.top()), Vec2::new(44.0, row_h)) };
                let resp = ui.interact(ar, ui.id().with(("gt", dir)), Sense::click());
                let x = if dir < 0 { ar.left() + 16.0 } else { ar.right() - 16.0 };
                p.text(Pos2::new(x, ar.center().y), a, if dir < 0 { "◀" } else { "▶" }, FontId::proportional(16.0), if resp.hovered() { ACCENT } else { DIM });
                if resp.clicked() {
                    self.ff_variant = (self.ff_variant as i32 + dir).rem_euclid(n as i32) as usize;
                }
            }
            let v = self.ff_variants.get(self.ff_variant).map(|v| super::menu::ff_variant_title(v)).unwrap_or_else(|| "No Firefight game types found".into());
            p.text(gt.center(), Align2::CENTER_CENTER, v, FontId::proportional(17.0), TEXT);
            p.text(Pos2::new(gt.left(), gt.bottom() + 8.0), Align2::LEFT_TOP, "Skulls come from the game type in Firefight.", FontId::proportional(12.5), DIM);
            mods = Rect::NOTHING;
        }
        if mods == Rect::NOTHING {
            return self.buttons(ui, rect);
        }
        p.text(Pos2::new(mods.left(), y), Align2::LEFT_TOP, if self.tab == Tab::Campaign { "CAMPAIGN MODIFIERS" } else { "MODIFIERS" }, FontId::proportional(12.0), DIM);
        let resp = ui.interact(mods, ui.id().with("mods"), Sense::click());
        p.rect_filled(mods, 2.0, if resp.hovered() { HOVER } else { PANEL });
        p.rect_stroke(mods, 2.0, Stroke::new(1.0, if resp.hovered() { ACCENT } else { LINE }), StrokeKind::Inside);
        let skull_r = Rect::from_min_size(mods.min + Vec2::new(12.0, 9.0), Vec2::splat(36.0));
        match mccart::texture(&ctx, "SKULL") {
            Some(t) => {
                p.image(t.id(), skull_r, full_uv(&t), Color32::from_gray(200));
            }
            None => {
                p.text(skull_r.center(), Align2::CENTER_CENTER, "☠", FontId::proportional(24.0), DIM);
            }
        }
        let on: Vec<_> = SKULLS.iter().filter(|s| self.skulls >> s.2 & 1 != 0).collect();
        let mut x = skull_r.right() + 12.0;
        if on.is_empty() {
            p.text(Pos2::new(x, mods.center().y), Align2::LEFT_CENTER, "Skulls: none", FontId::proportional(16.0), TEXT);
        } else {
            let label = if on.len() == 1 { on[0].1.to_string() } else { format!("{} skulls", on.len()) };
            let r = p.text(Pos2::new(x, mods.center().y), Align2::LEFT_CENTER, label, FontId::proportional(16.0), Color32::WHITE);
            x = r.right() + 12.0;
            for s in on.iter().take(((mods.right() - 120.0 - x) / 34.0).max(0.0) as usize) {
                let ir = Rect::from_min_size(Pos2::new(x, mods.center().y - 15.0), Vec2::splat(30.0));
                if let Some(t) = mccart::texture(&ctx, skull_info(s.0).0) {
                    p.image(t.id(), ir, full_uv(&t), Color32::WHITE);
                }
                x += 34.0;
            }
        }
        p.text(Pos2::new(mods.right() - 14.0, mods.center().y), Align2::RIGHT_CENTER, "CHANGE  ›", FontId::proportional(13.0), if resp.hovered() { ACCENT } else { DIM });
        if resp.clicked() {
            self.picker = true;
        }
        self.buttons(ui, rect)
    }

    /// PLAY SOLO / HOST CO-OP / players along the bottom of `rect`.
    fn buttons(&mut self, ui: &mut Ui, rect: Rect) -> Option<Action> {
        let by = rect.bottom() - 48.0;
        let mut out = None;
        let args = self.args();
        let mut row = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_max(Pos2::new(rect.left(), by), rect.max)).layout(egui::Layout::left_to_right(egui::Align::Center)));
        if theme::primary_button(&mut row, "Play solo", 220.0, args.is_some()).clicked() {
            out = args.clone().map(Action::Play);
        }
        row.add_space(8.0);
        if big_ghost(&mut row, "Host co-op", 200.0, args.is_some()).clicked() {
            out = args.clone().map(|a| Action::Host { players: self.players, args: a });
        }
        row.add_space(14.0);
        theme::caption(&mut row, "Players");
        row.add(egui::Slider::new(&mut self.players, 2..=4));
        let hint = if self.tab == Tab::Campaign { "Friends join before the mission starts." } else { "Friends join at any time." };
        row.add(egui::Label::new(egui::RichText::new(hint).size(12.5).color(DIM)).wrap());
        out
    }

    fn skull_picker(&mut self, ui: &mut Ui, full: Rect) {
        let ctx = ui.ctx().clone();
        let layer = egui::LayerId::new(egui::Order::Foreground, ui.id().with("skull-picker"));
        let p = ctx.layer_painter(layer);
        p.rect_filled(full, 0.0, Color32::from_black_alpha(170));
        let cols = 5usize;
        let rows = SKULLS.len().div_ceil(cols);
        let w = (full.width() - 120.0).min(980.0);
        let card = Vec2::new((w - 48.0 - 12.0 * (cols - 1) as f32) / cols as f32, 150.0);
        let h = (78.0 + rows as f32 * (card.y + 12.0) + 70.0).min(full.height() - 40.0);
        let panel = Rect::from_center_size(full.center(), Vec2::new(w, h));
        p.rect_filled(panel, 2.0, Color32::from_rgb(16, 18, 23));
        p.rect_stroke(panel, 2.0, Stroke::new(1.0, ACCENT.gamma_multiply(0.5)), StrokeKind::Inside);
        let inner = panel.shrink(24.0);
        p.text(inner.left_top(), Align2::LEFT_TOP, "SKULLS", FontId::proportional(26.0), TEXT);
        p.text(inner.left_top() + Vec2::new(0.0, 34.0), Align2::LEFT_TOP, "Click a skull to turn it on or off. They apply to the whole co-op game.", FontId::proportional(13.0), DIM);
        let mut ui2 = egui::Ui::new(ctx.clone(), ui.id().with("skull-ui"), egui::UiBuilder::new().layer_id(layer).max_rect(full));
        // Swallow pointer input over the whole window so the screen underneath does not react.
        let _ = ui2.interact(full, ui2.id().with("blocker"), Sense::click_and_drag());
        for (k, s) in SKULLS.iter().enumerate() {
            let (c, r) = (k % cols, k / cols);
            let rr = Rect::from_min_size(inner.min + Vec2::new(c as f32 * (card.x + 12.0), 66.0 + r as f32 * (card.y + 12.0)), card);
            let resp = ui2.interact(rr, ui2.id().with(("skull", k)), Sense::click());
            let on = self.skulls >> s.2 & 1 != 0;
            p.rect_filled(rr, 2.0, if on { ACCENT.gamma_multiply(0.20) } else if resp.hovered() { HOVER } else { PANEL });
            p.rect_stroke(rr, 2.0, Stroke::new(if on { 2.0 } else { 1.0 }, if on { ACCENT } else if resp.hovered() { ACCENT.gamma_multiply(0.6) } else { LINE }), StrokeKind::Inside);
            let (tex, desc) = skull_info(s.0);
            let ir = Rect::from_center_size(Pos2::new(rr.center().x, rr.top() + 34.0), Vec2::splat(52.0));
            match mccart::texture(&ctx, tex) {
                Some(t) => {
                    p.image(t.id(), ir, full_uv(&t), if on { Color32::WHITE } else { Color32::from_gray(150) });
                }
                None => {
                    p.text(ir.center(), Align2::CENTER_CENTER, "☠", FontId::proportional(30.0), if on { ACCENT } else { DIM });
                }
            }
            p.text(Pos2::new(rr.center().x, ir.bottom() + 6.0), Align2::CENTER_TOP, s.1.to_uppercase(), FontId::proportional(14.5), if on { Color32::WHITE } else { TEXT });
            let g = p.layout(desc.to_string(), FontId::proportional(11.5), DIM, rr.width() - 16.0);
            let gx = rr.center().x - g.size().x / 2.0;
            p.galley(Pos2::new(gx, ir.bottom() + 28.0), g, DIM);
            if resp.clicked() {
                self.skulls ^= 1 << s.2;
            }
        }
        let foot = Rect::from_min_max(Pos2::new(inner.left(), inner.bottom() - 44.0), inner.max);
        let mut row = ui2.new_child(egui::UiBuilder::new().max_rect(foot).layout(egui::Layout::left_to_right(egui::Align::Center)));
        let done = theme::primary_button(&mut row, "Done", 160.0, true).clicked();
        row.add_space(8.0);
        if theme::ghost_button(&mut row, "Clear all").clicked() {
            self.skulls = 0;
        }
        row.label(egui::RichText::new(format!("{} on", SKULLS.iter().filter(|s| self.skulls >> s.2 & 1 != 0).count())).color(DIM));
        if done || ctx.input(|i| i.key_pressed(egui::Key::Escape) || i.key_pressed(egui::Key::Enter)) {
            self.picker = false;
        }
    }
}

/// Paint `t` filling `r` (cropped to keep its aspect).
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
    p.image(t.id(), r, inset(t, uv), tint);
}

/// The whole texture, half a texel in from each edge: egui-directx11 samples with BORDER addressing and a white
/// border colour, so linear filtering at uv 0 / 1 bleeds a grey line along the image edge.
fn full_uv(t: &TextureHandle) -> Rect {
    inset(t, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)))
}

fn inset(t: &TextureHandle, uv: Rect) -> Rect {
    let [w, h] = t.size();
    let (dx, dy) = (0.5 / w as f32, 0.5 / h as f32);
    Rect::from_min_max(Pos2::new(uv.min.x.max(dx), uv.min.y.max(dy)), Pos2::new(uv.max.x.min(1.0 - dx), uv.max.y.min(1.0 - dy)))
}

/// Text-only stand-in when the player's install has no picture.
fn placeholder(p: &egui::Painter, r: Rect, label: &str) {
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

/// Outlined button as tall as `primary_button`.
fn big_ghost(ui: &mut Ui, label: &str, width: f32, enabled: bool) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(Vec2::new(width, 44.0), if enabled { Sense::click() } else { Sense::hover() });
    let hot = enabled && resp.hovered();
    ui.painter().rect_filled(r, 2.0, if hot { HOVER } else { Color32::TRANSPARENT });
    ui.painter().rect_stroke(r, 2.0, Stroke::new(1.5, if hot { ACCENT } else { ACCENT.gamma_multiply(0.55) }), StrokeKind::Inside);
    ui.painter().text(r.center(), Align2::CENTER_CENTER, label.to_uppercase(), FontId::proportional(19.0), if enabled { TEXT } else { DIM });
    resp
}

/// Listen host's campaign lobby (overlay passive layer while lobby.rs holds launch state 8): who is in, Enter to
/// start now, the countdown, and where friends can reach us.
pub fn lobby_panel(ui: &mut Ui, players: u32, max: u32, secs: u32, host: bool) {
    let full = ui.max_rect();
    let p = ui.painter();
    p.rect_filled(full, 0.0, Color32::from_black_alpha(150));
    let info = super::hostinfo::lines();
    let h = 190.0 + info.as_ref().map_or(0.0, |(l, _)| l.len() as f32 * 20.0 + 26.0);
    let r = Rect::from_center_size(full.center(), Vec2::new(560.0f32.min(full.width() - 40.0), h));
    p.rect_filled(r, 2.0, Color32::from_rgb(16, 18, 23));
    p.rect_stroke(r, 2.0, Stroke::new(1.0, ACCENT.gamma_multiply(0.5)), StrokeKind::Inside);
    let x = r.left() + 28.0;
    let mut y = r.top() + 24.0;
    p.text(Pos2::new(x, y), Align2::LEFT_TOP, "CAMPAIGN CO-OP", FontId::proportional(13.0), ACCENT);
    y += 22.0;
    p.text(Pos2::new(x, y), Align2::LEFT_TOP, format!("Waiting for friends  {players} / {max}"), FontId::proportional(28.0), Color32::WHITE);
    y += 44.0;
    // One pip per seat.
    for i in 0..max {
        let c = Rect::from_min_size(Pos2::new(x + i as f32 * 34.0, y), Vec2::new(26.0, 8.0));
        p.rect_filled(c, 1.0, if i < players { ACCENT } else { Color32::from_gray(60) });
    }
    y += 24.0;
    p.text(Pos2::new(x, y), Align2::LEFT_TOP, if host { "Press Enter to start now" } else { "Joined. Waiting for the others ..." }, FontId::proportional(17.0), TEXT);
    y += 26.0;
    let left = if secs > 0 { format!("The mission starts in {secs} s, or as soon as everyone is in.") } else { "The mission starts as soon as everyone is in.".into() };
    p.text(Pos2::new(x, y), Align2::LEFT_TOP, left, FontId::proportional(13.5), DIM);
    y += 22.0;
    p.text(Pos2::new(x, y), Align2::LEFT_TOP, "Nobody can join once the mission has started.", FontId::proportional(13.5), DIM);
    if let Some((lines, _)) = info {
        y += 34.0;
        for l in lines {
            p.text(Pos2::new(x, y), Align2::LEFT_TOP, l, FontId::proportional(13.5), TEXT);
            y += 20.0;
        }
    }
}
