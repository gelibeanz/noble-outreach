//! SERVER BROWSER screen (Project Reclaimer style): tabs, category chips, sortable table, details panel, direct
//! connect. Data: master list (`browser::fetch_any`) plus one UDP info query per server (`query.rs`: live players,
//! ping, playlist), favourites in settings.json. `menu.rs` only opens it, draws it and launches what it returns.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::Duration;

use egui::{pos2, vec2, Align, Align2, Color32, FontId, Layout, Rect, Sense, Stroke, Ui, UiBuilder};

use super::menu::{ff_variant_title, map_title, variant_title};
use super::settings::{self};
use super::theme::{self, ACCENT, DIM, LINE, TEXT};
use crate::browser::ServerInfo;
use crate::log;
use crate::query;

/// What the screen asks the menu to do.
pub enum Action {
    None,
    Back,
    /// Game arguments (after `--mcc`) of the server to join.
    Launch(Vec<String>),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Tab {
    All,
    Favourites,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum SortCol {
    Name,
    Map,
    Mode,
    Players,
    Ping,
    Version,
}

/// Category chips: index 0 = all, then `query::MODE_*` + 1 (Forge = `MODE_FORGE` + 1).
const CATS: &[&str] = &["All", "Multiplayer", "Firefight", "Campaign", "Forge"];
const PING_STEPS: &[u32] = &[0, 50, 100, 150, 250];
/// LAN tab: broadcast the info query to the usual host ports.
const LAN_PORTS: &[u16] = &[47800, 47801, 47802, 47803, 47900];
const OUR_VERSION: &str = env!("CARGO_PKG_VERSION");

/// One row: master data (or a bare favourite address), overwritten by the live UDP query when it answers.
#[derive(Clone)]
struct Entry {
    /// The address as listed or typed (a favourite may be a hostname); favourites are keyed by it.
    key: String,
    info: ServerInfo,
    mode: String,
    kind: u8,
    resolved: Option<SocketAddr>,
    ping: Option<u32>,
    live: bool,
    from_master: bool,
    /// Dedicated campaign whose mission is running (query flag): the engine refuses joins until it restarts.
    in_progress: bool,
    playlist: String,
    password: bool,
}

pub fn map_kind(map: &str) -> u8 {
    let b = map.as_bytes();
    if map.starts_with("ff") {
        query::MODE_FIREFIGHT
    } else if b.len() > 1 && b[0] == b'm' && b[1].is_ascii_digit() {
        query::MODE_CAMPAIGN
    } else {
        query::MODE_MULTIPLAYER
    }
}

/// Game type column: Firefight / campaign state, else the game variant's pretty name.
fn mode_label(kind: u8, variant: &str, in_progress: bool, lobby_secs: u8) -> String {
    match kind {
        query::MODE_FIREFIGHT if variant.is_empty() => "Firefight".into(),
        query::MODE_FIREFIGHT => ff_variant_title(variant),
        query::MODE_CAMPAIGN if in_progress => "Campaign (in progress)".into(),
        query::MODE_CAMPAIGN if lobby_secs > 0 => format!("Campaign (starts in {lobby_secs} s)"),
        query::MODE_CAMPAIGN => "Campaign".into(),
        query::MODE_FORGE => "Forge".into(),
        _ if variant.is_empty() => "Multiplayer".into(),
        _ => variant_title(variant),
    }
}

impl Entry {
    fn from_master(s: ServerInfo) -> Self {
        // The master listing has no mode: Forge servers run MCC's sandbox game variant.
        let kind = if s.game_variant.eq_ignore_ascii_case(crate::forge::FORGE_GAME_VARIANT) { query::MODE_FORGE } else { map_kind(&s.map) };
        Entry {
            key: s.addr.clone(),
            mode: mode_label(kind, &s.game_variant, false, 0),
            kind,
            resolved: query::resolve(&s.addr).ok(),
            ping: None,
            live: false,
            from_master: true,
            in_progress: false,
            playlist: String::new(),
            password: false,
            info: s,
        }
    }

    /// A favourite / LAN address nobody has described yet.
    fn bare(key: &str, resolved: Option<SocketAddr>) -> Self {
        let info = ServerInfo {
            name: key.to_string(),
            version: String::new(),
            addr: resolved.map_or_else(|| key.to_string(), |a| a.to_string()),
            network_id: String::new(),
            session_id: String::new(),
            map: String::new(),
            game_variant: String::new(),
            players: 0,
            max_players: 0,
            dedicated: false,
        };
        Entry { key: key.to_string(), info, mode: "—".into(), kind: query::MODE_MULTIPLAYER, resolved, ping: None, live: false, from_master: false, in_progress: false, playlist: String::new(), password: false }
    }

    fn apply(&mut self, r: &query::Reply) {
        let i = &r.info;
        self.ping = Some(r.ping_ms);
        self.live = true;
        self.info.players = r.players as u32;
        self.info.max_players = i.max_players as u32;
        if !i.name.is_empty() {
            self.info.name = i.name.clone();
        }
        if !i.map.is_empty() {
            self.info.map = i.map.clone();
        }
        if !i.game_variant.is_empty() {
            self.info.game_variant = i.game_variant.clone();
        }
        self.info.version = i.version.clone();
        self.info.dedicated = i.dedicated;
        self.info.network_id = format!("{:x}", i.network_id);
        self.info.session_id = format!("{:x}", i.session_id);
        self.kind = i.mode;
        self.in_progress = r.in_progress;
        self.playlist = i.playlist.clone();
        self.password = r.password;
        self.mode = mode_label(i.mode, &self.info.game_variant, r.in_progress, r.lobby_secs);
    }

    fn full(&self) -> bool {
        self.info.max_players > 0 && self.info.players >= self.info.max_players
    }

    fn cat(&self) -> usize {
        self.kind as usize + 1
    }
}

enum Msg {
    /// The master list arrived (rows show at once, pings follow).
    Master(Result<(String, Vec<ServerInfo>), String>),
    Done { internet: Vec<Entry>, favs: Vec<Entry>, lan: Vec<Entry> },
}

/// `REACH_BROWSER_FAKE=1`: made-up servers instead of the network (screenshot checks of the layout).
fn fake_entries() -> Vec<Entry> {
    // (name, map, variant, kind, players, max, ping, version, playlist)
    let rows: &[(&str, &str, &str, u8, u32, u32, u32, &str, &str)] = &[
        ("Zima Slayer Night", "forge_halo", "slayer_054.bin", 0, 7, 16, 18, OUR_VERSION, "Slayer"),
        ("Noble Team Firefight", "ff50_park", "hr_ff_standard_1set.bin", 1, 3, 4, 34, OUR_VERSION, "Firefight"),
        ("Campaign co-op: Winter Contingency", "m10", "slayer_054.bin", 2, 2, 4, 52, OUR_VERSION, ""),
        ("Big Team Battle with a very long server name that must be ellipsised", "70_boneyard", "team_slayer_054.bin", 0, 12, 16, 88, OUR_VERSION, "Team Slayer"),
        ("Old build server", "45_aftship", "slayer_pro_054.bin", 0, 4, 8, 133, "0.0.9", ""),
        ("Empty lobby", "cex_damnation", "slayer_054.bin", 0, 0, 12, 211, OUR_VERSION, ""),
        ("Full house", "35_island", "slayer_054.bin", 0, 8, 8, 25, OUR_VERSION, "Slayer"),
        ("Mission running", "m30", "slayer_054.bin", 2, 3, 4, 41, OUR_VERSION, ""),
        ("Glacier Holdout", "ff60_icecave", "hr_ff_generatorDefense_1set.bin", 1, 1, 4, 305, OUR_VERSION, ""),
        ("Sword Base Classic", "20_sword_slayer", "slayer_054.bin", 0, 5, 16, 64, OUR_VERSION, "Slayer"),
    ];
    rows.iter()
        .enumerate()
        .map(|(n, r)| {
            let info = ServerInfo {
                name: r.0.into(),
                version: r.7.into(),
                addr: format!("10.9.0.{}:{}", n + 2, 47800 + n),
                network_id: "0".into(),
                session_id: "1".into(),
                map: r.1.into(),
                game_variant: r.2.into(),
                players: r.4,
                max_players: r.5,
                dedicated: true,
            };
            let mut e = Entry::from_master(info);
            e.resolved = None;
            e.kind = r.3;
            e.in_progress = r.0 == "Mission running";
            e.mode = mode_label(r.3, r.2, e.in_progress, 0);
            e.ping = Some(r.6);
            e.live = true;
            e.playlist = r.8.into();
            e
        })
        .collect()
}

/// Background refresh: first master that answers, then one UDP info query to every listed server and favourite
/// (live players + ping), and a LAN broadcast alongside. `answered` / `total` feed the "X of Y" footer.
fn refresh_worker(masters: Vec<String>, favs: Vec<String>, tx: Sender<Msg>, answered: Arc<AtomicUsize>, total: Arc<AtomicUsize>) {
    if std::env::var_os("REACH_BROWSER_FAKE").is_some() {
        let list = fake_entries();
        total.store(list.len(), Ordering::Relaxed);
        let _ = tx.send(Msg::Master(Ok(("fake".into(), list.iter().map(|e| e.info.clone()).collect()))));
        for i in 0..list.len() {
            std::thread::sleep(Duration::from_millis(150));
            answered.store(i + 1, Ordering::Relaxed);
        }
        let favs = favs.iter().filter_map(|k| list.iter().find(|e| &e.key == k).cloned()).collect();
        let _ = tx.send(Msg::Done { internet: list, favs, lan: Vec::new() });
        return;
    }
    let lan = std::thread::spawn(|| query::query_broadcast(LAN_PORTS, Duration::from_millis(900)));
    let master = crate::browser::fetch_any(&masters);
    let mut internet: Vec<Entry> = master.as_ref().map(|(_, l)| l.iter().cloned().map(Entry::from_master).collect()).unwrap_or_default();
    let mut favs: Vec<Entry> = favs
        .iter()
        .map(|k| {
            let r = query::resolve(k).ok();
            match internet.iter().find(|e| r.is_some() && e.resolved == r) {
                Some(m) => Entry { key: k.clone(), ..m.clone() },
                None => Entry::bare(k, r),
            }
        })
        .collect();
    let mut targets: Vec<SocketAddr> = Vec::new();
    for a in internet.iter().chain(&favs).filter_map(|e| e.resolved) {
        if !targets.contains(&a) {
            targets.push(a);
        }
    }
    total.store(targets.len(), Ordering::Relaxed);
    let _ = tx.send(Msg::Master(master));
    let replies = query::query_many_progress(&targets, Duration::from_millis(1500), Some(&answered));
    for e in internet.iter_mut().chain(favs.iter_mut()) {
        if let Some(Some(r)) = e.resolved.and_then(|a| targets.iter().position(|t| *t == a)).map(|i| &replies[i]) {
            e.apply(r);
        }
    }
    let n = replies.iter().filter(|r| r.is_some()).count();
    log!("browser: {} of {} servers answered the info query", n, targets.len());
    let lan = lan
        .join()
        .unwrap_or_default()
        .into_iter()
        .map(|(a, r)| {
            let mut e = Entry::bare(&a.to_string(), Some(a));
            e.apply(&r);
            e
        })
        .collect();
    let _ = tx.send(Msg::Done { internet, favs, lan });
}

pub struct BrowserScreen {
    mcc: PathBuf,
    tab: Tab,
    cat: usize,
    hide_full: bool,
    hide_empty: bool,
    /// 0 = any.
    max_ping: u32,
    version_only: bool,
    playlist_only: bool,
    sort: SortCol,
    sort_desc: bool,
    search: String,
    internet: Vec<Entry>,
    favs: Vec<Entry>,
    lan: Vec<Entry>,
    status: String,
    master: String,
    pending: Option<Receiver<Msg>>,
    /// Refresh again when the running one ends (a favourite was added meanwhile).
    again: bool,
    selected: Option<String>,
    direct_open: bool,
    direct: String,
    direct_err: String,
    pub error: String,
    answered: Arc<AtomicUsize>,
    total: Arc<AtomicUsize>,
    /// Screenshot hook: select the first visible row once rows exist.
    select_first: bool,
    /// The install's HaloReach.png (tinted per map as the map card), loaded on first use.
    card: Option<Option<egui::TextureHandle>>,
}

impl BrowserScreen {
    pub fn new(mcc: &Path) -> Self {
        BrowserScreen {
            mcc: mcc.to_path_buf(),
            tab: Tab::All,
            cat: 0,
            hide_full: false,
            hide_empty: false,
            max_ping: 0,
            version_only: false,
            playlist_only: false,
            sort: SortCol::Players,
            sort_desc: true,
            search: String::new(),
            internet: Vec::new(),
            favs: Vec::new(),
            lan: Vec::new(),
            status: String::new(),
            master: String::new(),
            pending: None,
            again: false,
            selected: None,
            direct_open: false,
            direct: String::new(),
            direct_err: String::new(),
            error: String::new(),
            answered: Arc::new(AtomicUsize::new(0)),
            total: Arc::new(AtomicUsize::new(0)),
            select_first: false,
            card: None,
        }
    }

    /// No refresh in flight (screenshot steps wait for it).
    pub fn is_idle(&self) -> bool {
        self.pending.is_none()
    }

    /// Called when the screen opens: first visit loads the list.
    pub fn open(&mut self) {
        self.error.clear();
        if self.internet.is_empty() && self.pending.is_none() {
            self.refresh();
        }
    }

    /// Screenshot steps `browser-<what>` (menu.rs passes them on): `fav` favourites tab, `cat` Firefight chip,
    /// `sel` first row selected, `direct` direct-connect dialog open.
    pub fn test_hook(&mut self, what: &str) {
        match what {
            "fav" => {
                self.tab = Tab::Favourites;
                self.selected = None;
            }
            "cat" => self.cat = 2,
            "sel" => self.select_first = true,
            "direct" => {
                self.direct_open = true;
                self.direct = "127.0.0.1:47800".into();
            }
            _ => {}
        }
    }

    fn refresh(&mut self) {
        let (tx, rx) = channel();
        let masters = crate::browser::master_list();
        self.status = if masters.is_empty() { format!("{} (querying favourites and LAN)", crate::browser::NO_MASTER) } else { format!("Asking {} ...", masters.join(", ")) };
        self.pending = Some(rx);
        self.answered.store(0, Ordering::Relaxed);
        self.total.store(0, Ordering::Relaxed);
        let favs = settings::get().favourites;
        let (a, t) = (self.answered.clone(), self.total.clone());
        std::thread::spawn(move || refresh_worker(masters, favs, tx, a, t));
    }

    fn poll(&mut self) {
        let Some(rx) = &self.pending else { return };
        let Ok(msg) = rx.try_recv() else { return };
        match msg {
            Msg::Master(Ok((url, list))) => {
                self.status = format!("master {url} answered, querying servers ...");
                self.master = url;
                self.internet = list.into_iter().map(Entry::from_master).collect();
            }
            Msg::Master(Err(e)) => {
                self.status = if e == crate::browser::NO_MASTER { e } else { format!("no master answered ({e})") };
                self.master.clear();
                self.internet.clear();
            }
            Msg::Done { internet, favs, lan } => {
                self.pending = None;
                if !self.master.is_empty() {
                    self.status = format!("master {}", self.master);
                }
                self.internet = internet;
                self.favs = favs;
                self.lan = lan;
                if self.again {
                    self.again = false;
                    self.refresh();
                }
            }
        }
    }

    /// ALL SERVERS: master list plus LAN finds not already listed.
    fn all(&self) -> Vec<Entry> {
        let mut v = self.internet.clone();
        for l in &self.lan {
            if !v.iter().any(|e| e.resolved.is_some() && e.resolved == l.resolved) {
                v.push(l.clone());
            }
        }
        v
    }

    fn is_favourite(&self, e: &Entry) -> bool {
        settings::get().favourites.contains(&e.key) || self.favs.iter().any(|f| f.resolved.is_some() && f.resolved == e.resolved)
    }

    /// Add `addr` (host:port) to the favourites in settings.json and show it at once; the refresh queries it.
    pub fn add_favourite(&mut self, addr: &str) -> Result<(), String> {
        let addr = addr.trim().to_string();
        let (host, port) = addr.rsplit_once(':').ok_or_else(|| format!("{addr:?}: use host:port"))?;
        if host.is_empty() || port.parse::<u16>().is_err() {
            return Err(format!("{addr:?}: use host:port"));
        }
        let mut s = settings::get();
        if !s.favourites.contains(&addr) {
            s.favourites.push(addr.clone());
            settings::save(&s);
            log!("browser: favourite {addr} added");
        }
        if !self.favs.iter().any(|e| e.key == addr) {
            self.favs.push(Entry::bare(&addr, None));
        }
        if self.pending.is_some() {
            self.again = true;
        } else {
            self.refresh();
        }
        Ok(())
    }

    fn remove_favourite(&mut self, e: &Entry) {
        let mut s = settings::get();
        s.favourites.retain(|f| *f != e.key && !(e.resolved.is_some() && query::resolve(f).ok() == e.resolved));
        settings::save(&s);
        let fav = &s.favourites;
        self.favs.retain(|f| fav.contains(&f.key));
        log!("browser: favourite {} removed", e.key);
    }

    fn variant_path(&self, file: &str) -> PathBuf {
        self.mcc.join("haloreach").join("game_variants").join(file)
    }

    /// Join a row: full args when the master or the query described it, else the default-session direct connect.
    fn join_entry(&self, e: &Entry) -> Result<Vec<String>, String> {
        if e.in_progress {
            return Err(crate::lobby::IN_PROGRESS_MSG.into());
        }
        let mut s = e.info.clone();
        if let Some(a) = e.resolved {
            s.addr = a.to_string();
        }
        if s.session_id.is_empty() {
            return self.default_join(&s.addr);
        }
        self.join_args(&s)
    }

    /// Arguments to join a listed server (same checks as `--join-server`).
    fn join_args(&self, s: &ServerInfo) -> Result<Vec<String>, String> {
        let host_id = crate::browser::parse_hex(&s.network_id)?;
        if host_id != crate::net::net_id(0) {
            return Err(format!("server network id {host_id:#x} is not supported (only instance 0 hosts)"));
        }
        let addr = query::resolve(&s.addr)?;
        let kind = map_kind(&s.map);
        let gv = self.variant_path(&s.game_variant);
        if !gv.is_file() && kind != query::MODE_CAMPAIGN {
            return Err(format!("game variant {} is not in your MCC files", s.game_variant));
        }
        // Same as --join-server: dedicated and listen hosts both take late joiners, so every joiner gets a random
        // identity (friends must not share one) and an OS-chosen port.
        let mut a: Vec<String> = vec!["--net-role".into(), "client".into()];
        a.extend(joiner_identity());
        a.extend(["--peer".into(), format!("0={addr}"), "--session-id".into(), s.session_id.clone()]);
        a.extend(["--players".into(), s.max_players.max(2).to_string()]);
        match kind {
            query::MODE_FIREFIGHT => a.extend(["--firefight".into(), s.map.clone()]),
            query::MODE_CAMPAIGN => a.extend(["--campaign".into(), s.map.clone(), "--insertion".into(), "0".into()]),
            _ => a.extend(["--map".into(), s.map.clone()]),
        }
        if gv.is_file() {
            a.extend(["--game-variant".into(), gv.to_string_lossy().into_owned()]);
        }
        Ok(a)
    }

    /// Connect to a typed address: a row with that address if any list has one; otherwise assume a reach-host
    /// started with the default session id (map/variant come from the host's session).
    fn direct_args(&self, addr: &str) -> Result<Vec<String>, String> {
        let sa = query::resolve(addr)?;
        let all = self.all();
        if let Some(e) = all.iter().chain(&self.favs).find(|e| e.resolved == Some(sa) && !e.info.session_id.is_empty()) {
            return self.join_entry(e);
        }
        self.default_join(&sa.to_string())
    }

    fn default_join(&self, addr: &str) -> Result<Vec<String>, String> {
        let sa = query::resolve(addr)?;
        let mut a: Vec<String> = vec!["--net-role".into(), "client".into()];
        a.extend(joiner_identity());
        a.extend(["--players".into(), "2".into()]);
        a.extend(["--peer".into(), format!("0={sa}"), "--map".into(), "forge_world".into()]);
        Ok(a)
    }

    /// Whether JOIN is possible, else why not.
    fn join_block(e: &Entry) -> Option<&'static str> {
        if e.in_progress {
            Some("Mission in progress, wait for the next one")
        } else if e.full() {
            Some("Server is full")
        } else {
            None
        }
    }

    fn visible(&self, list: &[Entry]) -> Vec<usize> {
        let q = self.search.trim().to_lowercase();
        let mut v: Vec<usize> = (0..list.len())
            .filter(|&i| {
                let e = &list[i];
                let s = &e.info;
                (self.cat == 0 || e.cat() == self.cat)
                    && !(self.hide_full && e.full())
                    && !(self.hide_empty && s.players == 0)
                    && (self.max_ping == 0 || e.ping.is_some_and(|p| p <= self.max_ping))
                    && (!self.version_only || s.version == OUR_VERSION)
                    && (!self.playlist_only || !e.playlist.is_empty())
                    && (q.is_empty() || [&s.name, &e.key, &s.map, &map_title(&s.map), &e.mode, &e.playlist].iter().any(|t| t.to_lowercase().contains(&q)))
            })
            .collect();
        v.sort_by(|&x, &y| {
            let (x, y) = (&list[x], &list[y]);
            let o = match self.sort {
                SortCol::Name => x.info.name.to_lowercase().cmp(&y.info.name.to_lowercase()),
                SortCol::Mode => x.mode.cmp(&y.mode),
                SortCol::Map => map_title(&x.info.map).cmp(&map_title(&y.info.map)),
                SortCol::Players => x.info.players.cmp(&y.info.players),
                SortCol::Ping => x.ping.unwrap_or(u32::MAX).cmp(&y.ping.unwrap_or(u32::MAX)),
                SortCol::Version => x.info.version.cmp(&y.info.version),
            };
            if self.sort_desc { o.reverse() } else { o }
        });
        v
    }

    // ---- Drawing ---------------------------------------------------------------------------------------------

    pub fn ui(&mut self, ui: &mut Ui) -> Action {
        self.poll();
        let mut action = Action::None;
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            if self.direct_open {
                self.direct_open = false;
            } else {
                return Action::Back;
            }
        }
        let full = ui.max_rect();
        let area = Rect::from_min_max(full.min + vec2(48.0, 36.0), full.max - vec2(48.0, 32.0));

        // Title + BACK.
        let t = ui.painter().text(area.left_top(), Align2::LEFT_TOP, "SERVER BROWSER", FontId::proportional(40.0), egui::Color32::WHITE);
        ui.painter().line_segment([pos2(t.left(), t.bottom() + 4.0), pos2(t.left() + 56.0, t.bottom() + 4.0)], Stroke::new(3.0, ACCENT));
        let back = Rect::from_min_size(pos2(area.right() - 200.0, area.top()), vec2(200.0, 40.0));
        ui.scope_builder(UiBuilder::new().max_rect(back).layout(Layout::right_to_left(Align::Min)), |ui| {
            if theme::ghost_button(ui, "Back").clicked() {
                action = Action::Back;
            }
        });
        if !self.error.is_empty() {
            ui.painter().text(pos2(t.right() + 24.0, t.center().y), Align2::LEFT_CENTER, &self.error, FontId::proportional(15.0), Color32::from_rgb(255, 120, 90));
        }

        let all = self.all();
        let list: Vec<Entry> = if self.tab == Tab::All { all.clone() } else { self.favs.clone() };
        let top = t.bottom() + 22.0;
        let side_w = 300.0;
        let side = Rect::from_min_max(pos2(area.right() - side_w, top), area.max);
        let main = Rect::from_min_max(pos2(area.left(), top), pos2(side.left() - 20.0, area.bottom()));

        // Row 1: tabs + search.
        let tabs_h = 40.0;
        let mut new_tab = self.tab;
        ui.scope_builder(UiBuilder::new().max_rect(Rect::from_min_size(main.min, vec2(main.width(), tabs_h))).layout(Layout::left_to_right(Align::Center)), |ui| {
            for (label, t, n) in [("ALL SERVERS", Tab::All, all.len()), ("FAVORITES", Tab::Favourites, self.favs.len())] {
                if tab_button(ui, label, n, self.tab == t).clicked() {
                    new_tab = t;
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.add(egui::TextEdit::singleline(&mut self.search).hint_text("Search servers, hosts, maps, modes").desired_width(320.0).font(FontId::proportional(15.0)));
            });
        });
        if new_tab != self.tab {
            self.tab = new_tab;
            self.selected = None;
        }

        // Row 2: category chips + toggles + filters.
        let chips = Rect::from_min_size(pos2(main.left(), main.top() + tabs_h + 8.0), vec2(main.width(), 34.0));
        let counts: Vec<usize> = (0..CATS.len()).map(|c| if c == 0 { list.len() } else { list.iter().filter(|e| e.cat() == c).count() }).collect();
        ui.scope_builder(UiBuilder::new().max_rect(chips).layout(Layout::left_to_right(Align::Center)), |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            for (c, name) in CATS.iter().enumerate() {
                if chip(ui, name, Some(counts[c]), Some(c), self.cat == c).clicked() {
                    self.cat = c;
                }
            }
            ui.add_space(4.0);
            if chip(ui, "Hide full", None, None, self.hide_full).clicked() {
                self.hide_full = !self.hide_full;
            }
            if chip(ui, "Hide empty", None, None, self.hide_empty).clicked() {
                self.hide_empty = !self.hide_empty;
            }
            let active = (self.max_ping > 0) as usize + self.version_only as usize + self.playlist_only as usize;
            let flabel = if active > 0 { format!("Filters ({active})") } else { "Filters".to_string() };
            let f = chip(ui, &flabel, None, None, active > 0);
            egui::Popup::menu(&f).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
                ui.set_min_width(260.0);
                ui.label(egui::RichText::new("MAX PING").size(12.0).color(DIM));
                ui.horizontal(|ui| {
                    for &p in PING_STEPS {
                        ui.selectable_value(&mut self.max_ping, p, if p == 0 { "Any".to_string() } else { format!("{p}") });
                    }
                });
                ui.checkbox(&mut self.version_only, "Version match only");
                ui.add_enabled(false, egui::Checkbox::new(&mut false, "Has password (none yet)"));
                ui.checkbox(&mut self.playlist_only, "Playlist servers only");
                if ui.button("Clear filters").clicked() {
                    self.max_ping = 0;
                    self.version_only = false;
                    self.playlist_only = false;
                }
            });
        });

        let view = self.visible(&list);
        if self.select_first && !view.is_empty() {
            self.selected = Some(list[view[0]].key.clone());
            self.select_first = false;
        }
        if !self.selected.as_ref().is_some_and(|k| list.iter().any(|e| &e.key == k)) {
            self.selected = None;
        }

        // Table.
        let foot_h = 26.0;
        let table = Rect::from_min_max(pos2(main.left(), chips.bottom() + 10.0), pos2(main.right(), main.bottom() - foot_h));
        let (mut clicked, mut star, mut join_now) = (None, None, None);
        let pending = self.pending.is_some();
        let favs: Vec<bool> = list.iter().map(|e| self.is_favourite(e)).collect();
        ui.painter().rect_filled(table, 2.0, theme::PANEL);
        ui.painter().rect_stroke(table, 2.0, Stroke::new(1.0, LINE), egui::StrokeKind::Inside);
        let inner = table.shrink2(vec2(6.0, 4.0));
        let w = inner.width();
        // Column left edges: star, status dot, server, map, game type, players, ping, version.
        let xs = [0.0, 30.0, 52.0, 52.0 + (w - 52.0) * 0.27, 52.0 + (w - 52.0) * 0.42, 52.0 + (w - 52.0) * 0.60, 52.0 + (w - 52.0) * 0.72, 52.0 + (w - 52.0) * 0.86];
        ui.scope_builder(UiBuilder::new().max_rect(inner), |ui| {
            let (hr, _) = ui.allocate_exact_size(vec2(w, 28.0), Sense::hover());
            let heads: [(&str, Option<SortCol>); 8] = [("", None), ("", None), ("Server", Some(SortCol::Name)), ("Map", Some(SortCol::Map)), ("Game type", Some(SortCol::Mode)), ("Players", Some(SortCol::Players)), ("Ping", Some(SortCol::Ping)), ("Version", Some(SortCol::Version))];
            for (i, (label, col)) in heads.iter().enumerate() {
                let right = xs.get(i + 1).copied().unwrap_or(w);
                let r = Rect::from_min_max(pos2(hr.left() + xs[i], hr.top()), pos2(hr.left() + right, hr.bottom()));
                let mut color = DIM;
                let font = FontId::proportional(13.0);
                let text = label.to_uppercase();
                if let Some(c) = col {
                    let resp = ui.interact(r, ui.id().with(("sort", i)), Sense::click());
                    if resp.clicked() {
                        if self.sort == *c {
                            self.sort_desc = !self.sort_desc;
                        } else {
                            self.sort = *c;
                            self.sort_desc = matches!(c, SortCol::Players);
                        }
                    }
                    if self.sort == *c {
                        color = ACCENT;
                        let tw = ui.painter().layout_no_wrap(text.clone(), font.clone(), color).size().x;
                        let (x, y) = (r.left() + 4.0 + tw + 10.0 + if *c == SortCol::Players { 16.0 } else { 0.0 }, r.center().y);
                        let pts = if self.sort_desc { vec![pos2(x - 5.0, y - 3.0), pos2(x + 5.0, y - 3.0), pos2(x, y + 4.0)] } else { vec![pos2(x - 5.0, y + 4.0), pos2(x + 5.0, y + 4.0), pos2(x, y - 3.0)] };
                        ui.painter().add(egui::Shape::convex_polygon(pts, color, Stroke::NONE));
                    } else if resp.hovered() {
                        color = TEXT;
                    }
                    if *c == SortCol::Players {
                        draw_person(ui.painter(), pos2(r.left() + 4.0 + 6.0, r.center().y), 1.0, color);
                        ui.painter().text(pos2(r.left() + 20.0, r.center().y), Align2::LEFT_CENTER, text, font, color);
                        continue;
                    }
                }
                ui.painter().text(pos2(r.left() + 4.0, r.center().y), Align2::LEFT_CENTER, text, font, color);
            }
            ui.painter().line_segment([hr.left_bottom(), hr.right_bottom()], Stroke::new(1.0, LINE));
            egui::ScrollArea::vertical().id_salt("servers").auto_shrink([false, false]).show(ui, |ui| {
                for &i in &view {
                    let e = &list[i];
                    let s = &e.info;
                    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 36.0), Sense::click());
                    let sel = self.selected.as_deref() == Some(e.key.as_str());
                    if sel {
                        ui.painter().rect_filled(rect, 0.0, ACCENT.gamma_multiply(0.22));
                        ui.painter().rect_filled(Rect::from_min_size(rect.min, vec2(3.0, rect.height())), 0.0, ACCENT);
                    } else if resp.hovered() {
                        ui.painter().rect_filled(rect, 0.0, theme::HOVER);
                    }
                    let p = ui.painter().with_clip_rect(rect);
                    let cy = rect.center().y;
                    draw_star(&p, pos2(rect.left() + 15.0, cy), 8.0, favs[i], if favs[i] { ACCENT } else { DIM });
                    let dot = if !e.live { DIM } else if e.in_progress || e.full() { Color32::from_rgb(235, 170, 50) } else { Color32::from_rgb(80, 200, 110) };
                    p.circle_filled(pos2(rect.left() + 41.0, cy), 4.5, dot);
                    let cell = |n: usize, x_off: f32| Rect::from_min_max(pos2(rect.left() + xs[n] + x_off, rect.top()), pos2(rect.left() + xs.get(n + 1).copied().unwrap_or(w) - 8.0, rect.bottom()));
                    let name_col = if sel { Color32::WHITE } else { TEXT };
                    let mut elided: Vec<(Rect, String)> = Vec::new();
                    let mut put = |r: Rect, text: &str, color: Color32| {
                        if fit_text(&p, r, text, 15.0, color) {
                            elided.push((r, text.to_string()));
                        }
                    };
                    put(cell(2, 0.0), &s.name, name_col);
                    put(cell(3, 4.0), &if s.map.is_empty() { "—".to_string() } else { map_title(&s.map) }, name_col);
                    // Game type with a mode icon.
                    let mode_r = cell(4, 4.0);
                    draw_cat(&p, pos2(mode_r.left() + 8.0, cy), 7.0, e.cat(), DIM);
                    put(Rect::from_min_max(pos2(mode_r.left() + 22.0, mode_r.top()), mode_r.max), &e.mode, name_col);
                    let pr = cell(5, 4.0);
                    let players = if s.max_players == 0 { "?".to_string() } else { format!("{} / {}", s.players, s.max_players) };
                    put(pr, &players, name_col);
                    let pg = cell(6, 4.0);
                    draw_bars(&p, pos2(pg.left(), cy), e.ping);
                    let ping = match e.ping {
                        Some(ms) => format!("{ms} ms"),
                        None if pending => "…".into(),
                        None => "—".into(),
                    };
                    put(Rect::from_min_max(pos2(pg.left() + 24.0, pg.top()), pg.max), &ping, name_col);
                    let vc = if s.version.is_empty() { DIM } else if s.version != OUR_VERSION { Color32::from_rgb(235, 80, 70) } else { name_col };
                    put(cell(7, 4.0), if s.version.is_empty() { "—" } else { &s.version }, vc);
                    if resp.clicked() {
                        if resp.interact_pointer_pos().is_some_and(|pp| pp.x < rect.left() + xs[1]) {
                            star = Some(i);
                        }
                        clicked = Some(i);
                    }
                    if resp.double_clicked() {
                        join_now = Some(i);
                    }
                    if resp.hovered() {
                        if let Some(pp) = ui.input(|i| i.pointer.hover_pos()) {
                            if let Some((_, full_text)) = elided.iter().find(|(r, _)| r.x_range().contains(pp.x)) {
                                resp.on_hover_text(full_text.clone());
                            }
                        }
                    }
                }
                if view.is_empty() {
                    ui.add_space(24.0);
                    let msg = match (pending, self.tab) {
                        (true, _) => "Loading ...",
                        (false, Tab::Favourites) if list.is_empty() => "No favorites yet: star a server in ALL SERVERS, or use DIRECT CONNECT.",
                        _ if list.is_empty() => "No servers found. Press REFRESH to try again.",
                        _ => "No servers match the search or filters.",
                    };
                    ui.vertical_centered(|ui| ui.label(egui::RichText::new(msg).size(16.0).color(DIM)));
                }
            });
        });

        // Footer: refresh progress or the list summary.
        let foot = Rect::from_min_size(pos2(main.left(), table.bottom() + 4.0), vec2(main.width(), foot_h - 4.0));
        let foot_text = if pending {
            let t = self.total.load(Ordering::Relaxed);
            format!("Refreshing: {} of {} servers answered so far", self.answered.load(Ordering::Relaxed), if t == 0 && self.master.is_empty() { "…".to_string() } else { t.to_string() })
        } else {
            format!("{} of {} servers shown · {}", view.len(), list.len(), self.status)
        };
        ui.painter().with_clip_rect(foot).text(foot.left_center(), Align2::LEFT_CENTER, foot_text, FontId::proportional(14.0), DIM);

        // Apply table interactions.
        if let Some(i) = clicked {
            self.selected = Some(list[i].key.clone());
        }
        if let Some(i) = star {
            let e = list[i].clone();
            self.toggle_favourite(&e, favs[i]);
        }
        let mut join = join_now.map(|i| list[i].clone());

        // Side panel.
        let sel = self.selected.as_ref().and_then(|k| list.iter().find(|e| &e.key == k)).cloned();
                let (mut do_direct, mut do_refresh) = (false, false);
        let servers = all.len();
        let players: u32 = all.iter().map(|e| e.info.players).sum();
        theme::panel(ui, side, |ui| {
            let pr = ui.max_rect();
            ui.label(egui::RichText::new(format!("{servers} SERVER{}", if servers == 1 { "" } else { "S" })).size(30.0).color(ACCENT));
            ui.label(egui::RichText::new(format!("{players} PLAYERS ONLINE")).size(17.0).color(TEXT));
            ui.add_space(6.0);
            let (lr, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
            ui.painter().rect_filled(lr, 0.0, LINE);
            ui.add_space(8.0);
            let btn_h = 112.0;
            let body = Rect::from_min_max(ui.cursor().min, pos2(pr.right(), pr.bottom() - btn_h));
            ui.scope_builder(UiBuilder::new().max_rect(body), |ui| match &sel {
                None => {
                    ui.add_space(8.0);
                    ui.label(egui::RichText::new("Click a server to see its details.").size(15.0).color(DIM));
                    ui.label(egui::RichText::new("Double-click a server to join it.").size(15.0).color(DIM));
                    ui.label(egui::RichText::new("Click the star to add it to your favorites.").size(15.0).color(DIM));
                }
                Some(e) => {
                    egui::ScrollArea::vertical().id_salt("detail").auto_shrink([false, false]).show(ui, |ui| self.detail(ui, e));
                }
            });
            let btns = Rect::from_min_max(pos2(pr.left(), pr.bottom() - btn_h + 6.0), pr.max);
            ui.scope_builder(UiBuilder::new().max_rect(btns), |ui| {
                let block = sel.as_ref().and_then(Self::join_block);
                if let Some(b) = block {
                    ui.label(egui::RichText::new(b).size(13.0).color(Color32::from_rgb(255, 130, 100)));
                } else if sel.is_none() {
                    ui.label(egui::RichText::new("Select a server to join").size(13.0).color(DIM));
                } else {
                    ui.add_space(ui.text_style_height(&egui::TextStyle::Small) + 3.0);
                }
                if theme::primary_button(ui, "Join game", ui.available_width(), sel.is_some() && block.is_none()).clicked() && sel.is_some() && block.is_none() {
                    join = sel.clone();
                }
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    do_direct = theme::ghost_button(ui, "Direct connect").clicked();
                    do_refresh = theme::ghost_button(ui, if pending { "Refreshing…" } else { "Refresh" }).clicked();
                });
            });
        });
        if do_direct {
            self.direct_open = true;
            self.direct_err.clear();
        }
        if do_refresh && self.pending.is_none() {
            self.refresh();
        }
        if let Some(e) = join {
            match self.join_entry(&e) {
                Ok(a) => action = Action::Launch(a),
                Err(err) => self.error = err,
            }
        }
        if self.direct_open {
            if let Some(a) = self.direct_dialog(ui) {
                action = a;
            }
        }
        action
    }

    fn toggle_favourite(&mut self, e: &Entry, is_fav: bool) {
        if is_fav {
            self.remove_favourite(e);
        } else if let Err(err) = self.add_favourite(&e.key) {
            self.error = err;
        }
    }

    /// Selected-server details: map card, name, address, key facts.
    fn detail(&mut self, ui: &mut Ui, e: &Entry) {
        let s = &e.info;
        let w = ui.available_width();
        let (card, _) = ui.allocate_exact_size(vec2(w, w * 0.4), Sense::hover());
        ui.spacing_mut().item_spacing.y = 2.0;
        ui.spacing_mut().interact_size.y = 18.0;
        self.map_card(ui, card, &s.map);
        ui.add_space(4.0);
        ui.add(egui::Label::new(egui::RichText::new(&s.name).size(19.0).color(egui::Color32::WHITE)).truncate());
        ui.add(egui::Label::new(egui::RichText::new(&e.key).size(13.0).color(DIM)).truncate());
        ui.add_space(4.0);
        let cat = CATS[e.cat().min(CATS.len() - 1)];
        let kv = |ui: &mut Ui, k: &str, v: &str, color: Color32| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let (r, _) = ui.allocate_exact_size(vec2(78.0, 18.0), Sense::hover());
                ui.painter().text(r.left_center(), Align2::LEFT_CENTER, k.to_uppercase(), FontId::proportional(12.0), DIM);
                ui.add(egui::Label::new(egui::RichText::new(v).size(15.0).color(color)).truncate());
            });
        };
        kv(ui, "Mode", cat, TEXT);
        kv(ui, "Variant", &e.mode, TEXT);
        kv(ui, "Map", &if s.map.is_empty() { "—".to_string() } else { map_title(&s.map) }, TEXT);
        kv(ui, "Players", &if s.max_players > 0 { format!("{} / {}", s.players, s.max_players) } else { "?".into() }, TEXT);
        kv(ui, "Ping", &e.ping.map_or("—".to_string(), |p| format!("{p} ms")), TEXT);
        let vc = if !s.version.is_empty() && s.version != OUR_VERSION { Color32::from_rgb(235, 80, 70) } else { TEXT };
        kv(ui, "Version", &if s.version.is_empty() { "—".to_string() } else if s.version != OUR_VERSION { format!("{} (yours {OUR_VERSION})", s.version) } else { s.version.clone() }, vc);
        if !e.playlist.is_empty() {
            kv(ui, "Playlist", &e.playlist, ACCENT);
        }
        kv(ui, "Host", if s.dedicated { "Dedicated" } else { "Player host" }, TEXT);
        if !e.live {
            ui.add_space(2.0);
            ui.label(egui::RichText::new(if e.from_master { "Master data: no answer to the query" } else { "No answer from this address" }).size(12.0).color(DIM));
        }
    }

    /// The map's picture: the install's HaloReach.png tinted with a colour from the map name (the install has no
    /// per-map thumbnails outside its packed texture packs); a flat coloured card if that file is missing.
    fn map_card(&mut self, ui: &mut Ui, rect: Rect, map: &str) {
        if self.card.is_none() {
            let path = self.mcc.join("Data").join("UI").join("BackgroundVideos").join("Images").join("HaloReach.png");
            self.card = Some(load_png(&path).map(|img| ui.ctx().load_texture("map-card", img, egui::TextureOptions::LINEAR)));
        }
        let h = map.bytes().fold(7u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
        let hue = (h % 360) as f32 / 360.0;
        let tint: Color32 = egui::ecolor::Hsva::new(hue, 0.55, 0.85, 1.0).into();
        ui.painter().rect_filled(rect, 2.0, tint.gamma_multiply(0.35));
        if let Some(Some(tex)) = &self.card {
            ui.painter().image(tex.id(), rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), tint.gamma_multiply(0.75));
        }
        ui.painter().rect_stroke(rect, 2.0, Stroke::new(1.0, LINE), egui::StrokeKind::Inside);
        let title = if map.is_empty() { "UNKNOWN".to_string() } else { map_title(map).to_uppercase() };
        let bar = Rect::from_min_max(pos2(rect.left(), rect.bottom() - 30.0), rect.max);
        ui.painter().rect_filled(bar, 0.0, Color32::from_black_alpha(150));
        fit_text(&ui.painter().with_clip_rect(bar), bar.shrink2(vec2(8.0, 0.0)), &title, 17.0, egui::Color32::WHITE);
    }

    /// DIRECT CONNECT dialog. Returns the launch action when Connect succeeds.
    fn direct_dialog(&mut self, ui: &mut Ui) -> Option<Action> {
        let mut out = None;
        let (mut connect, mut fav, mut cancel) = (false, false, false);
        egui::Window::new("DIRECT CONNECT").anchor(Align2::CENTER_CENTER, [0.0, 0.0]).collapsible(false).resizable(false).fixed_size(vec2(420.0, 0.0)).show(ui.ctx(), |ui| {
            ui.label(egui::RichText::new("Server address (host:port)").size(13.0).color(DIM));
            let r = ui.add(egui::TextEdit::singleline(&mut self.direct).hint_text("tunnel.example.net:25195").desired_width(f32::INFINITY).font(FontId::proportional(17.0)));
            if !r.has_focus() && self.direct.is_empty() {
                r.request_focus();
            }
            if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                connect = true;
            }
            if !self.direct_err.is_empty() {
                ui.label(egui::RichText::new(&self.direct_err).size(13.0).color(Color32::from_rgb(255, 120, 90)));
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                connect |= theme::primary_button(ui, "Connect", 120.0, true).clicked();
                fav = theme::ghost_button(ui, "Add to favorites").clicked();
                cancel = theme::ghost_button(ui, "Cancel").clicked();
            });
        });
        if cancel {
            self.direct_open = false;
        }
        if fav {
            let a = self.direct.clone();
            match self.add_favourite(&a) {
                Ok(()) => {
                    self.direct_open = false;
                    self.direct.clear();
                    self.tab = Tab::Favourites;
                    self.selected = Some(a.trim().to_string());
                }
                Err(e) => self.direct_err = e,
            }
        }
        if connect {
            match self.direct_args(&self.direct) {
                Ok(a) => {
                    self.direct_open = false;
                    out = Some(Action::Launch(a));
                }
                Err(e) => self.direct_err = e,
            }
        }
        out
    }
}

