//! Campaign progress, saved as `<exe dir>\reach-data\progress.json`: which missions were finished on which
//! difficulty, and the last mission started (difficulty, skulls, insertion) for the CAMPAIGN screen's CONTINUE.
//! Written by main (a menu-launched solo or listen-host campaign mission starts) and mission.rs (it ends).

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::log;

pub const DIFFICULTY_NAMES: [&str; 4] = ["easy", "normal", "heroic", "legendary"];

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Progress {
    /// Map stem → difficulties it was finished on (bit d: 0 easy .. 3 legendary).
    pub finished: BTreeMap<String, u8>,
    /// The last mission started from the menu (solo or listen host).
    pub last: Option<Last>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Last {
    pub mission: String,
    /// 0 easy .. 3 legendary.
    pub difficulty: u8,
    /// `options::SKULLS` keys.
    pub skulls: Vec<String>,
    pub insertion: i32,
    /// It ended with a win (CONTINUE then goes to the next mission).
    pub done: bool,
}

/// What CONTINUE launches.
#[derive(Clone, Debug, PartialEq)]
pub struct Pick {
    pub mission: String,
    pub difficulty: u8,
    pub skulls: Vec<String>,
    /// Resume the last mission (a checkpoint may exist) rather than start the next one.
    pub resume: bool,
}

impl Progress {
    /// Highest difficulty `stem` was finished on.
    pub fn best(&self, stem: &str) -> Option<u8> {
        let b = *self.finished.get(stem)?;
        (0..4u8).rev().find(|d| b >> d & 1 != 0)
    }

    pub fn finished_on(&self, stem: &str, difficulty: u8) -> bool {
        self.finished.get(stem).is_some_and(|b| b >> difficulty & 1 != 0)
    }

    /// CONTINUE: the last mission when it was not won, else the next installed mission in story order (`installed`);
    /// after the last mission (or with no history) the first installed mission never finished. None when all are.
    pub fn continue_pick(&self, installed: &[&str]) -> Option<Pick> {
        let (difficulty, skulls) = self.last.as_ref().map_or((1, Vec::new()), |l| (l.difficulty.min(3), l.skulls.clone()));
        if let Some(l) = &self.last {
            if installed.contains(&l.mission.as_str()) {
                if !l.done {
                    return Some(Pick { mission: l.mission.clone(), difficulty, skulls, resume: true });
                }
                let i = installed.iter().position(|m| *m == l.mission).unwrap();
                if let Some(next) = installed.get(i + 1) {
                    return Some(Pick { mission: next.to_string(), difficulty, skulls, resume: false });
                }
            }
        }
        let m = installed.iter().find(|m| !self.finished.contains_key(**m))?;
        Some(Pick { mission: m.to_string(), difficulty, skulls, resume: false })
    }
}

static PROGRESS: Mutex<Option<Progress>> = Mutex::new(None);

/// `<exe dir>\reach-data` (settings, progress, checkpoint saves).
pub fn data_dir() -> PathBuf {
    let dir = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())).unwrap_or_default();
    dir.join("reach-data")
}

fn path() -> PathBuf {
    data_dir().join("progress.json")
}

pub fn get() -> Progress {
    let mut g = PROGRESS.lock().unwrap_or_else(|e| e.into_inner());
    g.get_or_insert_with(|| std::fs::read_to_string(path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()).clone()
}

fn update(f: impl FnOnce(&mut Progress)) {
    let mut p = get();
    f(&mut p);
    *PROGRESS.lock().unwrap_or_else(|e| e.into_inner()) = Some(p.clone());
    let file = path();
    let _ = std::fs::create_dir_all(file.parent().unwrap());
    if let Err(e) = std::fs::write(&file, serde_json::to_string_pretty(&p).unwrap()) {
        log!("progress: could not write {}: {e}", file.display());
    }
}

pub fn difficulty_index(name: &str) -> u8 {
    DIFFICULTY_NAMES.iter().position(|d| *d == name).unwrap_or(1) as u8
}

/// A menu-launched campaign mission starts.
pub fn record_start(mission: &str, difficulty: u8, skulls: u64, insertion: i32) {
    let keys: Vec<String> = crate::options::SKULLS.iter().filter(|s| skulls >> s.2 & 1 != 0).map(|s| s.0.to_string()).collect();
    log!("progress: started {mission} on {} (insertion {insertion})", DIFFICULTY_NAMES[difficulty.min(3) as usize]);
    update(|p| p.last = Some(Last { mission: mission.into(), difficulty: difficulty.min(3), skulls: keys, insertion, done: false }));
}

/// The mission ended with a win (game over without LEAVE GAME).
pub fn record_finished(mission: &str, difficulty: u8) {
    log!("progress: finished {mission} on {}", DIFFICULTY_NAMES[difficulty.min(3) as usize]);
    update(|p| {
        *p.finished.entry(mission.into()).or_insert(0) |= 1 << difficulty.min(3);
        if let Some(l) = p.last.as_mut().filter(|l| l.mission == mission) {
            l.done = true;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: &[&str] = &["m05", "m10", "m20", "m30"];

    fn last(m: &str, done: bool) -> Option<Last> {
        Some(Last { mission: m.into(), difficulty: 2, skulls: vec!["iron".into()], insertion: 0, done })
    }

    #[test]
    fn continue_resumes_unfinished_last() {
        let p = Progress { last: last("m10", false), ..Default::default() };
        let c = p.continue_pick(ALL).unwrap();
        assert_eq!((c.mission.as_str(), c.difficulty, c.resume), ("m10", 2, true));
        assert_eq!(c.skulls, vec!["iron".to_string()]);
    }

    #[test]
    fn continue_goes_to_next_after_win() {
        let mut p = Progress { last: last("m10", true), ..Default::default() };
        p.finished.insert("m10".into(), 1 << 2);
        let c = p.continue_pick(ALL).unwrap();
        assert_eq!((c.mission.as_str(), c.resume), ("m20", false));
    }

    #[test]
    fn continue_after_last_mission_picks_first_unbeaten() {
        let mut p = Progress { last: last("m30", true), ..Default::default() };
        for m in ["m05", "m30"] {
            p.finished.insert(m.into(), 1);
        }
        assert_eq!(p.continue_pick(ALL).unwrap().mission, "m10");
        for m in ALL {
            p.finished.insert(m.to_string(), 1);
        }
        assert_eq!(p.continue_pick(ALL), None);
    }

    #[test]
    fn fresh_profile_starts_at_first_mission() {
        let c = Progress::default().continue_pick(ALL).unwrap();
        assert_eq!((c.mission.as_str(), c.difficulty, c.resume), ("m05", 1, false));
    }

    #[test]
    fn best_difficulty() {
        let mut p = Progress::default();
        assert_eq!(p.best("m10"), None);
        p.finished.insert("m10".into(), 0b0101);
        assert_eq!(p.best("m10"), Some(2));
        assert!(p.finished_on("m10", 0) && !p.finished_on("m10", 1));
    }
}
