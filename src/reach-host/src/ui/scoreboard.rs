//! Tab / Back scoreboard. The engine's own in-game scoreboard screen never opens under our host (notes/frontend.md
//! "Scoreboard"), so while the engine's "show scores" action is held we draw our own from the engine's player table.
//! Addresses are for the frozen DLL (notes/frontend.md).

use std::sync::atomic::{AtomicUsize, Ordering};

use crate::log;

/// Engine action "show scores" (Tab / pad Back by default) in player 0's action records.
const ACTION_SCORES: usize = 0x3b;
/// Player 0's action records (`UpdatePlayerInputActions`): keyboard/mouse action a at +0x224 + a*0x10; pad actions
/// via the index table at +0x100 (u32 per action) into records at +0x00 + i*0x10. Record +2 = u8 held frames.
const RVA_ACTIONS: usize = 0x287ff94;
/// Last device per local player (u32: 0 pad, 1 keyboard/mouse).
const RVA_LAST_DEVICE: usize = 0x2dfbe74;

/// Engine TLS game-state block of the game thread (captured from slot 36, read from the render thread).
static GAME_TLS: AtomicUsize = AtomicUsize::new(0);
/// Listen server: the host's player has had a unit this game (starts the address banner once).
static SPAWNED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// Medal names read for this game (stats.rs, once the map is up).
static MEDALS_NAMED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Between two games in one process (seamless.rs): the next engine has a new game thread (TLS block).
pub fn reset() {
    GAME_TLS.store(0, Ordering::Relaxed);
    SPAWNED.store(false, Ordering::Relaxed);
    MEDALS_NAMED.store(false, Ordering::Relaxed);
}

/// Called on the game thread every input poll (host slot 36, player 0).
pub unsafe fn game_tick() {
    let base = crate::crash::ENGINE_BASE.load(Ordering::Relaxed);
    if base == 0 {
        return;
    }
    let block = GAME_TLS.load(Ordering::Relaxed);
    if block != 0 {
        // The players table has someone: the map's tags are loaded, so the medal block can be named.
        if !MEDALS_NAMED.load(Ordering::Relaxed) && !board().rows.is_empty() {
            MEDALS_NAMED.store(true, Ordering::Relaxed);
            crate::stats::init_medal_names(base);
        }
        crate::stats::debug_tick(base, block);
        // Listen server: the address banner starts when the host's player first has a unit.
        if !SPAWNED.load(Ordering::Relaxed) && crate::input::player0_view().is_some_and(|v| v.unit != u32::MAX && v.unit >> 16 != 0) {
            SPAWNED.store(true, Ordering::Relaxed);
            super::hostinfo::note_spawned();
        }
        return;
    }
    let idx = ((base + 0xc17b18) as *const u32).read() as usize;
    let tls: *const *const u8;
    core::arch::asm!("mov {}, gs:[0x58]", out(reg) tls);
    if tls.is_null() {
        return;
    }
    let block = *tls.add(idx);
    if !block.is_null() {
        GAME_TLS.store(block as usize, Ordering::Relaxed);
        log!("scoreboard: game-thread state block {block:?}");
    }
}

/// True while player 0 holds the engine's "show scores" action (respects key remapping and the pad layout).
pub fn held() -> bool {
    let base = crate::crash::ENGINE_BASE.load(Ordering::Relaxed);
    if base == 0 {
        return false;
    }
    unsafe {
        let rec = base + RVA_ACTIONS;
        let kbm = ((base + RVA_LAST_DEVICE) as *const u32).read_volatile() == 1;
        let r = if kbm {
            rec + 0x224 + ACTION_SCORES * 0x10
        } else {
            let i = ((rec + 0x100 + ACTION_SCORES * 4) as *const i32).read_volatile();
            if !(0..0x10).contains(&i) {
                return false;
            }
            rec + i as usize * 0x10
        };
        ((r + 2) as *const u8).read_volatile() != 0
    }
}

