//! Main menu, shown when reach-host starts without launch arguments. Runs its own frame loop on our window and
//! swap chain before the engine exists, and ends with a `Choice`: quit, or the command-line arguments of the game
//! to start (the same flags a player could type, so every path through the menu is also reachable from scripts).

use std::path::{Path, PathBuf};

use egui::{Rect, Ui, Vec2};
use windows::core::Interface;
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Direct3D11::ID3D11DeviceContext;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_UNKNOWN;
use windows::Win32::Graphics::Dxgi::{IDXGISwapChain, IDXGISwapChain1, DXGI_PRESENT, DXGI_SWAP_CHAIN_FLAG};
use windows::Win32::UI::WindowsAndMessaging::*;

use super::settings::{self, Settings};
use super::theme::{self, ACCENT, DIM};
use crate::log;

pub enum Choice {
    Quit,
    /// Extra command-line arguments (after `--mcc`) for the game to launch.
    Launch(Vec<String>),
}

/// Built-in multiplayer maps: (mapinfo file stem, name shown). Only those with a mapinfo file are offered.
pub const MAPS: &[(&str, &str)] = &[
    ("forge_halo", "Forge World"),
    ("20_sword_slayer", "Sword Base"),
    ("30_settlement", "Powerhouse"),
    ("35_island", "Spire"),
    ("45_aftship", "Zealot"),
    ("45_launch_station", "Countdown"),
    ("50_panopticon", "Boardwalk"),
    ("52_ivory_tower", "Reflection"),
    ("70_boneyard", "Boneyard"),
    ("cex_beaver_creek", "Battle Canyon"),
    ("cex_damnation", "Penance"),
    ("cex_hangemhigh", "High Noon"),
    ("cex_headlong", "Breakneck"),
    ("cex_prisoner", "Solitary"),
    ("cex_timberland", "Ridgeline"),
    ("condemned", "Condemned"),
    ("trainingpreserve", "Highlands"),
    ("dlc_invasion", "Breakpoint"),
    ("dlc_medium", "Tempest"),
    ("dlc_slayer", "Anchor 9"),
];

/// Firefight maps: (mapinfo stem, name shown; checked against each file's English title).
pub(super) const FF_MAPS: &[(&str, &str)] = &[
    ("ff50_park", "Beachhead"),
    ("ff10_prototype", "Overlook"),
    ("ff20_courtyard", "Courtyard"),
    ("ff30_waterfront", "Waterfront"),
    ("ff45_corvette", "Corvette"),
    ("ff60_icecave", "Glacier"),
    ("ff70_holdout", "Holdout"),
    ("ff60_airview", "Outpost"),
    ("ff_unearthed", "Unearthed"),
];

/// Campaign missions (m40 has no map index; m70_a is the credits).
const MISSIONS: &[(&str, &str)] = &[
    ("m05", "Noble Actual"),
    ("m10", "Winter Contingency"),
    ("m20", "ONI: Sword Base"),
    ("m30", "Nightfall"),
    ("m35", "Tip of the Spear"),
    ("m45", "Long Night of Solace"),
    ("m50", "Exodus"),
    ("m52", "New Alexandria"),
    ("m60", "The Package"),
    ("m70", "The Pillar of Autumn"),
    ("m70_bonus", "Lone Wolf"),
];

pub(super) const DIFFICULTIES: &[(&str, &str)] = &[("easy", "Easy"), ("normal", "Normal"), ("heroic", "Heroic"), ("legendary", "Legendary")];

/// `hr_ff_generatorDefense_1set.bin` -> `Generator Defense`.
pub(super) fn ff_variant_title(file: &str) -> String {
    let stem = file.trim_end_matches(".bin").trim_start_matches("hr_ff_");
    let stem = stem.rsplit_once('_').filter(|(_, t)| t.starts_with(|c: char| c.is_ascii_digit())).map_or(stem, |(a, _)| a);
    let mut out = String::new();
    for (i, c) in stem.chars().enumerate() {
        if i == 0 {
            out.extend(c.to_uppercase());
        } else {
            if c.is_uppercase() {
                out.push(' ');
            }
            out.push(c);
        }
    }
    out
}