/// `--instance <random 1..65535> --bind 0.0.0.0:0 --to-dedicated`, the late-join identity `--join-server` uses.
fn joiner_identity() -> [String; 5] {
    let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0) ^ std::process::id().rotate_left(16);
    ["--instance".into(), (1 + seed % 0xFFFF).to_string(), "--bind".into(), "0.0.0.0:0".into(), "--to-dedicated".into()]
}

fn load_png(path: &Path) -> Option<egui::ColorImage> {
    let mut dec = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).ok()?));
    dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut rd = dec.read_info().ok()?;
    let mut buf = vec![0; rd.output_buffer_size()?];
    let info = rd.next_frame(&mut buf).ok()?;
    let (w, h) = (info.width as usize, info.height as usize);
    let px = &buf[..info.buffer_size()];
    let rgba: Vec<u8> = match info.color_type {
        png::ColorType::Rgba => px.to_vec(),
        png::ColorType::Rgb => px.chunks(3).flat_map(|c| [c[0], c[1], c[2], 255]).collect(),
        png::ColorType::GrayscaleAlpha => px.chunks(2).flat_map(|c| [c[0], c[0], c[0], c[1]]).collect(),
        png::ColorType::Grayscale => px.iter().flat_map(|&c| [c, c, c, 255]).collect(),
        _ => return None,
    };
    Some(egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba))
}