/// `REACH_SB_DUMP=1`: on each press, write the TLS game-state block (0x400 bytes), ±7 MB of game state around
/// the game-engine globals and the DLL's .data to `sb-dump-N.bin` (layout discovery; see notes). Format:
/// segments of [u64 address][u64 length][bytes], clamped to committed memory (VirtualQuery).
pub fn maybe_dump() {
    static LAST: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    static N: AtomicUsize = AtomicUsize::new(0);
    let h = held();
    if LAST.swap(h, Ordering::Relaxed) == h || !h || std::env::var_os("REACH_SB_DUMP").is_none() {
        return;
    }
    let base = crate::crash::ENGINE_BASE.load(Ordering::Relaxed);
    let block = GAME_TLS.load(Ordering::Relaxed);
    if base == 0 || block == 0 {
        return;
    }
    unsafe {
        use windows::Win32::System::Memory::{VirtualQuery, MEMORY_BASIC_INFORMATION, MEM_COMMIT, PAGE_GUARD, PAGE_NOACCESS};
        let arr = *((block + 0x18) as *const usize);
        let data = if arr != 0 { *((arr + 0x50) as *const usize) } else { 0 };
        let g = *((block + 0x20) as *const usize);
        // The game state is one huge allocation (several VirtualQuery regions): the TLS globals lie within ±7 MB of
        // the engine globals.
        let clamp = |(a, len): (usize, usize), lo: usize, hi: usize| (a.max(lo), (a + len).min(hi) - a.max(lo));
        let (lo, hi) = (g.saturating_sub(0x700000), g + 0x700000);
        let mut regions: Vec<(usize, usize)> = vec![(block, 0x400)];
        // Committed memory in [lo, hi): game state around the engine globals, then the DLL's .data (0x180afa000..).
        let mut walk = |lo: usize, hi: usize| {
            let mut a = lo & !0xfff;
            while a < hi {
                let mut mbi = MEMORY_BASIC_INFORMATION::default();
                if VirtualQuery(Some(a as *const _), &mut mbi, std::mem::size_of::<MEMORY_BASIC_INFORMATION>()) == 0 {
                    break;
                }
                let r = (mbi.BaseAddress as usize, mbi.RegionSize);
                if mbi.State == MEM_COMMIT && (mbi.Protect.0 & (PAGE_GUARD.0 | PAGE_NOACCESS.0)) == 0 {
                    regions.push(clamp(r, lo, hi));
                }
                a = r.0 + r.1;
            }
        };
        walk(lo, hi);
        walk(base + 0xafa000, base + 0x4e54000);
        let mut out = Vec::new();
        for &(a, len) in &regions {
            out.extend_from_slice(&(a as u64).to_le_bytes());
            out.extend_from_slice(&(len as u64).to_le_bytes());
            out.extend_from_slice(std::slice::from_raw_parts(a as *const u8, len));
        }
        let n = N.fetch_add(1, Ordering::Relaxed);
        let p = std::env::current_exe().unwrap_or_default().with_file_name(format!("sb-dump-{n}.bin"));
        let _ = std::fs::write(&p, &out);
        let list: Vec<String> = regions.iter().map(|(a, l)| format!("{a:#x}+{l:#x}")).collect();
        log!("scoreboard: dumped {} bytes to {} (players data {data:#x}, engine globals {g:#x}; regions {})", out.len(), p.display(), list.join(" "));
    }
}

/// One scoreboard line.
pub struct Row {
    pub name: String,
    pub xuid: u64,
    /// Team index (datum +0xAD), -1 in free-for-all.
    pub team: i8,
    pub score: i32,
    pub kills: i32,
    pub deaths: i32,
    pub assists: i32,
    /// Round trip to the host in ms (ping.rs), None until the first table arrives (or offline).
    pub ping: Option<u16>,
    pub local: bool,
}

/// Kills / deaths / assists come from the engine's live game results (stats.rs, notes/frontend.md "Game statistics").
use crate::stats::{STAT_ASSISTS, STAT_DEATHS, STAT_KILLS};

/// What the panel shows.
pub struct Board {
    pub rows: Vec<Row>,
    /// Team game (game-engine globals +0 bit 3, as the engine's team-score getter 0x180079f1c checks).
    pub teams: bool,
    /// Team scores (s16 at the score table +0x80 + team * 8, +2 for the round score), teams 0..7.
    pub team_scores: [i32; 8],
    /// Firefight game (score = the metagame score, stats.rs `firefight_score`).
    pub firefight: bool,
}

