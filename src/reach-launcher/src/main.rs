//! reach-launcher: the one thing friends download. Finds MCC, checks it is the build reach-host is pinned to,
//! installs/updates reach-host.exe from the update source (GitHub releases or reach-master), and starts it (Windows directly, Linux via umu + Proton).
//! Notes: notes/launcher.md.
#![cfg_attr(windows, windows_subsystem = "windows")]

mod check;
mod play;
mod steam;
mod update;

use std::path::PathBuf;
use std::process::Child;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, RichText, Stroke};
use serde::{Deserialize, Serialize};

// Colours copied from reach-host's ui/theme.rs so both look like one product.
const ACCENT: Color32 = Color32::from_rgb(242, 140, 40);
const BG: Color32 = Color32::from_rgb(9, 11, 14);
const PANEL: Color32 = Color32::from_rgb(16, 19, 24);
const TEXT: Color32 = Color32::from_rgb(228, 231, 235);
const DIM: Color32 = Color32::from_rgb(138, 145, 155);
const OK: Color32 = Color32::from_rgb(120, 200, 120);
const BAD: Color32 = Color32::from_rgb(235, 90, 80);

const NAME_MAX: usize = 15;

#[derive(Serialize, Deserialize, Default, Clone)]
#[serde(default)]
struct Config {
    mcc_dir: Option<PathBuf>,
    player_name: String,
    /// Proton folder name (label) chosen on Linux; None = automatic.
    proton: Option<String>,
}

/// Data root: `REACH_LAUNCHER_DATA`, else the OS's local data dir + `NobleOutreach`.
fn data_dir() -> PathBuf {
    std::env::var_os("REACH_LAUNCHER_DATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| dirs::data_local_dir().unwrap_or_else(|| PathBuf::from(".")).join("NobleOutreach"))
}

/// One-time move of the data root from before the rename (`ReachReclaimer`: settings, client, Proton prefix) to
/// `NobleOutreach`, so players keep their settings. Only when the new folder does not exist yet. A rename on the
/// same disk is instant; if it fails (a file in use on Windows), copy the small things (settings, client files)
/// and leave the prefix to be set up again.
fn migrate_data_dir() {
    if std::env::var_os("REACH_LAUNCHER_DATA").is_some() {
        return;
    }
    let Some(base) = dirs::data_local_dir() else { return };
    let (old, new) = (base.join("ReachReclaimer"), base.join("NobleOutreach"));
    if new.exists() || !old.is_dir() {
        return;
    }
    if std::fs::rename(&old, &new).is_ok() {
        return;
    }
    let _ = std::fs::create_dir_all(new.join("client"));
    let _ = std::fs::copy(old.join("launcher.json"), new.join("launcher.json"));
    if let Ok(rd) = std::fs::read_dir(old.join("client")) {
        for e in rd.flatten() {
            if e.file_type().map(|t| t.is_file()).unwrap_or(false) {
                let _ = std::fs::copy(e.path(), new.join("client").join(e.file_name()));
            }
        }
    }
}

fn config_path() -> PathBuf {
    data_dir().join("launcher.json")
}

fn load_config() -> Config {
    std::fs::read_to_string(config_path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn save_config(c: &Config) {
    let _ = std::fs::create_dir_all(data_dir());
    let _ = std::fs::write(config_path(), serde_json::to_string_pretty(c).unwrap());
}

#[derive(Clone, PartialEq)]
enum Job<T> {
    Idle,
    Running(String),
    Done(T),
    Failed(String),
}

type Shared<T> = Arc<Mutex<Job<T>>>;

fn run_job<T: Send + 'static>(ctx: &egui::Context, slot: &Shared<T>, f: impl FnOnce(&dyn Fn(String)) -> Result<T, String> + Send + 'static) {
    *slot.lock().unwrap() = Job::Running("starting".into());
    let slot = slot.clone();
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        let s2 = slot.clone();
        let c2 = ctx.clone();
        let progress = move |t: String| {
            *s2.lock().unwrap() = Job::Running(t);
            c2.request_repaint();
        };
        let r = f(&progress);
        *slot.lock().unwrap() = match r {
            Ok(v) => Job::Done(v),
            Err(e) => Job::Failed(e),
        };
        ctx.request_repaint();
    });
}