/// Draw `text` on one line inside `r`, ending in an ellipsis when it does not fit. True if it was cut.
fn fit_text(p: &egui::Painter, r: Rect, text: &str, size: f32, color: Color32) -> bool {
    let mut job = egui::text::LayoutJob::simple(text.to_string(), FontId::proportional(size), color, r.width().max(1.0));
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    let g = p.layout_job(job);
    let elided = g.elided;
    p.galley(pos2(r.left(), r.center().y - g.size().y / 2.0), g, color);
    elided
}

/// Top tab: caption, count badge, accent underline when active.
fn tab_button(ui: &mut Ui, label: &str, n: usize, active: bool) -> egui::Response {
    let font = FontId::proportional(20.0);
    let g = ui.painter().layout_no_wrap(label.to_string(), font, if active { egui::Color32::WHITE } else { DIM });
    let badge = ui.painter().layout_no_wrap(n.to_string(), FontId::proportional(13.0), Color32::from_rgb(16, 12, 8));
    let size = vec2(g.size().x + badge.size().x + 38.0, 38.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let color = if active { egui::Color32::WHITE } else if resp.hovered() { TEXT } else { DIM };
    ui.painter().galley(pos2(rect.left() + 4.0, rect.center().y - g.size().y / 2.0), g.clone(), color);
    let br = Rect::from_center_size(pos2(rect.left() + 4.0 + g.size().x + 12.0 + badge.size().x / 2.0, rect.center().y), badge.size() + vec2(12.0, 4.0));
    ui.painter().rect_filled(br, 9.0, if active { ACCENT } else { DIM });
    ui.painter().galley(br.center() - badge.size() / 2.0, badge, Color32::BLACK);
    if active {
        ui.painter().rect_filled(Rect::from_min_max(pos2(rect.left(), rect.bottom() - 3.0), rect.right_bottom()), 0.0, ACCENT);
    }
    resp
}

/// Rounded filter chip with optional category icon and count.
fn chip(ui: &mut Ui, label: &str, count: Option<usize>, icon: Option<usize>, active: bool) -> egui::Response {
    let font = FontId::proportional(13.0);
    let text = label.to_uppercase();
    let g = ui.painter().layout_no_wrap(text, font.clone(), TEXT);
    let cg = count.map(|c| ui.painter().layout_no_wrap(c.to_string(), FontId::proportional(12.0), DIM));
    let icon_w = if icon.is_some() { 20.0 } else { 0.0 };
    let w = 16.0 + icon_w + g.size().x + cg.as_ref().map_or(0.0, |c| c.size().x + 10.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 32.0), Sense::click());
    let fill = if active { ACCENT.gamma_multiply(0.28) } else if resp.hovered() { theme::HOVER } else { Color32::from_rgba_premultiplied(8, 8, 8, 8) };
    ui.painter().rect_filled(rect, 16.0, fill);
    ui.painter().rect_stroke(rect, 16.0, Stroke::new(1.0, if active { ACCENT } else { DIM.gamma_multiply(0.5) }), egui::StrokeKind::Inside);
    let col = if active { egui::Color32::WHITE } else { TEXT };
    let mut x = rect.left() + 8.0;
    if let Some(c) = icon {
        draw_cat(ui.painter(), pos2(x + 7.0, rect.center().y), 7.0, c, if active { ACCENT } else { DIM });
        x += icon_w;
    }
    ui.painter().galley(pos2(x, rect.center().y - g.size().y / 2.0), g.clone(), col);
    if let Some(cg) = cg {
        let cx = x + g.size().x + 8.0;
        ui.painter().galley(pos2(cx, rect.center().y - cg.size().y / 2.0), cg, DIM);
    }
    resp
}

