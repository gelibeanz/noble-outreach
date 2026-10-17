//! The CUSTOMIZE screen: Spartan / Elite appearance, laid out like Reclaimer's (notes/frontend-reference.md). Left: the
//! player card and one row per piece / colour with its current value; centre: a flat preview (colour chips on a
//! drawn figure, piece names); right: the picker for the selected row (names, or Reach's colour swatches). Pointing
//! at a choice tries it on in the preview, clicking equips it (saved to settings.json and written into the slot 34
//! profile; a running engine re-reads it via message 5). Ids are MCC's customisation items, notes/customization.md.

use egui::{Align2, Color32, FontId, Rect, Sense, Stroke, StrokeKind, Ui, Vec2};

use super::settings::{self, Armour, Settings};
use super::theme::{self, ACCENT, DIM, HOVER, LINE, PANEL, TEXT};

/// Helmets: (MCC id, helmet, attachment). Attachment variants of one helmet are separate ids.
const HELMETS: &[(u32, &str, &str)] = &[
    (0x00, "Mark V [B]", "None"), (0x01, "Mark V [B]", "UA"), (0x02, "Mark V [B]", "UA/HUL"),
    (0x03, "CQC", "None"), (0x04, "CQC", "CBRN"), (0x05, "CQC", "UA/HUL"),
    (0x06, "ODST", "None"), (0x07, "ODST", "UA/CNM"), (0x08, "ODST", "CBRN/HUL"),
    (0x09, "HAZOP", "None"), (0x0A, "HAZOP", "CBRN/HUL"), (0x0B, "HAZOP", "CNM-I"),
    (0x0C, "EOD", "None"), (0x0D, "EOD", "CNM"), (0x0E, "EOD", "UA/HUL"),
    (0x0F, "Operator", "None"), (0x10, "Operator", "UA/HUL"), (0x11, "Operator", "CNM"),
    (0x12, "Grenadier", "None"), (0x13, "Grenadier", "UA"), (0x14, "Grenadier", "UA/FC"),
    (0x15, "Air Assault", "None"), (0x16, "Air Assault", "UA/CNM"), (0x17, "Air Assault", "FC-I"),
    (0x18, "Scout", "None"), (0x19, "Scout", "HU/RS"), (0x1A, "Scout", "CBRN/CNM"),
    (0x1B, "EVA", "None"), (0x1C, "EVA", "CNM"), (0x1D, "EVA", "UA/HUL3"),
    (0x1E, "JFO", "None"), (0x1F, "JFO", "HUL-I"), (0x20, "JFO", "UA"),
    (0x21, "Commando", "None"), (0x22, "Commando", "CBRN/CNM"), (0x23, "Commando", "UA/FC-I2"),
    (0x24, "EVA [C]", "None"), (0x25, "EVA [C]", "CNM"), (0x26, "EVA [C]", "UA/HUL3"),
    (0x27, "Mjolnir Mk V", "None"), (0x28, "Mjolnir Mk V", "CNM"), (0x29, "Mjolnir Mk V", "UA"),
    (0x2A, "Pilot", "None"), (0x2B, "Pilot", "HUL3"), (0x2C, "Pilot", "UA/HUL3"), (0x2D, "Pilot", "Haunted"),
    (0x2E, "Recon", "None"), (0x2F, "Recon", "HUL"), (0x30, "Recon", "UA/HUL3"),
    (0x31, "Mark VI", "None"), (0x32, "Mark VI", "FC-I2"), (0x33, "Mark VI", "UA/HUL-I"),
    (0x34, "Gungnir", "None"), (0x35, "Gungnir", "HU/RS"), (0x36, "Gungnir", "CBRN"),
    (0x37, "Security", "None"), (0x38, "Security", "UA/HUL"), (0x39, "Security", "CBRN/CNM"),
    (0x3A, "Military Police", "None"), (0x3B, "Military Police", "CBRN/HU/RS"), (0x3C, "Military Police", "HU/RS/CNM"),
    (0x3D, "CQB", "None"), (0x3E, "CQB", "HU/RS/CNM"), (0x3F, "CQB", "UA/HUL"),
    (0x40, "Mariner", "None"), (0x41, "Mariner", "Defiant"), (0x42, "Mariner", "Mister Chief"), (0x43, "Mariner", "Optrex"),
    (0x44, "GRD", "None"), (0x45, "GRD", "Optrex"), (0x46, "GRD", "CBRN"), (0x47, "GRD", "Optrex/CBRN"),
    (0x48, "Belos", "Corinthian"), (0x49, "Belos", "Transverse"), (0x4A, "Belos", "Kerata"),
    (0x4B, "Belos", "Leontokardos"), (0x4C, "Belos", "Stheno"), (0x4D, "Belos", "Cerberus"),
    (0x4E, "Blackguard", "None"), (0x4F, "Blackguard", "Bishop"), (0x50, "Blackguard", "Dragon"),
    (0x51, "Blackguard", "Wind"), (0x52, "Blackguard", "Visage"), (0x53, "Blackguard", "Crown"),
    (0x54, "Drengr", "None"), (0x55, "Drengr", "Valdemar"), (0x56, "Drengr", "Dvalinn"),
    (0x57, "Drengr", "Nari"), (0x58, "Drengr", "Thyra"), (0x59, "Drengr", "Tyr"),
];

