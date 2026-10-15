//! Dedicated server CPU throttle. The engine's game thread (`MAIN_THREAD`, loop at 0x180034818) has no frame
//! limiter: every pass pumps the message queue, advances the simulation by real elapsed time and copies the render
//! snapshot, then starts over, so it burns about a core while nothing is due. We patch the call to
//! `ProcessEngineMessageQueue` (0x18000f26c, first call of each pass, call site 0x180034871) to pace the passes.
//! The simulation is time-based, so slower passes only add latency (see notes/dedicated.md).

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use crate::log;

const CALL_SITE: usize = 0x34871;
const WAIT_SITE: usize = 0x33019;
const PUMP: usize = 0xf26c;
const LAUNCH_STATE: usize = 0xc1a114;
/// Launch state 8 = waiting for the first player.
const STATE_WAITING: u32 = 8;

/// Game-thread passes per second while players are in the game (`--server-fps`, default 120).
pub static FPS: AtomicU32 = AtomicU32::new(120);
/// Passes per second while the server waits for its first player (`--idle-fps`, default 20).
pub static IDLE_FPS: AtomicU32 = AtomicU32::new(20);
static BASE: AtomicU64 = AtomicU64::new(0);
static NEXT: AtomicU64 = AtomicU64::new(0); // game thread only
/// false for a listen server: the hook only gives us a game-thread tick (bye handling), no pacing.
static PACE: AtomicBool = AtomicBool::new(true);

/// Parse `--server-fps N` / `--idle-fps N` (0 = unthrottled) from the command line.
pub fn parse(argv: &[String]) -> Result<(), String> {
    for (flag, slot) in [("--server-fps", &FPS), ("--idle-fps", &IDLE_FPS)] {
        if let Some(i) = argv.iter().position(|a| a == flag) {
            let v = argv.get(i + 1).ok_or(format!("{flag} needs N"))?;
            slot.store(v.parse().map_err(|e| format!("{flag}: {e}"))?, Ordering::Relaxed);
        }
    }
    Ok(())
}

fn now_us() -> u64 {
    static T0: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    T0.get_or_init(std::time::Instant::now).elapsed().as_micros() as u64 + 1
}

unsafe extern "system" fn paced_pump() {
    let base = BASE.load(Ordering::Relaxed) as usize;
    let waiting = ((base + LAUNCH_STATE) as *const u32).read_volatile() == STATE_WAITING;
    let fps = if !PACE.load(Ordering::Relaxed) {
        0
    } else if waiting {
        IDLE_FPS.load(Ordering::Relaxed)
    } else {
        FPS.load(Ordering::Relaxed)
    };
    if fps > 0 {
        let period = 1_000_000 / fps as u64;
        let now = now_us();
        let next = NEXT.load(Ordering::Relaxed);
        if now < next {
            std::thread::sleep(std::time::Duration::from_micros(next - now));
        }
        // No catch-up burst after a long pass: next pass is one period after the later of "now" and the old target.
        NEXT.store(now.max(next) + period, Ordering::Relaxed);
    }
    crate::lobby::tick(base); // campaign lobby gather (game thread)
    crate::vote::game_tick(base); // playlist pre-game vote holds launch state 8 (after lobby, overrides it)
    crate::net::drop_left_peers(base); // peers that said bye (game thread)
    let pump: unsafe extern "system" fn() = std::mem::transmute(base + PUMP);
    pump();
}

unsafe extern "system" fn polite_tick_count() -> u32 {
    std::thread::sleep(std::time::Duration::from_millis(1));
    (now_us() / 1000) as u32 // only differences are used
}

/// Memory within 2 GB of the engine (a rel32 call cannot reach our exe) for the jump stubs.
unsafe fn alloc_near(base: usize) -> *mut u8 {
    use windows::Win32::System::Memory::*;
    for k in 0..64usize {
        let want = base + 0x6000000 + k * 0x10000;
        let p = VirtualAlloc(Some(want as _), 0x1000, MEM_COMMIT | MEM_RESERVE, PAGE_EXECUTE_READWRITE) as *mut u8;
        if !p.is_null() {
            return p;
        }
    }
    std::ptr::null_mut()
}

