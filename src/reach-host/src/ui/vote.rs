//! Vote screens (client side of vote.rs and callvote.rs), drawn as a passive overlay layer in host slot 1.
//! - Next-game vote, Reclaimer style: while the player is not in active play (post-game, pre-game waiting, dead,
//!   loading) a full lobby page: the game being played on the left, the 3 offered games as cards with map art, the
//!   game type and its description and a vote-share fill in the centre, the players on the right. In play a compact
//!   card at the top instead. Keys 1-4 vote at any time (input.rs hands them here while a vote is open); the page
//!   also takes the mouse (click a card) and the pad (d-pad up/down + A). Backspace hides the compact card.
//! - Call votes (shuffle, kick, skip, restart): a card at the right edge with the question, the yes share against the
//!   pass mark and F5 YES / F6 NO; then the result for a few seconds. The pause menu's CALL A VOTE section
//!   (`call_section`, `kick_picker`) starts them.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU32, Ordering};
use std::sync::Mutex;

use egui::{Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};

use super::theme::{self, ACCENT, DIM, LINE, PANEL, TEXT};
use crate::callvote::{self, CallKind};
use crate::log;
use crate::vote::{self, NONE};

/// We made the UI active (cursor shown) for the vote page; give it back when the vote ends.
static INTERACTIVE: AtomicBool = AtomicBool::new(false);
/// Pad selection (row index) and last pad buttons, for edge detection.
static SEL: AtomicU32 = AtomicU32::new(0);
static PAD_PREV: AtomicU16 = AtomicU16::new(0);
static FRAMES: AtomicU32 = AtomicU32::new(0);
/// Round id the player hid with Backspace (0 = none): the compact card stays hidden until the next round or result.
static HIDDEN: AtomicU16 = AtomicU16::new(0);
/// Longest countdown seen per round (round id, seconds): the OPEN message has no total, the bar needs one.
static ROUND_SECS: Mutex<(u16, u8)> = Mutex::new((0, 0));

const GREEN: Color32 = Color32::from_rgb(76, 175, 80);
const RED: Color32 = Color32::from_rgb(214, 69, 65);

/// What the passive layer draws this frame.
pub struct Frame {
    pub next: Option<vote::View>,
    pub call: Option<callvote::View>,
}

/// input.rs: Backspace while a vote is open hides the compact card for this round (voting with 1-4 still works).
/// True = consumed.
pub fn hide_key() -> bool {
    let Some(v) = vote::view() else { return false };
    let Some(o) = &v.open else { return false };
    HIDDEN.store(o.id, Ordering::Relaxed);
    log!("vote: panel hidden for round {}", o.id);
    true
}

/// input.rs: digit key `n` (0 = key 1) pressed while the game has the keyboard. True = consumed (a vote is open).
pub fn key(n: u8) -> bool {
    let ok = vote::cast(n);
    if ok {
        CARNAGE_TAB.lock().unwrap().1 = 2; // voting by key shows the vote tab
    }
    ok
}

/// Is a player unit alive under us? (No unit = pre-game / loading / dead: the full page shows and takes the mouse.)
fn in_game() -> bool {
    crate::input::player0_alive() // render thread: player0_view only works on the game thread
}

fn fake(what: &str) -> bool {
    std::env::var("REACH_VOTE_FAKE").is_ok_and(|v| v.split(',').any(|x| x == what))
}

/// `REACH_VOTE_FAKE=next`: a made-up next-game round (captures without a playlist server).
fn fake_next() -> Option<vote::View> {
    if !fake("next") {
        return None;
    }
    let opt = |name: &str, map: &str, gv: &str| vote::Opt { mode: 0, name: name.into(), map: map.into(), game_variant: gv.into() };
    let open = vote::OpenMsg {
        id: 1,
        secs_left: 8,
        none_allowed: true,
        keep: false,
        voters: 3,
        tally: [1, 0, 1, 1],
        playlist: "Team Slayer".into(),
        options: vec![
            opt("Swat on Sword Base", "20_sword_slayer", "SWAT_054.bin"),
            opt("Escalation on Powerhouse", "50_panopticon", "hr_ffa_escalationSlayer_mixed_vanilla.bin"),
            opt("CTF on Boardwalk", "45_aftship", "ctf_054.bin"),
        ],
    };
    Some(vote::View { open: Some(open), open_at: None, round_at: None, mine: Some(0), result: None })
}

/// Full page rather than the compact card (REACH_VOTE_FAKE=page forces it after 150 frames, for captures).
fn page_mode() -> bool {
    !in_game() || (fake("page") && FRAMES.load(Ordering::Relaxed) >= 150)
}

/// The full next-game page is up this frame (it shows the carnage report as its first tab).
pub(super) fn page_up(f: &Frame) -> bool {
    f.next.as_ref().is_some_and(|v| v.open.is_some()) && page_mode()
}

/// Passive layer (overlay.rs): what to draw, if anything; also switches the cursor on/off for the page.
pub fn current() -> Option<Frame> {
    let next = fake_next().or_else(vote::view);
    let call = callvote::view();
    let want = next.as_ref().is_some_and(|v| v.open.is_some()) && page_mode() && !fake("next");
    if INTERACTIVE.swap(want, Ordering::Relaxed) != want {
        super::post_ui(super::OWN_VOTE, want);
    }
    if next.is_none() && call.is_none() {
        FRAMES.store(0, Ordering::Relaxed);
        return None;
    }
    Some(Frame { next, call })
}

/// Render thread, after drawing: `REACH_VOTE_SHOT` captures the 90th frame (`vote-panel.bmp`), the 240th
/// (`vote-page.bmp`, the page forced by REACH_VOTE_FAKE=page) and a result card (`vote-result.bmp`).
pub fn after_draw(swapchain: *mut std::ffi::c_void, f: &Frame) {
    let n = FRAMES.fetch_add(1, Ordering::Relaxed) + 1;
    if std::env::var_os("REACH_VOTE_SHOT").is_none() {
        return;
    }
    let open = f.next.as_ref().is_some_and(|v| v.open.is_some()) || f.call.as_ref().is_some_and(|c| c.status.is_some());
    let name = if open && n == 90 {
        "vote-panel.bmp"
    } else if open && n == 240 {
        "vote-page.bmp"
    } else if !open && n % 1000 == 30 {
        "vote-result.bmp"
    } else {
        return;
    };
    let path = std::env::current_exe().unwrap_or_default().with_file_name(name);
    match unsafe { crate::host::capture_back_buffer(swapchain, &path) } {
        Ok((w, h)) => log!("vote: back buffer {w}x{h} saved to {}", path.display()),
        Err(e) => log!("vote: capture failed: {e}"),
    }
}

