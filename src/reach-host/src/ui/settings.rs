//! Player settings shown in the menu and the pause overlay, saved as `<exe dir>\reach-data\settings.json`.

use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::log;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub player_name: String,
    /// Multiplies input.rs's 0.0015 rad per mouse count (same as `--mouse-sens`).
    pub mouse_sens: f32,
    /// Empty = the default masters; a URL or comma-separated list overrides them.
    pub master_url: String,
    /// Server browser favourites: `host:port` as the player typed it (hostnames re-resolved on every refresh).
    pub favourites: Vec<String>,
    /// Last choices, so the menu opens where the player left it.
    pub last_map: String,
    pub last_variant: String,
    pub host_name: String,
    pub host_max_players: u32,
    pub host_register: bool,
    /// UDP port a menu-hosted (listen) server binds; friends forward / tunnel this one.
    pub host_port: u16,
    pub graphics: Graphics,
    pub audio: Audio,
    pub game: Game,
    pub mouse: Mouse,
    pub pad: Pad,
    /// Spartan / Elite customisation (ui/customize.rs), written into the slot 34 profile.
    pub armour: Armour,
    /// CAMPAIGN screen (ui/campaign.rs): last picks and finished missions.
    pub campaign: Campaign,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Campaign {
    /// Mission map stem (`m10`).
    pub mission: String,
    /// 0 easy .. 3 legendary.
    pub difficulty: u8,
    /// `options::SKULLS` keys.
    pub skulls: Vec<String>,
    /// Missions finished on any difficulty (map stems). TODO: nothing records a mission end yet (untested engine path).
    pub finished: Vec<String>,
}

impl Default for Campaign {
    fn default() -> Self {
        Campaign { mission: "m05".into(), difficulty: 1, skulls: Vec::new(), finished: Vec::new() }
    }
}

/// Window and engine video settings (host slot 32 and our own window / swap chain).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Graphics {
    /// 0 windowed, 1 borderless fullscreen, 2 fullscreen (DXGI exclusive).
    pub display_mode: u8,
    pub window_w: u32,
    pub window_h: u32,
    /// Swap-chain size as a percentage of the window's client area (DXGI stretches it to the window).
    pub render_scale: u32,
    /// Slot 32 +0x15: 0 = 60, 1 = 120, 2 = 180, 3 = 240, 4 = no limit.
    pub fps_cap: u8,
    /// Slot 32 +0x14 (the engine's Present sync interval).
    pub vsync: bool,
    /// Graphics quality for every slot-32 quality byte: 0 low, 1 medium, 2 high (detail distances x0.5 / x1 / x3).
    pub quality: u8,
    /// Menu and overlay size, percent of the automatic size (fits a 1280 x 720 layout to the window).
    pub ui_scale: u32,
}

impl Default for Graphics {
    fn default() -> Self {
        Graphics { display_mode: 0, window_w: 1280, window_h: 720, render_scale: 100, fps_cap: 0, vsync: true, quality: 2, ui_scale: 100 }
    }
}

/// Profile volumes (+0xA5C / +0xA60 / +0xA64), 0..1.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Audio {
    pub master: f32,
    pub music: f32,
    pub sfx: f32,
}

