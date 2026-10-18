//! The PLAY and HOST GAME screens (multiplayer and Firefight), laid out like the CAMPAIGN screen: a list with art on
//! the left, a detail panel with a banner on the right, tabs along the top and big buttons at the bottom.
//! Step 1 GAME TYPE: category tabs, type-to-filter search, FFA / team and matchmaking chips over every game type in
//! the player's files (catalog.rs). Step 2 MAP: the built-in maps, then the chosen map's variants (its default
//! first, Bungie's remixes, matchmaking maps, the player's saved Forge maps); Firefight picks a map and difficulty.
//! Step 3 SERVER (HOST only): server name, players, port, master. Keyboard: type to search, Up/Down to pick,
//! Left/Right for tabs (map step: maps / variants), Enter for the next step or start, Esc to clear or go back.
//! Launch flags: `--map M --game-variant V [--map-variant F]`, Firefight `--firefight M --difficulty D
//! --game-variant V`; HOST hands them to the menu's listen-server path.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use egui::{Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, Vec2};

use super::mccart;
use super::settings::Settings;
use super::theme::{self, ACCENT, DIM, HOVER, LINE, PANEL, TEXT};
use super::widgets::{big_ghost, chip, cover, full_uv, placeholder, search_box, tab_button};
use crate::catalog::{self, BaseMap, Catalog, Category, GameType, Source};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Step {
    Game,
    Map,
    Server,
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum Teams {
    Any,
    Ffa,
    Team,
}

pub struct PlayScreen {
    mcc: PathBuf,
    /// HOST GAME (adds the SERVER step) rather than PLAY.
    pub host: bool,
    pub step: Step,
    cat: Option<Category>,
    teams: Teams,
    /// Include `hopper_game_variants` (matchmaking playlists' game types).
    hopper: bool,
    /// Include community game types (the custom folder and Steam Workshop).
    community: bool,
    query: String,
    map_query: String,
    cat_data: Arc<Catalog>,
    /// Catalog maps plus saved Forge maps.
    maps: Vec<BaseMap>,
    /// Index into `cat_data.games`.
    gt: usize,
    map: usize,
    variant: usize,
    ff_map: usize,
    difficulty: usize,
    /// Players for a hosted Firefight game (multiplayer uses the Host form's max players).
    pub coop_players: u32,
    /// Map step: Up/Down move in the variant list (else the map list).
    focus_variants: bool,
    /// Scroll the selected row into view (after a keyboard move).
    scroll_to: bool,
}

/// Texture-pack key of a map: its English name in capitals without spaces (`Forge World` → `FORGEWORLD`).
fn art_key(name: &str) -> String {
    name.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_uppercase()
}

impl PlayScreen {
    pub fn new(mcc: &Path, s: &Settings) -> Self {
        mccart::init(mcc);
        let cat_data = catalog::get(mcc);
        let maps = catalog::with_saved(&cat_data);
        let ff_maps = super::menu::FF_MAPS;
        let mut p = PlayScreen {
            mcc: mcc.to_path_buf(),
            host: false,
            step: Step::Game,
            cat: None,
            teams: Teams::Any,
            hopper: true,
            community: true,
            query: String::new(),
            map_query: String::new(),
            gt: 0,
            map: 0,
            variant: 0,
            ff_map: ff_maps.iter().position(|m| mcc.join("haloreach").join("maps").join(format!("{}.map", m.0)).is_file()).unwrap_or(0),
            difficulty: 1,
            coop_players: 2,
            focus_variants: true,
            scroll_to: true,
            cat_data,
            maps,
        };
        p.select_game(&s.last_variant);
        p.select_map(&s.last_map, &s.last_map_variant);
        p
    }

    /// Pick a game type by file name (no-op if unknown).
    pub fn select_game(&mut self, file: &str) {
        if let Some(i) = self.cat_data.games.iter().position(|g| g.file.eq_ignore_ascii_case(file)) {
            self.gt = i;
        }
    }

    /// Pick a map (stem) and one of its variants by file name or path (empty = default). Firefight stems pick the
    /// Firefight map.
    pub fn select_map(&mut self, stem: &str, variant: &str) {
        if let Some(i) = super::menu::FF_MAPS.iter().position(|m| m.0 == stem) {
            self.ff_map = i;
            return;
        }
        let stem = if stem == "forge_world" { "forge_halo" } else { stem };
        if let Some(i) = self.maps.iter().position(|m| m.stem == stem) {
            self.map = i;
            self.variant = 0;
            if !variant.is_empty() {
                let want = Path::new(variant).file_name().map(|f| f.to_string_lossy().to_lowercase()).unwrap_or_default();
                if let Some(v) = self.maps[i].variants.iter().position(|v| v.path.as_ref().and_then(|p| p.file_name()).is_some_and(|f| f.to_string_lossy().to_lowercase() == want)) {
                    self.variant = v;
                }
            }
        }
    }

    /// Test hook (REACH_UI_MP): `cat=objective q=flag teams=ffa hopper=0 gv=<file> map=<stem> mv=<file> step=map`.
    pub fn apply_test(&mut self, spec: &str) {
        for kv in spec.split_whitespace() {
            let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
            match k {
                "cat" => self.cat = Category::ALL.iter().copied().find(|c| c.name().eq_ignore_ascii_case(v)),
                "q" => self.query = v.replace('+', " "),
                "mq" => self.map_query = v.replace('+', " "),
                "teams" => self.teams = match v { "ffa" => Teams::Ffa, "team" => Teams::Team, _ => Teams::Any },
                "hopper" => self.hopper = v != "0",
                "community" => self.community = v != "0",
                "gv" => self.select_game(v),
                "map" => self.select_map(v, ""),
                "mv" => {
                    let stem = self.maps.get(self.map).map(|m| m.stem.clone()).unwrap_or_default();
                    self.select_map(&stem, v);
                }
                "step" => self.step = match v { "map" => Step::Map, "server" => Step::Server, _ => Step::Game },
                _ => {}
            }
            if matches!(k, "cat" | "q" | "teams" | "hopper" | "community") {
                self.pick_first_visible();
            }
        }
    }

    pub fn game(&self) -> Option<&GameType> {
        self.cat_data.games.get(self.gt)
    }

    fn firefight(&self) -> bool {
        self.game().is_some_and(|g| g.kind.category == Category::Firefight)
    }

    /// Mode flags for the current pick.
    pub fn args(&self) -> Option<Vec<String>> {
        let g = self.game()?;
        let gv = g.path.to_string_lossy().into_owned();
        if self.firefight() {
            let m = super::menu::FF_MAPS.get(self.ff_map)?;
            return Some(vec!["--firefight".into(), m.0.into(), "--difficulty".into(), super::menu::DIFFICULTIES[self.difficulty].0.into(), "--game-variant".into(), gv]);
        }
        let m = self.maps.get(self.map)?;
        let mut a = vec!["--map".into(), m.stem.clone(), "--game-variant".into(), gv];
        if let Some(p) = m.variants.get(self.variant).and_then(|v| v.path.as_ref()) {
            a.extend(["--map-variant".into(), p.to_string_lossy().into_owned()]);
        }
        Some(a)
    }

    /// Players for HOST: the Host form's max players, or the Firefight co-op count.
    pub fn host_players(&self, s: &Settings) -> u32 {
        if self.firefight() { self.coop_players } else { s.host_max_players }
    }

    /// One line: "Slayer Pro on Forge World · Pinnacle".
    pub fn summary(&self) -> String {
        let Some(g) = self.game() else { return String::new() };
        if self.firefight() {
            let m = super::menu::FF_MAPS.get(self.ff_map).map_or("?", |m| m.1);
            return format!("{} on {m} · {}", g.title, super::menu::DIFFICULTIES[self.difficulty].1);
        }
        match self.maps.get(self.map) {
            Some(m) if self.variant > 0 => format!("{} on {} · {}", g.title, m.name, m.variants[self.variant].title),
            Some(m) => format!("{} on {}", g.title, m.name),
            None => g.title.clone(),
        }
    }

    /// Write the picks into `s` (the menu saves it).
    pub fn remember(&self, s: &mut Settings) {
        if let Some(g) = self.game() {
            s.last_variant = g.file.clone();
        }
        if self.firefight() {
            return;
        }
        if let Some(m) = self.maps.get(self.map) {
            s.last_map = m.stem.clone();
            s.last_map_variant = m.variants.get(self.variant).and_then(|v| v.path.as_ref()).map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
        }
    }

    // ---- Filtering -------------------------------------------------------------------------------------------

    /// Matchmaking and Community chips.
    fn shown(&self, g: &GameType) -> bool {
        (self.hopper || !g.hopper) && (self.community || !g.community())
    }

    fn visible_games(&self) -> Vec<usize> {
        let words: Vec<String> = self.query.to_lowercase().split_whitespace().map(String::from).collect();
        self.cat_data
            .games
            .iter()
            .enumerate()
            .filter(|(_, g)| self.cat.is_none_or(|c| g.kind.category == c))
            .filter(|(_, g)| match self.teams {
                Teams::Any => true,
                Teams::Ffa => g.teams == Some(false),
                Teams::Team => g.teams == Some(true),
            })
            .filter(|(_, g)| self.shown(g))
            .filter(|(_, g)| {
                if words.is_empty() {
                    return true;
                }
                let hay = format!("{} {} {} {} {} {}", g.title, g.kind.name, g.kind.category.name(), g.file, teams_label(g.teams), g.edition()).to_lowercase();
                words.iter().all(|w| hay.contains(w.as_str()))
            })
            .map(|(i, _)| i)
            .collect::<Vec<_>>()
            .into_iter()
            .fold(Vec::<usize>::new(), |mut out, i| {
                // One row per mode: the selected version stands for its group, else the first one listed.
                let g = &self.cat_data.games[i];
                match out.iter().position(|&j| self.cat_data.games[j].same_mode(g)) {
                    Some(k) if i == self.gt => out[k] = i,
                    Some(_) => {}
                    None => out.push(i),
                }
                out
            })
    }

    /// Every shown version of the mode at `i` (custom games first, then matchmaking, community), `i` included.
    fn versions(&self, i: usize) -> Vec<usize> {
        let Some(g) = self.cat_data.games.get(i) else { return Vec::new() };
        self.cat_data.games.iter().enumerate().filter(|(_, o)| self.shown(o) && o.same_mode(g)).map(|(j, _)| j).collect()
    }

    /// Map step lists: (maps shown, variants of the selected map shown). With a search, a map shows if its name or
    /// one of its variants matches.
    fn visible_maps(&self) -> (Vec<usize>, Vec<usize>) {
        if self.firefight() {
            let ff: Vec<usize> = (0..super::menu::FF_MAPS.len())
                .filter(|&i| self.mcc.join("haloreach").join("maps").join(format!("{}.map", super::menu::FF_MAPS[i].0)).is_file())
                .filter(|&i| self.map_query.is_empty() || super::menu::FF_MAPS[i].1.to_lowercase().contains(&self.map_query.to_lowercase()))
                .collect();
            return (ff, Vec::new());
        }
        let q = self.map_query.to_lowercase();
        let words: Vec<&str> = q.split_whitespace().collect();
        let hit = |t: &str| {
            let t = t.to_lowercase();
            words.iter().all(|w| t.contains(w))
        };
        let maps: Vec<usize> = (0..self.maps.len()).filter(|&i| words.is_empty() || hit(&self.maps[i].name) || self.maps[i].variants.iter().any(|v| hit(&v.title))).collect();
        let vars = match self.maps.get(self.map) {
            Some(m) if words.is_empty() || hit(&m.name) => (0..m.variants.len()).collect(),
            Some(m) => (0..m.variants.len()).filter(|&v| v == 0 || hit(&m.variants[v].title)).collect(),
            None => Vec::new(),
        };
        (maps, vars)
    }

    // ---- Frame -----------------------------------------------------------------------------------------------

    pub fn ui(&mut self, ui: &mut Ui, s: &mut Settings) -> Action {
        let full = ui.max_rect();
        let mut act = self.keys(ui, s);
        let m = 32.0;
        let top = Rect::from_min_max(full.min + Vec2::new(m, 22.0), Pos2::new(full.right() - m, full.top() + 74.0));
        // Header: title, steps, BACK.
        ui.scope_builder(egui::UiBuilder::new().max_rect(top), |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(if self.host { "HOST GAME" } else { "PLAY" }).size(30.0).color(TEXT));
                ui.add_space(20.0);
                let steps: &[(&str, Step)] = if self.host { &[("1  Game type", Step::Game), ("2  Map", Step::Map), ("3  Server", Step::Server)] } else { &[("1  Game type", Step::Game), ("2  Map", Step::Map)] };
                for &(label, st) in steps {
                    if tab_button(ui, label, self.step == st).clicked() {
                        self.step = st;
                        self.scroll_to = true;
                    }
                }
            });
        });
        ui.painter().line_segment([Pos2::new(top.left(), top.top() + 42.0), Pos2::new(top.left() + 36.0, top.top() + 42.0)], Stroke::new(2.0, ACCENT));
        let mut back = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_size(Pos2::new(top.right() - 76.0, top.top() + 2.0), Vec2::new(76.0, 36.0))));
        if theme::ghost_button(&mut back, "Back").clicked() {
            act = Action::Back;
        }

        let body = Rect::from_min_max(Pos2::new(full.left() + m, top.bottom() + 8.0), full.max - Vec2::new(m, 24.0));
        let list_w = (body.width() * 0.40).clamp(320.0, 520.0).min(body.width() * 0.5);
        let left = Rect::from_min_size(body.min, Vec2::new(list_w, body.height()));
        let right = Rect::from_min_max(Pos2::new(left.right() + 22.0, body.top()), body.max);
        let a = match self.step {
            Step::Game => {
                self.game_list(ui, left);
                self.game_detail(ui, right, s)
            }
            Step::Map => {
                self.map_list(ui, left);
                self.map_detail(ui, right, s)
            }
            Step::Server => self.server(ui, left, right, s),
        };
        if let Some(a) = a {
            act = a;
        }
        self.scroll_to = false;
        act
    }

    fn keys(&mut self, ui: &mut Ui, s: &Settings) -> Action {
        let typing = ui.ctx().memory(|m| m.focused().is_some());
        let (up, down, pgup, pgdn, l, r, enter, esc, tab) = ui.input(|i| {
            use egui::Key::*;
            (i.key_pressed(ArrowUp), i.key_pressed(ArrowDown), i.key_pressed(PageUp), i.key_pressed(PageDown), i.key_pressed(ArrowLeft), i.key_pressed(ArrowRight), i.key_pressed(Enter), i.key_pressed(Escape), i.key_pressed(Tab))
        });
        if typing {
            return Action::None; // server name field
        }
        let step_move: i32 = if up { -1 } else if down { 1 } else if pgup { -8 } else if pgdn { 8 } else { 0 };
        match self.step {
            Step::Game => {
                if step_move != 0 {
                    let v = self.visible_games();
                    if !v.is_empty() {
                        let cur = v.iter().position(|&i| i == self.gt);
                        let n = match cur {
                            Some(c) => (c as i32 + step_move).clamp(0, v.len() as i32 - 1) as usize,
                            None => 0,
                        };
                        self.gt = v[n];
                        self.scroll_to = true;
                    }
                }
                if l || r || tab {
                    let tabs: Vec<Option<Category>> = std::iter::once(None).chain(Category::ALL.iter().copied().map(Some)).collect();
                    let i = tabs.iter().position(|&c| c == self.cat).unwrap_or(0) as i32;
                    let d = if l { -1 } else { 1 };
                    self.cat = tabs[(i + d).rem_euclid(tabs.len() as i32) as usize];
                    self.pick_first_visible();
                }
                if esc {
                    if !self.query.is_empty() {
                        self.query.clear();
                    } else {
                        return Action::Back;
                    }
                }
                if enter && self.game().is_some() {
                    self.step = Step::Map;
                    self.scroll_to = true;
                }
            }
            Step::Map => {
                let (maps, vars) = self.visible_maps();
                if l || r {
                    self.focus_variants = r && !vars.is_empty();
                }
                if step_move != 0 {
                    let ff = self.firefight();
                    if self.focus_variants && !ff && !vars.is_empty() {
                        let c = vars.iter().position(|&i| i == self.variant).map_or(0, |c| (c as i32 + step_move).clamp(0, vars.len() as i32 - 1) as usize);
                        self.variant = vars[c];
                    } else if !maps.is_empty() {
                        let sel = if ff { self.ff_map } else { self.map };
                        let c = maps.iter().position(|&i| i == sel).map_or(0, |c| (c as i32 + step_move).clamp(0, maps.len() as i32 - 1) as usize);
                        if ff {
                            self.ff_map = maps[c];
                        } else if self.map != maps[c] {
                            self.map = maps[c];
                            self.variant = 0;
                        }
                    }
                    self.scroll_to = true;
                }
                if esc {
                    if !self.map_query.is_empty() {
                        self.map_query.clear();
                    } else {
                        self.step = Step::Game;
                        self.scroll_to = true;
                    }
                }
                if enter {
                    if self.host {
                        self.step = Step::Server;
                    } else if let Some(a) = self.args() {
                        return Action::Play(a);
                    }
                }
            }
            Step::Server => {
                if esc {
                    self.step = Step::Map;
                }
                if enter {
                    if let Some(a) = self.args() {
                        return Action::Host { players: self.host_players(s), args: a };
                    }
                }
            }
        }
        Action::None
    }

    /// After a filter change: keep the pick if it is still listed, else take the first listed one.
    fn pick_first_visible(&mut self) {
        let v = self.visible_games();
        if !v.contains(&self.gt) {
            if let Some(&f) = v.first() {
                self.gt = f;
            }
        }
        self.scroll_to = true;
    }

    // ---- Step 1: game type -----------------------------------------------------------------------------------

    fn game_list(&mut self, ui: &mut Ui, rect: Rect) {
        let p = ui.painter().clone();
        p.rect_filled(rect, 2.0, PANEL);
        p.rect_stroke(rect, 2.0, Stroke::new(1.0, LINE), StrokeKind::Inside);
        let inner = rect.shrink(14.0);
        // Category tabs (two rows of chips: they must fit the list width).
        let mut y = inner.top();
        let tabs: Vec<(String, Option<Category>)> = std::iter::once(("All".to_string(), None)).chain(Category::ALL.iter().map(|c| (c.name().to_string(), Some(*c)))).collect();
        let mut row = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_max(Pos2::new(inner.left(), y), Pos2::new(inner.right(), y + 64.0))).layout(egui::Layout::left_to_right(egui::Align::Min).with_main_wrap(true)));
        row.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
        let mut changed = false;
        for (label, c) in &tabs {
            let n = self.cat_data.games.iter().filter(|g| c.is_none_or(|c| g.kind.category == c) && self.shown(g)).count();
            if chip(&mut row, &format!("{label} {n}"), self.cat == *c).clicked() {
                self.cat = *c;
                changed = true;
            }
        }
        y = row.min_rect().bottom() + 10.0;
        let sr = Rect::from_min_size(Pos2::new(inner.left(), y), Vec2::new(inner.width(), 34.0));
        if search_box(ui, sr, &mut self.query, "Type to search game types", true) {
            changed = true;
        }
        y = sr.bottom() + 8.0;
        let mut row = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_max(Pos2::new(inner.left(), y), Pos2::new(inner.right(), y + 64.0))).layout(egui::Layout::left_to_right(egui::Align::Min).with_main_wrap(true)));
        row.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
        for (label, t) in [("Any", Teams::Any), ("FFA", Teams::Ffa), ("Teams", Teams::Team)] {
            if chip(&mut row, label, self.teams == t).clicked() {
                self.teams = t;
                changed = true;
            }
        }
        row.add_space(6.0);
        if chip(&mut row, "Matchmaking", self.hopper).on_hover_text("Also list the game types of MCC's matchmaking playlists").clicked() {
            self.hopper = !self.hopper;
            changed = true;
        }
        if chip(&mut row, "Community", self.community).on_hover_text("Also list game types from your custom folder and the Steam Workshop").clicked() {
            self.community = !self.community;
            changed = true;
        }
        if changed {
            self.pick_first_visible();
        }
        y = row.min_rect().bottom() + 8.0;
        let vis = self.visible_games();
        p.text(Pos2::new(inner.left(), y), Align2::LEFT_TOP, format!("{} GAME TYPES", vis.len()), FontId::proportional(11.5), DIM);
        let area = Rect::from_min_max(Pos2::new(inner.left(), y + 18.0), inner.max);
        if vis.is_empty() {
            p.text(area.left_top() + Vec2::new(0.0, 8.0), Align2::LEFT_TOP, "Nothing matches. Esc clears the search.", FontId::proportional(15.0), DIM);
            return;
        }
        let ctx = ui.ctx().clone();
        let mut double = false;
        ui.scope_builder(egui::UiBuilder::new().max_rect(area), |ui| {
            egui::ScrollArea::vertical().id_salt("mp-games").max_height(area.height()).auto_shrink([false, false]).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                let mut last_group = 0;
                for &i in &vis {
                    let g = &self.cat_data.games[i];
                    let group = if g.community() { 2 } else if g.hopper { 1 } else { 0 };
                    if group != last_group {
                        let (r, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 26.0), Sense::hover());
                        let head = if group == 2 { "COMMUNITY GAME TYPES" } else { "FROM MATCHMAKING PLAYLISTS" };
                        ui.painter().text(Pos2::new(r.left() + 4.0, r.bottom() - 4.0), Align2::LEFT_BOTTOM, head, FontId::proportional(11.0), ACCENT.gamma_multiply(0.8));
                    }
                    last_group = group;
                    let selected = i == self.gt;
                    let (r, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 46.0), Sense::click());
                    let p = ui.painter();
                    if selected {
                        p.rect_filled(r, 0.0, ACCENT.gamma_multiply(0.20));
                        p.rect_filled(Rect::from_min_size(r.min, Vec2::new(3.0, r.height())), 0.0, ACCENT);
                    } else if resp.hovered() {
                        p.rect_filled(r, 0.0, HOVER);
                    }
                    let ir = Rect::from_min_size(r.min + Vec2::new(10.0, 6.0), Vec2::splat(34.0));
                    match mccart::texture(&ctx, g.kind.icon) {
                        Some(t) => {
                            p.image(t.id(), ir, full_uv(&t), if selected { Color32::WHITE } else { Color32::from_gray(215) });
                        }
                        None => placeholder(p, ir, ""),
                    }
                    let x = ir.right() + 12.0;
                    let w = r.right() - x - 8.0;
                    line(p, Pos2::new(x, r.center().y + 1.0), Align2::LEFT_BOTTOM, &g.title, 16.5, if selected { Color32::WHITE } else { TEXT }, w);
                    let n = self.versions(i).len();
                    let sub = if n > 1 { format!("{} · {} · {n} versions", g.kind.name, teams_label(g.teams)) } else { format!("{} · {}", g.kind.name, teams_label(g.teams)) }.to_uppercase();
                    line(p, Pos2::new(x, r.center().y + 3.0), Align2::LEFT_TOP, &sub, 11.0, if selected { ACCENT } else { DIM }, w);
                    if selected && self.scroll_to {
                        resp.scroll_to_me(Some(egui::Align::Center));
                    }
                    if resp.clicked() {
                        self.gt = i;
                    }
                    if resp.double_clicked() {
                        double = true;
                    }
                }
            });
        });
        if double {
            self.step = Step::Map;
            self.scroll_to = true;
        }
    }

    fn game_detail(&mut self, ui: &mut Ui, rect: Rect, s: &Settings) -> Option<Action> {
        let ctx = ui.ctx().clone();
        let p = ui.painter().clone();
        let g = self.game()?.clone();
        let btn_top = rect.bottom() - 44.0;
        let card_h = 64.0;
        let card = Rect::from_min_size(Pos2::new(rect.left(), btn_top - 18.0 - card_h), Vec2::new(rect.width(), card_h));
        // Banner: as tall as the room above the description (at least 3 lines) and the map card allows.
        let bh = (rect.width() / (316.0 / 160.0)).min(card.top() - 22.0 - 96.0 - rect.top()).max(80.0);
        let br = Rect::from_min_size(rect.min, Vec2::new(rect.width(), bh));
        banner(&ctx, &p, br, g.kind.banner, g.kind.icon);
        let tx = br.left() + 22.0;
        let name_r = line(&p, Pos2::new(tx, br.bottom() - 14.0), Align2::LEFT_BOTTOM, &g.title.to_uppercase(), 32.0, Color32::WHITE, br.width() - 44.0);
        let kind = if g.kind.name == g.kind.category.name() { g.kind.name.to_string() } else { format!("{}  ·  {}", g.kind.category.name(), g.kind.name) };
        p.text(Pos2::new(tx, name_r.top()), Align2::LEFT_BOTTOM, kind.to_uppercase(), FontId::proportional(13.5), ACCENT);

        let mut y = br.bottom() + 12.0;
        let src = match g.source {
            Source::Custom => "Community (custom folder)",
            Source::Workshop => "Community (Steam Workshop)",
            _ if g.hopper => "Matchmaking playlist",
            _ => "Custom games",
        };
        let facts = format!("{}   ·   {}   ·   {}", teams_long(g.teams), src, g.file);
        line(&p, Pos2::new(rect.left(), y), Align2::LEFT_TOP, &facts, 12.5, DIM, rect.width());
        y += 22.0;
        // Several files of this mode: one chip per version, labelled by release and the settings in its file name.
        let versions = self.versions(self.gt);
        if versions.len() > 1 {
            let vr = Rect::from_min_max(Pos2::new(rect.left(), y), Pos2::new(rect.right(), y + 58.0));
            let mut row = ui.new_child(egui::UiBuilder::new().max_rect(vr).layout(egui::Layout::left_to_right(egui::Align::Min).with_main_wrap(true)));
            row.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
            for &j in &versions {
                let v = &self.cat_data.games[j];
                if chip(&mut row, &v.edition(), j == self.gt).on_hover_text(&v.file).clicked() {
                    self.gt = j;
                }
            }
            y = row.min_rect().bottom() + 10.0;
        }
        let desc_h = card.top() - 22.0 - y;
        lines(&p, Rect::from_min_size(Pos2::new(rect.left(), y), Vec2::new(rect.width(), desc_h)), &g.desc, 15.0, TEXT);

        // The map this game type will be played on; click to change.
        p.text(Pos2::new(card.left(), card.top() - 4.0), Align2::LEFT_BOTTOM, "MAP", FontId::proportional(12.0), DIM);
        let resp = ui.interact(card, ui.id().with("map-card"), Sense::click());
        p.rect_filled(card, 2.0, if resp.hovered() { HOVER } else { PANEL });
        p.rect_stroke(card, 2.0, Stroke::new(1.0, if resp.hovered() { ACCENT } else { LINE }), StrokeKind::Inside);
        let (map_name, var_name, thumb) = self.map_pick();
        let tr = Rect::from_min_size(card.min + Vec2::new(8.0, 8.0), Vec2::new((card_h - 16.0) * 16.0 / 9.0, card_h - 16.0));
        match mccart::texture(&ctx, &thumb) {
            Some(t) => cover(&p, &t, tr, Color32::WHITE),
            None => placeholder(&p, tr, ""),
        }
        let x = tr.right() + 14.0;
        let w = card.right() - x - 130.0;
        line(&p, Pos2::new(x, card.center().y - 1.0), Align2::LEFT_BOTTOM, &map_name.to_uppercase(), 12.0, ACCENT, w);
        line(&p, Pos2::new(x, card.center().y + 1.0), Align2::LEFT_TOP, &var_name, 18.0, Color32::WHITE, w);
        p.text(Pos2::new(card.right() - 14.0, card.center().y), Align2::RIGHT_CENTER, "CHANGE  ›", FontId::proportional(13.0), if resp.hovered() { ACCENT } else { DIM });
        if resp.clicked() {
            self.step = Step::Map;
            self.scroll_to = true;
        }
        self.buttons(ui, rect, s, true)
    }

    /// (map name, variant name or difficulty, thumbnail key) of the current map pick.
    fn map_pick(&self) -> (String, String, String) {
        if self.firefight() {
            let m = super::menu::FF_MAPS.get(self.ff_map).copied().unwrap_or(("", "?"));
            return (format!("Firefight · {}", super::menu::DIFFICULTIES[self.difficulty].1), m.1.to_string(), format!("REACHFIREFIGHT_{}", art_key(m.1)));
        }
        match self.maps.get(self.map) {
            Some(m) => (m.name.clone(), m.variants.get(self.variant).map_or_else(|| m.name.clone(), |v| v.title.clone()), format!("HREACHMP_{}", art_key(&m.name))),
            None => ("No maps found".into(), String::new(), String::new()),
        }
    }

    /// START GAME / START HOSTING and the step buttons along the bottom of `rect`.
    fn buttons(&mut self, ui: &mut Ui, rect: Rect, s: &Settings, game_step: bool) -> Option<Action> {
        let args = self.args();
        let mut out = None;
        let by = rect.bottom() - 44.0;
        let mut row = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_max(Pos2::new(rect.left(), by), rect.max)).layout(egui::Layout::left_to_right(egui::Align::Center)));
        row.spacing_mut().item_spacing.x = 8.0;
        let n = if self.host { 3.0 } else { 2.0 };
        let w = ((rect.width() - 8.0 * n) / n).min(220.0);
        let start = if self.host { "Start hosting" } else { "Start game" };
        if theme::primary_button(&mut row, start, w, args.is_some()).clicked() {
            out = args.clone().map(|a| if self.host { Action::Host { players: self.host_players(s), args: a } } else { Action::Play(a) });
        }
        if game_step {
            if big_ghost(&mut row, "Choose map  ›", w, true).clicked() {
                self.step = Step::Map;
                self.scroll_to = true;
            }
        } else if big_ghost(&mut row, "‹  Game type", w, true).clicked() {
            self.step = Step::Game;
            self.scroll_to = true;
        }
        if self.host && big_ghost(&mut row, "Server  ›", w, true).clicked() {
            self.step = Step::Server;
        }
        out
    }

    // ---- Step 2: map -----------------------------------------------------------------------------------------

    fn map_list(&mut self, ui: &mut Ui, rect: Rect) {
        let p = ui.painter().clone();
        p.rect_filled(rect, 2.0, PANEL);
        p.rect_stroke(rect, 2.0, Stroke::new(1.0, LINE), StrokeKind::Inside);
        let inner = rect.shrink(14.0);
        let ff = self.firefight();
        let sr = Rect::from_min_size(inner.min, Vec2::new(inner.width(), 34.0));
        search_box(ui, sr, &mut self.map_query, if ff { "Type to search maps" } else { "Type to search maps and variants" }, true);
        let y = sr.bottom() + 10.0;
        p.text(Pos2::new(inner.left(), y), Align2::LEFT_TOP, if ff { "FIREFIGHT MAPS" } else { "MAPS" }, FontId::proportional(12.0), if self.focus_variants && !ff { DIM } else { ACCENT });
        let area = Rect::from_min_max(Pos2::new(inner.left(), y + 20.0), inner.max);
        let (maps, _) = self.visible_maps();
        if maps.is_empty() {
            p.text(area.left_top() + Vec2::new(0.0, 8.0), Align2::LEFT_TOP, "No map matches. Esc clears the search.", FontId::proportional(15.0), DIM);
            return;
        }
        let row_h = (area.height() / maps.len() as f32).clamp(46.0, 60.0);
        let thumb = Vec2::new((row_h - 10.0) * 16.0 / 9.0, row_h - 10.0);
        let ctx = ui.ctx().clone();
        let mut double = false;
        ui.scope_builder(egui::UiBuilder::new().max_rect(area), |ui| {
            egui::ScrollArea::vertical().id_salt("mp-maps").max_height(area.height()).auto_shrink([false, false]).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for &i in &maps {
                    let (name, sub, th) = if ff {
                        let m = super::menu::FF_MAPS[i];
                        (m.1.to_string(), "Firefight".to_string(), format!("REACHFIREFIGHT_{}", art_key(m.1)))
                    } else {
                        let m = &self.maps[i];
                        let n = m.variants.len() - 1;
                        let sub = if n == 0 { "Default only".to_string() } else { format!("{n} variant{}", if n == 1 { "" } else { "s" }) };
                        (m.name.clone(), sub, format!("HREACHMP_{}", art_key(&m.name)))
                    };
                    let selected = if ff { self.ff_map == i } else { self.map == i };
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
                        None => placeholder(p, tr, ""),
                    }
                    p.rect_stroke(tr, 0.0, Stroke::new(1.0, if selected { ACCENT } else { LINE }), StrokeKind::Inside);
                    let x = tr.right() + 12.0;
                    p.text(Pos2::new(x, r.center().y - 2.0), Align2::LEFT_BOTTOM, sub.to_uppercase(), FontId::proportional(11.5), if selected { ACCENT } else { DIM });
                    line(p, Pos2::new(x, r.center().y), Align2::LEFT_TOP, &name, 17.0, if selected { Color32::WHITE } else { TEXT }, r.right() - x - 8.0);
                    if selected && self.scroll_to && !self.focus_variants {
                        resp.scroll_to_me(Some(egui::Align::Center));
                    }
                    if resp.clicked() {
                        if ff {
                            self.ff_map = i;
                        } else if self.map != i {
                            self.map = i;
                            self.variant = 0;
                        }
                        self.focus_variants = false;
                    }
                    if resp.double_clicked() {
                        double = true;
                    }
                }
            });
        });
        if double && ff {
            self.focus_variants = false;
        }
    }

    fn map_detail(&mut self, ui: &mut Ui, rect: Rect, s: &Settings) -> Option<Action> {
        let ctx = ui.ctx().clone();
        let p = ui.painter().clone();
        let ff = self.firefight();
        let (title, sub, big, desc) = if ff {
            let m = super::menu::FF_MAPS.get(self.ff_map).copied().unwrap_or(("", "?"));
            (m.1.to_string(), "FIREFIGHT".to_string(), format!("REACHFIREFIGHT_{}LARGE", art_key(m.1)), self.game().map(|g| g.title.clone()).unwrap_or_default())
        } else {
            let Some(m) = self.maps.get(self.map) else { return self.buttons(ui, rect, s, false) };
            let v = &m.variants[self.variant.min(m.variants.len() - 1)];
            let src = if v.source == Source::Default { m.name.to_uppercase() } else { format!("{}  ·  {}", m.name.to_uppercase(), v.source.label().to_uppercase()) };
            (v.title.clone(), src, format!("HREACHMP_{}LARGE", art_key(&m.name)), v.desc.clone())
        };
        let bh = (rect.width() / (668.0 / 292.0)).min(rect.height() * 0.42).max(110.0);
        let br = Rect::from_min_size(rect.min, Vec2::new(rect.width(), bh));
        banner(&ctx, &p, br, &big, "");
        let tx = br.left() + 22.0;
        let w = br.width() - 44.0;
        let desc_r = if desc.is_empty() { Rect::from_min_size(Pos2::new(tx, br.bottom() - 14.0), Vec2::ZERO) } else { line(&p, Pos2::new(tx, br.bottom() - 14.0), Align2::LEFT_BOTTOM, &desc, 14.0, Color32::from_gray(210), w) };
        let name_r = line(&p, Pos2::new(tx, desc_r.top() - 2.0), Align2::LEFT_BOTTOM, &title.to_uppercase(), 30.0, Color32::WHITE, w);
        p.text(Pos2::new(tx, name_r.top()), Align2::LEFT_BOTTOM, sub, FontId::proportional(13.0), ACCENT);

        let list = Rect::from_min_max(Pos2::new(rect.left(), br.bottom() + 14.0), Pos2::new(rect.right(), rect.bottom() - 58.0));
        if ff {
            self.difficulty_cards(ui, list);
        } else {
            self.variant_list(ui, list);
        }
        self.buttons(ui, rect, s, false)
    }

    fn variant_list(&mut self, ui: &mut Ui, rect: Rect) {
        let p = ui.painter().clone();
        let (_, vars) = self.visible_maps();
        let Some(m) = self.maps.get(self.map) else { return };
        let label = format!("VARIANTS OF {} ({})", m.name.to_uppercase(), m.variants.len() - 1);
        p.text(rect.left_top(), Align2::LEFT_TOP, label, FontId::proportional(12.0), if self.focus_variants { ACCENT } else { DIM });
        p.text(rect.right_top(), Align2::RIGHT_TOP, "LEFT / RIGHT: SWITCH LIST", FontId::proportional(11.0), DIM);
        let area = Rect::from_min_max(rect.min + Vec2::new(0.0, 20.0), rect.max);
        p.rect_filled(area, 2.0, PANEL);
        p.rect_stroke(area, 2.0, Stroke::new(1.0, LINE), StrokeKind::Inside);
        let inner = area.shrink2(Vec2::new(6.0, 4.0));
        let mut pick = None;
        ui.scope_builder(egui::UiBuilder::new().max_rect(inner), |ui| {
            egui::ScrollArea::vertical().id_salt(("mp-vars", self.map)).max_height(inner.height()).auto_shrink([false, false]).show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                for &v in &vars {
                    let var = &m.variants[v];
                    let selected = v == self.variant;
                    let (r, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 38.0), Sense::click());
                    let p = ui.painter();
                    if selected {
                        p.rect_filled(r, 0.0, ACCENT.gamma_multiply(0.20));
                        p.rect_filled(Rect::from_min_size(r.min, Vec2::new(3.0, r.height())), 0.0, ACCENT);
                    } else if resp.hovered() {
                        p.rect_filled(r, 0.0, HOVER);
                    }
                    let tag = var.source.label().to_uppercase();
                    let tag_r = p.text(Pos2::new(r.right() - 10.0, r.center().y), Align2::RIGHT_CENTER, tag, FontId::proportional(11.0), if matches!(var.source, Source::Saved | Source::Builtin | Source::Custom | Source::Workshop) { ACCENT } else { DIM });
                    let x = r.left() + 14.0;
                    let tw = (r.width() * 0.42).min(260.0);
                    line(p, Pos2::new(x, r.center().y), Align2::LEFT_CENTER, &var.title, 15.5, if selected { Color32::WHITE } else { TEXT }, tw);
                    line(p, Pos2::new(x + tw + 12.0, r.center().y), Align2::LEFT_CENTER, &var.desc, 12.5, DIM, (tag_r.left() - 12.0 - x - tw - 12.0).max(0.0));
                    if selected && self.scroll_to && self.focus_variants {
                        resp.scroll_to_me(Some(egui::Align::Center));
                    }
                    if resp.clicked() {
                        pick = Some(v);
                    }
                }
            });
        });
        if let Some(v) = pick {
            self.variant = v;
            self.focus_variants = true;
        }
    }

    fn difficulty_cards(&mut self, ui: &mut Ui, rect: Rect) {
        let ctx = ui.ctx().clone();
        let p = ui.painter().clone();
        p.text(rect.left_top(), Align2::LEFT_TOP, "DIFFICULTY", FontId::proportional(12.0), DIM);
        let gap = 12.0;
        let cw = (rect.width() - gap * 3.0) / 4.0;
        let ch = (rect.height() - 26.0).clamp(40.0, 84.0);
        let icons = ["DIFFICULTYLARGEEASY", "DIFFICULTYLARGENORMAL", "DIFFICULTYLARGEHEROIC", "DIFFICULTYLARGELEGENDARY"];
        for (k, d) in super::menu::DIFFICULTIES.iter().enumerate() {
            let r = Rect::from_min_size(Pos2::new(rect.left() + k as f32 * (cw + gap), rect.top() + 20.0), Vec2::new(cw, ch));
            let resp = ui.interact(r, ui.id().with(("mp-diff", k)), Sense::click());
            let on = self.difficulty == k;
            p.rect_filled(r, 2.0, if on { ACCENT.gamma_multiply(0.18) } else if resp.hovered() { HOVER } else { PANEL });
            p.rect_stroke(r, 2.0, Stroke::new(if on { 2.0 } else { 1.0 }, if on { ACCENT } else { LINE }), StrokeKind::Inside);
            // Icon beside the name when the card is wide enough, else the name alone.
            let wide = cw >= 170.0;
            let icon = Rect::from_min_size(r.min + Vec2::new(10.0, (ch - 40.0) / 2.0), Vec2::splat(40.0));
            if let (true, Some(t)) = (wide, mccart::texture(&ctx, icons[k])) {
                p.image(t.id(), icon, full_uv(&t), if on { Color32::WHITE } else { Color32::from_gray(170) });
            }
            let (pos, a) = if wide { (Pos2::new(icon.right() + 10.0, r.center().y), Align2::LEFT_CENTER) } else { (r.center(), Align2::CENTER_CENTER) };
            p.text(pos, a, d.1.to_uppercase(), FontId::proportional(16.0), if on { Color32::WHITE } else { TEXT });
            if resp.clicked() {
                self.difficulty = k;
            }
        }
        if self.host {
            let y = rect.top() + 20.0 + ch + 14.0;
            if y + 30.0 < rect.bottom() {
                let mut row = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_max(Pos2::new(rect.left(), y), rect.max)).layout(egui::Layout::left_to_right(egui::Align::Min)));
                theme::caption(&mut row, "Players");
                row.add(egui::Slider::new(&mut self.coop_players, 2..=crate::coop::MAX));
            }
        }
    }

    // ---- Step 3: server (HOST) -------------------------------------------------------------------------------

    fn server(&mut self, ui: &mut Ui, left: Rect, right: Rect, s: &mut Settings) -> Option<Action> {
        let ff = self.firefight();
        theme::panel(ui, left, |ui| {
            egui::ScrollArea::vertical().id_salt("mp-server").auto_shrink([false, false]).show(ui, |ui| {
                theme::caption(ui, "Server name");
                ui.add(egui::TextEdit::singleline(&mut s.host_name).desired_width(ui.available_width()).char_limit(48));
                ui.add_space(8.0);
                theme::caption(ui, if ff { "Players" } else { "Max players" });
                if ff {
                    ui.add(egui::Slider::new(&mut self.coop_players, 2..=crate::coop::MAX));
                } else {
                    ui.add(egui::Slider::new(&mut s.host_max_players, 2..=16));
                }
                ui.add_space(8.0);
                ui.checkbox(&mut s.host_register, "List it on the master server");
                ui.label(egui::RichText::new(if s.master_url.is_empty() { crate::browser::master_list().join(", ") } else { s.master_url.clone() }).size(12.5).color(DIM));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    theme::caption(ui, "UDP port");
                    ui.add(egui::DragValue::new(&mut s.host_port).range(1024..=65535));
                });
                let lan = super::hostinfo::lan_ip().map(|ip| format!("{ip}:{}", s.host_port)).unwrap_or_else(|| "unknown".into());
                ui.label(egui::RichText::new(format!("Friends on your LAN add {lan}; the master lists your internet address.")).size(13.0).color(DIM));
                ui.add_space(4.0);
                ui.label(egui::RichText::new(super::hostinfo::reach_hint(s.host_port)).size(12.5).color(DIM));
            });
        });
        // Summary of the game on the right.
        let ctx = ui.ctx().clone();
        let p = ui.painter().clone();
        if let Some(g) = self.game().cloned() {
            let bh = (right.width() / (316.0 / 160.0)).min(right.height() - 130.0).max(80.0);
            let br = Rect::from_min_size(right.min, Vec2::new(right.width(), bh));
            banner(&ctx, &p, br, g.kind.banner, g.kind.icon);
            let tx = br.left() + 22.0;
            let (map, var, _) = self.map_pick();
            let r = line(&p, Pos2::new(tx, br.bottom() - 14.0), Align2::LEFT_BOTTOM, &format!("{map} · {var}"), 15.0, Color32::from_gray(215), br.width() - 44.0);
            let r = line(&p, Pos2::new(tx, r.top() - 2.0), Align2::LEFT_BOTTOM, &g.title.to_uppercase(), 30.0, Color32::WHITE, br.width() - 44.0);
            p.text(Pos2::new(tx, r.top()), Align2::LEFT_BOTTOM, "YOU ARE HOSTING", FontId::proportional(13.0), ACCENT);
            let hint = "The game starts at once with you playing; friends join any time from the server browser.";
            lines(&p, Rect::from_min_max(Pos2::new(right.left(), br.bottom() + 12.0), Pos2::new(right.right(), right.bottom() - 56.0)), hint, 14.0, DIM);
        }
        let args = self.args();
        let by = right.bottom() - 44.0;
        let mut row = ui.new_child(egui::UiBuilder::new().max_rect(Rect::from_min_max(Pos2::new(right.left(), by), right.max)).layout(egui::Layout::left_to_right(egui::Align::Center)));
        let w = ((right.width() - 16.0) / 2.0).min(220.0);
        let mut out = None;
        if theme::primary_button(&mut row, "Start hosting", w, args.is_some()).clicked() {
            out = args.map(|a| Action::Host { players: self.host_players(s), args: a });
        }
        row.add_space(8.0);
        if big_ghost(&mut row, "‹  Map", w, true).clicked() {
            self.step = Step::Map;
        }
        out
    }
}