/// Pad: d-pad up/down moves the selection, A votes for it. Only while the page is interactive.
fn poll_pad(rows: u32) {
    use windows::Win32::UI::Input::XboxController::{XInputGetState, XINPUT_STATE};
    if !INTERACTIVE.load(Ordering::Relaxed) || rows == 0 {
        return;
    }
    let mut b = 0u16;
    for i in 0..4 {
        let mut s = XINPUT_STATE::default();
        if unsafe { XInputGetState(i, &mut s) } == 0 {
            b = s.Gamepad.wButtons.0;
            break;
        }
    }
    let pressed = b & !PAD_PREV.swap(b, Ordering::Relaxed);
    let sel = SEL.load(Ordering::Relaxed).min(rows - 1);
    if pressed & 0x0001 != 0 {
        SEL.store(sel.saturating_sub(1), Ordering::Relaxed);
    } else if pressed & 0x0002 != 0 {
        SEL.store((sel + 1).min(rows - 1), Ordering::Relaxed);
    } else if pressed & 0x1000 != 0 {
        vote::cast(if sel == 3 { NONE } else { sel as u8 });
    }
}

pub fn draw(ui: &mut egui::Ui, f: &Frame) {
    let on_page = f.next.as_ref().is_some_and(|v| v.open.is_some()) && page_mode();
    ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("vote-page-shown"), on_page));
    if let Some(v) = &f.next {
        draw_next(ui, v);
    }
    if let Some(c) = &f.call {
        draw_call(ui, c);
    }
}

// ---- game info -------------------------------------------------------------------------------------------------

/// MCC folder (menu-launched games set preview::MCC; CLI runs pass `--mcc` or MCC_PATH).
fn mcc_dir() -> Option<std::path::PathBuf> {
    if let Some(p) = crate::preview::MCC.get() {
        return Some(p.clone());
    }
    let args: Vec<String> = vote::LAUNCH_ARGS.get().cloned().unwrap_or_else(|| std::env::args().collect());
    args.iter().position(|a| a == "--mcc").and_then(|i| args.get(i + 1)).map(Into::into).or_else(|| std::env::var_os("MCC_PATH").map(Into::into))
}

fn init_art() {
    static DONE: AtomicBool = AtomicBool::new(false);
    if !DONE.swap(true, Ordering::Relaxed) {
        if let Some(m) = mcc_dir() {
            super::mccart::init(&m);
        }
    }
}

/// A game variant's own name and description, from the content header at the start of the `.bin` (chdr chunk at
/// 0x30: UTF-16 name at file 0xC0, description at 0x1C0, 128 characters each). Built-in variants carry a `$hr_gvar_…`
/// string-table key as description: None then.
fn variant_text(file: &str) -> (Option<String>, Option<String>) {
    static CACHE: Mutex<Option<HashMap<String, (Option<String>, Option<String>)>>> = Mutex::new(None);
    if file.is_empty() {
        return (None, None);
    }
    let mut g = CACHE.lock().unwrap();
    let c = g.get_or_insert_with(HashMap::new);
    if let Some(v) = c.get(file) {
        return v.clone();
    }
    let read = || -> Option<Vec<u8>> {
        let f = std::path::Path::new(file);
        let p = if f.is_absolute() { f.to_path_buf() } else { crate::catalog::game_variant_path(&mcc_dir()?, file) };
        std::fs::read(p).ok()
    };
    let text = |b: &[u8], at: usize| -> Option<String> {
        let u: Vec<u16> = b.get(at..at + 0x100)?.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&c| c != 0).collect();
        let s = String::from_utf16_lossy(&u).trim().to_string();
        (!s.is_empty() && !s.starts_with('$')).then_some(s)
    };
    let v = match read() {
        Some(b) if b.get(0x30..0x34) == Some(b"chdr") => (text(&b, 0xc0), text(&b, 0x1c0)),
        _ => (None, None),
    };
    c.insert(file.to_string(), v.clone());
    v
}

/// Base game mode of a variant file and a one-line description in our own words (used when the variant has none).
fn base_mode(o: &vote::Opt) -> (&'static str, &'static str) {
    match o.mode {
        crate::query::MODE_FIREFIGHT => return ("FIREFIGHT", "Hold out together against waves of Covenant."),
        crate::query::MODE_CAMPAIGN => return ("CAMPAIGN", "Play the mission together, start to finish."),
        _ => {}
    }
    let f = o.game_variant.to_ascii_lowercase();
    const MODES: &[(&str, &str, &str)] = &[
        ("escalation", "SLAYER", "Every kill moves you to the next weapon. Finish the list first to win."),
        ("swat", "SLAYER", "Precision weapons, no shields, no radar. One headshot is all it takes."),
        ("ctf", "CTF", "Steal the enemy flag and bring it home while guarding your own."),
        ("flag", "CTF", "Steal the enemy flag and bring it home while guarding your own."),
        ("assault", "ASSAULT", "Carry the bomb into the enemy base and arm it."),
        ("bomb", "ASSAULT", "Carry the bomb into the enemy base and arm it."),
        ("koth", "KING OF THE HILL", "Hold the hill to score. The hill moves, so keep up."),
        ("crazyking", "KING OF THE HILL", "Hold the hill to score. The hill moves, so keep up."),
        ("oddball", "ODDBALL", "Grab the skull and hold on to it for as long as you can."),
        ("juggernaut", "JUGGERNAUT", "One big target, everyone else hunting it. Kill the Juggernaut to become it."),
        ("infection", "INFECTION", "Survivors against a growing horde of infected. Last one standing wins."),
        ("safehaven", "INFECTION", "Survivors against a growing horde of infected. Last one standing wins."),
        ("headhunter", "HEADHUNTER", "Kills drop skulls. Bank them in the moving zones to score."),
        ("stockpile", "STOCKPILE", "Gather the neutral flags and hold them in your base when the timer ticks."),
        ("territor", "TERRITORIES", "Capture and hold territories across the map."),
        ("plot", "TERRITORIES", "Capture and hold territories across the map."),
        ("invasion", "INVASION", "Spartans against Elites, one objective phase after another."),
        ("race", "RACE", "Drive through the checkpoints faster than everyone else."),
        ("rally", "RACE", "Drive through the checkpoints faster than everyone else."),
        ("skeeball", "HALO BALL", "Score the ball in the enemy goal."),
        ("editing", "FORGE", "Build and change the map together."),
    ];
    MODES.iter().find(|m| f.contains(m.0)).map_or(("SLAYER", "Kill the other players. Most kills when time runs out wins."), |m| (m.1, m.2))
}