/// Engine player table (TLS +0x18 data array, datum 0x490 bytes at +0x50; salt u16 at +0x00 non-zero = in use):
/// +0x08 u64 xuid, +0xAD i8 team (-1 none), +0xB0 UTF-16 name[16]. Score (as the HUD's `#local_player_score`,
/// 0x180079b20): game-engine globals TLS +0x20, valid while +0x841C is 1..4, s16 at +0x3270 + index*8
/// (+2 when TLS +0x48 → +0x1EAB8 is set: round score). Kills/deaths/assists: game results (above), by xuid.
pub fn board() -> Board {
    let mut b = Board { rows: Vec::new(), teams: false, team_scores: [0; 8], firefight: false };
    let block = GAME_TLS.load(Ordering::Relaxed);
    let base = crate::crash::ENGINE_BASE.load(Ordering::Relaxed);
    if block == 0 || base == 0 {
        return b;
    }
    unsafe {
        let rd = |a: usize| (a as *const usize).read_volatile();
        let arr = rd(block + 0x18);
        if arr == 0 {
            return b;
        }
        let data = rd(arr + 0x50);
        let g = rd(block + 0x20);
        let opts = rd(block + 0x48);
        let scores = if g != 0 && (1..=4).contains(&((g + 0x841c) as *const i32).read_volatile()) {
            let round = opts != 0 && ((opts + 0x1eab8) as *const u8).read_volatile() != 0;
            Some(g + 0x3270 + if round { 2 } else { 0 })
        } else {
            None
        };
        b.teams = g != 0 && ((g as *const u8).read_volatile() & 8) != 0;
        if let (Some(s), true) = (scores, b.teams) {
            for t in 0..8 {
                b.team_scores[t] = ((s + 0x80 + t * 8) as *const i16).read_volatile() as i32;
            }
        }
        b.firefight = crate::stats::options_mode(block) == Some(crate::stats::MODE_SURVIVAL);
        let stats_of = |xuid: u64| crate::stats::block_of(base, xuid);
        let me = crate::host::LOCAL_XUID.load(Ordering::Relaxed);
        let host_counts = crate::ping::host_counts();
        for i in 0..16usize {
            let p = data + i * 0x490;
            if data == 0 || ((p) as *const u16).read_volatile() == 0 {
                continue;
            }
            let name: Vec<u16> = (0..16).map(|k| ((p + 0xb0 + 2 * k) as *const u16).read_volatile()).take_while(|&c| c != 0).collect();
            let xuid = ((p + 0x08) as *const u64).read_unaligned();
            let st = stats_of(xuid);
            let stat = |n: usize| st.map_or(0, |a| crate::stats::word(a, n));
            let net = crate::ping::entry(xuid);
            // A client's own statistics miss deaths it did not see: the host's counts win when it sends them.
            let host = net.filter(|e| host_counts && e.kills != crate::ping::NO_COUNTS);
            b.rows.push(Row {
                name: String::from_utf16_lossy(&name),
                xuid,
                team: ((p + 0xad) as *const i8).read_volatile(),
                // Firefight keeps the score table at 0: its score is the campaign metagame's (HUD survival score).
                score: if b.firefight {
                    crate::stats::firefight_score(block, xuid).unwrap_or(0)
                } else {
                    scores.map_or(0, |s| ((s + i * 8) as *const i16).read_volatile() as i32)
                },
                kills: host.map_or(stat(STAT_KILLS), |e| e.kills as i32),
                deaths: host.map_or(stat(STAT_DEATHS), |e| e.deaths as i32),
                assists: host.map_or(stat(STAT_ASSISTS), |e| e.assists as i32),
                ping: net.and_then(|e| e.ms),
                local: xuid == me,
            });
        }
    }
    b.rows.sort_by(|a, b| b.score.cmp(&a.score).then(b.kills.cmp(&a.kills)).then(a.deaths.cmp(&b.deaths)).then(a.name.cmp(&b.name)));
    b
}

/// Host (ping.rs thread): (xuid, kills, deaths, assists) of every player, from this machine's game results.
pub fn stats() -> Vec<(u64, i16, i16, i16)> {
    board().rows.iter().map(|r| (r.xuid, r.kills as i16, r.deaths as i16, r.assists as i16)).collect()
}

/// Players in the game, best first (hostinfo's join toast uses the names).
pub fn rows() -> Vec<Row> {
    board().rows
}

