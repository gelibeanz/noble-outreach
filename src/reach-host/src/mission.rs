//! Campaign: a finished mission rolls into the next one, as in MCC (notes/campaign-firefight.md "Mission end").
//! The engine ends a mission with set_game_state(0xa) (game over) and then stops its game thread (life cycle back
//! to the main menu, which a hosted engine turns into an exit); it cannot load another map in-process. So the host
//! relaunches itself with the next mission. A listen host first tells its joiners (the vote RESULT message with
//! restart set), and they follow it like a vote restart. Noble Actual (m05) is only the opening cutscene, so it
//! ends this way about 15 s in.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::OnceLock;

use crate::log;

/// This process plays a campaign mission it may roll on from: (mission stem, difficulty name, MCC folder).
/// Set by main for menu-launched campaign games (solo or listen host), not for dedicated servers or joiners.
pub static CURRENT: OnceLock<(String, String, std::path::PathBuf)> = OnceLock::new();
/// The mission ended (game over seen without LEAVE GAME): relaunch into the next one when the game thread ends.
static DONE: AtomicBool = AtomicBool::new(false);
/// Other machines in the session when the mission ended (they are following us).
static PEERS: AtomicU32 = AtomicU32::new(0);

/// Host slot 3, state 0xa (game over).
pub fn on_game_over() {
    let Some((stem, difficulty, mcc)) = CURRENT.get() else { return };
    if crate::ui::overlay::leave_requested().is_some() || DONE.swap(true, Ordering::Relaxed) {
        return;
    }
    let Some(next) = crate::ui::campaign::next_mission(stem, mcc) else {
        log!("mission: {stem} finished; it was the last mission, back to the menu");
        return;
    };
    let peers = if crate::lobby::LISTEN.load(Ordering::Relaxed) {
        crate::vote::announce_next_mission(next, crate::ui::campaign::mission_name(next), difficulty)
    } else {
        0
    };
    PEERS.store(peers as u32, Ordering::Relaxed);
    log!("mission: {stem} finished; next mission {next} ({peers} joiner(s) told to follow)");
}

/// Main thread, game thread ended: the arguments to relaunch with for the next mission, if one is due.
pub fn relaunch_args(argv: &[String]) -> Option<Vec<String>> {
    if !DONE.load(Ordering::Relaxed) {
        return None;
    }
    let (stem, difficulty, mcc) = CURRENT.get()?;
    let next = crate::ui::campaign::next_mission(stem, mcc)?;
    let mode = ["--campaign".to_string(), next.to_string(), "--difficulty".into(), difficulty.clone(), "--insertion".into(), "0".into()];
    let mut out = crate::playlist::replace_mode_args(argv, &mode);
    if crate::lobby::LISTEN.load(Ordering::Relaxed) {
        // Start as soon as everyone who played the last mission is back (the host counts as one), or after 60 s;
        // alone, at once.
        let peers = PEERS.load(Ordering::Relaxed);
        let mut kept = Vec::new();
        let mut it = out.iter();
        while let Some(a) = it.next() {
            if a == "--lobby-wait" || (a == "--players" && peers > 0) {
                it.next();
                continue;
            }
            kept.push(a.clone());
        }
        out = kept;
        if peers > 0 {
            out.extend(["--players".into(), (peers + 1).to_string(), "--lobby-wait".into(), "60".into()]);
        } else {
            out.extend(["--lobby-wait".into(), "0".into()]);
        }
    }
    Some(out)
}