/// Game type name: the variant's own name, else one made from its file name.
pub(super) fn variant_name(o: &vote::Opt) -> String {
    match o.mode {
        crate::query::MODE_CAMPAIGN => "Campaign".into(),
        _ => variant_text(&o.game_variant).0.unwrap_or_else(|| super::menu::variant_title(&o.game_variant)),
    }
}

fn description(o: &vote::Opt) -> String {
    variant_text(&o.game_variant).1.unwrap_or_else(|| base_mode(o).1.to_string())
}

/// Thumbnail texture of a map (316x160 art in the player's texture packs).
fn thumb(o: &vote::Opt) -> Option<String> {
    let key = |name: &str| name.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_uppercase();
    let title = super::menu::map_title(&o.map);
    Some(match o.mode {
        crate::query::MODE_CAMPAIGN => super::campaign::mission_art(&o.map)?,
        crate::query::MODE_FIREFIGHT => format!("REACHFIREFIGHT_{}", key(&title)),
        _ => format!("HREACHMP_{}", key(&title)),
    })
}

/// The game being played now, from this process's own arguments (map / variant) or the host's query info.
pub(super) fn current_game() -> Option<vote::Opt> {
    if let Some(i) = crate::query::info() {
        return Some(vote::Opt { mode: i.mode, name: String::new(), map: i.map, game_variant: i.game_variant });
    }
    let args: Vec<String> = vote::LAUNCH_ARGS.get().cloned().unwrap_or_else(|| std::env::args().collect());
    let val = |f: &str| args.iter().position(|a| a == f).and_then(|i| args.get(i + 1)).cloned();
    let file = |p: String| p.rsplit(['\\', '/']).next().unwrap_or_default().to_string();
    let (mode, map) = if let Some(m) = val("--firefight") {
        (crate::query::MODE_FIREFIGHT, m)
    } else if let Some(m) = val("--campaign") {
        (crate::query::MODE_CAMPAIGN, m)
    } else {
        (crate::query::MODE_MULTIPLAYER, val("--map")?)
    };
    Some(vote::Opt { mode, name: String::new(), map, game_variant: val("--game-variant").map(file).unwrap_or_default() })
}

// ---- drawing helpers ---------------------------------------------------------------------------------------------

fn text(ui: &egui::Ui, pos: Pos2, anchor: Align2, s: impl ToString, size: f32, c: Color32) -> Rect {
    ui.painter().text(pos, anchor, s.to_string(), FontId::proportional(size), c)
}

/// Card background: our panel colour, a thin outline, an optional accent bar on the left.
fn card(ui: &egui::Ui, r: Rect, bar: Option<Color32>) {
    let p = ui.painter();
    p.rect_filled(r, 2.0, PANEL);
    p.rect_stroke(r, 2.0, Stroke::new(1.0, LINE), StrokeKind::Inside);
    if let Some(c) = bar {
        p.rect_filled(Rect::from_min_size(r.min, Vec2::new(4.0, r.height())), 0.0, c);
    }
}

/// Small outlined chip (e.g. the base mode next to the game type). Returns its width.
fn chip(ui: &egui::Ui, at: Pos2, s: &str, c: Color32) -> f32 {
    let g = ui.painter().layout_no_wrap(s.to_string(), FontId::proportional(11.0), c);
    let r = Rect::from_min_size(at - Vec2::new(0.0, g.size().y / 2.0 + 3.0), g.size() + Vec2::new(12.0, 6.0));
    ui.painter().rect_stroke(r, 2.0, Stroke::new(1.0, c.gamma_multiply(0.7)), StrokeKind::Inside);
    let w = r.width();
    ui.painter().galley(r.min + Vec2::new(6.0, 3.0), g, c);
    w
}

/// Map art in `r`, centre-cropped to its aspect (uvs inset half a texel: notes/frontend.md "Campaign screen").
fn art(ui: &egui::Ui, r: Rect, name: Option<String>, fallback: &str) {
    let t = name.and_then(|n| super::mccart::texture(ui.ctx(), &n));
    match t {
        Some(t) => {
            let [w, h] = t.size();
            let (w, h) = (w as f32, h as f32);
            let (mut u, mut v) = (0.5 / w, 0.5 / h);
            let (src, dst) = (w / h, r.width() / r.height());
            if src > dst {
                u = (1.0 - dst / src) / 2.0 + u;
            } else {
                v = (1.0 - src / dst) / 2.0 + v;
            }
            ui.painter().image(t.id(), r, Rect::from_min_max(egui::pos2(u, v), egui::pos2(1.0 - u, 1.0 - v)), Color32::WHITE);
        }
        None => {
            ui.painter().rect_filled(r, 0.0, Color32::from_rgb(30, 34, 40));
            text(ui, r.center(), Align2::CENTER_CENTER, fallback.to_uppercase(), 13.0, DIM);
        }
    }
}

/// Little line icons for the GAME column: 0 map pin, 1 target, 2 list.
fn icon(ui: &egui::Ui, c: Pos2, kind: u8) {
    let p = ui.painter();
    let s = Stroke::new(1.6, DIM);
    match kind {
        0 => {
            p.circle_stroke(c - Vec2::new(0.0, 3.0), 5.0, s);
            p.line_segment([c + Vec2::new(-4.0, 0.0), c + Vec2::new(0.0, 7.0)], s);
            p.line_segment([c + Vec2::new(4.0, 0.0), c + Vec2::new(0.0, 7.0)], s);
        }
        1 => {
            p.circle_stroke(c, 7.0, s);
            p.circle_stroke(c, 3.5, s);
            p.circle_filled(c, 1.2, DIM);
        }
        _ => {
            for k in 0..3 {
                let y = c.y - 5.0 + k as f32 * 5.0;
                p.circle_filled(egui::pos2(c.x - 6.0, y), 1.2, DIM);
                p.line_segment([egui::pos2(c.x - 3.0, y), egui::pos2(c.x + 7.0, y)], s);
            }
        }
    }
}

