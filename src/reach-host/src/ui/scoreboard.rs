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

/// Called on the game thread every input poll (host slot 36, player 0).
pub unsafe fn game_tick() {
    let base = crate::crash::ENGINE_BASE.load(Ordering::Relaxed);
    if base == 0 {
        return;
    }
    if GAME_TLS.load(Ordering::Relaxed) != 0 {
        // Listen server: the address banner starts when the host's player first has a unit.
        static SPAWNED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
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

/// `REACH_SB_DUMP=1`: on each press, write the player table and game-results globals to `sb-dump-N.bin`
/// (layout discovery; see notes).
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
        let mut out = Vec::new();
        let arr = *((block + 0x18) as *const usize);
        out.extend_from_slice(std::slice::from_raw_parts(arr as *const u8, 0x60));
        let data = *((arr + 0x50) as *const usize);
        out.extend_from_slice(std::slice::from_raw_parts(data as *const u8, 16 * 0x490));
        out.extend_from_slice(std::slice::from_raw_parts((base + 0x28b83b0) as *const u8, 0x1828c9060 - 0x1828b83b0));
        let n = N.fetch_add(1, Ordering::Relaxed);
        let p = std::env::current_exe().unwrap_or_default().with_file_name(format!("sb-dump-{n}.bin"));
        let _ = std::fs::write(&p, &out);
        log!("scoreboard: dumped {} bytes to {} (players array {arr:#x}, data {data:#x})", out.len(), p.display());
    }
}

/// One scoreboard line.
pub struct Row {
    pub name: String,
    /// Team index (datum +0xAD), -1 in free-for-all.
    pub team: i8,
    pub score: i32,
    pub local: bool,
}

/// Engine player table (TLS +0x18 data array, datum 0x490 bytes at +0x50; salt u16 at +0x00 non-zero = in use):
/// +0x08 u64 xuid, +0xAD i8 team (-1 none), +0xB0 UTF-16 name[16]. Score (as the HUD's `#local_player_score`,
/// 0x180079b20): game-engine globals TLS +0x20, valid while +0x841C is 1..4, s16 at +0x3270 + index*8
/// (+2 when TLS +0x48 → +0x1EAB8 is set: round score).
pub fn rows() -> Vec<Row> {
    let block = GAME_TLS.load(Ordering::Relaxed);
    let mut out = Vec::new();
    if block == 0 {
        return out;
    }
    unsafe {
        let rd = |a: usize| (a as *const usize).read_volatile();
        let arr = rd(block + 0x18);
        if arr == 0 {
            return out;
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
        let me = crate::host::LOCAL_XUID.load(Ordering::Relaxed);
        for i in 0..16usize {
            let p = data + i * 0x490;
            if data == 0 || ((p) as *const u16).read_volatile() == 0 {
                continue;
            }
            let name: Vec<u16> = (0..16).map(|k| ((p + 0xb0 + 2 * k) as *const u16).read_volatile()).take_while(|&c| c != 0).collect();
            out.push(Row {
                name: String::from_utf16_lossy(&name),
                team: ((p + 0xad) as *const i8).read_volatile(),
                score: scores.map_or(0, |s| ((s + i * 8) as *const i16).read_volatile() as i32),
                local: ((p + 0x08) as *const u64).read_unaligned() == me,
            });
        }
    }
    out.sort_by(|a, b| b.score.cmp(&a.score).then(a.name.cmp(&b.name)));
    out
}

/// Team colours (Reach's red, blue, green, orange, purple, gold, brown, pink).
fn team_colour(t: i8) -> egui::Color32 {
    const C: [(u8, u8, u8); 8] = [(200, 50, 50), (50, 100, 210), (60, 160, 60), (230, 130, 30), (140, 70, 190), (210, 180, 40), (130, 90, 50), (220, 110, 170)];
    match C.get(t as usize) {
        Some(&(r, g, b)) if t >= 0 => egui::Color32::from_rgb(r, g, b),
        _ => egui::Color32::from_gray(90),
    }
}

static LOGGED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Should the overlay draw it this frame (and is there anything to draw)?
pub fn visible() -> bool {
    GAME_TLS.load(Ordering::Relaxed) != 0 && held()
}

/// Centered panel: rank, name, score; the local player highlighted. Logged once per press.
pub fn draw(ui: &mut egui::Ui) {
    use super::theme;
    let rows = rows();
    if !LOGGED.swap(true, Ordering::Relaxed) {
        let list: Vec<String> = rows.iter().map(|r| format!("{} (team {}) {}", r.name, r.team, r.score)).collect();
        log!("scoreboard: shown, {} players: {}", rows.len(), list.join(", "));
    }
    let full = ui.max_rect();
    let w = 560.0;
    let h = 110.0 + 34.0 * rows.len().max(1) as f32;
    let r = egui::Rect::from_center_size(egui::pos2(full.center().x, full.top() + 90.0 + h / 2.0), egui::vec2(w, h));
    theme::panel(ui, r, |ui| {
        theme::heading(ui, "Scoreboard");
        ui.add_space(8.0);
        let mut row = |ui: &mut egui::Ui, rank: &str, name: &str, score: &str, colour: Option<egui::Color32>, hl: bool, size: f32| {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 30.0), egui::Sense::hover());
            if hl {
                ui.painter().rect_filled(rect, 2.0, egui::Color32::from_rgba_unmultiplied(242, 140, 40, 40));
            }
            if let Some(c) = colour {
                ui.painter().rect_filled(egui::Rect::from_min_size(rect.min, egui::vec2(5.0, rect.height())), 0.0, c);
            }
            let y = rect.center().y;
            let col = if hl { theme::ACCENT } else { egui::Color32::from_gray(225) };
            let font = egui::FontId::proportional(size);
            ui.painter().text(egui::pos2(rect.left() + 16.0, y), egui::Align2::LEFT_CENTER, rank, font.clone(), theme::DIM);
            ui.painter().text(egui::pos2(rect.left() + 56.0, y), egui::Align2::LEFT_CENTER, name, font.clone(), col);
            ui.painter().text(egui::pos2(rect.right() - 12.0, y), egui::Align2::RIGHT_CENTER, score, font, col);
        };
        row(ui, "#", "PLAYER", "SCORE", None, false, 13.0);
        if rows.is_empty() {
            ui.label(egui::RichText::new("No players yet").color(theme::DIM));
        }
        for (i, r) in rows.iter().enumerate() {
            row(ui, &(i + 1).to_string(), &r.name, &r.score.to_string(), Some(team_colour(r.team)), r.local, 18.0);
        }
    });
}

/// Not drawn this frame: the next press logs its rows again.
pub fn hidden() {
    LOGGED.store(false, Ordering::Relaxed);
}