pub fn map_title(stem: &str) -> String {
    if let Some(m) = FF_MAPS.iter().chain(MISSIONS).find(|m| m.0 == stem) {
        return m.1.to_string();
    }
    let stem = if stem == "forge_world" { "forge_halo" } else { stem };
    MAPS.iter().find(|m| m.0.eq_ignore_ascii_case(stem)).map_or_else(|| stem.to_string(), |m| m.1.to_string())
}

/// `slayer_pro_054.bin` -> `Slayer Pro`.
pub fn variant_title(file: &str) -> String {
    if file.eq_ignore_ascii_case(crate::forge::FORGE_GAME_VARIANT) {
        return "Forge (Basic Editing)".into();
    }
    let stem = file.trim_end_matches(".bin").trim_end_matches("_054");
    stem.split('_')
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            if matches!(w, "ctf" | "koth" | "dlc" | "ffa" | "dmr" | "ar") {
                return w.to_uppercase();
            }
            c.next().map_or(String::new(), |f| f.to_uppercase().collect::<String>() + c.as_str())
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Screen {
    Home,
    Play,
    Browser,
    Host,
    Settings,
    Customize,
    Campaign,
    Forge,
}

struct Menu {
    mcc: PathBuf,
    screen: Screen,
    settings: Settings,
    browser: super::browser_screen::BrowserScreen,
    choice: Option<Choice>,
    /// The SETTINGS screen while it is open (settings_screen.rs; it saves and applies on its own).
    settings_screen: Option<super::settings_screen::SettingsScreen>,
    /// The CUSTOMIZE screen while it is open (customize.rs; saves and applies on its own).
    customize: Option<super::customize::CustomizeScreen>,
    /// The CAMPAIGN screen (campaign.rs; Firefight tab too).
    campaign: Option<super::campaign::CampaignScreen>,
    /// The FORGE screen (ui/forge.rs).
    forge: Option<super::forge::ForgeScreen>,
    /// PLAY and HOST GAME (play_screen.rs): one screen, `host` switches the SERVER step on.
    mp: super::play_screen::PlayScreen,
}

impl Menu {
    fn new(mcc: &Path) -> Self {
        let settings = settings::get();
        let mut mp = super::play_screen::PlayScreen::new(mcc, &settings);
        if let Ok(spec) = std::env::var("REACH_UI_MP") {
            mp.apply_test(&spec);
        }
        log!("menu: {} in {}", mp.summary(), mcc.display());
        Menu {
            mcc: mcc.to_path_buf(),
            screen: Screen::Home,
            settings,
            browser: super::browser_screen::BrowserScreen::new(mcc),
            choice: None,
            settings_screen: None,
            customize: None,
            campaign: None,
            forge: None,
            mp,
        }
    }

    fn remember_selection(&mut self) {
        // Start from the saved file (the settings screen saves on its own), keep this screen's host form.
        let mut s = settings::get();
        s.host_name = std::mem::take(&mut self.settings.host_name);
        s.host_max_players = self.settings.host_max_players;
        s.host_register = self.settings.host_register;
        s.host_port = self.settings.host_port;
        self.mp.remember(&mut s);
        self.settings = s;
        settings::save(&self.settings);
    }

    fn launch(&mut self, args: Vec<String>) {
        log!("menu: launch {}", args.join(" "));
        self.choice = Some(Choice::Launch(args));
    }

    fn play_args(&self) -> Option<Vec<String>> {
        self.mp.args()
    }

    fn open(&mut self, s: Screen) {
        self.screen = s;
        if s == Screen::Browser {
            self.browser.open();
        }
        if matches!(s, Screen::Play | Screen::Host) {
            self.mp.host = s == Screen::Host;
            if self.mp.step == super::play_screen::Step::Server && !self.mp.host {
                self.mp.step = super::play_screen::Step::Map;
            }
        }
    }

    /// The CAMPAIGN screen for REACH_UI_SHOT steps; `REACH_UI_CAMP="m10 heroic iron,blind"` sets its picks once.
    fn campaign_for_test(&mut self) -> &mut super::campaign::CampaignScreen {
        let fresh = self.campaign.is_none();
        let c = self.campaign.get_or_insert_with(|| super::campaign::CampaignScreen::new(&self.mcc));
        if fresh {
            let v = std::env::var("REACH_UI_CAMP").unwrap_or_default();
            let mut w = v.split_whitespace();
            if let Some(m) = w.next() {
                c.select(m);
            }
            if let Some(d) = w.next() {
                c.set_difficulty(d);
            }
            if let Some(k) = w.next() {
                c.skulls = crate::options::parse_skulls(k).unwrap_or(0);
            }
        }
        c
    }

    /// Listen-server flags from the Host form (port, master) around `mode_args` (HOST GAME, HOST CO-OP, HOST FORGE).
    fn listen_args(&self, players: u32, mode_args: Vec<String>) -> Vec<String> {
        let port = if self.settings.host_port == 0 { 47800 } else { self.settings.host_port };
        let mut a: Vec<String> = vec!["--listen".into(), "--players".into(), players.clamp(2, 16).to_string(), "--bind".into(), format!("0.0.0.0:{port}")];
        a.extend(mode_args);
        if self.settings.host_register {
            a.extend(["--master".into(), crate::browser::master_list().join(","), "--server-name".into(), self.settings.host_name.clone()]);
        }
        a
    }

    /// HOST GAME's listen-server flags for the current pick.
    fn host_args(&self) -> Option<Vec<String>> {
        Some(self.listen_args(self.mp.host_players(&self.settings), self.mp.args()?))
    }

    // ---- Drawing ---------------------------------------------------------------------------------------------

    fn ui(&mut self, ui: &mut Ui) {
        theme::backdrop(ui);
        let full = ui.max_rect();
        if self.screen == Screen::Home {
            super::menu_scene::draw(ui, full, &self.mcc, &self.settings); // live Spartan or still art
        }
        if self.screen == Screen::Settings {
            let ss = self.settings_screen.get_or_insert_with(|| super::settings_screen::SettingsScreen::new(false));
            if ss.ui(ui) {
                self.settings_screen = None;
                self.settings = settings::get();
                self.open(Screen::Home);
            }
            return;
        }
        if self.screen == Screen::Browser {
            match self.browser.ui(ui) {
                super::browser_screen::Action::None => {}
                super::browser_screen::Action::Back => self.open(Screen::Home),
                super::browser_screen::Action::Launch(a) => self.launch(a),
            }
            return;
        }
        if matches!(self.screen, Screen::Play | Screen::Host) {
            use super::play_screen::Action;
            match self.mp.ui(ui, &mut self.settings) {
                Action::None => {}
                Action::Back => self.open(Screen::Home),
                Action::Play(a) => {
                    self.remember_selection();
                    self.launch(a);
                }
                Action::Host { players, args } => {
                    self.remember_selection();
                    let a = self.listen_args(players, args);
                    self.launch(a);
                }
            }
            return;
        }
        if self.screen == Screen::Campaign {
            let mcc = self.mcc.clone();
            match self.campaign.get_or_insert_with(|| super::campaign::CampaignScreen::new(&mcc)).ui(ui) {
                super::campaign::Action::None => {}
                super::campaign::Action::Back => self.open(Screen::Home),
                super::campaign::Action::Play(a) => self.launch(a),
                super::campaign::Action::Host { players, args } => self.launch(self.listen_args(players, args)),
            }
            return;
        }
        if self.screen == Screen::Forge {
            let mcc = self.mcc.clone();
            match self.forge.get_or_insert_with(|| super::forge::ForgeScreen::new(&mcc)).ui(ui) {
                super::forge::Action::None => {}
                super::forge::Action::Back => {
                    self.forge = None;
                    self.open(Screen::Home);
                }
                super::forge::Action::Play(a) => self.launch(a),
                super::forge::Action::Host { players, args } => self.launch(self.listen_args(players, args)),
            }
            return;
        }
        if self.screen == Screen::Customize {
            if self.customize.get_or_insert_with(super::customize::CustomizeScreen::new).ui(ui) {
                self.customize = None;
                self.settings = settings::get();
                self.open(Screen::Home);
            }
            return;
        }

        // Home: left column with title + navigation, the ready card.
        let nav = Rect::from_min_size(full.min + Vec2::new(40.0, 48.0), Vec2::new(300.0, full.height() - 96.0));
        ui.scope_builder(egui::UiBuilder::new().max_rect(nav), |ui| {
            ui.label(egui::RichText::new("HALO: REACH").size(15.0).color(ACCENT));
            ui.label(egui::RichText::new("NOBLE OUTREACH").size(32.0).color(egui::Color32::WHITE));
            ui.label(egui::RichText::new("standalone community client").size(13.0).color(DIM));
            ui.add_space(40.0);
            for (label, s) in [("Play", Screen::Play), ("Campaign", Screen::Campaign), ("Forge", Screen::Forge), ("Server browser", Screen::Browser), ("Host game", Screen::Host), ("Customize", Screen::Customize), ("Settings", Screen::Settings)] {
                if theme::nav_item(ui, label, self.screen == s).clicked() {
                    self.open(s);
                }
            }
            ui.add_space(14.0);
            if theme::nav_item(ui, "Quit", false).clicked() {
                self.choice = Some(Choice::Quit);
            }
            ui.add_space((ui.available_height() - 24.0).max(0.0));
            ui.label(egui::RichText::new(format!("{}  ·  v{}", self.settings.player_name, env!("CARGO_PKG_VERSION"))).size(13.0).color(DIM));
        });
        let content = Rect::from_min_max(egui::pos2(nav.right() + 40.0, full.top() + 40.0), full.max - Vec2::new(40.0, 40.0));
        self.home(ui, content);
    }

    fn home(&mut self, ui: &mut Ui, content: Rect) {
        let card = Rect::from_min_size(egui::pos2(content.left(), content.bottom() - 210.0), Vec2::new(content.width().min(560.0), 210.0));
        theme::panel(ui, card, |ui| {
            theme::caption(ui, "Ready");
            let c = crate::catalog::get(&self.mcc);
            let nv: usize = c.maps.iter().map(|m| m.variants.len() - 1).sum();
            ui.label(egui::RichText::new(format!("{} maps · {} map variants · {} game types in your MCC files", c.maps.len(), nv, c.games.len())).size(17.0));
            ui.add_space(6.0);
            ui.label(egui::RichText::new(format!("Playing as {}", self.settings.player_name)).color(DIM));
            let ms = crate::browser::master_list();
            let ml = if ms.is_empty() { "Master server: none (Direct Connect, Favourites and LAN)".to_string() } else { format!("Master server {}", ms.join(" or ")) };
            ui.label(egui::RichText::new(ml).color(DIM));
            ui.add_space(10.0);
            let a = self.play_args();
            let label = format!("Quick play: {}", self.mp.summary());
            let w = ui.available_width().min(500.0);
            if theme::primary_button(ui, &label, w, a.is_some()).clicked() {
                if let Some(a) = a {
                    self.launch(a);
                }
            }
        });
    }
}

/// Run the menu until the player quits or picks a game. Uses (and leaves) the swap chain for the engine.
pub unsafe fn run(hwnd: HWND, sc: &IDXGISwapChain1, dc: &ID3D11DeviceContext, mcc: &Path) -> Choice {
    super::set_active(true);
    let mut menu = Menu::new(mcc);
    crate::preview::start(); // the 3D preview engine loads in the background (preview.rs), ready for CUSTOMIZE
    let mcc_for_test = mcc.to_path_buf();
    let steps = super::shot_steps();
    let relaunched = std::env::var_os("REACH_UI_RELAUNCHED").is_some();
    if relaunched {
        log!("menu: back from a game");
    }
    // Why the last game ended (e.g. the host left): shown on the server browser, where the player goes next.
    if let Some(n) = std::env::var("REACH_UI_NOTICE").ok().filter(|n| !n.is_empty()) {
        log!("menu: notice \"{n}\"");
        std::env::remove_var("REACH_UI_NOTICE");
        menu.open(Screen::Browser);
        menu.browser.error = n;
    }
    let mut step = 0usize;
    let mut step_frames = 0u32;
    let mut msg = MSG::default();
    let sc0: IDXGISwapChain = sc.cast().expect("swap chain");
    let choice = 'outer: loop {
        while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
            if msg.message == WM_QUIT {
                break 'outer Choice::Quit;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        crate::preview::menu_tick();
        let mut r = RECT::default();
        let _ = GetClientRect(hwnd, &mut r);
        let (cw, ch) = (r.right as u32, r.bottom as u32);
        if let Ok(d) = sc.GetDesc1() {
            if cw > 0 && ch > 0 && (d.Width, d.Height) != (cw, ch) {
                let _ = sc.ResizeBuffers(0, 0, 0, DXGI_FORMAT_UNKNOWN, DXGI_SWAP_CHAIN_FLAG(0));
            }
        }

        // Unattended check (REACH_UI_SHOT): open each listed screen, capture it, then quit or launch.
        let mut capture = None;
        if !steps.is_empty() {
            if relaunched {
                if step_frames == 30 {
                    capture = Some("ui-relaunched.bmp".to_string());
                }
            } else {
                while step < steps.len() && !matches!(steps[step].as_str(), "home" | "home-live" | "play-screen" | "play-map" | "play-ff" | "play-camp" | "host-ff" | "host-camp" | "host-map" | "host-server" | "browser" | "browser-fav" | "browser-cat" | "browser-sel" | "browser-direct" | "host" | "settings" | "play" | "play-ff-start" | "host-start" | "host-ff-start" | "host-camp-start" | "customize" | "forge" | "forge-saved" | "forge-start" | "forge-host")
                    && !steps[step].starts_with("settings-") && !steps[step].starts_with("customize-") && !steps[step].starts_with("campaign") {
                    step += 1;
                }
                match steps.get(step).map(|s| s.as_str()) {
                    None => {}
                    Some("play-ff-start") => {
                        menu.mp.select_game("hr_ff_standard_1set.bin");
                        if let Some(a) = menu.play_args() {
                            menu.launch(a);
                        }
                    }
                    Some(s @ ("host-start" | "host-ff-start" | "host-camp-start")) => {
                        // Press START HOSTING on the Host screen with the saved form (listen server); campaign co-op
                        // is HOST CO-OP on the CAMPAIGN screen.
                        let a = match s {
                            "host-camp-start" => menu.campaign_for_test().args().map(|a| menu.listen_args(2, a)),
                            "host-ff-start" => {
                                menu.mp.select_game("hr_ff_standard_1set.bin");
                                menu.host_args()
                            }
                            _ => menu.host_args(),
                        };
                        if let Some(a) = a {
                            menu.launch(a);
                        }
                    }
                    Some("campaign-host") => {
                        // HOST CO-OP on the CAMPAIGN screen (2 players, Host form's port / master settings).
                        let a = menu.campaign_for_test().args();
                        if let Some(a) = a {
                            let a = menu.listen_args(2, a);
                            menu.launch(a);
                        }
                    }
                    Some("campaign-start") => {
                        // PLAY SOLO on the CAMPAIGN screen (REACH_UI_CAMP picks mission / difficulty / skulls).
                        let c = menu.campaign_for_test();
                        if let Some(a) = c.args() {
                            menu.launch(a);
                        }
                    }
                    Some(s @ ("forge-start" | "forge-host")) => {
                        // START FORGE / HOST FORGE on the FORGE screen (first map, or REACH_UI_FORGE=<map stem>).
                        let f = menu.forge.get_or_insert_with(|| super::forge::ForgeScreen::new(&mcc_for_test));
                        f.tab = super::forge::Tab::Maps;
                        let mut a = f.forge_args().unwrap_or_default();
                        if let Ok(m) = std::env::var("REACH_UI_FORGE") {
                            a = vec!["--forge".into(), m];
                        }
                        if s == "forge-host" {
                            a = menu.listen_args(4, a);
                        }
                        menu.launch(a);
                    }
                    Some("play") => {
                        if let Some(a) = menu.play_args() {
                            menu.launch(a);
                        }
                    }
                    Some(s) => {
                        if step_frames == 0 {
                            // play-screen / host: GAME TYPE step, play-map / host-map / play-ff / host-ff: MAP step,
                            // host-server: SERVER step (REACH_UI_MP sets the picks and filters, play_screen.rs).
                            if matches!(s, "play-ff" | "host-ff") {
                                menu.mp.select_game("hr_ff_standard_1set.bin");
                            }
                            menu.mp.step = match s {
                                "play-map" | "host-map" | "play-ff" | "host-ff" => super::play_screen::Step::Map,
                                "host-server" => super::play_screen::Step::Server,
                                _ => menu.mp.step,
                            };
                            menu.open(match s {
                                "play-screen" | "play-map" | "play-ff" => Screen::Play,
                                "play-camp" | "host-camp" => Screen::Campaign,
                                s if s.starts_with("browser") => Screen::Browser,
                                "host" | "host-ff" | "host-map" | "host-server" => Screen::Host,
                                s if s.starts_with("customize") => Screen::Customize,
                                s if s.starts_with("settings") => Screen::Settings,
                                s if s.starts_with("campaign") => Screen::Campaign,
                                s if s.starts_with("forge") => Screen::Forge,
                                _ => Screen::Home,
                            });
                            if s.starts_with("campaign") {
                                // campaign, campaign-skulls (picker open), campaign-ff (Firefight tab)
                                let c = menu.campaign_for_test();
                                c.picker = s == "campaign-skulls";
                                if s == "campaign-ff" {
                                    c.tab = super::campaign::Tab::Firefight;
                                }
                            }
                            if s.starts_with("forge") {
                                let f = menu.forge.get_or_insert_with(|| super::forge::ForgeScreen::new(&mcc_for_test));
                                f.tab = if s == "forge-saved" { super::forge::Tab::Saved } else { super::forge::Tab::Maps };
                            }
                            if let Some(w) = s.strip_prefix("browser-") {
                                if w == "fav" {
                                    // REACH_UI_ADD=host:port[,host:port]: add by address, then show the Favorites tab.
                                    for a in std::env::var("REACH_UI_ADD").unwrap_or_default().split(',').filter(|a| !a.is_empty()) {
                                        if let Err(e) = menu.browser.add_favourite(a) {
                                            log!("menu: add favourite {a}: {e}");
                                        }
                                    }
                                }
                                menu.browser.test_hook(w);
                            }
                            if let Some(f) = s.strip_prefix("customize-") {
                                menu.customize.get_or_insert_with(super::customize::CustomizeScreen::new).open_field(f);
                            }
                            if let Some(cat) = s.strip_prefix("settings-") {
                                let ss = menu.settings_screen.get_or_insert_with(|| super::settings_screen::SettingsScreen::new(false));
                                ss.open_category(cat);
                            }
                        }
                        let ready = step_frames >= 30 && (menu.browser.is_idle() || step_frames > 600);
                        // CUSTOMIZE: wait for the 3D preview engine (preview.rs) to show the model, up to ~70 s.
                        let ready = ready && (!s.starts_with("customize") || crate::preview::settled() || step_frames > 20000);
                        // home-live: the live background, once it has faded in (main menu background, menu_scene.rs).
                        let ready = ready && (s != "home-live" || crate::preview::live_secs() > std::env::var("REACH_PREVIEW_SETTLE_SECS").ok().and_then(|v| v.parse().ok()).unwrap_or(3.0) || step_frames > 20000);
                        if ready {
                            capture = Some(format!("ui-{s}.bmp"));
                        }
                    }
                }
            }
        }

        match super::back_buffer_rtv(&sc0) {
            Ok((rtv, size)) => {
                dc.ClearRenderTargetView(&rtv, &[0.035, 0.043, 0.055, 1.0]);
                if let Err(e) = super::draw(dc, &rtv, size, false, |ui| menu.ui(ui)) {
                    log!("menu: draw failed: {e}");
                }
            }
            Err(e) => log!("menu: no back buffer: {e}"),
        }
        step_frames += 1;
        if let Some(name) = capture {
            let path = std::env::current_exe().unwrap_or_default().with_file_name(&name);
            match crate::host::capture_back_buffer(sc.as_raw(), &path) {
                Ok((w, h)) => log!("menu: back buffer {w}x{h} saved to {}", path.display()),
                Err(e) => log!("menu: capture failed: {e}"),
            }
            step += 1;
            step_frames = 0;
            if relaunched || step >= steps.len() {
                menu.choice.get_or_insert(Choice::Quit);
            }
        }
        let _ = sc.Present(1, DXGI_PRESENT(0));
        if let Some(c) = menu.choice.take() {
            break c;
        }
    };
    dc.ClearState();
    dc.Flush();
    super::set_active(false);
    choice
}