struct App {
    cfg: Config,
    mcc_text: String,
    check: Shared<check::Check>,
    /// (where from, version, files downloaded, master for reach-host or empty)
    sync: Shared<(String, String, usize, String)>,
    #[cfg(not(windows))]
    protons: Vec<(String, PathBuf)>,
    #[cfg(not(windows))]
    umu: Option<PathBuf>,
    /// "Launcher update failed: ..." from the last self-update attempt.
    launcher_msg: Arc<Mutex<String>>,
    child: Option<Child>,
    play_msg: String,
    last_tail: Instant,
    tail: Vec<String>,
    /// `REACH_LAUNCHER_AUTOPLAY=1`: press Play once as soon as it is enabled (unattended checks).
    autoplay: bool,
}

impl App {
    fn new(cc: &eframe::CreationContext) -> Self {
        style(&cc.egui_ctx);
        let mut cfg = load_config();
        if cfg.mcc_dir.as_ref().map_or(true, |d| !steam::looks_like_mcc(d)) {
            if let Some(found) = steam::find_mcc() {
                cfg.mcc_dir = Some(found);
            }
        }
        if cfg.player_name.is_empty() {
            cfg.player_name = std::env::var("USER").or_else(|_| std::env::var("USERNAME")).unwrap_or_else(|_| "Player".into());
            cfg.player_name.truncate(NAME_MAX);
        }
        save_config(&cfg);
        let mut app = App {
            mcc_text: cfg.mcc_dir.as_ref().map(|d| d.display().to_string()).unwrap_or_default(),
            cfg,
            check: Arc::new(Mutex::new(Job::Idle)),
            sync: Arc::new(Mutex::new(Job::Idle)),
            #[cfg(not(windows))]
            protons: play::protons(),
            #[cfg(not(windows))]
            umu: play::find_umu(),
            launcher_msg: Arc::new(Mutex::new(String::new())),
            child: None,
            play_msg: String::new(),
            last_tail: Instant::now(),
            tail: Vec::new(),
            autoplay: std::env::var_os("REACH_LAUNCHER_AUTOPLAY").is_some(),
        };
        app.start_check(&cc.egui_ctx);
        app.start_sync(&cc.egui_ctx);
        app
    }

    fn client_dir(&self) -> PathBuf {
        data_dir().join("client")
    }

    fn start_check(&mut self, ctx: &egui::Context) {
        match self.cfg.mcc_dir.clone() {
            Some(d) => run_job(ctx, &self.check, move |p| {
                p("checking haloreach.dll".into());
                Ok(check::check(&d))
            }),
            None => *self.check.lock().unwrap() = Job::Idle,
        }
    }

    fn start_sync(&mut self, ctx: &egui::Context) {
        let dir = self.client_dir();
        let launcher_msg = self.launcher_msg.clone();
        launcher_msg.lock().unwrap().clear();
        run_job(ctx, &self.sync, move |p| {
            p("checking for updates".into());
            let u = update::resolve()?;
            // Launcher first: a failure is shown but must not block the client update.
            if let Err(e) = update::self_update(&u.manifest, p) {
                *launcher_msg.lock().unwrap() = format!("Launcher update failed: {e}");
            }
            p("checking client files".into());
            let (v, n) = update::sync(&u.manifest, &dir, p)?;
            Ok((u.label, v, n, u.master))
        });
    }

    fn set_mcc(&mut self, ctx: &egui::Context, d: PathBuf) {
        self.mcc_text = d.display().to_string();
        self.cfg.mcc_dir = Some(d);
        save_config(&self.cfg);
        self.start_check(ctx);
    }

    #[cfg(not(windows))]
    fn proton_path(&self) -> Option<PathBuf> {
        let want = self.cfg.proton.as_ref()?;
        self.protons.iter().find(|(n, _)| n == want).map(|(_, p)| p.clone())
    }

