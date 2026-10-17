//! The SETTINGS screen, shared by the main menu and the pause overlay: categories on the left (each with a summary
//! of its values), rows of toggles / pickers / sliders / key slots in the middle, and a help panel on the right
//! that explains the setting under the pointer or the keyboard focus. Changes are saved to settings.json and
//! applied as soon as the pointer is released (settings_apply.rs). Field-to-engine mapping: notes/frontend.md.

use std::sync::Mutex;

use egui::{Align2, Color32, FontId, Rect, Sense, Stroke, StrokeKind, Ui, Vec2};

use super::settings::{self, Settings};
use super::theme::{ACCENT, DIM, HOVER, LINE, PANEL, TEXT};
use crate::input;

// ---- Key capture (filled by ui::window_message while a key slot is listening) ----------------------------------

#[derive(Clone, Copy, PartialEq)]
enum Capture {
    Idle,
    Listening,
    Got(u32),
}

static CAPTURE: Mutex<Capture> = Mutex::new(Capture::Idle);
static GPU: Mutex<String> = Mutex::new(String::new());

fn capture() -> std::sync::MutexGuard<'static, Capture> {
    CAPTURE.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn capturing() -> bool {
    *capture() == Capture::Listening
}

pub fn captured(code: u32) {
    let mut c = capture();
    if *c == Capture::Listening {
        *c = Capture::Got(code);
    }
}

/// Remember the graphics card the device runs on (DXGI adapter description), shown on the Graphics page.
pub fn note_gpu(device: &windows::Win32::Graphics::Direct3D11::ID3D11Device) {
    use windows::core::Interface;
    let desc = (|| unsafe {
        let dxgi: windows::Win32::Graphics::Dxgi::IDXGIDevice = device.cast().ok()?;
        dxgi.GetAdapter().ok()?.GetDesc().ok()
    })();
    if let Some(d) = desc {
        let n = d.Description.iter().position(|&c| c == 0).unwrap_or(d.Description.len());
        let name = String::from_utf16_lossy(&d.Description[..n]);
        let mb = d.DedicatedVideoMemory / (1024 * 1024);
        *GPU.lock().unwrap_or_else(|e| e.into_inner()) =
            if mb >= 1024 { format!("{name} ({:.0} GB of video memory)", mb as f64 / 1024.0) } else { format!("{name} ({mb} MB of video memory)") };
    }
}

// ---- Tables ----------------------------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cat {
    Graphics,
    Audio,
    Game,
    Player,
    Keyboard,
    Controller,
}

const CATS: &[(&str, &[(Cat, &str)])] = &[
    ("Display & sound", &[(Cat::Graphics, "Graphics"), (Cat::Audio, "Audio")]),
    ("Game", &[(Cat::Game, "Game options"), (Cat::Player, "Player & network")]),
    ("Controls", &[(Cat::Keyboard, "Keyboard & mouse"), (Cat::Controller, "Controller")]),
];

const ALL_CATS: [Cat; 6] = [Cat::Graphics, Cat::Audio, Cat::Game, Cat::Player, Cat::Keyboard, Cat::Controller];

const WINDOW_SIZES: &[(u32, u32)] = &[(1280, 720), (1366, 768), (1600, 900), (1920, 1080), (2560, 1440), (3840, 2160)];
const RENDER_SCALES: &[u32] = &[50, 67, 75, 85, 100, 125, 150, 200];
const UI_SCALES: &[u32] = &[60, 70, 80, 90, 100, 110, 125];
const FPS_CAPS: &[&str] = &["60 fps", "120 fps", "180 fps", "240 fps", "No limit"];
/// Profile +0x1C8 values the engine accepts for Reach (others fall back to the default layout), named after the
/// engine's internal preset ids.
const BUTTON_PRESETS: &[(u8, &str)] = &[
    (0, "Default"),
    (2, "Boxer"),
    (3, "Green Thumb"),
    (4, "Professional"),
    (7, "Recon"),
    (8, "Reclaimer"),
    (9, "Zoom and Shoot"),
    (10, "Bumper Jumper"),
    (11, "Green Fingers"),
];
const STICK_PRESETS: &[&str] = &["Default", "Southpaw", "Legacy", "Legacy southpaw"];

/// Rebindable actions: (group, MCC action index, name, what it does).
const ACTIONS: &[(&str, u32, &str, &str)] = &[
    ("Movement", 16, "Move forward", "Walk or drive forward."),
    ("Movement", 17, "Move backward", "Walk or drive backward."),
    ("Movement", 18, "Strafe left", "Step sideways to the left."),
    ("Movement", 19, "Strafe right", "Step sideways to the right."),
    ("Movement", 0, "Jump", "Jump. In a vehicle with a jetpack-like boost it may also lift."),
    ("Movement", 9, "Crouch", "Crouch while held: quieter, steadier and hidden from motion trackers."),
    ("Movement", 23, "Armor ability", "Use your armor ability: sprint, jetpack, armor lock, active camo and the rest."),
    ("Combat", 8, "Fire", "Fire the weapon in your hands."),
    ("Combat", 10, "Zoom", "Look down the scope, or zoom in on weapons that have one."),
    ("Combat", 3, "Reload", "Reload the weapon in your hands."),
    ("Combat", 2, "Action", "Pick up a weapon, enter a vehicle, press a switch or carry an objective."),
    ("Combat", 5, "Melee", "Hit whoever is in front of you. From behind, it is an assassination."),
    ("Combat", 7, "Throw grenade", "Throw the selected grenade."),
    ("Combat", 1, "Switch grenade", "Cycle between frag and plasma grenades."),
    ("Combat", 4, "Switch weapon", "Swap between your two weapons."),
    ("Combat", 6, "Flashlight", "Turn your helmet light on or off (campaign and Firefight)."),
    ("Vehicles", 21, "Vehicle trick 1", "First vehicle trick, such as a Banshee flip."),
    ("Vehicles", 22, "Vehicle trick 2", "Second vehicle trick, such as a Banshee barrel roll."),
    ("Interface", 20, "Scoreboard", "Show the scores while held."),
    ("Forge", 25, "Raise object", "Forge: move the monitor or the held object up."),
    ("Forge", 26, "Lower object", "Forge: move the monitor or the held object down."),
    ("Forge", 27, "Grab / drop object", "Forge: pick up or drop the object you are looking at."),
    ("Forge", 28, "Move faster", "Forge: move the monitor faster while held."),
    ("Forge", 29, "Precision", "Forge: slow, precise movement and rotation while held."),
    ("Forge", 30, "Delete object", "Forge: delete the object you are looking at or holding."),
    ("Forge", 31, "Create objects", "Forge: open the menu of objects you can place."),
    ("Forge", 32, "Object properties", "Forge: open the tools and properties of the object you are looking at."),
    ("Forge", 33, "Monitor mode", "Forge: switch between the monitor and playing as a Spartan."),
    ("Forge", 34, "Rotate held object", "Forge: hold to rotate the object you are carrying with the mouse."),
    ("Forge", 35, "Lock object", "Forge: hold an object still while you move around it."),
    ("Theater", 36, "Toggle panel", "Theater: show or hide the playback panel."),
    ("Theater", 37, "Toggle interface", "Theater: show or hide the HUD and interface."),
    ("Theater", 38, "Change view", "Theater: switch between first and third person."),
    ("Theater", 40, "Fast forward", "Theater: skip forward."),
    ("Theater", 41, "Rewind", "Theater: skip back."),
    ("Theater", 42, "Pause / play", "Theater: stop or continue playback."),
    ("Theater", 44, "Free camera", "Theater: detach the camera and fly around."),
    ("Theater", 47, "Camera up", "Theater: free camera up."),
    ("Theater", 48, "Camera down", "Theater: free camera down."),
];

