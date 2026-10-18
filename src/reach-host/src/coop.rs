//! `--coop-max N` (N 5..8): campaign and Firefight co-op beyond 4 players. Reach was built for 4 co-op players in
//! campaign; Firefight already allows 8. The engine refuses to build campaign game options for more than 4 players
//! in three places, all constants we raise to N (notes/campaign-firefight.md, "More than 4 co-op players"):
//! 1. `BuildGameOptionsFromSession` 0x18040a81c: for a campaign session (game mode 1) the session's player count
//!    (session +0x1738) must be <= 4: `cmp edx, 4` at 0x18040ad63 (survival, mode 6, has `cmp edx, 8` at 0x18040ae62).
//! 2. `ValidateGameOptions` 0x180057c90: for campaign (options mode 1) the active players must be 1..4:
//!    `mov ecx, 4` at 0x18005804a (survival keeps 8).
//! 3. The session start check 0x180408ddc (launch state 10): for campaign (mode 1) the player count must be <= the
//!    network-config value at 0x180d1fd90 (4, set by 0x1803bc828), else start error 0x1a "too many for net coop"
//!    (exit reason 4): `cmp edi, [rip+..]` at 0x180409544 → `cmp edi, N` (survival, mode 6, has `cmp edi, 8`).
//! Off by default (today's 4-player cap). The patch is applied on every instance that sets it: a server with
//! `--coop-max N`, clients that join a co-op server with more than 4 slots (they rebuild options from the session) and a
//! listen host (menu HOST CO-OP) started with `--players` above 4.

use std::sync::atomic::{AtomicU32, Ordering};

use crate::log;

/// Engine limit we do not lift: Firefight's checks stop at 8 and so does our target.
pub const MAX: u32 = 8;
/// 0 = off (engine default 4), else the campaign player limit written into the engine.
pub static COOP_MAX: AtomicU32 = AtomicU32::new(0);

pub fn parse(argv: &[String]) -> Result<(), String> {
    if let Some(i) = argv.iter().position(|a| a == "--coop-max") {
        let v = argv.get(i + 1).ok_or("--coop-max needs N (5..8)")?;
        let n: u32 = v.parse().map_err(|e| format!("--coop-max: {e}"))?;
        if !(4..=MAX).contains(&n) {
            return Err(format!("--coop-max {n}: must be 4..{MAX}"));
        }
        COOP_MAX.store(if n > 4 { n } else { 0 }, Ordering::Relaxed);
    }
    Ok(())
}

/// Co-op player cap for this run: 4 unless `--coop-max` raised it.
pub fn cap() -> u32 {
    match COOP_MAX.load(Ordering::Relaxed) {
        0 => 4,
        n => n,
    }
}

/// A client joining a co-op server that offers more than 4 slots needs the same patch; a listen host asked for more.
pub fn enable_for(players: u32) {
    if players > 4 && COOP_MAX.load(Ordering::Relaxed) == 0 {
        COOP_MAX.store(players.min(MAX), Ordering::Relaxed);
    }
}

pub unsafe fn install(base: usize) {
    let n = COOP_MAX.load(Ordering::Relaxed);
    if n == 0 {
        return;
    }
    let a = crate::lobby::patch_bytes((base + 0x40ad63) as *mut u8, &[0x83, 0xFA, 0x04], &[0x83, 0xFA, n as u8]);
    let b = crate::lobby::patch_bytes((base + 0x05804a) as *mut u8, &[0xB9, 0x04, 0x00, 0x00, 0x00], &[0xB9, n as u8, 0x00, 0x00, 0x00]);
    let c = crate::lobby::patch_bytes(
        (base + 0x409544) as *mut u8,
        &[0x3B, 0x3D, 0x46, 0x68, 0x91, 0x00],
        &[0x83, 0xFF, n as u8, 0x90, 0x90, 0x90],
    );
    log!("coop-max: campaign player limit 4 -> {n}: 0x18040ad63 {a}, 0x18005804a {b}, 0x180409544 {c}");
}