    fn running(&mut self) -> bool {
        if let Some(c) = &mut self.child {
            match c.try_wait() {
                Ok(None) => return true,
                Ok(Some(st)) => {
                    self.play_msg = format!("Noble Outreach closed ({st})");
                    self.child = None;
                }
                Err(_) => self.child = None,
            }
        }
        false
    }

    fn play(&mut self) {
        // No update source reachable: play the installed client (the master setting in reach-host stays as it is).
        let master = match self.sync.lock().unwrap().clone() {
            Job::Done((_, _, _, m)) => m,
            Job::Failed(_) if update::local_version(&self.client_dir()).is_some() => String::new(),
            _ => return,
        };
        let Some(mcc) = self.cfg.mcc_dir.clone() else { return };
        let client = self.client_dir();
        let prefix = std::env::var_os("WINEPREFIX").map(PathBuf::from).unwrap_or_else(|| data_dir().join("prefix"));
        #[cfg(not(windows))]
        let proton = self.proton_path();
        #[cfg(windows)]
        let proton: Option<PathBuf> = None;
        let log = data_dir().join("launch.log");
        let r = play::spawn(&play::Launch {
            client: &client,
            mcc: &mcc,
            name: &self.cfg.player_name,
            master: &master,
            prefix: &prefix,
            proton: proton.as_deref(),
            log: &log,
        });
        match r {
            Ok(c) => {
                self.play_msg = if cfg!(windows) {
                    "Starting Noble Outreach...".into()
                } else {
                    "Starting Noble Outreach (the first start sets up Proton and can take a minute)...".into()
                };
                self.child = Some(c);
            }
            Err(e) => self.play_msg = e,
        }
    }
}

fn style(ctx: &egui::Context) {
    ctx.all_styles_mut(|s| {
        use egui::{FontFamily::Proportional, FontId, TextStyle::*};
        s.text_styles = [
            (Heading, FontId::new(28.0, Proportional)),
            (Body, FontId::new(15.0, Proportional)),
            (Button, FontId::new(15.0, Proportional)),
            (Small, FontId::new(12.0, Proportional)),
            (Monospace, FontId::new(12.0, egui::FontFamily::Monospace)),
        ]
        .into();
        s.spacing.item_spacing = egui::vec2(10.0, 8.0);
        s.spacing.interact_size.y = 28.0;
        let v = &mut s.visuals;
        *v = egui::Visuals::dark();
        v.override_text_color = Some(TEXT);
        v.panel_fill = BG;
        v.window_fill = PANEL;
        v.extreme_bg_color = Color32::from_rgb(5, 6, 8);
        v.selection.bg_fill = ACCENT.gamma_multiply(0.6);
        v.selection.stroke = Stroke::new(1.0, ACCENT);
        v.text_cursor.stroke = Stroke::new(2.0, ACCENT);
        for w in [&mut v.widgets.inactive, &mut v.widgets.noninteractive, &mut v.widgets.hovered, &mut v.widgets.active] {
            w.corner_radius = 0.0.into();
        }
        v.widgets.inactive.weak_bg_fill = Color32::from_rgb(28, 32, 38);
        v.widgets.inactive.bg_fill = Color32::from_rgb(28, 32, 38);
        v.widgets.hovered.weak_bg_fill = Color32::from_rgb(40, 44, 52);
        v.widgets.hovered.bg_stroke = Stroke::new(1.0, ACCENT);
        v.widgets.active.bg_fill = ACCENT;
        v.widgets.active.weak_bg_fill = ACCENT;
    });
}

fn section(ui: &mut egui::Ui, title: &str, body: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new().fill(PANEL).inner_margin(12.0).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new(title.to_uppercase()).color(ACCENT).size(13.0).strong());
        body(ui);
    });
    ui.add_space(6.0);
}