/// Team colours (Reach's red, blue, green, orange, purple, gold, brown, pink).
pub(super) fn team_colour(t: i8) -> egui::Color32 {
    const C: [(u8, u8, u8); 8] = [(200, 50, 50), (50, 100, 210), (60, 160, 60), (230, 130, 30), (140, 70, 190), (210, 180, 40), (130, 90, 50), (220, 110, 170)];
    match C.get(t as usize) {
        Some(&(r, g, b)) if t >= 0 => egui::Color32::from_rgb(r, g, b),
        _ => egui::Color32::from_gray(90),
    }
}

pub(super) const TEAM_NAMES: [&str; 8] = ["RED", "BLUE", "GREEN", "ORANGE", "PURPLE", "GOLD", "BROWN", "PINK"];

static LOGGED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// Interactive (mouse) mode: right-click while Tab is held. The UI then owns the mouse (cursor freed, game input
/// withheld) and the board stays up until Esc, Tab or a right-click.
static PINNED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static LAST_RMB: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// Open "≡" menu: player name and where it opened.
static MENU: std::sync::Mutex<Option<(String, egui::Pos2)>> = std::sync::Mutex::new(None);
/// Last menu action's result line and when it was set.
static NOTICE: std::sync::Mutex<Option<(String, std::time::Instant)>> = std::sync::Mutex::new(None);

fn unpin() {
    if PINNED.swap(false, Ordering::Relaxed) {
        *MENU.lock().unwrap_or_else(|e| e.into_inner()) = None;
        super::post_ui(super::OWN_SCOREBOARD, false);
        log!("scoreboard: mouse mode off");
    }
}

/// Should the overlay draw it this frame (and is there anything to draw)? Also turns mouse mode on: a right-click
/// (game input, so the engine sees it as one zoom press) while Tab is held.
pub fn visible() -> bool {
    if GAME_TLS.load(Ordering::Relaxed) == 0 {
        unpin();
        return false;
    }
    let h = held();
    let rmb = crate::input::mouse_buttons() & 4 != 0;
    if LAST_RMB.swap(rmb, Ordering::Relaxed) != rmb && rmb && h && !PINNED.load(Ordering::Relaxed) && !super::active() {
        PINNED.store(true, Ordering::Relaxed);
        super::post_ui(super::OWN_SCOREBOARD, true);
        log!("scoreboard: mouse mode on (right-click while Tab held)");
    }
    PINNED.load(Ordering::Relaxed) || h
}

/// Copy `text` to the Windows clipboard (CF_UNICODETEXT).
fn copy_to_clipboard(text: &str) -> bool {
    #[link(name = "user32")]
    unsafe extern "system" {
        fn OpenClipboard(hwnd: *mut core::ffi::c_void) -> i32;
        fn EmptyClipboard() -> i32;
        fn SetClipboardData(format: u32, mem: *mut core::ffi::c_void) -> *mut core::ffi::c_void;
        fn CloseClipboard() -> i32;
    }
    use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
    const CF_UNICODETEXT: u32 = 13;
    let w: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
    unsafe {
        let Ok(mem) = GlobalAlloc(GMEM_MOVEABLE, w.len() * 2) else { return false };
        let p = GlobalLock(mem) as *mut u16;
        if p.is_null() {
            return false;
        }
        std::ptr::copy_nonoverlapping(w.as_ptr(), p, w.len());
        let _ = GlobalUnlock(mem);
        if OpenClipboard(crate::host::WINDOW.load(Ordering::Relaxed) as _) == 0 {
            return false;
        }
        EmptyClipboard();
        let ok = !SetClipboardData(CF_UNICODETEXT, mem.0).is_null();
        CloseClipboard();
        ok
    }
}

fn notice(text: String) {
    log!("scoreboard: {text}");
    *NOTICE.lock().unwrap_or_else(|e| e.into_inner()) = Some((text, std::time::Instant::now()));
}