/// Team colours (Reach's red, blue, green, orange, purple, gold, brown, pink).
fn team_colour(t: i8) -> Color32 {
    const C: [(u8, u8, u8); 8] = [(200, 50, 50), (50, 100, 210), (60, 160, 60), (230, 130, 30), (140, 70, 190), (210, 180, 40), (130, 90, 50), (220, 110, 170)];
    match C.get(t as usize) {
        Some(&(r, g, b)) if t >= 0 => Color32::from_rgb(r, g, b),
        _ => Color32::from_gray(90),
    }
}

// ---- next-game vote ------------------------------------------------------------------------------------------------

/// Seconds this round started with (the longest countdown seen), for the shrinking bar.
fn round_total(o: &vote::OpenMsg) -> f32 {
    let mut g = ROUND_SECS.lock().unwrap();
    if g.0 != o.id {
        *g = (o.id, o.secs_left);
    }
    g.1 = g.1.max(o.secs_left);
    g.1.max(1) as f32
}

/// Choice row k: (choice byte, title, detail) for the "none" / "keep" row.
fn none_row(o: &vote::OpenMsg) -> (&'static str, &'static str) {
    if o.keep {
        ("KEEP PLAYING", "Cancel the vote and carry on with this game")
    } else {
        ("NONE OF THE ABOVE", "Offer three different games")
    }
}

fn votes(n: u8) -> String {
    format!("{n} vote{}", if n == 1 { "" } else { "s" })
}

fn draw_next(ui: &mut egui::Ui, v: &vote::View) {
    init_art();
    if let Some(o) = &v.open {
        let rows = o.options.len() + o.none_allowed as usize;
        for k in 0..4u8 {
            let key = [egui::Key::Num1, egui::Key::Num2, egui::Key::Num3, egui::Key::Num4][k as usize];
            if ui.input(|i| i.key_pressed(key)) {
                CARNAGE_TAB.lock().unwrap().1 = 2; // voting by key shows the vote tab
                vote::cast(k);
            }
        }
        if page_mode() {
            poll_pad(rows as u32);
            page(ui, v, o);
        } else if HIDDEN.load(Ordering::Relaxed) != o.id {
            compact(ui, v, o);
        }
    } else if let Some((res, _)) = &v.result {
        let full = ui.max_rect();
        let r = Rect::from_center_size(egui::pos2(full.center().x, full.top() + 92.0), Vec2::new(520.0, 120.0));
        card(ui, r, Some(ACCENT));
        art(ui, Rect::from_min_size(r.min + Vec2::new(16.0, 16.0), Vec2::new(144.0, 88.0)), thumb(&res.opt), &super::menu::map_title(&res.opt.map));
        let x = r.left() + 176.0;
        text(ui, egui::pos2(x, r.top() + 24.0), Align2::LEFT_CENTER, "NEXT GAME", 12.0, DIM);
        text(ui, egui::pos2(x, r.top() + 50.0), Align2::LEFT_CENTER, super::menu::map_title(&res.opt.map).to_uppercase(), 22.0, TEXT);
        text(ui, egui::pos2(x, r.top() + 74.0), Align2::LEFT_CENTER, format!("{}  ·  {}", variant_name(&res.opt).to_uppercase(), base_mode(&res.opt).0), 13.0, ACCENT);
        let note = if vote::following() {
            "Server restarting, rejoining when it is back ..."
        } else if res.restart {
            "Server restarting ..."
        } else if res.winner == NONE {
            "Vote cancelled, carrying on"
        } else {
            "Already playing it, carrying on"
        };
        text(ui, egui::pos2(x, r.top() + 98.0), Align2::LEFT_CENTER, note, 13.0, DIM);
    } else if let Some(t) = vote::call_notice() {
        let full = ui.max_rect();
        let r = Rect::from_center_size(egui::pos2(full.center().x, full.top() + 56.0), Vec2::new(520.0, 48.0));
        card(ui, r, Some(ACCENT));
        text(ui, r.center(), Align2::CENTER_CENTER, t, 15.0, TEXT);
    }
}

/// In play: a small card at the top centre (the match stays visible); 1-4 vote, Backspace hides it.
fn compact(ui: &egui::Ui, v: &vote::View, o: &vote::OpenMsg) {
    let full = ui.max_rect();
    let rows = o.options.len() + o.none_allowed as usize;
    let r = Rect::from_min_size(egui::pos2(full.center().x - 230.0, full.top() + 14.0), Vec2::new(460.0, 58.0 + 24.0 * rows as f32));
    card(ui, r, Some(ACCENT));
    let x = r.left() + 18.0;
    text(ui, egui::pos2(x, r.top() + 20.0), Align2::LEFT_CENTER, "VOTE FOR THE NEXT GAME", 14.0, ACCENT);
    text(ui, egui::pos2(r.right() - 16.0, r.top() + 20.0), Align2::RIGHT_CENTER, format!("{}s", o.secs_left), 16.0, ACCENT);
    let total = round_total(o);
    let bar = Rect::from_min_size(egui::pos2(x, r.top() + 34.0), Vec2::new((r.width() - 34.0) * o.secs_left as f32 / total, 2.0));
    ui.painter().rect_filled(bar, 0.0, ACCENT);
    for k in 0..rows {
        let choice = if k == 3 { NONE } else { k as u8 };
        let y = r.top() + 52.0 + 24.0 * k as f32;
        let mine = v.mine == Some(choice);
        let title = match o.options.get(k) {
            Some(opt) if k < 3 => format!("{}  ·  {}", super::menu::map_title(&opt.map).to_uppercase(), variant_name(opt)),
            _ => none_row(o).0.to_string(),
        };
        if mine {
            ui.painter().rect_filled(Rect::from_min_size(egui::pos2(x - 6.0, y - 11.0), Vec2::new(r.width() - 24.0, 22.0)), 2.0, ACCENT.gamma_multiply(0.22));
        }
        text(ui, egui::pos2(x, y), Align2::LEFT_CENTER, k + 1, 14.0, if mine { ACCENT } else { DIM });
        text(ui, egui::pos2(x + 22.0, y), Align2::LEFT_CENTER, title, 14.0, TEXT);
        text(ui, egui::pos2(r.right() - 16.0, y), Align2::RIGHT_CENTER, o.tally[choice as usize], 14.0, if o.tally[choice as usize] > 0 { TEXT } else { DIM });
    }
}