const SHOULDERS: &[&str] = &[
    "Default", "FJ/PARA", "HAZOP", "JFO", "Recon", "UA/Multi-Threat", "Jump Jet", "EVA", "Gungnir", "ODST",
    "UA/Base Security", "CQC", "Operator", "Commando", "Grenadier", "Sniper", "Mjolnir Mk V", "Security",
];
const CHESTS: &[(u32, &str)] = &[
    (0x7E, "Default"), (0x7F, "HP/HALO"), (0x80, "UA/Counterassault"), (0x81, "Tactical/LRP"), (0x82, "UA/ODST"),
    (0x83, "Tactical/Recon"), (0x84, "Collar/Grenadier"), (0x85, "Tactical/Patrol"), (0x86, "Collar/Breacher"),
    (0x87, "Assault/Sapper"), (0x88, "Assault/Commando"), (0x89, "HP/Parafoil"), (0x8A, "Collar/Grenadier [UA]"),
    (0x8B, "UA/Base Security [W]"), (0x8C, "Collar/Breacher [R]"), (0x8D, "HP/Parafoil [R]"), (0x8E, "Assault/Sapper [R]"),
    (0x8F, "Condemned"), (0xE8, "UA/Base Security"), (0xE9, "UA/Multi-Threat"), (0xEA, "UA/Multi-Threat [W]"),
    (0xEB, "Default (prosthetic)"), (0xEC, "Tactical/Patrol (prosthetic)"), (0xED, "Tactical/Recon (prosthetic)"),
    (0xEE, "Collar/Grenadier [UA] (prosthetic)"), (0xEF, "UA/Counterassault (prosthetic)"),
];
const WRISTS: &[&str] = &["Default", "UA/Buckler", "UA/Bracer", "Tactical/TACPAD", "Tactical/UGPS", "Assault/Breacher"];
const UTILITIES: &[&str] = &["Default", "UA/Chobham", "Tactical/Hard Case", "UA/NxRA", "Tactical/Trauma Kit", "Tactical/Soft Case", "Vacuum Flask"];
const KNEES: &[&str] = &["Default", "FJ/PARA", "Gungnir", "Grenadier"];
const VISORS: &[(&str, Color32)] = &[
    ("Default", Color32::from_rgb(214, 160, 52)),
    ("Silver", Color32::from_rgb(196, 202, 208)),
    ("Blue", Color32::from_rgb(70, 120, 210)),
    ("Black", Color32::from_rgb(30, 32, 36)),
    ("Gold", Color32::from_rgb(232, 196, 70)),
];
const EFFECTS: &[&str] = &["None", "Birthday Party", "Heart Attack", "Pestilence", "Inclement Weather", "Flaming Helmet", "Blue Flames"];
const VOICES: &[&str] = &[
    "Noble Six", "Jorge-052", "Jun-A266", "Emile-A239", "Kat-B320", "Auntie Dot", "Carter-A259", "Gunnery Sgt. Stacker",
    "Gunnery Sgt. Buck", "Sgt. Maj. Johnson", "Cortana", "John-117",
];
const ELITES: &[&str] = &["Minor", "Spec Ops", "Ranger", "Ultra", "Zealot", "General", "Field Marshal", "Officer"];
const BODIES: &[&str] = &["Male", "Female"];

/// Reach's 30 armour colours in palette order (MCC ids 0xCA..0xE7, engine index 0..29), three shades per hue.
/// The RGB values are our own approximations for the swatches; the engine draws its own palette.
pub const COLOURS: &[(&str, Color32)] = &[
    ("Steel", Color32::from_rgb(92, 94, 96)), ("Silver", Color32::from_rgb(150, 152, 156)), ("White", Color32::from_rgb(214, 214, 210)),
    ("Brown", Color32::from_rgb(78, 54, 38)), ("Tan", Color32::from_rgb(138, 106, 74)), ("Khaki", Color32::from_rgb(186, 160, 118)),
    ("Sage", Color32::from_rgb(106, 120, 92)), ("Olive", Color32::from_rgb(88, 88, 44)), ("Drab", Color32::from_rgb(104, 98, 72)),
    ("Forest", Color32::from_rgb(34, 66, 38)), ("Green", Color32::from_rgb(58, 118, 52)), ("Seafoam", Color32::from_rgb(98, 166, 138)),
    ("Teal", Color32::from_rgb(28, 96, 98)), ("Aqua", Color32::from_rgb(36, 152, 162)), ("Cyan", Color32::from_rgb(88, 206, 226)),
    ("Blue", Color32::from_rgb(38, 58, 138)), ("Cobalt", Color32::from_rgb(58, 92, 198)), ("Ice", Color32::from_rgb(140, 182, 228)),
    ("Violet", Color32::from_rgb(78, 42, 128)), ("Orchid", Color32::from_rgb(146, 82, 186)), ("Lavender", Color32::from_rgb(186, 160, 222)),
    ("Maroon", Color32::from_rgb(88, 22, 30)), ("Brick", Color32::from_rgb(148, 42, 32)), ("Rose", Color32::from_rgb(208, 90, 118)),
    ("Rust", Color32::from_rgb(156, 72, 30)), ("Coral", Color32::from_rgb(228, 112, 80)), ("Peach", Color32::from_rgb(238, 170, 128)),
    ("Gold", Color32::from_rgb(186, 138, 32)), ("Yellow", Color32::from_rgb(228, 198, 52)), ("Pale", Color32::from_rgb(238, 228, 160)),
];
const COLOUR_BASE: u32 = 0xCA;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Field {
    Body,
    Helmet,
    Attachment,
    LeftShoulder,
    RightShoulder,
    Chest,
    Wrist,
    Utility,
    Knees,
    Visor,
    Effect,
    EliteArmour,
    EliteEffect,
    Primary,
    Secondary,
    Voice,
}