/// Ping chip: dark rounded box, 4 signal bars and "53 ms"; green, amber from 100 ms, red from 180 ms.
fn ping_chip(p: &egui::Painter, rect: egui::Rect, ms: Option<u16>) {
    p.rect_filled(rect, 4.0, egui::Color32::from_rgba_unmultiplied(0, 0, 0, 150));
    let (col, bars) = match ms {
        Some(m) if m < 100 => (egui::Color32::from_rgb(90, 200, 90), 4),
        Some(m) if m < 180 => (egui::Color32::from_rgb(230, 170, 50), 3),
        Some(m) if m < 300 => (egui::Color32::from_rgb(220, 70, 60), 2),
        Some(_) => (egui::Color32::from_rgb(220, 70, 60), 1),
        None => (super::theme::DIM, 0),
    };
    let x0 = rect.left() + 8.0;
    let bottom = rect.bottom() - 6.0;
    for k in 0..4 {
        let h = 4.0 + 3.0 * k as f32;
        let r = egui::Rect::from_min_max(egui::pos2(x0 + k as f32 * 4.0, bottom - h), egui::pos2(x0 + k as f32 * 4.0 + 2.5, bottom));
        p.rect_filled(r, 0.5, if k < bars { col } else { egui::Color32::from_gray(70) });
    }
    let text = ms.map_or("--".to_string(), |m| format!("{m} ms"));
    p.text(egui::pos2(rect.right() - 8.0, rect.center().y), egui::Align2::RIGHT_CENTER, text, egui::FontId::proportional(14.0), col);
}

/// Columns, right to left from the menu button: x centres relative to the row's right edge.
const W_MENU: f32 = 34.0;
const W_PING: f32 = 88.0;
const W_NUM: f32 = 76.0;
const ROW_H: f32 = 32.0;
const PANEL_W: f32 = 820.0;

/// Centre x of numeric column `k` (0 score, 1 kills, 2 deaths, 3 assists).
fn num_x(right: f32, k: usize) -> f32 {
    right - W_MENU - W_PING - 8.0 - W_NUM * (3 - k) as f32 - W_NUM / 2.0
}

/// `theme::panel_fit`, centred on `top.x` (panel_fit's Align::Center puts the panel's centre a quarter width right).
pub(super) fn fit_centered<R>(ui: &mut egui::Ui, top: egui::Pos2, width: f32, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    use super::theme;
    const PAD: f32 = 22.0;
    let bg = ui.painter().add(egui::Shape::Noop);
    let left = top.x - width / 2.0;
    let max = egui::Rect::from_min_size(egui::pos2(left + PAD, top.y + PAD), egui::vec2(width - 2.0 * PAD, 10_000.0));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(max));
    let inner = add(&mut child);
    let r = egui::Rect::from_min_size(egui::pos2(left, top.y), egui::vec2(width, child.min_rect().height() + 2.0 * PAD));
    ui.painter().set(bg, egui::Shape::Vec(vec![
        egui::Shape::rect_filled(r, 2.0, theme::PANEL),
        egui::Shape::rect_stroke(r, 2.0, egui::Stroke::new(1.0, theme::LINE), egui::StrokeKind::Inside),
    ]));
    inner
}