/// Not in play: the full lobby page.
fn page(ui: &mut egui::Ui, v: &vote::View, o: &vote::OpenMsg) {
    let full = ui.max_rect();
    ui.painter().rect_filled(full, 0.0, Color32::from_rgba_unmultiplied(8, 10, 13, 236));
    let m = 32.0;
    // Top bar: title and the tabs that work here.
    text(ui, egui::pos2(full.left() + m, full.top() + 44.0), Align2::LEFT_CENTER, "CUSTOM GAME", 34.0, TEXT);
    ui.painter().line_segment([egui::pos2(full.left() + m, full.top() + 70.0), egui::pos2(full.left() + m + 40.0, full.top() + 70.0)], Stroke::new(2.0, ACCENT));
    let mut tx = full.right() - m;
    for (label, act) in [("LEAVE LOBBY", 1), ("SETTINGS", 0)] {
        let g = ui.painter().layout_no_wrap(label.into(), FontId::proportional(15.0), TEXT);
        let r = Rect::from_min_size(egui::pos2(tx - g.size().x - 24.0, full.top() + 30.0), Vec2::new(g.size().x + 24.0, 30.0));
        let resp = ui.interact(r, ui.id().with(("vote-tab", act)), Sense::click());
        if resp.hovered() {
            ui.painter().rect_filled(r, 2.0, theme::HOVER);
            ui.painter().line_segment([r.left_bottom(), r.right_bottom()], Stroke::new(2.0, ACCENT));
        }
        ui.painter().galley(r.center() - g.size() / 2.0, g, if resp.hovered() { Color32::WHITE } else { DIM });
        if resp.clicked() {
            if act == 1 {
                log!("vote page: LEAVE LOBBY");
                super::overlay::leave_game();
            } else {
                super::overlay::open_settings();
            }
        }
        tx = r.left() - 8.0;
    }
    let top = full.top() + 96.0;
    let bottom = full.bottom() - m;
    let left = Rect::from_min_max(egui::pos2(full.left() + m, top), egui::pos2(full.left() + m + 300.0, bottom));
    let right = Rect::from_min_max(egui::pos2(full.right() - m - 300.0, top), egui::pos2(full.right() - m, bottom));
    let centre = Rect::from_min_max(egui::pos2(left.right() + 24.0, top), egui::pos2(right.left() - 24.0, bottom));
    game_column(ui, left, o);
    // After a game: CARNAGE REPORT / VOTE tabs next to the title; the report is the first tab for its first
    // seconds, then the vote (keys 1-4 vote either way and switch to it).
    let carnage = if o.keep { None } else { super::carnage::recent() };
    if let Some(r) = carnage {
        let since = v.round_at.map_or(0.0, |t| t.elapsed().as_secs_f32());
        let mut tab = CARNAGE_TAB.lock().unwrap();
        if tab.0 != o.id {
            *tab = (o.id, 0);
        }
        let showing = tab.1 == 1 || (tab.1 == 0 && since < CARNAGE_FIRST_SECS && !fake("vote-tab"));
        let mut tx = full.left() + m + 290.0;
        for (label, t) in [("CARNAGE REPORT", 1u8), ("VOTE", 2)] {
            let on = (t == 1) == showing;
            let g = ui.painter().layout_no_wrap(label.into(), FontId::proportional(15.0), TEXT);
            let rr = Rect::from_min_size(egui::pos2(tx, full.top() + 30.0), Vec2::new(g.size().x + 24.0, 30.0));
            let resp = ui.interact(rr, ui.id().with(("carnage-tab", t)), Sense::click());
            if on || resp.hovered() {
                ui.painter().rect_filled(rr, 2.0, theme::HOVER);
                ui.painter().line_segment([rr.left_bottom(), rr.right_bottom()], Stroke::new(2.0, ACCENT));
            }
            ui.painter().galley(rr.center() - g.size() / 2.0, g, if on || resp.hovered() { Color32::WHITE } else { DIM });
            if resp.clicked() {
                tab.1 = t;
            }
            tx = rr.right() + 8.0;
        }
        drop(tab);
        if showing {
            let area = Rect::from_min_max(centre.min, right.max);
            let w = area.width().min(super::carnage::PANEL_W);
            super::carnage::panel(ui, &r, egui::pos2(area.center().x, area.top()), w);
            return;
        }
    }
    vote_column(ui, centre, v, o);
    players_column(ui, right);
}

/// Vote page tab per round: (round id, 0 automatic, 1 carnage report, 2 vote).
static CARNAGE_TAB: Mutex<(u16, u8)> = Mutex::new((0, 0));
/// Automatic tab: the carnage report for this long after the round opens, then the vote.
const CARNAGE_FIRST_SECS: f32 = 8.0;

fn game_column(ui: &egui::Ui, r: Rect, o: &vote::OpenMsg) {
    card(ui, r, None);
    let x = r.left() + 20.0;
    text(ui, egui::pos2(x, r.top() + 24.0), Align2::LEFT_CENTER, "GAME", 13.0, DIM);
    let g = current_game();
    let map = g.as_ref().map_or("—".to_string(), |g| super::menu::map_title(&g.map).to_uppercase());
    let (mut gt, base) = g.as_ref().map_or(("—".to_string(), ""), |g| (variant_name(g).to_uppercase(), base_mode(g).0));
    if gt.is_empty() {
        gt = base.to_string(); // no variant file known: the base mode alone
    }
    let playlist = if o.playlist.is_empty() { "—".to_string() } else { o.playlist.to_uppercase() };
    let mut y = r.top() + 62.0;
    for (i, (cap, val)) in [("MAP", map), ("GAME TYPE", gt), ("PLAYLIST", playlist)].into_iter().enumerate() {
        icon(ui, egui::pos2(x + 8.0, y + 10.0), i as u8);
        text(ui, egui::pos2(x + 28.0, y), Align2::LEFT_CENTER, cap, 11.5, DIM);
        let vr = text(ui, egui::pos2(x + 28.0, y + 20.0), Align2::LEFT_CENTER, &val, 17.0, TEXT);
        if i == 1 && !base.is_empty() && base != val {
            chip(ui, egui::pos2(vr.right() + 8.0, y + 20.0), base, DIM);
        }
        y += 62.0;
    }
    // State banner.
    let b = Rect::from_min_max(egui::pos2(r.left() + 14.0, r.bottom() - 96.0), egui::pos2(r.right() - 14.0, r.bottom() - 14.0));
    ui.painter().rect_stroke(b, 2.0, Stroke::new(1.0, ACCENT.gamma_multiply(0.8)), StrokeKind::Inside);
    ui.painter().rect_filled(b.shrink(1.0), 2.0, ACCENT.gamma_multiply(0.08));
    text(ui, egui::pos2(b.left() + 16.0, b.top() + 30.0), Align2::LEFT_CENTER, "VOTE FOR THE NEXT GAME", 17.0, TEXT);
    text(ui, egui::pos2(b.left() + 16.0, b.top() + 56.0), Align2::LEFT_CENTER, "The most votes wins", 13.0, DIM);
}