const SPARTAN_ROWS: &[(&str, &[Field])] = &[
    ("Armor", &[Field::Helmet, Field::Attachment, Field::LeftShoulder, Field::RightShoulder, Field::Chest, Field::Wrist, Field::Utility, Field::Knees, Field::Visor, Field::Effect]),
    ("Colors", &[Field::Primary, Field::Secondary]),
    ("Spartan", &[Field::Body, Field::Voice]),
];
const ELITE_ROWS: &[(&str, &[Field])] = &[
    ("Armor", &[Field::EliteArmour, Field::EliteEffect]),
    ("Colors", &[Field::Primary, Field::Secondary]),
    ("Firefight", &[Field::Voice]),
];

impl Field {
    fn label(self) -> &'static str {
        match self {
            Field::Body => "Gender",
            Field::Helmet => "Helmet",
            Field::Attachment => "Helmet attachment",
            Field::LeftShoulder => "Left shoulder",
            Field::RightShoulder => "Right shoulder",
            Field::Chest => "Chest",
            Field::Wrist => "Wrist",
            Field::Utility => "Utility",
            Field::Knees => "Knee guards",
            Field::Visor => "Visor color",
            Field::Effect | Field::EliteEffect => "Armor effect",
            Field::EliteArmour => "Elite armor",
            Field::Primary => "Primary",
            Field::Secondary => "Secondary",
            Field::Voice => "Firefight voice",
        }
    }

    fn is_colour(self) -> bool {
        matches!(self, Field::Primary | Field::Secondary)
    }

    /// Every choice for this field as (MCC id, name), given the current armour (attachments depend on the helmet).
    fn choices(self, a: &Armour) -> Vec<(u32, String)> {
        let seq = |base: u32, names: &[&str]| names.iter().enumerate().map(|(i, n)| (base + i as u32, n.to_string())).collect::<Vec<_>>();
        match self {
            Field::Body => seq(0xC0, BODIES),
            Field::Helmet => {
                let mut v: Vec<(u32, String)> = Vec::new();
                for (id, name, _) in HELMETS {
                    if !v.iter().any(|(_, n)| n == name) {
                        v.push((*id, name.to_string()));
                    }
                }
                v
            }
            Field::Attachment => {
                let family = helmet_family(a.helmet);
                HELMETS.iter().filter(|h| h.1 == family).map(|h| (h.0, h.2.to_string())).collect()
            }
            Field::LeftShoulder => seq(0x5A, SHOULDERS),
            Field::RightShoulder => seq(0x6C, SHOULDERS),
            Field::Chest => CHESTS.iter().map(|(i, n)| (*i, n.to_string())).collect(),
            Field::Wrist => seq(0x90, WRISTS),
            Field::Utility => seq(0x96, UTILITIES),
            Field::Knees => seq(0xA2, KNEES),
            Field::Visor => VISORS.iter().enumerate().map(|(i, v)| (0x9D + i as u32, v.0.to_string())).collect(),
            Field::Effect => seq(0xA6, EFFECTS),
            Field::EliteEffect => seq(0xAD, EFFECTS),
            Field::EliteArmour => seq(0xC2, ELITES),
            Field::Voice => seq(0xB4, VOICES),
            Field::Primary | Field::Secondary => COLOURS.iter().enumerate().map(|(i, c)| (COLOUR_BASE + i as u32, c.0.to_string())).collect(),
        }
    }

    fn get(self, a: &Armour) -> u32 {
        match self {
            Field::Body => a.body,
            Field::Helmet | Field::Attachment => a.helmet,
            Field::LeftShoulder => a.left_shoulder,
            Field::RightShoulder => a.right_shoulder,
            Field::Chest => a.chest,
            Field::Wrist => a.wrist,
            Field::Utility => a.utility,
            Field::Knees => a.knees,
            Field::Visor => a.visor,
            Field::Effect => a.effect,
            Field::EliteEffect => a.elite_effect,
            Field::EliteArmour => a.elite_armour,
            Field::Voice => a.voice,
            Field::Primary => a.primary,
            Field::Secondary => a.secondary,
        }
    }

    fn set(self, a: &mut Armour, v: u32) {
        match self {
            Field::Body => a.body = v,
            Field::Helmet | Field::Attachment => a.helmet = v,
            Field::LeftShoulder => a.left_shoulder = v,
            Field::RightShoulder => a.right_shoulder = v,
            Field::Chest => a.chest = v,
            Field::Wrist => a.wrist = v,
            Field::Utility => a.utility = v,
            Field::Knees => a.knees = v,
            Field::Visor => a.visor = v,
            Field::Effect => a.effect = v,
            Field::EliteEffect => a.elite_effect = v,
            Field::EliteArmour => a.elite_armour = v,
            Field::Voice => a.voice = v,
            Field::Primary => a.primary = v,
            Field::Secondary => a.secondary = v,
        }
    }

    /// Name of the current value (helmet: the helmet only; attachment: the attachment only).
    fn value_name(self, a: &Armour) -> String {
        let v = self.get(a);
        if self == Field::Helmet {
            return helmet_family(v).to_string();
        }
        self.choices(a).into_iter().find(|c| c.0 == v).map_or_else(|| format!("#{v:#x}"), |c| c.1)
    }
}