fn action_name(a: u32) -> &'static str {
    ACTIONS.iter().find(|x| x.1 == a).map_or("another action", |x| x.2)
}

/// Bindings only clash inside one context: playing, Forge, or Theater.
fn context(group: &str) -> u8 {
    match group {
        "Forge" => 1,
        "Theater" => 2,
        _ => 0,
    }
}

// ---- Screen ----------------------------------------------------------------------------------------------------

#[derive(Default)]
struct Keys {
    up: bool,
    down: bool,
    left: bool,
    right: bool,
    enter: bool,
    tab: bool,
    shift: bool,
}

pub struct SettingsScreen {
    pub s: Settings,
    applied: Settings,
    cat: Cat,
    focus: usize,
    rows: usize,
    row_i: usize,
    help: Option<(String, String)>,
    focus_help: Option<(String, String)>,
    listen: Option<(u32, usize)>,
    status: String,
    keys: Keys,
    scroll_focus: bool,
    in_game: bool,
}

impl SettingsScreen {
    pub fn new(in_game: bool) -> Self {
        let s = settings::get();
        SettingsScreen {
            applied: s.clone(),
            s,
            cat: Cat::Graphics,
            focus: 0,
            rows: 0,
            row_i: 0,
            help: None,
            focus_help: None,
            listen: None,
            status: String::new(),
            keys: Keys::default(),
            scroll_focus: false,
            in_game,
        }
    }

    pub fn open_category(&mut self, name: &str) {
        self.cat = match name {
            "audio" => Cat::Audio,
            "game" => Cat::Game,
            "player" => Cat::Player,
            "keyboard" => Cat::Keyboard,
            "controller" => Cat::Controller,
            _ => Cat::Graphics,
        };
        self.focus = 0;
    }

    fn set_cat(&mut self, c: Cat) {
        if self.cat != c {
            self.cat = c;
            self.focus = 0;
            self.stop_listening();
        }
    }

    fn stop_listening(&mut self) {
        self.listen = None;
        *capture() = Capture::Idle;
    }

    /// Save and apply whatever changed (not while a slider is being dragged).
    fn commit(&mut self, ui: &Ui) {
        if self.s == self.applied || ui.input(|i| i.pointer.any_down()) {
            return;
        }
        super::settings_apply::apply(&self.applied, &self.s);
        settings::save(&self.s);
        self.applied = self.s.clone();
    }

