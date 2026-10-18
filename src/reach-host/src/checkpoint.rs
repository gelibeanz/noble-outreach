//! Campaign checkpoints (notes/campaign-firefight.md "Saves"). At every checkpoint the engine hands the host its
//! whole game state through host slot 5: the 0xA60000-byte core save (`WriteCoreSaveToHost` 0x180035538). We keep
//! the last one per mission as `<reach-data>\saves\<mission>.bin`. `--checkpoint <file>` puts such a file behind
//! the saved-game pointer of the launch options (+0x2BF10 size, +0x2BF18 pointer), and launch state 5
//! (`LaunchStateLoadSavedGameState` 0x18000e4xx) starts the session from it instead of from the mission start.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;

use crate::log;

/// Size of the engine's core save (game state); the only size launch state 5 restores from memory.
pub const CORE_SAVE_SIZE: usize = 0xA60000;

/// The save the current launch restores from; kept alive until the next game (the engine reads it during load).
static LOADED: Mutex<Option<Vec<u8>>> = Mutex::new(None);
static SAVES: AtomicU32 = AtomicU32::new(0);

pub fn path(mission: &str) -> PathBuf {
    crate::progress::data_dir().join("saves").join(format!("{mission}.bin"))
}

/// A checkpoint of `mission` exists.
pub fn exists(mission: &str) -> bool {
    std::fs::metadata(path(mission)).is_ok_and(|m| m.len() == CORE_SAVE_SIZE as u64)
}

/// The mission was won: its checkpoint is no longer a place to resume from.
pub fn remove(mission: &str) {
    if std::fs::remove_file(path(mission)).is_ok() {
        log!("checkpoint: {mission} finished, save removed");
    }
}

/// Host slot 5 (`game_auto_save`): `void (const u8* data, u32 len)`, on the game thread.
pub unsafe extern "system" fn slot_auto_save(_this: *mut std::ffi::c_void, data: *const u8, len: u32) {
    let n = SAVES.fetch_add(1, Ordering::Relaxed) + 1;
    let Some(mission) = crate::mission::current_mission() else {
        if n == 1 {
            log!("host slot 5 (game_auto_save) {len:#x} bytes: not a menu-launched campaign, not kept");
        }
        return;
    };
    if data.is_null() || len as usize != CORE_SAVE_SIZE {
        log!("checkpoint: slot 5 save of {len:#x} bytes ignored (not a core save)");
        return;
    }
    // The engine reuses its buffer: copy now, write the file off the game thread.
    let bytes = std::slice::from_raw_parts(data, len as usize).to_vec();
    let file = path(&mission);
    log!("checkpoint: {mission} save {n} ({len:#x} bytes) -> {}", file.display());
    std::thread::spawn(move || {
        let _ = std::fs::create_dir_all(file.parent().unwrap());
        let tmp = file.with_extension("tmp");
        if let Err(e) = std::fs::write(&tmp, &bytes).and_then(|_| std::fs::rename(&tmp, &file)) {
            log!("checkpoint: could not write {}: {e}", file.display());
        }
    });
}

/// Launch options: point the saved-game fields at the save in `file` (campaign only).
pub fn apply(options: &mut [u8], file: &Path) -> Result<(), String> {
    let bytes = std::fs::read(file).map_err(|e| format!("checkpoint {}: {e}", file.display()))?;
    if bytes.len() != CORE_SAVE_SIZE {
        return Err(format!("checkpoint {}: {} bytes, expected {CORE_SAVE_SIZE:#x}", file.display(), bytes.len()));
    }
    let mut g = LOADED.lock().unwrap_or_else(|e| e.into_inner());
    let buf = g.insert(bytes);
    options[0x2BF10..0x2BF14].copy_from_slice(&(CORE_SAVE_SIZE as u32).to_le_bytes());
    options[0x2BF18..0x2BF20].copy_from_slice(&(buf.as_ptr() as u64).to_le_bytes());
    log!("  options+0x2bf18: saved game {} ({CORE_SAVE_SIZE:#x} bytes at {:?}) -> launch state 5", file.display(), buf.as_ptr());
    Ok(())
}