fn helmet_family(id: u32) -> &'static str {
    HELMETS.iter().find(|h| h.0 == id).map_or("Mark V [B]", |h| h.1)
}

pub fn colour(id: u32) -> Color32 {
    COLOURS.get(id.wrapping_sub(COLOUR_BASE) as usize).map_or(COLOURS[0].1, |c| c.1)
}

pub struct CustomizeScreen {
    s: Settings,
    field: Field,
    /// Hovered choice: shown in the preview ("trying on") until the pointer leaves it.
    trying: Option<(Field, u32)>,
    /// 3D preview camera (preview.rs): yaw (radians, 0 = facing us), pitch, distance.
    yaw: f32,
    pitch: f32,
    dist: f32,
}

/// Default 3D preview camera: facing the model, slightly above, whole body in frame.
const VIEW_YAW: f32 = 0.35;
const VIEW_PITCH: f32 = 0.05;
const VIEW_DIST: f32 = 6.4;

impl CustomizeScreen {
    pub fn new() -> Self {
        crate::preview::start(); // the engine-rendered model (a hidden child process, ~20 s to load)
        let s = settings::get();
        let field = if s.armour.elite { Field::EliteArmour } else { Field::Helmet };
        CustomizeScreen { s, field, trying: None, yaw: VIEW_YAW, pitch: VIEW_PITCH, dist: VIEW_DIST }
    }

    /// Unattended captures (REACH_UI_SHOT=customize-<field>): select a row by its label, e.g. `primary`, `elite`.
    pub fn open_field(&mut self, name: &str) {
        if name == "elite" {
            self.s.armour.elite = true; // shown only, not saved
            self.field = Field::EliteArmour;
            return;
        }
        if let Some(f) = self.fields().into_iter().find(|f| f.label().to_lowercase().replace(' ', "-") == name) {
            self.field = f;
        }
    }