/// Category icon: all (grid), multiplayer (crosshair), firefight (flame), campaign (shield), forge (crossed bars).
fn draw_cat(p: &egui::Painter, c: egui::Pos2, s: f32, cat: usize, col: Color32) {
    let st = Stroke::new(1.5, col);
    match cat {
        0 => {
            for (dx, dy) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                p.rect_filled(Rect::from_center_size(c + vec2(dx, dy) * s * 0.5, vec2(s * 0.8, s * 0.8)), 1.0, col);
            }
        }
        1 => {
            p.circle_stroke(c, s * 0.7, st);
            for (dx, dy) in [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)] {
                p.line_segment([c + vec2(dx, dy) * s * 0.4, c + vec2(dx, dy) * s * 1.1], st);
            }
        }
        2 => {
            let pts = vec![c + vec2(0.0, -s * 1.1), c + vec2(s * 0.75, s * 0.25), c + vec2(0.0, s), c + vec2(-s * 0.75, s * 0.25)];
            p.add(egui::Shape::convex_polygon(pts, col, Stroke::NONE));
        }
        3 => {
            let pts = vec![c + vec2(-s * 0.85, -s * 0.9), c + vec2(s * 0.85, -s * 0.9), c + vec2(s * 0.85, s * 0.1), c + vec2(0.0, s * 1.05), c + vec2(-s * 0.85, s * 0.1)];
            p.add(egui::Shape::convex_polygon(pts, col, Stroke::NONE));
        }
        _ => {
            p.line_segment([c + vec2(-s, -s), c + vec2(s, s)], Stroke::new(2.5, col));
            p.line_segment([c + vec2(s, -s), c + vec2(-s, s)], Stroke::new(2.5, col));
        }
    }
}