fn teams_label(t: Option<bool>) -> &'static str {
    match t {
        Some(true) => "Teams",
        Some(false) => "FFA",
        None => "Co-op",
    }
}

fn teams_long(t: Option<bool>) -> &'static str {
    match t {
        Some(true) => "Team game",
        Some(false) => "Free for all",
        None => "Everyone together",
    }
}

/// Banner picture `key` filling `r` (else the icon on a plain card), darkened towards the bottom for text.
fn banner(ctx: &egui::Context, p: &egui::Painter, r: Rect, key: &str, icon: &str) {
    match mccart::texture(ctx, key) {
        Some(t) => cover(p, &t, r, Color32::WHITE),
        None => {
            placeholder(p, r, "");
            if let Some(t) = mccart::texture(ctx, icon) {
                let s = (r.height() * 0.6).min(140.0);
                p.image(t.id(), Rect::from_center_size(Pos2::new(r.right() - s * 0.9, r.center().y), Vec2::splat(s)), full_uv(&t), Color32::from_white_alpha(120));
            }
        }
    }
    let mut mesh = egui::Mesh::default();
    let fade_top = r.top() + r.height() * 0.30;
    mesh.colored_vertex(Pos2::new(r.left(), fade_top), Color32::from_black_alpha(0));
    mesh.colored_vertex(Pos2::new(r.right(), fade_top), Color32::from_black_alpha(0));
    mesh.colored_vertex(r.right_bottom(), Color32::from_black_alpha(220));
    mesh.colored_vertex(r.left_bottom(), Color32::from_black_alpha(220));
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    p.add(mesh);
    p.rect_stroke(r, 0.0, Stroke::new(1.0, LINE), StrokeKind::Inside);
}

/// One line of text at most `max_w` wide (cut with an ellipsis), anchored at `pos`. Returns its rect.
fn line(p: &egui::Painter, pos: Pos2, anchor: Align2, text: &str, size: f32, color: Color32, max_w: f32) -> Rect {
    let mut job = egui::text::LayoutJob::single_section(text.to_string(), egui::TextFormat::simple(FontId::proportional(size), color));
    job.wrap = egui::text::TextWrapping { max_width: max_w.max(1.0), max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
    let g = p.layout_job(job);
    let r = anchor.anchor_size(pos, g.size());
    p.galley(r.min, g, color);
    r
}

/// Wrapped text filling `r`, as many lines as fit (the last one cut with an ellipsis).
fn lines(p: &egui::Painter, r: Rect, text: &str, size: f32, color: Color32) {
    let rows = ((r.height() / (size * 1.3)).floor() as usize).max(1);
    let mut job = egui::text::LayoutJob::single_section(text.to_string(), egui::TextFormat::simple(FontId::proportional(size), color));
    job.wrap = egui::text::TextWrapping { max_width: r.width().max(1.0), max_rows: rows, break_anywhere: false, overflow_character: Some('…') };
    p.galley(r.min, p.layout_job(job), color);
}