    fn rows(&self) -> &'static [(&'static str, &'static [Field])] {
        if self.s.armour.elite { ELITE_ROWS } else { SPARTAN_ROWS }
    }

    fn fields(&self) -> Vec<Field> {
        self.rows().iter().flat_map(|r| r.1.iter().copied()).collect()
    }

    fn equip(&mut self, f: Field, v: u32) {
        let old = self.s.clone();
        f.set(&mut self.s.armour, v);
        self.commit(old);
    }

    fn commit(&mut self, old: Settings) {
        if old != self.s {
            super::settings_apply::apply(&old, &self.s);
            settings::save(&self.s);
        }
    }

    fn set_elite(&mut self, elite: bool) {
        let old = self.s.clone();
        self.s.armour.elite = elite;
        if !self.fields().contains(&self.field) {
            self.field = self.fields()[0];
        }
        self.commit(old);
    }

    /// Steps the selected field's value by `d` (keyboard / pad left-right).
    fn step(&mut self, d: i32) {
        let c = self.field.choices(&self.s.armour);
        let cur = self.field.get(&self.s.armour);
        let i = if self.field == Field::Helmet {
            c.iter().position(|x| x.1 == helmet_family(cur))
        } else {
            c.iter().position(|x| x.0 == cur)
        }
        .unwrap_or(0) as i32;
        let n = c.len() as i32;
        let v = c[((i + d) % n + n) as usize % n as usize].0;
        self.equip(self.field, v);
    }

    /// Draws the screen; true when the player leaves it.
    pub fn ui(&mut self, ui: &mut Ui) -> bool {
        let full = ui.max_rect();
        let mut back = false;
        let typing = ui.ctx().memory(|m| m.focused().is_some());
        let (mut up, mut down, mut left, mut right, mut tab) = (false, false, false, false, false);
        if !typing {
            ui.input(|i| {
                up = i.key_pressed(egui::Key::ArrowUp);
                down = i.key_pressed(egui::Key::ArrowDown);
                left = i.key_pressed(egui::Key::ArrowLeft);
                right = i.key_pressed(egui::Key::ArrowRight);
                tab = i.key_pressed(egui::Key::Tab);
                back = i.key_pressed(egui::Key::Escape);
            });
        }
        let fields = self.fields();
        let at = fields.iter().position(|f| *f == self.field).unwrap_or(0);
        if down {
            self.field = fields[(at + 1) % fields.len()];
        }
        if up {
            self.field = fields[(at + fields.len() - 1) % fields.len()];
        }
        if left || right {
            self.step(if right { 1 } else { -1 });
        }
        if tab {
            self.set_elite(!self.s.armour.elite);
        }

        // Title, Spartan / Elite tabs, BACK.
        let p = ui.painter();
        p.text(full.min + Vec2::new(40.0, 30.0), Align2::LEFT_TOP, "CUSTOMIZE", FontId::proportional(40.0), TEXT);
        let tabs = Rect::from_center_size(egui::pos2(full.center().x - 20.0, full.top() + 52.0), Vec2::new(300.0, 38.0));
        for (i, (label, elite)) in [("SPARTAN", false), ("ELITE", true)].into_iter().enumerate() {
            let r = Rect::from_min_size(tabs.min + Vec2::new(i as f32 * 150.0, 0.0), Vec2::new(150.0, 38.0));
            let resp = ui.interact(r, ui.id().with(("cust-tab", i)), Sense::click());
            let on = self.s.armour.elite == elite;
            let p = ui.painter();
            p.rect_filled(r, 0.0, if on { ACCENT.gamma_multiply(0.25) } else if resp.hovered() { HOVER } else { PANEL });
            p.rect_stroke(r, 0.0, Stroke::new(1.0, if on { ACCENT } else { LINE }), StrokeKind::Inside);
            if on {
                p.rect_filled(Rect::from_min_size(r.left_bottom() - Vec2::new(0.0, 3.0), Vec2::new(r.width(), 3.0)), 0.0, ACCENT);
            }
            p.text(r.center(), Align2::CENTER_CENTER, label, FontId::proportional(17.0), if on { Color32::WHITE } else { DIM });
            if resp.clicked() {
                self.set_elite(elite);
            }
        }
        let mut b = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_size(egui::pos2(full.right() - 116.0, full.top() + 34.0), Vec2::new(80.0, 36.0))));
        if theme::ghost_button(&mut b, "Back").clicked() {
            back = true;
        }

        let top = full.top() + 100.0;
        let bottom = full.bottom() - 30.0;
        let left_col = Rect::from_min_max(egui::pos2(full.left() + 40.0, top), egui::pos2(full.left() + 360.0, bottom));
        let right_col = Rect::from_min_max(egui::pos2(full.right() - 360.0, top), egui::pos2(full.right() - 36.0, bottom));
        let centre = Rect::from_min_max(egui::pos2(left_col.right() + 24.0, top), egui::pos2(right_col.left() - 24.0, bottom));

        self.left_list(ui, left_col);
        let shown = self.preview_armour();
        self.preview(ui, centre, &shown);
        self.picker(ui, right_col);
        back
    }

    fn left_list(&mut self, ui: &mut Ui, rect: Rect) {
        ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
            // Player card.
            let (card, _) = ui.allocate_exact_size(Vec2::new(rect.width(), 62.0), Sense::hover());
            let p = ui.painter();
            p.rect_filled(card, 2.0, ACCENT);
            p.rect_filled(Rect::from_center_size(card.left_center() + Vec2::new(30.0, 0.0), Vec2::splat(34.0)), 3.0, colour(self.s.armour.primary));
            p.rect_stroke(Rect::from_center_size(card.left_center() + Vec2::new(30.0, 0.0), Vec2::splat(34.0)), 3.0, Stroke::new(3.0, colour(self.s.armour.secondary)), StrokeKind::Inside);
            p.text(card.left_top() + Vec2::new(58.0, 10.0), Align2::LEFT_TOP, &self.s.player_name, FontId::proportional(21.0), Color32::from_rgb(20, 14, 8));
            p.text(card.left_top() + Vec2::new(58.0, 37.0), Align2::LEFT_TOP, if self.s.armour.elite { "ELITE" } else { "SPARTAN" }, FontId::proportional(12.0), Color32::from_rgb(70, 40, 12));
            ui.add_space(8.0);
            egui::ScrollArea::vertical().id_salt("cust-left").auto_shrink([false, false]).show(ui, |ui| {
                for (section, fields) in self.rows() {
                    ui.add_space(6.0);
                    theme::caption(ui, section);
                    for f in fields.iter() {
                        self.field_row(ui, *f);
                    }
                }
                ui.add_space(6.0);
                theme::caption(ui, "Emblem");
                for label in ["Foreground", "Background", "Colors"] {
                    let (r, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 28.0), Sense::hover());
                    ui.painter().text(r.left_center() + Vec2::new(12.0, 0.0), Align2::LEFT_CENTER, label.to_uppercase(), FontId::proportional(14.0), DIM.gamma_multiply(0.7));
                    ui.painter().text(r.right_center() - Vec2::new(8.0, 0.0), Align2::RIGHT_CENTER, "not yet", FontId::proportional(13.0), DIM.gamma_multiply(0.7));
                }
            });
        });
    }

    fn field_row(&mut self, ui: &mut Ui, f: Field) {
        let (r, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 30.0), Sense::click());
        let sel = self.field == f;
        let p = ui.painter();
        if sel {
            p.rect_filled(r, 0.0, ACCENT.gamma_multiply(0.22));
            p.rect_filled(Rect::from_min_size(r.min, Vec2::new(3.0, r.height())), 0.0, ACCENT);
        } else if resp.hovered() {
            p.rect_filled(r, 0.0, HOVER);
        }
        p.text(r.left_center() + Vec2::new(12.0, 0.0), Align2::LEFT_CENTER, f.label().to_uppercase(), FontId::proportional(14.0), if sel { Color32::WHITE } else { TEXT });
        let mut x = r.right() - 8.0;
        let val = f.value_name(&self.s.armour);
        let g = p.layout_no_wrap(val, FontId::proportional(13.5), if sel { Color32::WHITE } else { DIM });
        x -= g.size().x;
        p.galley(egui::pos2(x, r.center().y - g.size().y / 2.0), g, DIM);
        if f.is_colour() || f == Field::Visor {
            let c = if f == Field::Visor { VISORS[(self.s.armour.visor.wrapping_sub(0x9D) as usize).min(4)].1 } else { colour(f.get(&self.s.armour)) };
            let chip = Rect::from_center_size(egui::pos2(x - 14.0, r.center().y), Vec2::splat(13.0));
            p.rect_filled(chip, 1.0, c);
            p.rect_stroke(chip, 1.0, Stroke::new(1.0, Color32::from_gray(90)), StrokeKind::Inside);
        }
        if resp.clicked() {
            self.field = f;
            self.trying = None;
        }
    }

    /// The armour as previewed: the saved one with the hovered choice tried on.
    fn preview_armour(&self) -> Armour {
        let mut a = self.s.armour.clone();
        if let Some((f, v)) = self.trying {
            f.set(&mut a, v);
        }
        a
    }

    /// Centre pane: the real model rendered by the preview engine (drag = rotate, wheel = zoom, RESET VIEW), with
    /// the flat figure while it loads or if it cannot start. No stance picker: `unit_lower_weapon` crashes the engine
    /// on a multiplayer biped, and a pistol stance would need a pistol loadout.
    fn preview(&mut self, ui: &mut Ui, rect: Rect, a: &Armour) {
        crate::preview::set_armour(a);
        let frame = crate::preview::frame(ui.ctx());
        let crate::preview::Frame::Live(tex, size) = frame else {
            self.flat_preview(ui, rect, a);
            let msg = match frame {
                crate::preview::Frame::Loading(t) => format!("LOADING 3D PREVIEW {}", ".".repeat(1 + (t * 2.0) as usize % 3)),
                crate::preview::Frame::Failed(e) => format!("3D preview unavailable: {e}"),
                crate::preview::Frame::Live(..) => unreachable!(),
            };
            ui.painter().text(egui::pos2(rect.center().x, rect.top() + 6.0), Align2::CENTER_TOP, msg, FontId::proportional(13.0), ACCENT);
            ui.ctx().request_repaint();
            return;
        };
        // Cover the pane with the frame (crop the wider side), keeping its aspect.
        let (fa, ra) = (size.x / size.y, rect.width() / rect.height());
        let uv = if fa > ra {
            let w = ra / fa;
            Rect::from_min_max(egui::pos2(0.5 - w / 2.0, 0.0), egui::pos2(0.5 + w / 2.0, 1.0))
        } else {
            let h = fa / ra;
            Rect::from_min_max(egui::pos2(0.0, 0.5 - h / 2.0), egui::pos2(1.0, 0.5 + h / 2.0))
        };
        let resp = ui.interact(rect, ui.id().with("cust-3d"), Sense::click_and_drag());
        if resp.dragged() {
            let d = resp.drag_delta();
            self.yaw -= d.x * 0.012;
            self.pitch = (self.pitch + d.y * 0.006).clamp(-0.5, 0.9);
        }
        if resp.hovered() {
            let wheel = ui.input(|i| i.smooth_scroll_delta.y);
            if wheel != 0.0 {
                self.dist = (self.dist * (-wheel * 0.0025).exp()).clamp(1.6, 7.5);
            }
        }
        // Elites stand taller: step back a little so the whole model stays in frame.
        crate::preview::set_camera(self.yaw, self.pitch, if a.elite { self.dist * 1.25 } else { self.dist });
        let p = ui.painter();
        p.image(tex, rect, uv, Color32::WHITE);
        if crate::preview::switching() {
            // Spartan <-> Elite: the engine respawns the biped (~5 s); dim the last frame meanwhile.
            p.rect_filled(rect, 0.0, Color32::from_black_alpha(150));
            let t = ui.input(|i| i.time) as f32;
            let msg = format!("SWITCHING TO {} {}", if a.elite { "ELITE" } else { "SPARTAN" }, ".".repeat(1 + (t * 2.0) as usize % 3));
            p.text(rect.center(), Align2::CENTER_CENTER, msg, FontId::proportional(16.0), ACCENT);
        }
        p.rect_stroke(rect, 0.0, Stroke::new(1.0, LINE), StrokeKind::Inside);
        // Name of what is shown, over a fade at the bottom.
        let fade = Rect::from_min_max(egui::pos2(rect.left(), rect.bottom() - 74.0), rect.max);
        p.rect_filled(fade, 0.0, Color32::from_black_alpha(120));
        let name = if a.elite {
            format!("{} Elite", Field::EliteArmour.value_name(a))
        } else {
            let att = Field::Attachment.value_name(a);
            if att == "None" { Field::Helmet.value_name(a) } else { format!("{} {att}", Field::Helmet.value_name(a)) }
        };
        p.text(egui::pos2(rect.center().x, fade.top() + 10.0), Align2::CENTER_TOP, name, FontId::proportional(19.0), TEXT);
        p.text(
            egui::pos2(rect.center().x, fade.top() + 34.0),
            Align2::CENTER_TOP,
            format!("{} / {}", Field::Primary.value_name(a), Field::Secondary.value_name(a)),
            FontId::proportional(14.0),
            DIM,
        );
        p.text(egui::pos2(rect.left() + 10.0, rect.bottom() - 8.0), Align2::LEFT_BOTTOM, "DRAG: ROTATE  ·  WHEEL: ZOOM", FontId::proportional(11.0), DIM);
        // RESET VIEW, top right of the pane.
        let mut b = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_size(rect.right_top() + Vec2::new(-128.0, 10.0), Vec2::new(118.0, 30.0))));
        if theme::ghost_button(&mut b, "Reset view").clicked() {
            (self.yaw, self.pitch, self.dist) = (VIEW_YAW, VIEW_PITCH, VIEW_DIST);
        }
        ui.ctx().request_repaint();
    }

    /// Flat preview: a drawn figure in the chosen colours and the equipped piece names (fallback for the 3D model).
    fn flat_preview(&self, ui: &mut Ui, rect: Rect, a: &Armour) {
        let p = ui.painter();
        let (pri, sec) = (colour(a.primary), colour(a.secondary));
        let visor = VISORS[(a.visor.wrapping_sub(0x9D) as usize).min(4)].1;
        let dark = Color32::from_rgb(34, 36, 40);
        let s = (rect.height() / 620.0).clamp(0.6, 1.3);
        let c = egui::pos2(rect.center().x, rect.top() + 40.0 * s);
        let r = |x: f32, y: f32, w: f32, h: f32| Rect::from_min_size(c + Vec2::new(x * s, y * s), Vec2::new(w * s, h * s));
        // Floor ring.
        p.add(egui::Shape::ellipse_stroke(c + Vec2::new(0.0, 470.0 * s), Vec2::new(150.0 * s, 26.0 * s), Stroke::new(2.0, ACCENT.gamma_multiply(0.5))));
        if a.elite {
            p.rect_filled(r(-38.0, 0.0, 76.0, 70.0), 18.0 * s, pri);
            p.rect_filled(r(-8.0, 46.0, 50.0, 22.0), 6.0 * s, dark);
            p.rect_filled(r(-80.0, 80.0, 160.0, 150.0), 20.0 * s, pri);
            p.rect_filled(r(-50.0, 110.0, 100.0, 60.0), 10.0 * s, sec);
            for side in [-1.0f32, 1.0] {
                p.rect_filled(r(if side < 0.0 { -128.0 } else { 88.0 }, 90.0, 40.0, 170.0), 14.0 * s, dark);
                p.rect_filled(r(if side < 0.0 { -132.0 } else { 84.0 }, 76.0, 48.0, 48.0), 16.0 * s, sec);
                p.rect_filled(r(if side < 0.0 { -66.0 } else { 14.0 }, 238.0, 52.0, 120.0), 14.0 * s, pri);
                p.rect_filled(r(if side < 0.0 { -76.0 } else { 24.0 }, 352.0, 52.0, 100.0), 12.0 * s, dark);
            }
        } else {
            // Helmet with visor.
            p.rect_filled(r(-36.0, 0.0, 72.0, 78.0), 26.0 * s, pri);
            p.rect_filled(r(-26.0, 30.0, 52.0, 24.0), 9.0 * s, visor);
            // Torso.
            p.rect_filled(r(-74.0, 92.0, 148.0, 150.0), 14.0 * s, pri);
            p.rect_filled(r(-46.0, 108.0, 92.0, 70.0), 8.0 * s, sec);
            p.rect_filled(r(-50.0, 196.0, 100.0, 46.0), 6.0 * s, dark);
            for side in [-1.0f32, 1.0] {
                // Left-side x mirrored for the right side.
                let m = |x: f32, w: f32| if side < 0.0 { x } else { -x - w };
                // Shoulders (secondary), arms, thighs, knee guards, shins.
                p.rect_filled(r(m(-124.0, 56.0), 86.0, 56.0, 50.0), 14.0 * s, sec);
                p.rect_filled(r(m(-114.0, 36.0), 136.0, 36.0, 130.0), 10.0 * s, dark);
                p.rect_filled(r(m(-62.0, 52.0), 250.0, 52.0, 110.0), 10.0 * s, pri);
                p.rect_filled(r(m(-60.0, 48.0), 348.0, 48.0, 30.0), 8.0 * s, sec);
                p.rect_filled(r(m(-60.0, 48.0), 378.0, 48.0, 84.0), 8.0 * s, dark);
            }
        }
        // Equipped names under the figure.
        let mut y = c.y + 500.0 * s;
        let lines: Vec<String> = if a.elite {
            vec![
                format!("{} Elite", Field::EliteArmour.value_name(a)),
                format!("{} / {}", Field::Primary.value_name(a), Field::Secondary.value_name(a)),
            ]
        } else {
            let mut h = Field::Helmet.value_name(a);
            let att = Field::Attachment.value_name(a);
            if att != "None" {
                h = format!("{h} {att}");
            }
            vec![
                h,
                format!("{} / {}", Field::Primary.value_name(a), Field::Secondary.value_name(a)),
            ]
        };
        for (i, l) in lines.iter().enumerate() {
            p.text(egui::pos2(c.x, y), Align2::CENTER_TOP, l, FontId::proportional(if i == 0 { 20.0 } else { 15.0 }), if i == 0 { TEXT } else { DIM });
            y += if i == 0 { 26.0 } else { 20.0 };
        }
        p.text(egui::pos2(rect.left(), rect.bottom()), Align2::LEFT_BOTTOM, "FLAT PREVIEW  ·  ARROWS: ROW / VALUE  ·  TAB: SPARTAN / ELITE", FontId::proportional(11.5), DIM);
    }

    fn picker(&mut self, ui: &mut Ui, rect: Rect) {
        ui.painter().rect_filled(rect, 2.0, PANEL);
        ui.painter().rect_stroke(rect, 2.0, Stroke::new(1.0, LINE), StrokeKind::Inside);
        let f = self.field;
        let cur = f.get(&self.s.armour);
        let choices = f.choices(&self.s.armour);
        let mut hovered = None;
        ui.scope_builder(egui::UiBuilder::new().max_rect(rect.shrink(16.0)), |ui| {
            ui.label(egui::RichText::new(format!("{} {}", f.label().to_uppercase(), if f.is_colour() { "COLOR" } else { "" })).size(22.0).color(TEXT));
            let note = match self.trying.filter(|t| t.0 == f) {
                Some((_, v)) => {
                    let name = choices.iter().find(|c| c.0 == v).map(|c| c.1.clone()).unwrap_or_default();
                    format!("Trying on {name} - click to equip")
                }
                None => format!("Equipped: {}", f.value_name(&self.s.armour)),
            };
            ui.label(egui::RichText::new(note).size(13.0).color(ACCENT));
            ui.add_space(8.0);
            let foot = if f.is_colour() { 40.0 } else { 8.0 };
            let h = ui.available_height() - foot;
            egui::ScrollArea::vertical().id_salt(("cust-pick", f as u8)).max_height(h).auto_shrink([false, false]).show(ui, |ui| {
                if f.is_colour() {
                    let w = (ui.available_width() - 12.0) / 3.0;
                    for row in choices.chunks(3) {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            for (id, name) in row {
                                let (r, resp) = ui.allocate_exact_size(Vec2::new(w, 32.0), Sense::click());
                                let c = colour(*id);
                                let p = ui.painter();
                                p.rect_filled(r, 2.0, c);
                                let on = *id == cur;
                                if on || resp.hovered() {
                                    p.rect_stroke(r, 2.0, Stroke::new(2.0, if on { Color32::WHITE } else { ACCENT }), StrokeKind::Inside);
                                }
                                let light = (c.r() as u32 * 3 + c.g() as u32 * 6 + c.b() as u32) / 10 > 150;
                                let tc = if light { Color32::from_rgb(20, 20, 20) } else { Color32::WHITE };
                                p.text(r.left_center() + Vec2::new(9.0, 0.0), Align2::LEFT_CENTER, name, FontId::proportional(13.5), tc);
                                if on {
                                    p.text(r.right_center() - Vec2::new(9.0, 0.0), Align2::RIGHT_CENTER, "✔", FontId::proportional(13.0), tc);
                                }
                                if resp.hovered() {
                                    hovered = Some(*id);
                                }
                                if resp.clicked() {
                                    self.equip(f, *id);
                                }
                            }
                        });
                        ui.add_space(4.0);
                    }
                } else {
                    for (id, name) in &choices {
                        let on = if f == Field::Helmet { helmet_family(cur) == name.as_str() } else { *id == cur };
                        let resp = theme::list_row(ui, &[(0.0, name.clone()), (ui.available_width() - 40.0, if on { "✔".into() } else { String::new() })], on, 30.0);
                        if resp.hovered() {
                            hovered = Some(*id);
                        }
                        if resp.clicked() {
                            self.equip(f, *id);
                        }
                    }
                }
            });
            if f.is_colour() {
                ui.add_space(8.0);
                ui.label(egui::RichText::new("Team games may override your primary color.").size(12.5).color(DIM));
            }
        });
        self.trying = hovered.map(|v| (f, v));
    }
}

impl Default for CustomizeScreen {
    fn default() -> Self {
        Self::new()
    }
}