fn vote_column(ui: &mut egui::Ui, r: Rect, v: &vote::View, o: &vote::OpenMsg) {
    let x = r.left();
    text(ui, egui::pos2(x, r.top() + 14.0), Align2::LEFT_CENTER, "Connected", 26.0, TEXT);
    text(ui, egui::pos2(x, r.top() + 42.0), Align2::LEFT_CENTER, "Waiting on the server for the next game", 13.0, DIM);
    let g = ui.painter().layout_no_wrap("JOINED".into(), FontId::proportional(12.0), GREEN);
    let br = Rect::from_min_size(egui::pos2(r.right() - g.size().x - 20.0, r.top() + 4.0), g.size() + Vec2::new(20.0, 10.0));
    ui.painter().rect_stroke(br, 2.0, Stroke::new(1.0, GREEN), StrokeKind::Inside);
    ui.painter().galley(br.min + Vec2::new(10.0, 5.0), g, GREEN);
    // Header, countdown and the shrinking line.
    let hy = r.top() + 84.0;
    text(ui, egui::pos2(x, hy), Align2::LEFT_CENTER, "VOTE FOR THE NEXT GAME", 18.0, TEXT);
    text(ui, egui::pos2(r.right(), hy), Align2::RIGHT_CENTER, format!("{}s", o.secs_left), 20.0, ACCENT);
    let total = round_total(o);
    ui.painter().rect_filled(Rect::from_min_size(egui::pos2(x, hy + 16.0), Vec2::new(r.width(), 2.0)), 0.0, Color32::from_white_alpha(18));
    ui.painter().rect_filled(Rect::from_min_size(egui::pos2(x, hy + 16.0), Vec2::new(r.width() * o.secs_left as f32 / total, 2.0)), 0.0, ACCENT);
    // Cards.
    let voted: u32 = o.tally.iter().map(|&t| t as u32).sum();
    let sel = SEL.load(Ordering::Relaxed) as usize;
    let rows = o.options.len() + o.none_allowed as usize;
    let mut y = hy + 32.0;
    let avail = r.bottom() - y - 24.0;
    let none_h = 46.0;
    let card_h = ((avail - none_h - 10.0 * o.options.len() as f32) / o.options.len().max(1) as f32).clamp(84.0, 134.0);
    for k in 0..rows {
        let choice = if k == 3 { NONE } else { k as u8 };
        let h = if k < o.options.len() { card_h } else { none_h };
        let cr = Rect::from_min_size(egui::pos2(x, y), Vec2::new(r.width(), h));
        let resp = ui.interact(cr, ui.id().with(("vote-card", k)), Sense::click());
        if resp.clicked() {
            vote::cast(choice);
        }
        let mine = v.mine == Some(choice);
        let hot = resp.hovered() || (INTERACTIVE.load(Ordering::Relaxed) && sel == k);
        let count = o.tally[choice as usize];
        let share = if voted > 0 { count as f32 / voted as f32 } else { 0.0 };
        let p = ui.painter();
        p.rect_filled(cr, 2.0, if hot { Color32::from_rgba_unmultiplied(30, 34, 42, 235) } else { PANEL });
        p.rect_filled(Rect::from_min_size(cr.min, Vec2::new(cr.width() * share, cr.height())), 2.0, Color32::from_white_alpha(14));
        p.rect_stroke(cr, 2.0, Stroke::new(if mine { 2.0 } else { 1.0 }, if mine { ACCENT } else { LINE }), StrokeKind::Inside);
        text(ui, egui::pos2(cr.right() - 14.0, cr.top() + 18.0), Align2::RIGHT_CENTER, votes(count), 13.0, if count > 0 { TEXT } else { DIM });
        match o.options.get(k).filter(|_| k < 3) {
            Some(opt) => {
                let th = (h - 20.0).min(110.0);
                let tr = Rect::from_min_size(egui::pos2(cr.left() + 10.0, cr.top() + (h - th) / 2.0), Vec2::new(th * 180.0 / 110.0, th));
                art(ui, tr, thumb(opt), &super::menu::map_title(&opt.map));
                text(ui, tr.left_top() + Vec2::new(6.0, 6.0), Align2::LEFT_TOP, k + 1, 14.0, Color32::WHITE);
                let tx = tr.right() + 16.0;
                text(ui, egui::pos2(tx, cr.top() + 22.0), Align2::LEFT_CENTER, super::menu::map_title(&opt.map).to_uppercase(), 21.0, if mine { ACCENT } else { TEXT });
                text(ui, egui::pos2(tx, cr.top() + 46.0), Align2::LEFT_CENTER, format!("{}  ·  {}", variant_name(opt).to_uppercase(), base_mode(opt).0), 13.0, ACCENT.gamma_multiply(0.9));
                let wrap = cr.right() - tx - 14.0;
                let gal = ui.painter().layout(description(opt), FontId::proportional(12.5), DIM, wrap);
                let clip = Rect::from_min_size(egui::pos2(tx, cr.top() + 60.0), Vec2::new(wrap, (h - 66.0).clamp(0.0, 34.0)));
                ui.painter().with_clip_rect(clip).galley(clip.min, gal, DIM);
            }
            None => {
                let (t, d) = none_row(o);
                text(ui, egui::pos2(cr.left() + 16.0, cr.center().y), Align2::LEFT_CENTER, t, 16.0, if mine { ACCENT } else { TEXT });
                text(ui, egui::pos2(cr.left() + 200.0, cr.center().y), Align2::LEFT_CENTER, d, 13.0, DIM);
            }
        }
        y += h + 10.0;
    }
    text(ui, egui::pos2(x, r.bottom() - 6.0), Align2::LEFT_BOTTOM, "Press 1-4 or click a game to vote  ·  d-pad + A on a controller", 12.0, DIM);
}