/// Centered panel: title with players / max, column headers, team blocks (or one list), ping chips, "≡" menus.
/// Logged once per press.
pub fn draw(ui: &mut egui::Ui) {
    use super::theme;
    let b = board();
    let pinned = PINNED.load(Ordering::Relaxed);
    if !LOGGED.swap(true, Ordering::Relaxed) {
        let list: Vec<String> = b.rows.iter()
            .map(|r| format!("{} (team {}) score {} K {} D {} A {} ping {}", r.name, r.team, r.score, r.kills, r.deaths, r.assists, r.ping.map_or("-".into(), |m| format!("{m} ms"))))
            .collect();
        let teams = if b.teams { format!(", team scores {:?}", &b.team_scores[..4]) } else { String::new() };
        log!("scoreboard: shown, {} players{teams}: {}", b.rows.len(), list.join(", "));
    }
    // Mouse mode: Esc / Tab / right-click closes; a click outside an open menu closes the menu.
    let (esc, tab, rclick) = ui.input(|i| (i.key_pressed(egui::Key::Escape), i.key_pressed(egui::Key::Tab), i.pointer.secondary_clicked()));
    if pinned && (esc || tab || rclick) {
        unpin();
    }
    let max = crate::ping::MAX_PLAYERS.load(Ordering::Relaxed).max(b.rows.len() as u32);
    let full = ui.max_rect();
    let mut menu_req: Option<(String, egui::Pos2)> = None;
    fit_centered(ui, egui::pos2(full.center().x, full.top() + 70.0), PANEL_W, |ui| {
        theme::heading(ui, &format!("Scoreboard ({}/{max})", b.rows.len()));
        // Column headers.
        let (hr, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 20.0), egui::Sense::hover());
        let small = egui::FontId::proportional(13.0);
        let p = ui.painter();
        p.text(egui::pos2(hr.left() + 14.0, hr.center().y), egui::Align2::LEFT_CENTER, "PLAYER", small.clone(), theme::DIM);
        for (k, h) in ["SCORE", "KILLS", "DEATHS", "ASSISTS"].iter().enumerate() {
            p.text(egui::pos2(num_x(hr.right(), k), hr.center().y), egui::Align2::CENTER_CENTER, *h, small.clone(), theme::DIM);
        }
        p.text(egui::pos2(hr.right() - W_MENU - W_PING / 2.0, hr.center().y), egui::Align2::CENTER_CENTER, "PING", small, theme::DIM);
        ui.add_space(4.0);
        if b.rows.is_empty() {
            ui.label(egui::RichText::new("No players yet").color(theme::DIM));
        }
        // Groups: one per team in a team game (best team first), else everyone.
        let mut groups: Vec<(Option<i8>, Vec<&Row>)> = Vec::new();
        if b.teams {
            let mut ts: Vec<i8> = b.rows.iter().map(|r| r.team).filter(|&t| (0..8).contains(&t)).collect();
            ts.sort();
            ts.dedup();
            ts.sort_by_key(|&t| std::cmp::Reverse(b.team_scores[t as usize]));
            for t in ts {
                groups.push((Some(t), b.rows.iter().filter(|r| r.team == t).collect()));
            }
            let rest: Vec<&Row> = b.rows.iter().filter(|r| !(0..8).contains(&r.team)).collect();
            if !rest.is_empty() {
                groups.push((None, rest));
            }
        } else {
            groups.push((None, b.rows.iter().collect()));
        }
        for (gi, (team, rows)) in groups.iter().enumerate() {
            if gi > 0 {
                ui.add_space(10.0);
            }
            if let Some(t) = *team {
                let (r, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 26.0), egui::Sense::hover());
                let c = team_colour(t);
                let font = egui::FontId::proportional(17.0);
                ui.painter().text(egui::pos2(r.left() + 14.0, r.center().y), egui::Align2::LEFT_CENTER, format!("{} TEAM ({})", TEAM_NAMES[t as usize], rows.len()), font.clone(), c);
                ui.painter().text(egui::pos2(num_x(r.right(), 0), r.center().y), egui::Align2::CENTER_CENTER, b.team_scores[t as usize].to_string(), font, c);
                ui.add_space(2.0);
            }
            for row in rows {
                let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), ROW_H), egui::Sense::hover());
                let p = ui.painter();
                let fill = match team {
                    Some(t) => team_colour(*t).gamma_multiply(0.85),
                    None => egui::Color32::from_rgba_unmultiplied(60, 66, 76, 200),
                };
                p.rect_filled(rect, 2.0, fill);
                if row.local {
                    p.rect_stroke(rect.shrink(1.0), 2.0, egui::Stroke::new(2.0, egui::Color32::from_rgb(232, 190, 70)), egui::StrokeKind::Inside);
                }
                let y = rect.center().y;
                let font = egui::FontId::proportional(18.0);
                p.text(egui::pos2(rect.left() + 14.0, y), egui::Align2::LEFT_CENTER, &row.name, font.clone(), theme::TEXT);
                for (k, v) in [row.score, row.kills, row.deaths, row.assists].iter().enumerate() {
                    p.text(egui::pos2(num_x(rect.right(), k), y), egui::Align2::CENTER_CENTER, v.to_string(), font.clone(), theme::TEXT);
                }
                let chip = egui::Rect::from_center_size(egui::pos2(rect.right() - W_MENU - W_PING / 2.0, y), egui::vec2(W_PING - 10.0, ROW_H - 8.0));
                ping_chip(p, chip, row.ping);
                if row.local {
                    ui.add_space(2.0);
                    continue;
                }
                // "≡" menu button (drawn as three bars; the default font has no ≡).
                let mb = egui::Rect::from_center_size(egui::pos2(rect.right() - W_MENU / 2.0 - 2.0, y), egui::vec2(26.0, ROW_H - 8.0));
                let resp = ui.interact(mb, ui.id().with(("sb-menu", row.xuid)), egui::Sense::click());
                let p = ui.painter();
                let hot = pinned && resp.hovered();
                p.rect_filled(mb, 4.0, egui::Color32::from_rgba_unmultiplied(0, 0, 0, if hot { 200 } else { 130 }));
                for k in 0..3 {
                    let yy = mb.center().y - 5.0 + 5.0 * k as f32;
                    p.line_segment([egui::pos2(mb.center().x - 6.0, yy), egui::pos2(mb.center().x + 6.0, yy)], egui::Stroke::new(1.6, if pinned { theme::TEXT } else { theme::DIM }));
                }
                if pinned && resp.clicked() {
                    menu_req = Some((row.name.clone(), mb.right_bottom()));
                }
                // Test hook: `REACH_SB_MENU` (any value) opens the first other player's menu as soon as mouse mode is on.
                if pinned && std::env::var_os("REACH_SB_MENU").is_some() && MENU.lock().unwrap_or_else(|e| e.into_inner()).is_none() && menu_req.is_none() {
                    menu_req = Some((row.name.clone(), mb.right_bottom()));
                }
                ui.add_space(2.0);
            }
        }
        ui.add_space(8.0);
        let note = NOTICE.lock().unwrap_or_else(|e| e.into_inner()).clone().filter(|(_, at)| at.elapsed().as_secs() < 4);
        let hint = match (&note, pinned) {
            (Some((t, _)), _) => t.clone(),
            (None, true) => "Esc or Tab to close".into(),
            (None, false) => "Right-click to use the mouse".into(),
        };
        ui.label(egui::RichText::new(hint).size(13.0).color(theme::DIM));
    });
    let opened_now = menu_req.is_some();
    if let Some(m) = menu_req {
        log!("scoreboard: menu for {}", m.0);
        *MENU.lock().unwrap_or_else(|e| e.into_inner()) = Some(m);
    }
    if !pinned {
        return;
    }
    let open = MENU.lock().unwrap_or_else(|e| e.into_inner()).clone();
    if let Some((name, at)) = open {
        let mut close = false;
        let r = egui::Rect::from_min_size(egui::pos2(at.x - 180.0, at.y + 2.0), egui::vec2(180.0, 100.0));
        let area = egui::Area::new(egui::Id::new("sb-player-menu")).fixed_pos(r.min).order(egui::Order::Foreground);
        let resp = area.show(ui.ctx(), |ui| {
            ui.painter().rect_filled(r, 3.0, egui::Color32::from_rgba_unmultiplied(14, 16, 20, 245));
            ui.painter().rect_stroke(r, 3.0, egui::Stroke::new(1.0, theme::DIM.gamma_multiply(0.5)), egui::StrokeKind::Inside);
            ui.scope_builder(egui::UiBuilder::new().max_rect(r.shrink(12.0)), |ui| {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 8.0;
                    // Test hook: `REACH_SB_MENU=kick|copy` presses that item once.
                    let auto = std::env::var("REACH_SB_MENU").unwrap_or_default();
                    static AUTO_DONE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
                    let auto = if AUTO_DONE.swap(true, Ordering::Relaxed) { String::new() } else { auto };
                    if theme::ghost_button(ui, "Vote to kick").clicked() || auto == "kick" {
                        let ok = crate::vote::call_vote(crate::vote::CallKind::Kick, Some(&name));
                        notice(if ok { format!("Vote to kick {name} called") } else { "Votes are not available on this server".into() });
                        close = true;
                    }
                    if theme::ghost_button(ui, "Copy name").clicked() || auto == "copy" {
                        notice(if copy_to_clipboard(&name) { format!("Copied \"{name}\"") } else { "Could not copy to the clipboard".into() });
                        close = true;
                    }
                });
            });
        });
        let clicked_elsewhere = ui.input(|i| i.pointer.primary_clicked()) && !resp.response.rect.contains(ui.input(|i| i.pointer.interact_pos().unwrap_or_default()));
        if close || (clicked_elsewhere && !opened_now) {
            *MENU.lock().unwrap_or_else(|e| e.into_inner()) = None;
        }
    }
}

/// Not drawn this frame: the next press logs its rows again.
pub fn hidden() {
    LOGGED.store(false, Ordering::Relaxed);
}