    /// Draw the whole screen into `ui.max_rect()`. Returns true when the player backs out (BACK or Esc).
    pub fn ui(&mut self, ui: &mut Ui) -> bool {
        let full = ui.max_rect();
        let mut back = false;

        // A captured key for the slot that is listening.
        let got = match *capture() {
            Capture::Got(c) => Some(c),
            _ => None,
        };
        if let (Some(code), Some((action, slot))) = (got, self.listen) {
            self.stop_listening();
            if code == 0x1B {
                self.status = "Cancelled.".into();
            } else if !input::bindable(code) {
                self.status = format!("{} cannot be bound: the game does not read that key.", input::key_name(code));
            } else {
                let mut k = self.s.mouse.keys(action);
                k[slot] = code;
                if k[0] == k[1] {
                    k[1 - slot] = 0;
                }
                self.s.mouse.set_keys(action, k);
                self.status = format!("{} is now on {}", action_name(action), input::key_name(code));
            }
        }

        let typing = ui.ctx().memory(|m| m.focused().is_some());
        self.keys = Keys::default();
        if self.listen.is_none() {
            ui.input(|i| {
                if !typing {
                    self.keys = Keys {
                        up: i.key_pressed(egui::Key::ArrowUp),
                        down: i.key_pressed(egui::Key::ArrowDown),
                        left: i.key_pressed(egui::Key::ArrowLeft),
                        right: i.key_pressed(egui::Key::ArrowRight),
                        enter: i.key_pressed(egui::Key::Enter),
                        tab: i.key_pressed(egui::Key::Tab),
                        shift: i.modifiers.shift,
                    };
                }
                back = !typing && i.key_pressed(egui::Key::Escape);
            });
        }
        if self.keys.tab {
            let i = ALL_CATS.iter().position(|c| *c == self.cat).unwrap_or(0);
            let n = ALL_CATS.len();
            self.set_cat(ALL_CATS[if self.keys.shift { (i + n - 1) % n } else { (i + 1) % n }]);
        }
        self.scroll_focus = false;
        if self.keys.down && self.focus + 1 < self.rows {
            self.focus += 1;
            self.scroll_focus = true;
        }
        if self.keys.up && self.focus > 0 {
            self.focus -= 1;
            self.scroll_focus = true;
        }

        // Title and BACK.
        let title = Rect::from_min_size(full.min + Vec2::new(40.0, 28.0), Vec2::new(full.width() - 80.0, 50.0));
        ui.painter().text(title.left_center(), Align2::LEFT_CENTER, "SETTINGS", FontId::proportional(40.0), Color32::WHITE);
        let back_r = Rect::from_min_size(egui::pos2(title.right() - 90.0, title.top() + 8.0), Vec2::new(90.0, 34.0));
        let br = ui.interact(back_r, ui.id().with("settings-back"), Sense::click());
        ui.painter().text(back_r.center(), Align2::CENTER_CENTER, "BACK", FontId::proportional(20.0), if br.hovered() { ACCENT } else { TEXT });
        if br.clicked() {
            back = true;
        }

        let top = title.bottom() + 20.0;
        let bottom = full.bottom() - 28.0;
        let cats_r = Rect::from_min_max(egui::pos2(full.left() + 40.0, top), egui::pos2(full.left() + 300.0, bottom));
        let help_w = (full.width() * 0.24).clamp(240.0, 340.0);
        let help_r = Rect::from_min_max(egui::pos2(full.right() - 40.0 - help_w, top), egui::pos2(full.right() - 40.0, bottom));
        let rows_r = Rect::from_min_max(egui::pos2(cats_r.right() + 28.0, top), egui::pos2(help_r.left() - 28.0, bottom));

        self.categories(ui, cats_r);

        // Rows.
        self.help = None;
        self.focus_help = None;
        self.row_i = 0;
        ui.scope_builder(egui::UiBuilder::new().max_rect(rows_r), |ui| {
            let mut area = egui::ScrollArea::vertical().id_salt(("settings-rows", self.cat as u8)).auto_shrink([false, false]);
            // REACH_UI_SCROLL=<pixels>: scroll the rows for unattended captures (REACH_UI_SHOT=settings-keyboard).
            if let Some(y) = std::env::var("REACH_UI_SCROLL").ok().and_then(|v| v.parse::<f32>().ok()) {
                area = area.vertical_scroll_offset(y);
            }
            area.show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                match self.cat {
                    Cat::Graphics => self.graphics(ui),
                    Cat::Audio => self.audio(ui),
                    Cat::Game => self.game(ui),
                    Cat::Player => self.player(ui),
                    Cat::Keyboard => self.keyboard(ui),
                    Cat::Controller => self.controller(ui),
                }
                ui.add_space(24.0);
            });
        });
        self.rows = self.row_i;
        if self.focus >= self.rows {
            self.focus = self.rows.saturating_sub(1);
        }
        ui.painter().line_segment([egui::pos2(rows_r.right() + 12.0, rows_r.top()), egui::pos2(rows_r.right() + 12.0, rows_r.bottom())], Stroke::new(1.0, LINE));

        self.help_panel(ui, help_r);
        self.commit(ui);
        if back && self.listen.is_none() {
            self.s = self.applied.clone();
            self.commit(ui);
            return true;
        }
        false
    }

    // ---- Left column ---------------------------------------------------------------------------------------------

    fn summary(&self, c: Cat) -> String {
        let s = &self.s;
        match c {
            Cat::Graphics => {
                let mode = ["Windowed", "Borderless", "Fullscreen"][s.graphics.display_mode.min(2) as usize];
                let fps = if s.graphics.fps_cap >= 4 { "No limit".to_string() } else { format!("{} fps", 60 * (s.graphics.fps_cap as u32 + 1)) };
                format!("{mode} · {fps}{}", if s.graphics.vsync { " · VSync" } else { "" })
            }
            Cat::Audio => format!("Volume {:.0} %", s.audio.master * 100.0),
            Cat::Game => format!("{}° · {} crosshair", s.game.fov, if s.game.crosshair_lowered { "Lowered" } else { "Standard" }),
            Cat::Player => s.player_name.clone(),
            Cat::Keyboard => format!("Sensitivity {:.2}", s.mouse_sens),
            Cat::Controller => {
                let preset = BUTTON_PRESETS.iter().find(|p| p.0 == s.pad.button_preset).map_or("Default", |p| p.1);
                format!("{} · {preset}", if input::pad_connected() { "Connected" } else { "Not connected" })
            }
        }
    }

    fn categories(&mut self, ui: &mut Ui, r: Rect) {
        ui.scope_builder(egui::UiBuilder::new().max_rect(r), |ui| {
            for (group, items) in CATS {
                section_line(ui, group);
                for &(c, name) in items.iter() {
                    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 56.0), Sense::click());
                    let sel = self.cat == c;
                    if sel {
                        let mut mesh = egui::Mesh::default();
                        let a = ACCENT.gamma_multiply(0.45);
                        let b = ACCENT.gamma_multiply(0.08);
                        mesh.colored_vertex(rect.left_top(), a);
                        mesh.colored_vertex(rect.right_top(), b);
                        mesh.colored_vertex(rect.right_bottom(), b);
                        mesh.colored_vertex(rect.left_bottom(), a);
                        mesh.add_triangle(0, 1, 2);
                        mesh.add_triangle(0, 2, 3);
                        ui.painter().add(mesh);
                        ui.painter().rect_filled(Rect::from_min_size(rect.min, Vec2::new(4.0, rect.height())), 0.0, ACCENT);
                    } else if resp.hovered() {
                        ui.painter().rect_filled(rect, 0.0, HOVER);
                    }
                    let p = ui.painter();
                    p.text(rect.left_top() + Vec2::new(18.0, 8.0), Align2::LEFT_TOP, name.to_uppercase(), FontId::proportional(19.0), if sel || resp.hovered() { Color32::WHITE } else { TEXT });
                    p.text(rect.left_top() + Vec2::new(18.0, 32.0), Align2::LEFT_TOP, self.summary(c), FontId::proportional(13.5), DIM);
                    if resp.clicked() {
                        self.set_cat(c);
                    }
                }
                ui.add_space(8.0);
            }
            ui.add_space((ui.available_height() - 44.0).max(0.0));
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width() - 20.0, 38.0), Sense::click());
            ui.painter().rect_filled(rect, 2.0, if resp.hovered() { HOVER } else { Color32::from_black_alpha(60) });
            ui.painter().rect_stroke(rect, 2.0, Stroke::new(1.0, if resp.hovered() { ACCENT } else { LINE }), StrokeKind::Inside);
            ui.painter().text(rect.center(), Align2::CENTER_CENTER, "RESTORE DEFAULTS", FontId::proportional(15.0), if resp.hovered() { TEXT } else { DIM });
            if resp.hovered() {
                self.help = Some(("Restore defaults".into(), "Puts every setting on this page back to how it started. Other pages keep their values.".into()));
            }
            if resp.clicked() {
                self.restore_defaults();
            }
        });
    }

    fn restore_defaults(&mut self) {
        let d = Settings::default();
        match self.cat {
            Cat::Graphics => self.s.graphics = d.graphics,
            Cat::Audio => self.s.audio = d.audio,
            Cat::Game => self.s.game = d.game,
            Cat::Player => {
                self.s.player_name = d.player_name;
                self.s.master_url = d.master_url;
            }
            Cat::Keyboard => {
                self.s.mouse = d.mouse;
                self.s.mouse_sens = d.mouse_sens;
            }
            Cat::Controller => self.s.pad = d.pad,
        }
        self.status = "Defaults restored for this page.".into();
    }

    // ---- Right column ----------------------------------------------------------------------------------------------

    fn help_panel(&mut self, ui: &mut Ui, r: Rect) {
        ui.painter().rect_filled(r, 2.0, PANEL);
        ui.painter().rect_stroke(r, 2.0, Stroke::new(1.0, LINE), StrokeKind::Inside);
        let inner = r.shrink(22.0);
        let help = self.help.clone().or_else(|| self.focus_help.clone());
        ui.scope_builder(egui::UiBuilder::new().max_rect(inner), |ui| {
            let (title, body) = help.clone().unwrap_or_else(|| (self.page_title().into(), self.page_blurb().into()));
            ui.label(egui::RichText::new(title.to_uppercase()).size(24.0).color(Color32::WHITE));
            ui.add_space(4.0);
            ui.label(egui::RichText::new(body).size(15.0).color(TEXT));
            ui.add_space(10.0);
            let y = ui.cursor().top();
            ui.painter().line_segment([egui::pos2(inner.left(), y), egui::pos2(inner.right(), y)], Stroke::new(1.0, LINE));
            ui.add_space(14.0);
            let t = help.as_ref().map(|h| h.0.as_str()).unwrap_or("");
            if t == "Field of view" || t == "Vehicle field of view" {
                let deg = if t == "Field of view" { self.s.game.fov } else { self.s.game.vehicle_fov };
                fov_wedge(ui, deg as f32);
            } else if help.is_none() && self.cat == Cat::Keyboard {
                key_legend(ui);
            }
            if !self.status.is_empty() {
                ui.add_space(10.0);
                ui.label(egui::RichText::new(&self.status).size(14.0).color(ACCENT));
            }
        });
        let foot = Rect::from_min_max(egui::pos2(inner.left(), inner.bottom() - 34.0), inner.max);
        ui.scope_builder(egui::UiBuilder::new().max_rect(foot), |ui| {
            ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                ui.label(egui::RichText::new("Hover a setting, or reach it with the arrow keys, for details.").size(12.5).color(DIM));
            });
        });
    }

    fn page_title(&self) -> &'static str {
        match self.cat {
            Cat::Graphics => "Graphics",
            Cat::Audio => "Audio",
            Cat::Game => "Game options",
            Cat::Player => "Player & network",
            Cat::Keyboard => "Keyboard & mouse",
            Cat::Controller => "Controller",
        }
    }

    fn page_blurb(&self) -> &'static str {
        match self.cat {
            Cat::Graphics => "How the game window behaves, the resolution the game renders at, and how fast it may draw frames.",
            Cat::Audio => "How loud the game is overall, and its music and sound effects on their own.",
            Cat::Game => "Field of view, crosshair and voice. They apply to you only; the host still decides the game's rules.",
            Cat::Player => "The name other players see, and the master server that lists games in the server browser.",
            Cat::Keyboard => "Mouse feel, and up to two keys or mouse buttons for every action. Click a slot, then press the key you want.",
            Cat::Controller => "Stick sensitivity and feel, button and stick layouts, and rumble for an Xbox-style controller.",
        }
    }

    // ---- Row building blocks ---------------------------------------------------------------------------------------

    /// One setting row: allocates it, paints focus/hover and the label, records help. Returns the control rect,
    /// whether the row has keyboard focus, and the row response.
    fn row(&mut self, ui: &mut Ui, label: &str, help: &str, enabled: bool) -> (Rect, bool, egui::Response) {
        let idx = self.row_i;
        self.row_i += 1;
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 40.0), Sense::click());
        if resp.clicked() || resp.secondary_clicked() {
            self.focus = idx;
        }
        let focused = idx == self.focus;
        let h = (label.to_string(), help.to_string());
        if resp.hovered() {
            self.help = Some(h.clone());
        }
        if focused {
            self.focus_help = Some(h);
            if self.scroll_focus {
                resp.scroll_to_me(None);
            }
            ui.painter().rect_filled(rect, 0.0, ACCENT.gamma_multiply(0.10));
            ui.painter().rect_filled(Rect::from_min_size(rect.min, Vec2::new(3.0, rect.height())), 0.0, ACCENT);
        } else if resp.hovered() {
            ui.painter().rect_filled(rect, 0.0, HOVER);
        }
        ui.painter().text(
            rect.left_center() + Vec2::new(18.0, 0.0),
            Align2::LEFT_CENTER,
            label.to_uppercase(),
            FontId::proportional(16.5),
            if enabled { TEXT } else { DIM.gamma_multiply(0.7) },
        );
        let w = (rect.width() * 0.46).clamp(220.0, 300.0);
        let ctrl = Rect::from_min_max(egui::pos2(rect.right() - w - 8.0, rect.top() + 4.0), egui::pos2(rect.right() - 8.0, rect.bottom() - 4.0));
        (ctrl, focused && enabled, resp)
    }

    fn section(&mut self, ui: &mut Ui, title: &str) {
        ui.add_space(8.0);
        section_line(ui, title);
    }

    fn note(&mut self, ui: &mut Ui, text: &str) {
        egui::Frame::new().inner_margin(egui::Margin { left: 18, right: 8, top: 0, bottom: 4 }).show(ui, |ui| {
            ui.label(egui::RichText::new(text).size(14.0).color(DIM));
        });
    }

    fn toggle(&mut self, ui: &mut Ui, label: &str, help: &str, v: &mut bool) -> bool {
        let (ctrl, focused, resp) = self.row(ui, label, help, true);
        let pill = Rect::from_min_size(egui::pos2(ctrl.right() - 56.0, ctrl.center().y - 14.0), Vec2::new(56.0, 28.0));
        let hit = ui.interact(pill, resp.id.with("pill"), Sense::click());
        let flip = resp.clicked() || hit.clicked() || (focused && (self.keys.enter || self.keys.left || self.keys.right));
        if flip {
            *v = !*v;
        }
        let p = ui.painter();
        p.rect_filled(pill, 14.0, if *v { ACCENT } else { Color32::from_black_alpha(140) });
        p.rect_stroke(pill, 14.0, Stroke::new(1.0, if *v { ACCENT } else { DIM.gamma_multiply(0.5) }), StrokeKind::Inside);
        let kx = if *v { pill.right() - 14.0 } else { pill.left() + 14.0 };
        p.circle_filled(egui::pos2(kx, pill.center().y), 10.0, Color32::from_rgb(235, 238, 242));
        p.text(egui::pos2(pill.left() - 14.0, pill.center().y), Align2::RIGHT_CENTER, if *v { "ON" } else { "OFF" }, FontId::proportional(16.0), if *v { TEXT } else { DIM });
        flip
    }

    /// ◀ value ▶ picker over `opts`; returns true when the choice changed.
    fn picker(&mut self, ui: &mut Ui, label: &str, help: &str, opts: &[String], i: &mut usize, enabled: bool) -> bool {
        let (ctrl, focused, resp) = self.row(ui, label, help, enabled);
        let n = opts.len().max(1);
        let old = *i;
        let left = Rect::from_min_size(ctrl.min, Vec2::new(34.0, ctrl.height()));
        let right = Rect::from_min_size(egui::pos2(ctrl.right() - 34.0, ctrl.top()), Vec2::new(34.0, ctrl.height()));
        if enabled {
            if ui.interact(left, resp.id.with("prev"), Sense::click()).clicked() || (focused && self.keys.left) {
                *i = i.saturating_sub(1);
            }
            if ui.interact(right, resp.id.with("next"), Sense::click()).clicked() || (focused && self.keys.right) {
                *i = (*i + 1).min(n - 1);
            }
            if focused && self.keys.enter {
                *i = (*i + 1) % n;
            }
        }
        let p = ui.painter();
        p.rect_filled(ctrl, 2.0, Color32::from_black_alpha(if enabled { 120 } else { 60 }));
        p.rect_stroke(ctrl, 2.0, Stroke::new(1.0, if focused { ACCENT.gamma_multiply(0.7) } else { LINE }), StrokeKind::Inside);
        let arrow = |p: &egui::Painter, r: Rect, dir: f32, on: bool| {
            let c = r.center();
            let col = if on && enabled { Color32::WHITE } else { DIM.gamma_multiply(0.4) };
            p.add(egui::Shape::convex_polygon(
                vec![egui::pos2(c.x + 6.0 * dir, c.y), egui::pos2(c.x - 5.0 * dir, c.y - 7.0), egui::pos2(c.x - 5.0 * dir, c.y + 7.0)],
                col,
                Stroke::NONE,
            ));
        };
        arrow(p, left, -1.0, *i > 0);
        arrow(p, right, 1.0, *i + 1 < n);
        let text = opts.get(*i).cloned().unwrap_or_default();
        p.text(ctrl.center() - Vec2::new(0.0, 3.0), Align2::CENTER_CENTER, text, FontId::proportional(16.5), if enabled { TEXT } else { DIM });
        if n <= 12 {
            let w = 7.0;
            let x0 = ctrl.center().x - (n as f32 * (w + 3.0)) / 2.0;
            for k in 0..n {
                let r = Rect::from_min_size(egui::pos2(x0 + k as f32 * (w + 3.0), ctrl.bottom() - 6.0), Vec2::new(w, 2.0));
                p.rect_filled(r, 0.0, if k == *i { ACCENT } else { DIM.gamma_multiply(0.4) });
            }
        }
        *i != old
    }

    fn slider(&mut self, ui: &mut Ui, label: &str, help: &str, v: &mut f32, (lo, hi, step): (f32, f32, f32), fmt: &dyn Fn(f32) -> String) -> bool {
        let (ctrl, focused, resp) = self.row(ui, label, help, true);
        let old = *v;
        let track = Rect::from_min_max(egui::pos2(ctrl.left() + 6.0, ctrl.center().y - 10.0), egui::pos2(ctrl.right() - 84.0, ctrl.center().y + 10.0));
        let hit = ui.interact(track, resp.id.with("track"), Sense::click_and_drag());
        let mut moved = false;
        if let Some(pos) = hit.interact_pointer_pos() {
            let t = ((pos.x - track.left()) / track.width()).clamp(0.0, 1.0);
            *v = lo + t * (hi - lo);
            moved = true;
        }
        // Value box: shows the value; click it and type an exact one (Enter or clicking away applies it).
        let box_r = Rect::from_min_max(egui::pos2(ctrl.right() - 70.0, ctrl.center().y - 14.0), egui::pos2(ctrl.right(), ctrl.center().y + 14.0));
        let txt_id = resp.id.with("txt");
        let editing = ui.memory(|m| m.has_focus(txt_id));
        let mut text = if editing { ui.data_mut(|d| d.get_temp::<String>(txt_id)).unwrap_or_else(|| fmt(*v)) } else { fmt(*v) };
        let te = ui.put(box_r, egui::TextEdit::singleline(&mut text).id(txt_id).font(FontId::proportional(16.5)).horizontal_align(egui::Align::Max).char_limit(8));
        if te.has_focus() {
            ui.data_mut(|d| d.insert_temp(txt_id, text.clone()));
        }
        if te.lost_focus() {
            if let Ok(n) = text.trim().replace(',', ".").parse::<f32>() {
                *v = n.clamp(lo, hi);
            }
            ui.data_mut(|d| d.remove::<String>(txt_id));
        }
        let typing = te.has_focus();
        if focused && !typing && self.keys.left {
            *v -= step;
            moved = true;
        }
        if focused && !typing && self.keys.right {
            *v += step;
            moved = true;
        }
        if moved {
            // Snap to the step without float noise (0.2 stays 0.2).
            let n = ((*v - lo) / step).round();
            *v = ((lo + n * step) * 1000.0).round() / 1000.0;
            *v = v.clamp(lo, hi);
        }
        let t = (*v - lo) / (hi - lo);
        let p = ui.painter();
        let y = track.center().y;
        p.line_segment([egui::pos2(track.left(), y), egui::pos2(track.right(), y)], Stroke::new(4.0, Color32::from_white_alpha(28)));
        let x = track.left() + t * track.width();
        p.line_segment([egui::pos2(track.left(), y), egui::pos2(x, y)], Stroke::new(4.0, ACCENT.gamma_multiply(0.85)));
        let knob = Rect::from_center_size(egui::pos2(x, y), Vec2::new(10.0, 22.0));
        p.rect_filled(knob, 2.0, if hit.hovered() || hit.dragged() || focused { Color32::WHITE } else { Color32::from_rgb(200, 205, 212) });
        *v != old
    }

    /// A key-binding row: two slots. Click a slot and press a key; right-click a slot to clear it.
    fn binding(&mut self, ui: &mut Ui, group: &str, action: u32, label: &str, help: &str, clashes: &[(u32, u32, u8)]) {
        let help_full = format!("{help} Click a slot and press a key or mouse button (Esc cancels); right-click a slot to clear it.");
        let (ctrl, focused, resp) = self.row(ui, label, &help_full, true);
        let keys = self.s.mouse.keys(action);
        let gap = 8.0;
        let w = (ctrl.width() - gap) / 2.0;
        let mut start = None;
        for slot in 0..2 {
            let r = Rect::from_min_size(egui::pos2(ctrl.left() + slot as f32 * (w + gap), ctrl.top()), Vec2::new(w, ctrl.height()));
            let hit = ui.interact(r, resp.id.with(("slot", slot)), Sense::click());
            let listening = self.listen == Some((action, slot));
            let code = input::effective_keys(action as usize, keys)[slot];
            let clash: Vec<u32> = if code != 0 {
                clashes.iter().filter(|c| c.0 == code && c.1 != action && c.2 == context(group)).map(|c| c.1).collect()
            } else {
                Vec::new()
            };
            let p = ui.painter();
            let bound = code != 0;
            p.rect_filled(r, 2.0, if listening { ACCENT.gamma_multiply(0.25) } else if bound { Color32::from_rgba_premultiplied(30, 44, 60, 200) } else { Color32::from_black_alpha(120) });
            let stroke = if listening || hit.hovered() { ACCENT } else if !clash.is_empty() { Color32::from_rgb(230, 90, 70) } else { LINE };
            p.rect_stroke(r, 2.0, Stroke::new(1.0, stroke), StrokeKind::Inside);
            let (text, col) = if listening {
                ("Press a key ...".to_string(), ACCENT)
            } else if !clash.is_empty() {
                (input::key_name(code), Color32::from_rgb(255, 130, 110))
            } else {
                (input::key_name(code), if bound { Color32::from_rgb(140, 200, 255) } else { DIM })
            };
            p.text(r.center(), Align2::CENTER_CENTER, text, FontId::proportional(15.5), col);
            if hit.hovered() && !clash.is_empty() {
                let names: Vec<&str> = clash.iter().map(|a| action_name(*a)).collect();
                self.help = Some((label.to_string(), format!("{} is also bound to {}. Both actions fire when you press it.", input::key_name(code), names.join(", "))));
            }
            if hit.clicked() {
                start = Some(slot);
            }
            if hit.secondary_clicked() {
                let mut k = keys;
                k[slot] = 0;
                self.s.mouse.set_keys(action, k);
            }
        }
        if focused && self.keys.enter {
            start = Some(0);
        }
        if let Some(slot) = start {
            self.focus = self.row_i - 1;
            self.listen = Some((action, slot));
            self.status.clear();
            *capture() = Capture::Listening;
        }
    }

    // ---- Pages -----------------------------------------------------------------------------------------------------

    fn graphics(&mut self, ui: &mut Ui) {
        self.section(ui, "Graphics card");
        let gpu = GPU.lock().unwrap_or_else(|e| e.into_inner()).clone();
        self.note(ui, &format!("In use: {}", if gpu.is_empty() { "not known yet" } else { &gpu }));

        self.section(ui, "Display");
        let g = &mut self.s.graphics.clone();
        let mut mode = g.display_mode.min(2) as usize;
        let modes = ["Windowed", "Borderless fullscreen", "Fullscreen"].map(String::from);
        if self.picker(ui, "Display mode", "Windowed: a normal window you can move and resize. Borderless fullscreen: a window that covers the whole screen, so switching to other programs is instant. Fullscreen: the game takes over the display.", &modes, &mut mode, true) {
            g.display_mode = mode as u8;
        }
        let mut sizes: Vec<(u32, u32)> = WINDOW_SIZES.to_vec();
        if !sizes.contains(&(g.window_w, g.window_h)) {
            sizes.push((g.window_w, g.window_h));
        }
        let mut si = sizes.iter().position(|s| *s == (g.window_w, g.window_h)).unwrap_or(0);
        let names: Vec<String> = sizes.iter().map(|(w, h)| format!("{w} × {h}")).collect();
        let windowed = g.display_mode == 0;
        let size_help = if windowed { "The size of the game window's picture area." } else { "Only used in windowed mode; fullscreen modes fill your display." };
        if self.picker(ui, "Window size", size_help, &names, &mut si, windowed) {
            (g.window_w, g.window_h) = sizes[si];
        }
        let mut ri = RENDER_SCALES.iter().position(|s| *s == g.render_scale).unwrap_or(4);
        let (rw, rh) = crate::host::render_size(&settings::Graphics { render_scale: 100, ..g.clone() });
        let names: Vec<String> = RENDER_SCALES
            .iter()
            .map(|p| if *p == 100 { "Native".to_string() } else { format!("{p} % ({} × {})", rw * p / 100, rh * p / 100) })
            .collect();
        if self.picker(ui, "Rendering resolution", "The resolution the game draws at, as a share of the window. Lower is faster and softer; above 100 % draws more pixels than the screen has and scales them down, for a sharper picture.", &names, &mut ri, true) {
            g.render_scale = RENDER_SCALES[ri];
        }
        let mut ui_i = UI_SCALES.iter().position(|s| *s == g.ui_scale).unwrap_or(4);
        let names: Vec<String> = UI_SCALES.iter().map(|p| if *p == 100 { "100 % (automatic)".to_string() } else { format!("{p} %") }).collect();
        if self.picker(ui, "Menu size", "Size of the menus and in-game panels. Pick a smaller size if a screen does not fit your window.", &names, &mut ui_i, true) {
            g.ui_scale = UI_SCALES[ui_i];
        }
        let mut fi = g.fps_cap.min(4) as usize;
        let caps: Vec<String> = FPS_CAPS.iter().map(|s| s.to_string()).collect();
        if self.picker(ui, "Frame-rate limit", "The most frames the game draws each second. The engine offers 60, 120, 180 and 240, or no limit. Applies straight away.", &caps, &mut fi, true) {
            g.fps_cap = fi as u8;
        }
        let mut vs = g.vsync;
        if self.toggle(ui, "VSync", "Waits for the display before showing each frame: no tearing, at the cost of a little input delay. Off lets the frame-rate limit decide.", &mut vs) {
            g.vsync = vs;
        }
        self.s.graphics = g.clone();

        self.section(ui, "Quality");
        let mut qi = g.quality.min(2) as usize;
        let levels = ["Low", "Medium", "High"].map(String::from);
        if self.picker(ui, "Graphics quality", "Detail distance, lights, shadows, effects, water and texture filtering, like MCC's own preset. High draws full-detail objects much further away; Low is the old Xbox-like look and the fastest.", &levels, &mut qi, true) {
            g.quality = qi as u8;
        }
        let mut lm = g.live_menu_scene;
        if self.toggle(ui, "Live menu background", "Your Spartan, drawn live by the preview engine (the hidden copy of the game the Customize screen uses) behind the main menu, at 30 frames a second while it shows. Off shows still art from your install instead.", &mut lm) {
            g.live_menu_scene = lm;
        }
        self.s.graphics = g.clone();
    }

    fn audio(&mut self, ui: &mut Ui) {
        self.section(ui, "Volume");
        let a = &mut self.s.audio.clone();
        let pct = |v: f32| format!("{:.0} %", v * 100.0);
        let mut v = a.master;
        if self.slider(ui, "Master volume", "Everything the game plays.", &mut v, (0.0, 1.0, 0.05), &|v: f32| pct(v)) {
            a.master = v;
        }
        let mut v = a.music;
        if self.slider(ui, "Music", "The soundtrack, on top of the master volume.", &mut v, (0.0, 1.0, 0.05), &|v: f32| pct(v)) {
            a.music = v;
        }
        let mut v = a.sfx;
        if self.slider(ui, "Sound effects", "Weapons, vehicles, footsteps, voices and the rest, on top of the master volume.", &mut v, (0.0, 1.0, 0.05), &|v: f32| pct(v)) {
            a.sfx = v;
        }
        let mut v = a.menu_music;
        if self.slider(ui, "Menu music", "The main menu's theme. 0 turns it off.", &mut v, (0.0, 1.0, 0.05), &|v: f32| pct(v)) {
            a.menu_music = v;
        }
        self.s.audio = a.clone();
        self.section(ui, "Output");
        self.note(ui, "The game plays through Windows' default output device.");
    }

    fn game(&mut self, ui: &mut Ui) {
        self.section(ui, "Camera");
        let g = &mut self.s.game.clone();
        let mut v = g.fov as f32;
        if self.slider(ui, "Field of view", "How wide you see on foot, in degrees. Wider shows more around you but makes everything smaller. The game's default is 78°.", &mut v, (65.0, 120.0, 1.0), &|v: f32| format!("{v:.0}°")) {
            g.fov = v as i32;
        }
        let mut v = g.vehicle_fov as f32;
        if self.slider(ui, "Vehicle field of view", "The same, while you drive, fly or ride in a vehicle.", &mut v, (65.0, 120.0, 1.0), &|v: f32| format!("{v:.0}°")) {
            g.vehicle_fov = v as i32;
        }
        self.section(ui, "HUD");
        let mut ci = g.crosshair_lowered as usize;
        if self.picker(ui, "Crosshair position", "Where the aiming reticle sits: the game's standard spot, or lower on the screen as in older Halo games.", &["Standard".to_string(), "Lowered".to_string()], &mut ci, true) {
            g.crosshair_lowered = ci == 1;
        }
        self.section(ui, "Character");
        let mut vi = g.female_voice as usize;
        if self.picker(ui, "Spartan voice", "The voice your Spartan uses for callouts and pain sounds.", &["Male".to_string(), "Female".to_string()], &mut vi, true) {
            g.female_voice = vi == 1;
        }
        self.s.game = g.clone();
    }

    fn player(&mut self, ui: &mut Ui) {
        self.section(ui, "Player");
        let (ctrl, _, _) = self.row(ui, "Player name", "The name over your head, on the scoreboard and in the server browser. Up to 15 characters; applies from your next game.", true);
        ui.put(ctrl, egui::TextEdit::singleline(&mut self.s.player_name).char_limit(15));
        self.section(ui, "Network");
        let (ctrl, _, _) = self.row(ui, "Master server", "The server that lists community games for the server browser, and where your hosted games are announced. Empty = your build's default list (public builds have none: then the browser shows Favourites, LAN and Direct Connect only). Several URLs can be separated by commas.", true);
        ui.put(ctrl, egui::TextEdit::singleline(&mut self.s.master_url));
    }

    fn keyboard(&mut self, ui: &mut Ui) {
        self.section(ui, "Mouse");
        let mut v = self.s.mouse_sens;
        if self.slider(ui, "Mouse sensitivity", "How far you turn for each movement of the mouse. 1.00 turns about 0.086° per mouse count.", &mut v, (0.1, 5.0, 0.05), &|v: f32| format!("{v:.2}")) {
            self.s.mouse_sens = v;
        }
        let m = &mut self.s.mouse.clone();
        let mut v = m.acceleration;
        if self.slider(ui, "Aim acceleration", "Fast mouse movements turn further than slow ones. Off gives the same turn for the same distance at any speed.", &mut v, (0.0, 1.0, 0.05), &|v: f32| if v <= 0.0 { "Off".into() } else { format!("{:.0} %", v * 100.0) }) {
            m.acceleration = v;
        }
        let mut v = m.smoothing;
        if self.slider(ui, "Mouse smoothing", "Blends each mouse movement with the last one: steadier aim on a jittery mouse, a touch of delay.", &mut v, (0.0, 1.0, 0.05), &|v: f32| if v <= 0.0 { "Off".into() } else { format!("{:.0} %", v * 100.0) }) {
            m.smoothing = v;
        }
        let mut b = m.invert_look;
        if self.toggle(ui, "Invert look", "Moving the mouse forward looks down instead of up.", &mut b) {
            m.invert_look = b;
        }
        let mut b = m.invert_flight;
        if self.toggle(ui, "Invert flight", "Flying vehicles pitch like an aircraft: mouse forward dives.", &mut b) {
            m.invert_flight = b;
        }
        self.s.mouse.invert_look = m.invert_look;
        self.s.mouse.invert_flight = m.invert_flight;
        self.s.mouse.acceleration = m.acceleration;
        self.s.mouse.smoothing = m.smoothing;

        let clashes: Vec<(u32, u32, u8)> = ACTIONS
            .iter()
            .flat_map(|a| {
                let k = input::effective_keys(a.1 as usize, self.s.mouse.keys(a.1));
                [(k[0], a.1, context(a.0)), (k[1], a.1, context(a.0))]
            })
            .filter(|c| c.0 != 0)
            .collect();
        let mut group = "";
        for &(g, action, name, help) in ACTIONS {
            if g != group {
                group = g;
                self.section(ui, g);
            }
            self.binding(ui, g, action, name, help, &clashes);
        }
        self.section(ui, "Fixed keys");
        self.note(ui, "Esc pauses and frees the mouse; F10 frees or captures the mouse. These cannot be rebound.");
        if self.in_game {
            self.note(ui, "Key changes reach the game at once.");
        }
    }

    fn controller(&mut self, ui: &mut Ui) {
        self.section(ui, "Look");
        let p = &mut self.s.pad.clone();
        let mut v = p.look_sens as f32 + 1.0;
        if self.slider(ui, "Look sensitivity", "How fast the right stick turns you. The game's default is 5.", &mut v, (1.0, 19.0, 1.0), &|v: f32| format!("{v:.0}")) {
            p.look_sens = (v as u8).saturating_sub(1);
        }
        let mut b = p.invert_look;
        if self.toggle(ui, "Invert look", "Pushing the right stick up looks down.", &mut b) {
            p.invert_look = b;
        }
        let mut b = p.invert_flight;
        if self.toggle(ui, "Invert flight", "Flying vehicles pitch like an aircraft: stick up dives.", &mut b) {
            p.invert_flight = b;
        }
        let mut b = p.auto_center;
        if self.toggle(ui, "Auto-centre look", "Brings your view back to level while you move without aiming.", &mut b) {
            p.auto_center = b;
        }
        let mut v = p.axial_dead_zone;
        if self.slider(ui, "Axial dead zone", "How far the stick must move along one axis before it counts. Raise it if your view drifts.", &mut v, (0.0, 0.2, 0.01), &|v: f32| format!("{:.0} %", v * 100.0)) {
            p.axial_dead_zone = v;
        }
        let mut v = p.radial_dead_zone;
        if self.slider(ui, "Radial dead zone", "How far the stick must move in any direction before it counts.", &mut v, (0.0, 0.2, 0.01), &|v: f32| format!("{:.0} %", v * 100.0)) {
            p.radial_dead_zone = v;
        }
        self.section(ui, "Layout");
        let names: Vec<String> = BUTTON_PRESETS.iter().map(|b| b.1.to_string()).collect();
        let mut i = BUTTON_PRESETS.iter().position(|b| b.0 == p.button_preset).unwrap_or(0);
        if self.picker(ui, "Button layout", "Which button does what: the game's built-in layouts.", &names, &mut i, true) {
            p.button_preset = BUTTON_PRESETS[i].0;
        }
        let names: Vec<String> = STICK_PRESETS.iter().map(|s| s.to_string()).collect();
        let mut i = p.stick_preset.min(3) as usize;
        if self.picker(ui, "Stick layout", "Which stick moves and which one looks.", &names, &mut i, true) {
            p.stick_preset = i as u8;
        }
        let mut b = p.swap_triggers;
        if self.toggle(ui, "Swap triggers and bumpers", "Puts the trigger actions on the bumpers and the other way round.", &mut b) {
            p.swap_triggers = b;
        }
        self.section(ui, "Feel");
        let mut b = p.vibration;
        if self.toggle(ui, "Vibration", "Rumble when you fire, take hits or drive over rough ground.", &mut b) {
            p.vibration = b;
        }
        let mut b = p.crouch_lock;
        if self.toggle(ui, "Crouch lock", "Crouch stays on after you let go of the button, until you press it again.", &mut b) {
            p.crouch_lock = b;
        }
        let mut b = p.clench_protection;
        if self.toggle(ui, "Clench protection", "Stops a hard squeeze of the controller from pressing the stick in by accident.", &mut b) {
            p.clench_protection = b;
        }
        self.s.pad = p.clone();
        self.note(ui, if input::pad_connected() { "A controller is connected." } else { "No controller found. Plug one in; the game picks it up within a second." });
    }
}