fn players_column(ui: &egui::Ui, r: Rect) {
    card(ui, r, None);
    let rows = super::scoreboard::rows();
    let x = r.left() + 18.0;
    text(ui, egui::pos2(x, r.top() + 24.0), Align2::LEFT_CENTER, format!("PLAYERS  {}", rows.len()), 13.0, DIM);
    let mut y = r.top() + 50.0;
    let teams_y = r.bottom() - 92.0;
    if rows.is_empty() {
        text(ui, egui::pos2(x, y + 14.0), Align2::LEFT_CENTER, "No players yet", 14.0, DIM);
    }
    let mut sorted: Vec<&super::scoreboard::Row> = rows.iter().collect();
    sorted.sort_by(|a, b| a.team.cmp(&b.team).then(a.name.cmp(&b.name)));
    for p in sorted {
        if y + 34.0 > teams_y - 8.0 {
            break;
        }
        let row = Rect::from_min_size(egui::pos2(r.left() + 10.0, y), Vec2::new(r.width() - 20.0, 32.0));
        let c = team_colour(p.team);
        ui.painter().rect_filled(row, 2.0, c.gamma_multiply(if p.local { 0.42 } else { 0.25 }));
        ui.painter().rect_filled(Rect::from_min_size(row.min, Vec2::new(4.0, row.height())), 0.0, c);
        text(ui, egui::pos2(row.left() + 16.0, row.center().y), Align2::LEFT_CENTER, &p.name, 15.0, if p.local { Color32::WHITE } else { TEXT });
        if p.local {
            text(ui, egui::pos2(row.right() - 10.0, row.center().y), Align2::RIGHT_CENTER, "YOU", 11.0, DIM);
        }
        y += 36.0;
    }
    // Team sizes (read only: changing team is the host's call; see CALL A VOTE > Shuffle teams).
    text(ui, egui::pos2(x, teams_y + 10.0), Align2::LEFT_CENTER, "TEAMS", 12.0, DIM);
    let bw = (r.width() - 36.0 - 7.0 * 6.0) / 8.0;
    for t in 0..8i8 {
        let n = rows.iter().filter(|p| p.team == t).count();
        let b = Rect::from_min_size(egui::pos2(x + t as f32 * (bw + 6.0), teams_y + 26.0), Vec2::new(bw, 40.0));
        let c = team_colour(t);
        ui.painter().rect_filled(b, 2.0, c.gamma_multiply(if n > 0 { 0.9 } else { 0.25 }));
        text(ui, b.center(), Align2::CENTER_CENTER, n, 15.0, if n > 0 { Color32::WHITE } else { Color32::from_white_alpha(90) });
    }
}

// ---- call votes ------------------------------------------------------------------------------------------------------

/// A key chip like [F5].
fn key_chip(ui: &egui::Ui, at: Pos2, k: &str, c: Color32) -> Rect {
    let g = ui.painter().layout_no_wrap(k.into(), FontId::proportional(11.5), Color32::from_rgb(12, 14, 16));
    let r = Rect::from_min_size(at - Vec2::new(0.0, 10.0), Vec2::new(g.size().x + 10.0, 20.0));
    ui.painter().rect_filled(r, 2.0, c);
    ui.painter().galley(r.center() - g.size() / 2.0, g, Color32::from_rgb(12, 14, 16));
    r
}

/// Right edge, mid-height: the open call vote, or its result for a few seconds.
fn draw_call(ui: &mut egui::Ui, c: &callvote::View) {
    let full = ui.max_rect();
    // Over the next-game page it sits on the players column, else at the right edge, mid-height.
    let on_page = ui.ctx().data(|d| d.get_temp::<bool>(egui::Id::new("vote-page-shown"))).unwrap_or(false);
    let (w, right, cy) = if on_page { (300.0, full.right() - 32.0, full.top() + 330.0) } else { (380.0, full.right() - 16.0, full.center().y) };
    if let Some(st) = &c.status {
        let kind = CallKind::from_byte(st.kind).unwrap_or(CallKind::Shuffle);
        let r = Rect::from_min_size(egui::pos2(right - w, cy - 110.0), Vec2::new(w, 220.0));
        card(ui, r, Some(ACCENT));
        let x = r.left() + 20.0;
        let rx = r.right() - 16.0;
        text(ui, egui::pos2(x, r.top() + 22.0), Align2::LEFT_CENTER, format!("VOTE  ·  {}", kind.title().to_uppercase()), 13.0, ACCENT);
        text(ui, egui::pos2(rx, r.top() + 22.0), Align2::RIGHT_CENTER, format!("{}s", st.secs_left), 14.0, TEXT);
        let q = ui.painter().layout(kind.question(&st.target), FontId::proportional(21.0), TEXT, rx - x);
        let qh = q.size().y;
        ui.painter().galley(egui::pos2(x, r.top() + 38.0), q, TEXT);
        let mut y = r.top() + 44.0 + qh;
        text(ui, egui::pos2(x, y + 6.0), Align2::LEFT_CENTER, format!("Called by {}", st.caller), 12.5, DIM);
        y += 24.0;
        // Yes share against the pass mark.
        let bar = Rect::from_min_size(egui::pos2(x, y), Vec2::new(rx - x, 8.0));
        let voters = st.voters.max(1) as f32;
        ui.painter().rect_filled(bar, 2.0, Color32::from_white_alpha(20));
        ui.painter().rect_filled(Rect::from_min_size(bar.min, Vec2::new(bar.width() * (st.yes as f32 / voters).min(1.0), bar.height())), 2.0, GREEN);
        let tick = bar.left() + bar.width() * (st.needed as f32 / voters).min(1.0);
        ui.painter().line_segment([egui::pos2(tick, bar.top() - 3.0), egui::pos2(tick, bar.bottom() + 3.0)], Stroke::new(2.0, Color32::WHITE));
        y += 22.0;
        text(ui, egui::pos2(x, y), Align2::LEFT_CENTER, format!("YES {}   NO {}", st.yes, st.no), 13.0, TEXT);
        text(ui, egui::pos2(rx, y), Align2::RIGHT_CENTER, format!("{} OF {} NEEDED", st.needed, st.voters), 13.0, DIM);
        y += 20.0;
        // Buttons. Clickable only while the UI has the mouse (next-game page, Esc menu); in game the mouse is the
        // engine's, so they are key hints (F5 / F6, input.rs) and the Esc menu's CALL A VOTE section takes clicks.
        let bw = (rx - x - 12.0) / 2.0;
        let live = super::active();
        for (i, (k, label, col)) in [("F5", "YES", GREEN), ("F6", "NO", RED)].into_iter().enumerate() {
            let b = Rect::from_min_size(egui::pos2(x + i as f32 * (bw + 12.0), y), Vec2::new(bw, 34.0));
            let picked = c.mine == Some(i == 0);
            let resp = ui.interact(b, ui.id().with(("call-btn", i)), if live { Sense::click() } else { Sense::hover() });
            if live && resp.clicked() {
                callvote::cast(i == 0);
            }
            ui.painter().rect_filled(b, 2.0, if picked { col.gamma_multiply(0.35) } else if live && resp.hovered() { theme::HOVER } else { Color32::TRANSPARENT });
            ui.painter().rect_stroke(b, 2.0, Stroke::new(if picked { 2.0 } else { 1.0 }, col), StrokeKind::Inside);
            let kr = key_chip(ui, egui::pos2(b.left() + 12.0, b.center().y), k, col);
            text(ui, egui::pos2(kr.right() + 10.0, b.center().y), Align2::LEFT_CENTER, label, 15.0, if picked { Color32::WHITE } else { TEXT });
        }
        if ui.input(|i| i.key_pressed(egui::Key::F5)) {
            callvote::cast(true);
        }
        if ui.input(|i| i.key_pressed(egui::Key::F6)) {
            callvote::cast(false);
        }
    } else if let Some(o) = &c.outcome {
        let kind = CallKind::from_byte(o.kind).map_or("Vote", |k| k.title());
        let (word, col) = match o.result {
            callvote::PASSED => ("PASSED", GREEN),
            callvote::FAILED => ("FAILED", RED),
            _ => ("NOT STARTED", DIM),
        };
        let r = Rect::from_min_size(egui::pos2(right - w, cy - 50.0), Vec2::new(w, 100.0));
        card(ui, r, Some(col));
        let x = r.left() + 20.0;
        text(ui, egui::pos2(x, r.top() + 22.0), Align2::LEFT_CENTER, format!("VOTE  ·  {}", kind.to_uppercase()), 13.0, ACCENT);
        text(ui, egui::pos2(x, r.top() + 50.0), Align2::LEFT_CENTER, word, 22.0, col);
        if o.result != callvote::REFUSED {
            text(ui, egui::pos2(r.right() - 16.0, r.top() + 50.0), Align2::RIGHT_CENTER, format!("YES {}   NO {}", o.yes, o.no), 13.0, DIM);
        }
        text(ui, egui::pos2(x, r.top() + 78.0), Align2::LEFT_CENTER, &o.text, 13.0, TEXT);
    }
}