fn status(ui: &mut egui::Ui, color: Color32, text: impl Into<String>) {
    ui.label(RichText::new(text.into()).color(color));
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let running = self.running();
        if running || self.last_tail.elapsed() > Duration::from_secs(1) {
            self.last_tail = Instant::now();
            self.tail = play::tail(&self.client_dir().join("reach-host.log"), 6);
            ctx.request_repaint_after(Duration::from_secs(1));
        }
        egui::Frame::new().fill(BG).inner_margin(16.0).show(ui, |ui| {
            ui.set_min_size(ui.available_size());
            ui.horizontal(|ui| {
                ui.label(RichText::new("NOBLE").heading().color(TEXT).strong());
                ui.label(RichText::new("OUTREACH").heading().color(ACCENT));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(format!("launcher {}", update::VERSION)).small().color(DIM));
                });
            });
            ui.label(RichText::new("Halo: Reach community client. Uses your own Steam copy of the Master Chief Collection.").color(DIM));
            ui.add_space(8.0);

            // ---- MCC folder + build check
            let check = self.check.lock().unwrap().clone();
            section(ui, "Halo: MCC folder", |ui| {
                ui.horizontal(|ui| {
                    let w = ui.available_width() - 190.0;
                    let r = ui.add_sized([w.max(100.0), 28.0], egui::TextEdit::singleline(&mut self.mcc_text).hint_text("Halo The Master Chief Collection folder"));
                    if r.lost_focus() && !self.mcc_text.trim().is_empty() {
                        let d = PathBuf::from(self.mcc_text.trim());
                        if Some(&d) != self.cfg.mcc_dir.as_ref() {
                            self.set_mcc(&ctx, d);
                        }
                    }
                    if ui.button("Browse...").clicked() {
                        if let Some(d) = rfd::FileDialog::new().set_title("Halo The Master Chief Collection folder").pick_folder() {
                            self.set_mcc(&ctx, d);
                        }
                    }
                    if ui.button("Find").on_hover_text("Look in your Steam libraries again").clicked() {
                        match steam::find_mcc() {
                            Some(d) => self.set_mcc(&ctx, d),
                            None => *self.check.lock().unwrap() = Job::Failed("MCC not found in any Steam library. Use Browse...".into()),
                        }
                    }
                });
                match &check {
                    Job::Idle => status(ui, DIM, "Pick the folder with haloreach inside (Steam: right-click MCC > Manage > Browse local files)."),
                    Job::Running(t) => status(ui, DIM, format!("{t}...")),
                    Job::Done(check::Check::Ok { build }) => status(ui, OK, format!("MCC build {build}: supported")),
                    Job::Done(check::Check::Missing(m)) => status(ui, BAD, format!("Not an MCC folder with Halo: Reach installed (missing {}).", m.join(", "))),
                    Job::Done(check::Check::WrongBuild { build, .. }) => status(
                        ui,
                        BAD,
                        format!("Your MCC build is {build}; Noble Outreach needs build {}. Ask your server host for the matching client before playing.", check::FROZEN_BUILD),
                    ),
                    Job::Done(check::Check::Error(e)) | Job::Failed(e) => status(ui, BAD, e.clone()),
                }
            });

            // ---- Client
            let sync = self.sync.lock().unwrap().clone();
            section(ui, "Client", |ui| {
                ui.horizontal(|ui| {
                    match &sync {
                        Job::Idle => status(ui, DIM, "not checked"),
                        Job::Running(t) => {
                            ui.spinner();
                            status(ui, DIM, t.clone());
                        }
                        Job::Done((m, v, n, _)) => {
                            ui.vertical(|ui| {
                                status(ui, OK, format!("Up to date, version {v}{}", if *n > 0 { format!(", {n} file(s) downloaded") } else { String::new() }));
                                ui.label(RichText::new(m).small().color(DIM));
                            });
                        }
                        Job::Failed(e) => {
                            let local = update::local_version(&self.client_dir());
                            ui.vertical(|ui| {
                                ui.set_max_width((ui.available_width() - 150.0).max(200.0)); // room for the button
                                status(ui, BAD, match local {
                                    Some(v) => format!("No update source reachable. Installed version {v} kept."),
                                    None => "No update source reachable: the client can't be installed yet.".to_string(),
                                });
                                ui.add(egui::Label::new(RichText::new(e).small().color(DIM)).wrap());
                            });
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add_enabled(!matches!(sync, Job::Running(_)) && !running, egui::Button::new("Check for updates")).clicked() {
                            self.start_sync(&ctx);
                        }
                    });
                });
            });

            let lm = self.launcher_msg.lock().unwrap().clone();
            if !lm.is_empty() {
                status(ui, BAD, lm);
            }

            // ---- Settings
            section(ui, "Settings", |ui| {
                egui::Grid::new("settings").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                    ui.label("Player name");
                    let r = ui.add(egui::TextEdit::singleline(&mut self.cfg.player_name).char_limit(NAME_MAX).desired_width(220.0));
                    if r.lost_focus() {
                        save_config(&self.cfg);
                    }
                    ui.end_row();
                    #[cfg(not(windows))]
                    {
                        ui.label("Proton");
                        let cur = self.cfg.proton.clone().unwrap_or_else(|| "Automatic (latest GE-Proton via umu)".into());
                        let mut changed = false;
                        egui::ComboBox::from_id_salt("proton").selected_text(cur).width(320.0).show_ui(ui, |ui| {
                            changed |= ui.selectable_value(&mut self.cfg.proton, None, "Automatic (latest GE-Proton via umu)").changed();
                            for (n, _) in &self.protons {
                                changed |= ui.selectable_value(&mut self.cfg.proton, Some(n.clone()), n).changed();
                            }
                        });
                        if changed {
                            save_config(&self.cfg);
                        }
                        ui.end_row();
                        if self.umu.is_none() {
                            ui.label("");
                            ui.label(
                                RichText::new(
                                    "umu-run is missing. Install umu-launcher from your distro (Fedora/Nobara: dnf install umu-launcher; \
                                     Arch: pacman -S umu-launcher; others: github.com/Open-Wine-Components/umu-launcher releases), \
                                     then restart the launcher.",
                                )
                                .color(BAD),
                            );
                            ui.end_row();
                        }
                    }
                });
            });

            // ---- Play
            let can_play = matches!(check, Job::Done(check::Check::Ok { .. }))
                && (matches!(sync, Job::Done(_)) || (matches!(sync, Job::Failed(_)) && update::local_version(&self.client_dir()).is_some()))
                && !running
                && !self.cfg.player_name.trim().is_empty();
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let label = if running { "RUNNING" } else { "PLAY" };
                let b = egui::Button::new(RichText::new(label).size(22.0).strong().color(if can_play { BG } else { DIM }))
                    .fill(if can_play { ACCENT } else { PANEL })
                    .min_size(egui::vec2(180.0, 46.0));
                if ui.add_enabled(can_play, b).clicked() || (can_play && std::mem::take(&mut self.autoplay)) {
                    save_config(&self.cfg);
                    self.play();
                }
                ui.vertical(|ui| {
                    if !self.play_msg.is_empty() {
                        ui.label(self.play_msg.clone());
                    }
                    if ui.link("Open logs folder").clicked() {
                        let _ = std::fs::create_dir_all(self.client_dir());
                        play::open_folder(&self.client_dir());
                    }
                });
            });
            if !self.tail.is_empty() {
                ui.add_space(6.0);
                egui::Frame::new().fill(Color32::from_rgb(5, 6, 8)).inner_margin(8.0).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    for l in &self.tail {
                        ui.add(egui::Label::new(RichText::new(l).monospace().color(DIM)).truncate());
                    }
                });
            }
        });
    }
}

fn main() -> eframe::Result {
    update::cleanup_old();
    migrate_data_dir();
    // The Windows build under Wine/Proton: wgpu's default (D3D12 via vkd3d) draws a white window there, Vulkan works.
    // Wine passes the Unix environment through, so WINE* variables mean we are not on real Windows.
    if cfg!(windows) && std::env::var_os("WGPU_BACKEND").is_none() && std::env::vars_os().any(|(k, _)| k.to_string_lossy().starts_with("WINE")) {
        std::env::set_var("WGPU_BACKEND", "vulkan");
    }
    let opts = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Noble Outreach")
            .with_icon(egui::IconData { rgba: include_bytes!("../assets/icon-64.rgba").to_vec(), width: 64, height: 64 })
            .with_inner_size([820.0, 600.0])
            .with_min_inner_size([640.0, 480.0]),
        ..Default::default()
    };
    eframe::run_native("Noble Outreach", opts, Box::new(|cc| Ok(Box::new(App::new(cc)))))
}