fn section_line(ui: &mut Ui, title: &str) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 30.0), Sense::hover());
    let p = ui.painter();
    let g = p.layout_no_wrap(title.to_uppercase(), FontId::proportional(13.5), DIM);
    let w = g.size().x;
    p.galley(egui::pos2(rect.left(), rect.center().y - g.size().y / 2.0), g, DIM);
    p.line_segment([egui::pos2(rect.left() + w + 12.0, rect.center().y), egui::pos2(rect.right(), rect.center().y)], Stroke::new(1.0, LINE));
}

/// Help-panel picture for the FOV sliders: a wedge as wide as the chosen angle.
fn fov_wedge(ui: &mut Ui, deg: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 200.0), Sense::hover());
    let p = ui.painter();
    p.text(egui::pos2(rect.center().x, rect.top()), Align2::CENTER_TOP, format!("{deg:.0}°"), FontId::proportional(30.0), Color32::WHITE);
    let apex = egui::pos2(rect.center().x, rect.bottom() - 6.0);
    let r = 120.0;
    let half = deg.to_radians() / 2.0;
    let mut pts = vec![apex];
    for k in 0..=24 {
        let a = -half + 2.0 * half * k as f32 / 24.0;
        pts.push(apex + Vec2::new(a.sin() * r, -a.cos() * r));
    }
    p.add(egui::Shape::convex_polygon(pts.clone(), ACCENT.gamma_multiply(0.25), Stroke::NONE));
    p.add(egui::Shape::closed_line(pts, Stroke::new(2.0, ACCENT)));
    let d = 78f32.to_radians() / 2.0;
    for s in [-1.0, 1.0] {
        p.line_segment([apex, apex + Vec2::new((s * d).sin() * r, -(d).cos() * r)], Stroke::new(1.0, DIM.gamma_multiply(0.6)));
    }
    p.text(apex + Vec2::new(-d.sin() * r, -d.cos() * r - 4.0), Align2::CENTER_BOTTOM, "78° default", FontId::proportional(12.0), DIM);
}

/// Help-panel legend of the keys that drive this screen.
fn key_legend(ui: &mut Ui) {
    for (k, what) in [
        ("UP · DOWN", "Move between settings"),
        ("LEFT · RIGHT", "Change the selected setting"),
        ("ENTER", "Toggle, or listen for a new key"),
        ("TAB", "Next page (Shift+Tab: previous)"),
        ("ESC", "Cancel listening, or leave Settings"),
    ] {
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(Vec2::new(112.0, 30.0), Sense::hover());
            ui.painter().rect_filled(r, 3.0, Color32::from_rgba_premultiplied(30, 44, 60, 200));
            ui.painter().rect_stroke(r, 3.0, Stroke::new(1.0, LINE), StrokeKind::Inside);
            ui.painter().text(r.center(), Align2::CENTER_CENTER, k, FontId::proportional(14.0), Color32::from_rgb(140, 200, 255));
            ui.label(egui::RichText::new(what).size(14.0).color(TEXT));
        });
    }
}