/// Five-point star, filled (favorite) or outlined.
fn draw_star(p: &egui::Painter, c: egui::Pos2, r: f32, filled: bool, col: Color32) {
    let pts: Vec<egui::Pos2> = (0..10)
        .map(|i| {
            let a = -std::f32::consts::FRAC_PI_2 + i as f32 * std::f32::consts::PI / 5.0;
            let rr = if i % 2 == 0 { r } else { r * 0.42 };
            c + vec2(a.cos(), a.sin()) * rr
        })
        .collect();
    if filled {
        let mut m = egui::Mesh::default();
        m.colored_vertex(c, col);
        for q in &pts {
            m.colored_vertex(*q, col);
        }
        for i in 0..10u32 {
            m.add_triangle(0, 1 + i, 1 + (i + 1) % 10);
        }
        p.add(m);
    } else {
        p.add(egui::Shape::closed_line(pts, Stroke::new(1.3, col)));
    }
}

/// Person icon (head + shoulders).
fn draw_person(p: &egui::Painter, c: egui::Pos2, k: f32, col: Color32) {
    p.circle_filled(c + vec2(0.0, -3.0 * k), 2.8 * k, col);
    p.rect_filled(Rect::from_center_size(c + vec2(0.0, 3.5 * k), vec2(8.0 * k, 4.5 * k)), egui::CornerRadius { nw: 4, ne: 4, sw: 0, se: 0 }, col);
}

/// Four signal bars, lit by ping (green best, red worst); unknown ping = all dim.
fn draw_bars(p: &egui::Painter, left_center: egui::Pos2, ping: Option<u32>) {
    let (lit, color) = match ping {
        None => (0, DIM),
        Some(ms) if ms < 50 => (4, Color32::from_rgb(80, 200, 110)),
        Some(ms) if ms < 100 => (3, Color32::from_rgb(160, 210, 80)),
        Some(ms) if ms < 180 => (2, Color32::from_rgb(235, 170, 50)),
        Some(_) => (1, Color32::from_rgb(235, 80, 70)),
    };
    for i in 0..4 {
        let h = 4.0 + i as f32 * 3.5;
        let x = left_center.x + i as f32 * 5.0;
        let r = Rect::from_min_max(pos2(x, left_center.y + 8.0 - h), pos2(x + 3.5, left_center.y + 8.0));
        p.rect_filled(r, 0.5, if i < lit { color } else { DIM.gamma_multiply(0.35) });
    }
}