/// Pause menu: what the CALL A VOTE section asked for.
pub enum CallAction {
    None,
    /// A vote was called: close the pause menu so the panel shows.
    Called,
    /// Kick player: show the player picker.
    PickKick,
}

/// Pause menu section CALL A VOTE: the four kinds, greyed with the reason when the host cannot run them now.
/// Falls back to the old VOTE FOR A NEW GAME item on a host without call votes.
pub fn call_section(ui: &mut egui::Ui) -> CallAction {
    if !callvote::in_network_game() {
        return CallAction::None;
    }
    let caps = callvote::caps();
    ui.add_space(14.0);
    theme::caption(ui, "Call a vote");
    ui.add_space(4.0);
    // A call vote is running: vote on it here (the in-game card only takes F5 / F6).
    if let Some(v) = callvote::view() {
        if let Some(st) = &v.status {
            let kind = CallKind::from_byte(st.kind).unwrap_or(CallKind::Shuffle);
            ui.label(egui::RichText::new(kind.question(&st.target)).size(15.0).color(TEXT));
            ui.label(egui::RichText::new(format!("YES {}  NO {}  ·  {} of {} needed  ·  {} s", st.yes, st.no, st.needed, st.voters, st.secs_left)).size(12.5).color(DIM));
            ui.add_space(4.0);
            if theme::nav_item(ui, "Vote yes  (F5)", v.mine == Some(true)).clicked() {
                callvote::cast(true);
            }
            if theme::nav_item(ui, "Vote no  (F6)", v.mine == Some(false)).clicked() {
                callvote::cast(false);
            }
            return CallAction::None;
        }
    }
    let Some((mask, cooldown, busy)) = caps else {
        if vote::can_call() && theme::nav_item(ui, "Vote for a new game", false).clicked() {
            vote::call();
            return CallAction::Called;
        }
        ui.label(egui::RichText::new("Asking the host ...").size(13.0).color(DIM));
        return CallAction::None;
    };
    let mut act = CallAction::None;
    for k in callvote::ALL {
        let ok = callvote::kind_available(mask, k) && cooldown == 0 && !busy;
        let why = if !callvote::kind_available(mask, k) {
            k.unavailable().to_string()
        } else if busy {
            "A vote is running".into()
        } else if cooldown > 0 {
            format!("Wait {cooldown} s")
        } else {
            String::new()
        };
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 34.0), if ok { Sense::click() } else { Sense::hover() });
        if ok && resp.hovered() {
            ui.painter().rect_filled(rect, 0.0, theme::HOVER);
            ui.painter().rect_filled(Rect::from_min_size(rect.min, Vec2::new(3.0, rect.height())), 0.0, ACCENT);
        }
        text(ui, rect.left_center() + Vec2::new(18.0, 0.0), Align2::LEFT_CENTER, k.title().to_uppercase(), 17.0, if ok { if resp.hovered() { Color32::WHITE } else { TEXT } } else { DIM.gamma_multiply(0.7) });
        if !why.is_empty() {
            let clip = Rect::from_min_max(egui::pos2(rect.left() + 190.0, rect.top()), rect.max);
            ui.painter().with_clip_rect(clip).text(egui::pos2(rect.right() - 6.0, rect.center().y), Align2::RIGHT_CENTER, why, FontId::proportional(11.5), DIM.gamma_multiply(0.8));
        }
        if ok && resp.clicked() {
            if k == CallKind::Kick {
                act = CallAction::PickKick;
            } else {
                callvote::call_vote(k, None);
                act = CallAction::Called;
            }
        }
    }
    act
}

/// Pause menu page: pick the player to kick. Some(true) = vote called (close the menu), Some(false) = back.
pub fn kick_picker(ui: &mut egui::Ui) -> Option<bool> {
    theme::caption(ui, "Kick which player?");
    ui.add_space(6.0);
    let rows = super::scoreboard::rows();
    let mut out = None;
    for p in rows.iter().filter(|p| !p.local) {
        if theme::nav_item(ui, &p.name, false).clicked() {
            callvote::call_vote(CallKind::Kick, Some(&p.name));
            out = Some(true);
        }
    }
    if rows.iter().all(|p| p.local) {
        ui.label(egui::RichText::new("Nobody else is here").size(14.0).color(DIM));
    }
    ui.add_space(12.0);
    if theme::nav_item(ui, "Back", false).clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        out = Some(false);
    }
    out
}