/// `mov rax, target; jmp rax` at `page + slot * 16`.
unsafe fn write_stub(page: *mut u8, slot: usize, target: usize) -> *mut u8 {
    let at = page.add(slot * 16);
    let mut code = vec![0x48u8, 0xB8];
    code.extend_from_slice(&(target as u64).to_le_bytes());
    code.extend_from_slice(&[0xFF, 0xE0]);
    std::ptr::copy_nonoverlapping(code.as_ptr(), at, code.len());
    at
}

unsafe fn patch(site: *mut u8, bytes: &[u8]) -> bool {
    use windows::Win32::System::Memory::*;
    let mut old = PAGE_PROTECTION_FLAGS(0);
    if VirtualProtect(site as _, bytes.len(), PAGE_EXECUTE_READWRITE, &mut old).is_err() {
        return false;
    }
    std::ptr::copy_nonoverlapping(bytes.as_ptr(), site, bytes.len());
    let _ = VirtualProtect(site as _, bytes.len(), old, &mut old);
    true
}

/// Listen server: route the game loop's pump call through `paced_pump` without pacing, for the per-pass
/// game-thread work (dropping peers that said bye). The player's frame rate is untouched.
pub unsafe fn install_tick_only(base: usize) {
    BASE.store(base as u64, Ordering::Relaxed);
    PACE.store(false, Ordering::Relaxed);
    let site = (base + CALL_SITE) as *mut u8;
    if *site != 0xE8 || (site.add(1) as *const i32).read_unaligned() != (PUMP as i64 - (CALL_SITE as i64 + 5)) as i32 {
        log!("throttle: unexpected bytes at the game loop call site, game-thread tick not installed");
        return;
    }
    let page = alloc_near(base);
    if page.is_null() {
        log!("throttle: no stub memory near the engine, game-thread tick not installed");
        return;
    }
    let stub = write_stub(page, 0, paced_pump as usize);
    let mut c = vec![0xE8u8];
    c.extend_from_slice(&((stub as i64 - (site as i64 + 5)) as i32).to_le_bytes());
    log!("throttle: game-thread tick installed, unpaced (patched {})", patch(site, &c));
}

/// Patch the game loop's call so it goes through `paced_pump`, and the render thread's snapshot wait (a
/// `GetTickCount` busy loop of up to 66 ms per frame at 0x180033019) so it sleeps 1 ms per poll.
pub unsafe fn install(base: usize) {
    BASE.store(base as u64, Ordering::Relaxed);
    let site = (base + CALL_SITE) as *mut u8;
    if *site != 0xE8 || (site.add(1) as *const i32).read_unaligned() != (PUMP as i64 - (CALL_SITE as i64 + 5)) as i32 {
        log!("throttle: unexpected bytes at the game loop call site, not installed");
        return;
    }
    let spin = (base + WAIT_SITE) as *mut u8;
    if std::slice::from_raw_parts(spin, 6) != [0xFF, 0x15, 0xB1, 0xB2, 0x8E, 0x00] {
        log!("throttle: unexpected bytes at the render wait call site, not installed");
        return;
    }
    let page = alloc_near(base);
    if page.is_null() {
        log!("throttle: no stub memory near the engine, not installed");
        return;
    }
    let stub = write_stub(page, 0, paced_pump as usize);
    let stub2 = write_stub(page, 1, polite_tick_count as usize);
    let rel = |from: *mut u8, to: *mut u8| (to as i64 - (from as i64 + 5)) as i32;
    let mut c = vec![0xE8u8];
    c.extend_from_slice(&rel(site, stub).to_le_bytes());
    let ok1 = patch(site, &c);
    let mut c = vec![0xE8u8];
    c.extend_from_slice(&rel(spin, stub2).to_le_bytes());
    c.push(0x90);
    let ok2 = patch(spin, &c);
    log!("throttle: game loop paced at {} passes/s ({} while waiting for the first player), render wait sleeps (patched {ok1} {ok2})",
        FPS.load(Ordering::Relaxed), IDLE_FPS.load(Ordering::Relaxed));
}