impl Default for Audio {
    fn default() -> Self {
        Audio { master: 1.0, music: 1.0, sfx: 1.0 }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Game {
    /// Profile +0x14 / +0x18 (degrees).
    pub fov: i32,
    pub vehicle_fov: i32,
    /// Profile +0x1C (crosshair location).
    pub crosshair_lowered: bool,
    /// Profile +0x27.
    pub female_voice: bool,
}

impl Default for Game {
    fn default() -> Self {
        Game { fov: 78, vehicle_fov: 78, crosshair_lowered: false, female_voice: false }
    }
}

/// One keyboard/mouse binding: MCC action index (0..66) and its two codes (input.rs `key_name` for the meaning).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct KeyBinding {
    pub action: u32,
    pub keys: [u32; 2],
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Mouse {
    pub invert_look: bool,
    /// Profile +0x22 (mouse aircraft inversion, applied by the engine).
    pub invert_flight: bool,
    /// 0..1, input.rs.
    pub acceleration: f32,
    /// 0..1, input.rs.
    pub smoothing: f32,
    /// Only actions that differ from input.rs's defaults need to be here.
    pub bindings: Vec<KeyBinding>,
}

impl Default for Mouse {
    fn default() -> Self {
        Mouse { invert_look: false, invert_flight: false, acceleration: 0.0, smoothing: 0.0, bindings: Vec::new() }
    }
}

impl Mouse {
    pub fn keys(&self, action: u32) -> [u32; 2] {
        self.bindings.iter().find(|b| b.action == action).map_or_else(|| crate::input::default_keys(action as usize), |b| b.keys)
    }

    pub fn set_keys(&mut self, action: u32, keys: [u32; 2]) {
        self.bindings.retain(|b| b.action != action);
        if keys != crate::input::default_keys(action as usize) {
            self.bindings.push(KeyBinding { action, keys });
            self.bindings.sort_by_key(|b| b.action);
        }
    }
}

/// Controller settings, all profile fields the engine reads in `ApplyProfileLookSettingsToPlayer` (0x1802eda08).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Pad {
    /// +0x1B5 / +0x1B6, index 0..18 (engine default 4).
    pub look_sens: u8,
    /// +0x1D.
    pub invert_look: bool,
    /// +0x21.
    pub invert_flight: bool,
    /// +0x1B8 / +0x1BC, the engine accepts 0..0.2.
    pub axial_dead_zone: f32,
    pub radial_dead_zone: f32,
    /// +0x1C8, MCC's preset number (see settings_screen.rs BUTTON_PRESETS).
    pub button_preset: u8,
    /// +0x1C9: 0..3.
    pub stick_preset: u8,
    /// +0x1D7.
    pub swap_triggers: bool,
    /// Inverse of +0x1F; also gates our XInput rumble.
    pub vibration: bool,
    /// +0x24.
    pub crouch_lock: bool,
    /// +0x26.
    pub clench_protection: bool,
    /// +0x23.
    pub auto_center: bool,
}

impl Default for Pad {
    fn default() -> Self {
        Pad {
            look_sens: 4,
            invert_look: false,
            invert_flight: false,
            axial_dead_zone: 0.2,
            radial_dead_zone: 0.2,
            button_preset: 0,
            stick_preset: 0,
            swap_triggers: false,
            vibration: true,
            crouch_lock: false,
            clench_protection: false,
            auto_center: false,
        }
    }
}

/// Appearance. Every piece is MCC's own customisation item id (the engine converts it to its index,
/// notes/customization.md); offsets are into the slot 34 profile.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Armour {
    /// +0x38: play as an Elite where the game type allows it.
    pub elite: bool,
    /// +0x64: 0xC0 male, 0xC1 female Spartan body.
    pub body: u32,
    /// +0x40 helmet (includes its attachment), +0x44 / +0x48 shoulders, +0x4C chest, +0x50 wrist, +0x54 utility,
    /// +0x6C knee guards, +0x70 visor colour, +0x68 armour effect.
    pub helmet: u32,
    pub left_shoulder: u32,
    pub right_shoulder: u32,
    pub chest: u32,
    pub wrist: u32,
    pub utility: u32,
    pub knees: u32,
    pub visor: u32,
    pub effect: u32,
    /// +0x8C Elite armour type, +0x90 Elite armour effect.
    pub elite_armour: u32,
    pub elite_effect: u32,
    /// +0x94 Firefight voice.
    pub voice: u32,
    /// +0x98 / +0x9C colours (0xCA steel .. 0xE7 pale).
    pub primary: u32,
    pub secondary: u32,
}

impl Default for Armour {
    fn default() -> Self {
        // Reach's stock look: Mark V [B], default pieces, steel / steel.
        Armour {
            elite: false,
            body: 0xC0,
            helmet: 0x00,
            left_shoulder: 0x5A,
            right_shoulder: 0x6C,
            chest: 0x7E,
            wrist: 0x90,
            utility: 0x96,
            knees: 0xA2,
            visor: 0x9D,
            effect: 0xA6,
            elite_armour: 0xC2,
            elite_effect: 0xAD,
            voice: 0xB4,
            primary: 0xCA,
            secondary: 0xCA,
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            player_name: "Player".into(),
            mouse_sens: 1.0,
            // Empty = browser::default_masters() (set at build time; none by default).
            master_url: String::new(),
            favourites: Vec::new(),
            last_map: "forge_halo".into(),
            last_variant: "slayer_054.bin".into(),
            host_name: "Reach host".into(),
            host_max_players: 8,
            host_register: true,
            host_port: 47800,
            graphics: Graphics::default(),
            audio: Audio::default(),
            game: Game::default(),
            mouse: Mouse::default(),
            pad: Pad::default(),
            armour: Armour::default(),
            campaign: Campaign::default(),
        }
    }
}

static SETTINGS: Mutex<Option<Settings>> = Mutex::new(None);

fn path() -> PathBuf {
    let dir = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())).unwrap_or_default();
    dir.join("reach-data").join("settings.json")
}

/// Current settings (loaded from disk on first use; defaults if the file is missing or unreadable).
pub fn get() -> Settings {
    let mut g = SETTINGS.lock().unwrap_or_else(|e| e.into_inner());
    g.get_or_insert_with(|| {
        let p = path();
        match std::fs::read_to_string(&p).map_err(|e| e.to_string()).and_then(|t| serde_json::from_str(&t).map_err(|e| e.to_string())) {
            Ok(s) => {
                log!("settings loaded from {}", p.display());
                s
            }
            Err(e) => {
                log!("settings: using defaults ({}: {e})", p.display());
                Settings::default()
            }
        }
    })
    .clone()
}

pub fn save(s: &Settings) {
    *SETTINGS.lock().unwrap_or_else(|e| e.into_inner()) = Some(s.clone());
    let p = path();
    let _ = std::fs::create_dir_all(p.parent().unwrap());
    match std::fs::write(&p, serde_json::to_string_pretty(s).unwrap()) {
        Ok(()) => log!("settings saved to {}", p.display()),
        Err(e) => log!("settings: could not write {}: {e}", p.display()),
    }
}
