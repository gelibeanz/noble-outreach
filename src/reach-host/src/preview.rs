//! 3D armour preview for the CUSTOMIZE screen (notes/customization.md "3D preview").
//! Engine side: script commands through engine slot 9 (`HS: ` prefix), used to put the director's orbit camera on
//! the local player's biped.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};

use crate::log;

/// Engine object and its slot 9 (`EnginePostCommand(const char*)`), set by main once the engine exists.
pub static ENGINE: AtomicUsize = AtomicUsize::new(0);
pub static COMMAND: AtomicUsize = AtomicUsize::new(0);

/// Queue one Halo script expression, e.g. `(chud_show 0)`. The engine compiles and runs it on the game thread
/// while a map is loaded (dispatcher in `ProcessEngineMessageQueue`; failures go to host slot 44).
pub fn hs(expr: &str) -> bool {
    let (engine, post) = (ENGINE.load(Ordering::Relaxed), COMMAND.load(Ordering::Relaxed));
    if engine == 0 || post == 0 {
        return false;
    }
    let post: unsafe extern "system" fn(*mut std::ffi::c_void, *const u8) = unsafe { std::mem::transmute(post) };
    let text = format!("HS: {expr}\0");
    unsafe { post(engine as *mut _, text.as_ptr()) };
    log!("preview: hs {expr}");
    true
}

/// True in the hidden preview child process (`--preview <name>`).
pub static CHILD: AtomicBool = AtomicBool::new(false);

/// Orbit view the child applies every game tick: yaw / pitch offsets (radians, yaw relative to the biped's facing)
/// and camera distance (world units). f32 bits.
static VIEW_YAW: AtomicU32 = AtomicU32::new(0);
static VIEW_PITCH: AtomicU32 = AtomicU32::new(0);
static VIEW_DIST: AtomicU32 = AtomicU32::new(0);

pub fn set_view(yaw: f32, pitch: f32, dist: f32) {
    VIEW_YAW.store(yaw.to_bits(), Ordering::Relaxed);
    VIEW_PITCH.store(pitch.to_bits(), Ordering::Relaxed);
    VIEW_DIST.store(dist.to_bits(), Ordering::Relaxed);
}

/// Engine RVAs (frozen DLL, notes/customization.md "3D preview").
const RVA_TLS_INDEX: usize = 0xc17b18;
/// `DirectorSetCameraMode(director, mode, transition secs (xmm2), force (r9b))`.
const RVA_DIRECTOR_SET_MODE: usize = 0xc5834;
/// Vtable of the observer director: orbits the player's unit, yaw/pitch at director +0x198/+0x19C, camera mode from
/// the table at 0xa2df60 indexed by +0x190 (entry 0 = mode 1, the orbiting camera).
const RVA_OBSERVER_DIRECTOR_VTABLE: usize = 0xa2df08;

/// Unit handle the orbit camera was installed for (respawns get a new unit and need a fresh install).
static ORBIT_UNIT: AtomicU32 = AtomicU32::new(u32::MAX);
static SPAWN_TICKS: AtomicU32 = AtomicU32::new(0);
/// Species of the current biped (1 = Elite) and the unit last killed for a species change (once per biped).
static BIPED_ELITE: AtomicU32 = AtomicU32::new(0);
static KILLED_UNIT: AtomicU32 = AtomicU32::new(u32::MAX);

unsafe fn director0() -> Option<*mut u8> {
    let base = crate::crash::ENGINE_BASE.load(Ordering::Relaxed);
    if base == 0 {
        return None;
    }
    let idx = ((base + RVA_TLS_INDEX) as *const u32).read() as usize;
    let tls: *const *const u8;
    core::arch::asm!("mov {}, gs:[0x58]", out(reg) tls);
    if tls.is_null() {
        return None;
    }
    let block = *tls.add(idx);
    if block.is_null() {
        return None;
    }
    let dirs = *(block.add(0x188) as *const *mut u8);
    (!dirs.is_null()).then_some(dirs)
}

/// Game thread, every input poll (host slot 36), preview child only. Once player 0 has a unit: hide the HUD, swap
/// director 0 for the observer director (orbit camera on the player's own biped; the game director would force the
/// first-person camera back every tick), then keep writing the requested yaw / pitch / distance.
pub unsafe fn game_tick() {
    if !CHILD.load(Ordering::Relaxed) {
        return;
    }
    static PATCHED: std::sync::Once = std::sync::Once::new();
    PATCHED.call_once(|| disable_observer_collision());
    let Some(v) = crate::input::player0_view() else { return };
    let spawned = v.unit != u32::MAX && v.unit >> 16 != 0;
    if std::env::var_os("REACH_PREVIEW_DUMP").is_some() {
        player_dump();
    }
    apply_pending(spawned);
    if !spawned {
        SPAWN_TICKS.store(0, Ordering::Relaxed);
        // First spawn only: after a species-change kill the camera does not re-target a biped that respawned at
        // once (notes/customization.md, open), so swaps keep the variant's respawn wait.
        if ORBIT_UNIT.load(Ordering::Relaxed) == u32::MAX {
            skip_respawn_wait();
        }
        return;
    }
    if let Some(t) = KILLED_AT.lock().unwrap_or_else(|e| e.into_inner()).take() {
        log!("preview: new biped {} ms after the kill", t.elapsed().as_millis());
    }
    let Some(dir) = director0() else { return };
    let base = crate::crash::ENGINE_BASE.load(Ordering::Relaxed);
    let want_elite = *crate::host::profile_ptr().add(0x38) as u32;
    if ORBIT_UNIT.load(Ordering::Relaxed) != v.unit {
        if SPAWN_TICKS.load(Ordering::Relaxed) == 0 {
            // The species is fixed when the biped is made, from the look applied (profile + msg 5) by then.
            BIPED_ELITE.store(want_elite, Ordering::Relaxed);
        }
        // Give the spawn a few ticks to settle (the game director sets itself up on the new unit first).
        let t = SPAWN_TICKS.fetch_add(1, Ordering::Relaxed);
        if t == 2 {
            log!("preview: biped {:#x} spawned at {:?} facing {:.2}", v.unit, v.pos, v.yaw);
        }
        if t < 10 {
            return;
        }
        // Not earlier: during its first ticks the spawn still sets the new biped's aim to the spawn point's facing.
        if let Some(sp) = spot() {
            teleport(v.unit, sp);
        }
        ORBIT_UNIT.store(v.unit, Ordering::Relaxed);
        FACING.store(spot().map_or(v.yaw, |s| s[3]).to_bits(), Ordering::Relaxed);
        hs("(chud_show 0)");
        hs("(player_disable_movement 1)");
        *(dir as *mut usize) = base + RVA_OBSERVER_DIRECTOR_VTABLE;
        *(dir.add(0x190) as *mut u32) = 0;
        *dir.add(0x194) = 1;
        *(dir.add(0x198) as *mut f32) = 0.0;
        let set_mode: unsafe extern "system" fn(*mut u8, u32, f32, u8) -> u8 = std::mem::transmute(base + RVA_DIRECTOR_SET_MODE);
        // Transition time travels in xmm2 (third argument), r9b = force (as the engine's own director restore).
        let ok = set_mode(dir, 1, 0.0, 0);
        log!("preview: orbit camera on unit {:#x} (facing {:.2} rad), set_mode -> {ok}", v.unit, v.yaw);
        // Frames resume 400 ms after the (re)install; the menu kept the last good one meanwhile.
        *LIVE_AT.lock().unwrap_or_else(|e| e.into_inner()) = Some(std::time::Instant::now());
        let mut quiet = crate::ui::settings::get().audio;
        quiet.master = 0.0;
        crate::audio::apply_volume(&quiet); // the menu has its own sound; the preview stays silent
    }
    // The move is a script command, which the engine now and then drops: check twice a second and repeat it.
    if SPAWN_TICKS.fetch_add(1, Ordering::Relaxed) % 30 == 29 {
        if let (Some(sp), Some(p)) = (spot(), v.pos) {
            if (p[0] - sp[0]).powi(2) + (p[1] - sp[1]).powi(2) > 1.0 {
                SPOT_MOVED.store(true, Ordering::Relaxed);
            }
        }
    }
    if SPOT_MOVED.swap(false, Ordering::Relaxed) {
        if let Some(sp) = spot() {
            teleport(v.unit, sp);
            FACING.store(sp[3].to_bits(), Ordering::Relaxed);
        }
    }
    if want_elite != BIPED_ELITE.load(Ordering::Relaxed) && KILLED_UNIT.swap(v.unit, Ordering::Relaxed) != v.unit {
        // Spartan <-> Elite needs a new biped: respawn (Slayer respawns in ~5 s). Stop publishing frames so the
        // menu shows the last model under "switching" instead of the death camera and the loadout picker.
        *LIVE_AT.lock().unwrap_or_else(|e| e.into_inner()) = None;
        if let Some(h) = header() {
            h.add(4).write_volatile(STATE_SWITCHING);
        }
        hs("(unit_kill_silent (player_get 0))");
        *KILLED_AT.lock().unwrap_or_else(|e| e.into_inner()) = Some(std::time::Instant::now());
        return;
    }
    // Hold the look yaw on the facing the camera orbits from (the player's look drives the biped's aim and body).
    if spot().is_some() {
        if let Some(block) = crate::input::engine_tls_block() {
            let pc = *(block.add(0xd8) as *const *mut u8);
            if !pc.is_null() {
                (pc.add(0x94) as *mut f32).write_unaligned(f32::from_bits(FACING.load(Ordering::Relaxed)));
            }
        }
    }
    // Orbiting camera (director +8): +0x18 yaw, +0x1C pitch, +0x20 distance (the camera clamps it to the target's
    // size), looking at the target from that angle. The observer director copies its +0x198 / +0x19C into the
    // camera's +0x24 (height added to the eye point) / +0x28 (marker offset scale) every tick.
    let cam = dir.add(8);
    // Measured on captures: the model faces the camera at facing + π/2 (with facing + π, as before 2026-10-17, the
    // view yaw 0 showed the biped's right side).
    let yaw = f32::from_bits(FACING.load(Ordering::Relaxed)) + std::f32::consts::FRAC_PI_2 + f32::from_bits(VIEW_YAW.load(Ordering::Relaxed));
    *(cam.add(0x18) as *mut f32) = yaw;
    *(cam.add(0x1c) as *mut f32) = f32::from_bits(VIEW_PITCH.load(Ordering::Relaxed));
    let dist = f32::from_bits(VIEW_DIST.load(Ordering::Relaxed));
    if dist > 0.0 {
        *(cam.add(0x20) as *mut f32) = dist;
    }
    // Zoom = field of view (profile +0x14, re-read on engine message 5).
    static FOV: AtomicU32 = AtomicU32::new(0);
    let fov = std::env::var("REACH_PREVIEW_FOV").ok().and_then(|v| v.parse::<u32>().ok()).unwrap_or(VIEW_FOV.load(Ordering::Relaxed));
    if fov > 0 && FOV.swap(fov, Ordering::Relaxed) != fov {
        (crate::host::profile_ptr().add(0x14) as *mut i32).write_unaligned(fov as i32);
        crate::ui::settings_apply::post_engine(5);
        log!("preview: fov {fov}");
    }
    *(dir.add(0x198) as *mut f32) = 0.0;
    *(dir.add(0x19c) as *mut f32) = 0.0;
}

/// Call of the observer's camera collision test (0x18018a138) in `UpdateObserverCollision` 0x1800ca90c. The test
/// pulls the orbit camera in front of any wall between it and the biped (it scales the distance by the hit
/// fraction); without it both fractions stay 1.0 and the camera orbits at the requested distance through anything.
const RVA_OBSERVER_COLLISION_CALL: usize = 0xcaa0d;
const RVA_OBSERVER_COLLISION_TEST: usize = 0x18a138;

/// Preview child only: NOP that call, so the camera can circle the biped a full 360 degrees.
unsafe fn disable_observer_collision() {
    let base = crate::crash::ENGINE_BASE.load(Ordering::Relaxed);
    let at = (base + RVA_OBSERVER_COLLISION_CALL) as *mut u8;
    let rel = (at.add(1) as *const i32).read_unaligned();
    let target = (at as usize + 5).wrapping_add(rel as isize as usize);
    if *at != 0xe8 || target != base + RVA_OBSERVER_COLLISION_TEST {
        log!("preview: observer collision call not found (other DLL build?): camera keeps its collision");
        return;
    }
    use windows::Win32::System::Memory::{VirtualProtect, PAGE_EXECUTE_READWRITE, PAGE_PROTECTION_FLAGS};
    let mut old = PAGE_PROTECTION_FLAGS(0);
    if VirtualProtect(at as *const _, 5, PAGE_EXECUTE_READWRITE, &mut old).is_ok() {
        std::ptr::copy_nonoverlapping([0x0f, 0x1f, 0x44, 0x00, 0x00].as_ptr(), at, 5);
        let _ = VirtualProtect(at as *const _, 5, old, &mut old);
        log!("preview: observer camera collision off (haloreach+{RVA_OBSERVER_COLLISION_CALL:#x} nop)");
    }
}

/// `ObjectTeleport(object, position*, forward*, up*, detach)`: the body of the `object_teleport` script command
/// (0x18019be14), which also turns a unit's aim to `forward`.
const RVA_OBJECT_TELEPORT: usize = 0x1ec468;

/// Where the preview biped stands, per map: x, y, z, facing (radians). REACH_PREVIEW_SPOT="x y z yaw" overrides.
/// Spire: open sand near the middle, facing -x so the camera looks at the spire. The biped drops ~0.3 units.
const SPOTS: &[(&str, [f32; 4])] = &[("35_island", [-178.8, -386.0, -59.4, std::f32::consts::PI])];

/// Test override of the spot (`set spot x y z yaw` script step); a change moves the current biped.
static SPOT_TEST: std::sync::Mutex<Option<[f32; 4]>> = std::sync::Mutex::new(None);
static SPOT_MOVED: AtomicBool = AtomicBool::new(false);

pub fn set_spot(s: [f32; 4]) {
    *SPOT_TEST.lock().unwrap_or_else(|e| e.into_inner()) = Some(s);
    SPOT_MOVED.store(true, Ordering::Relaxed);
}

fn spot() -> Option<[f32; 4]> {
    if let Some(s) = *SPOT_TEST.lock().unwrap_or_else(|e| e.into_inner()) {
        return Some(s);
    }
    if let Some(v) = std::env::var("REACH_PREVIEW_SPOT").ok().map(|v| v.split_whitespace().filter_map(|x| x.parse::<f32>().ok()).collect::<Vec<_>>()) {
        if v.len() == 4 {
            return Some([v[0], v[1], v[2], v[3]]);
        }
    }
    let map = std::env::var("REACH_PREVIEW_MAP").unwrap_or_else(|_| PREVIEW_MAP.into());
    SPOTS.iter().find(|s| s.0 == map).map(|s| s.1)
}

/// Game thread: put the biped on the preview spot, facing `s[3]`. Physics calls are only safe inside the game's
/// own update (from the input poll, haloreach+0x14173 faults on a null per-thread physics table), so the move runs
/// as a script command: `(cheat_teleport_to_camera)` is a stripped stub with no parameters, and its evaluate
/// pointer is pointed at `hs_teleport` below.
unsafe fn teleport(unit: u32, s: [f32; 4]) {
    if !install_teleport_command() {
        return;
    }
    *PENDING_TELEPORT.lock().unwrap_or_else(|e| e.into_inner()) = Some((unit, s, Some(f32::NAN)));
    hs("(cheat_teleport_to_camera)");
}

/// Test scripts (`set teleport`), game thread: move player `n`'s biped to `s` (x, y, z, facing yaw). On a host any
/// player (the host simulates every biped); player 0 also turns the local view.
pub unsafe fn teleport_player(n: usize, s: [f32; 4], pitch: f32) {
    let unit = if n == 0 {
        crate::input::player0_view().map_or(u32::MAX, |v| v.unit)
    } else {
        player0_datum().map_or(u32::MAX, |d| {
            let p = d.add(n * 0x490);
            if (p as *const u16).read() == 0 { u32::MAX } else { (p.add(0x28) as *const u32).read() }
        })
    };
    if unit == u32::MAX {
        log!("preview: teleport: player {n} has no unit");
        return;
    }
    if install_teleport_command() {
        *PENDING_TELEPORT.lock().unwrap_or_else(|e| e.into_inner()) = Some((unit, s, (n == 0).then_some(pitch)));
        hs("(cheat_teleport_to_camera)");
    }
}

/// (unit, x y z yaw, turn the local player's view too: Some(pitch), NaN = keep the pitch).
static PENDING_TELEPORT: std::sync::Mutex<Option<(u32, [f32; 4], Option<f32>)>> = std::sync::Mutex::new(None);
/// Script function table (0x180a28d40, pointers to definitions: +8 name, +0x20 evaluate, +0x38 parameter count).
const RVA_HS_FUNCTIONS: usize = 0xa28d40;
const HS_TELEPORT_INDEX: usize = 388;
/// Stripped evaluate stub (returns nothing) and `ScriptReturn(thread, value)` 0x180085de0.
const RVA_HS_STUB: usize = 0x86cbc;
const RVA_HS_RETURN: usize = 0x85de0;

/// Point `cheat_teleport_to_camera`'s evaluate at `hs_teleport` (once; checks the entry is the expected stub).
unsafe fn install_teleport_command() -> bool {
    static DONE: AtomicU32 = AtomicU32::new(0); // 0 not tried, 1 installed, 2 failed
    match DONE.load(Ordering::Relaxed) {
        1 => return true,
        2 => return false,
        _ => {}
    }
    let base = crate::crash::ENGINE_BASE.load(Ordering::Relaxed);
    let def = *((base + RVA_HS_FUNCTIONS + HS_TELEPORT_INDEX * 8) as *const *mut u8);
    let name = std::ffi::CStr::from_ptr(*(def.add(8) as *const *const std::ffi::c_char));
    let eval = def.add(0x20) as *mut usize;
    if name.to_bytes() != b"cheat_teleport_to_camera" || eval.read() != base + RVA_HS_STUB || *(def.add(0x38) as *const u16) != 0 {
        log!("preview: script entry {HS_TELEPORT_INDEX} is not the expected stub (other DLL build?): no teleport");
        DONE.store(2, Ordering::Relaxed);
        return false;
    }
    use windows::Win32::System::Memory::{VirtualProtect, PAGE_PROTECTION_FLAGS, PAGE_READWRITE};
    let mut old = PAGE_PROTECTION_FLAGS(0);
    if VirtualProtect(eval as *const _, 8, PAGE_READWRITE, &mut old).is_err() {
        DONE.store(2, Ordering::Relaxed);
        return false;
    }
    eval.write(hs_teleport as *const () as usize);
    let _ = VirtualProtect(eval as *const _, 8, old, &mut old);
    DONE.store(1, Ordering::Relaxed);
    true
}

/// Script evaluate (function index, script thread, initialize), on the game thread inside the game's update.
unsafe extern "system" fn hs_teleport(_index: i16, thread: u32, _init: u8) {
    let base = crate::crash::ENGINE_BASE.load(Ordering::Relaxed);
    if let Some((unit, s, turn)) = PENDING_TELEPORT.lock().unwrap_or_else(|e| e.into_inner()).take() {
        // Per-thread physics context: TLS +0x338 indexes the table at [0x180b93550] + 0x34318; on the thread that
        // runs script commands the slot is empty (null deref at haloreach+0x14173). Borrow the first filled slot.
        let block = crate::input::engine_tls_block().unwrap_or(std::ptr::null());
        let slot = block.add(0x338) as *mut i32;
        let saved = if block.is_null() { None } else { Some(slot.read()) };
        let g = *((base + 0xb93550) as *const *const *const usize);
        let table = if g.is_null() { std::ptr::null() } else { *((g as usize + 0x34318) as *const *const usize) };
        let filled = (0..8).find(|&i| !table.is_null() && *table.add(i) != 0);
        let (Some(_), Some(i)) = (saved, filled) else {
            let ret: unsafe extern "system" fn(u32, usize) = std::mem::transmute(base + RVA_HS_RETURN);
            ret(thread, 0);
            return;
        };
        slot.write(i as i32);
        let f: unsafe extern "system" fn(u32, *const f32, *const f32, *const f32, u8) = std::mem::transmute(base + RVA_OBJECT_TELEPORT);
        let (pos, fwd, up) = ([s[0], s[1], s[2]], [s[3].cos(), s[3].sin(), 0.0f32], [0.0f32, 0.0, 1.0]);
        f(unit, pos.as_ptr(), fwd.as_ptr(), up.as_ptr(), 1);
        slot.write(saved.unwrap_or(0));
        // The player's look yaw drives the biped's facing: turn it too.
        if let Some(block) = crate::input::engine_tls_block().filter(|_| turn.is_some()) {
            let pc = *(block.add(0xd8) as *const *mut u8);
            if !pc.is_null() {
                (pc.add(0x94) as *mut f32).write_unaligned(s[3]);
                if let Some(p) = turn.filter(|p| !p.is_nan()) {
                    (pc.add(0x98) as *mut f32).write_unaligned(p);
                }
            }
        }
        log!("preview: biped {unit:#x} moved to {pos:?} facing {:.2}", s[3]);
    }
    let ret: unsafe extern "system" fn(u32, usize) = std::mem::transmute(base + RVA_HS_RETURN);
    ret(thread, 0);
}

/// Set when the child kills its biped for a species change; cleared when the new one appears.
static KILLED_AT: std::sync::Mutex<Option<std::time::Instant>> = std::sync::Mutex::new(None);

/// Player 0's datum: players table (TLS +0x18 -> +0x50), 0x490 bytes per player, unit handle at +0x28.
unsafe fn player0_datum() -> Option<*mut u8> {
    let block = crate::input::engine_tls_block()?;
    let players = *(block.add(0x18) as *const *const u8);
    if players.is_null() {
        return None;
    }
    let data = *(players.add(0x50) as *const *mut u8);
    (!data.is_null()).then_some(data)
}

/// Game thread, while the killed biped waits for its respawn: the player's respawn countdown (+0x1BC, game ticks;
/// +0x1C8 is the whole seconds the spawn screen shows) is set from the game variant's respawn time at death. Cut
/// it to one tick, so the new biped (with the new species) appears on the next tick instead of ~5 s later.
unsafe fn skip_respawn_wait() {
    let Some(p) = player0_datum() else { return };
    let timer = p.add(0x1bc) as *mut u32;
    if timer.read_unaligned() > 1 {
        timer.write_unaligned(1);
        (p.add(0x1c8) as *mut u32).write_unaligned(0);
    }
}

/// Debug (REACH_PREVIEW_DUMP): log the u32 words of player 0's datum (players table TLS +0x18 -> +0x50, 0x490 bytes
/// each, unit at +0x28) that changed since the last call, every 0.25 s.
unsafe fn player_dump() {
    static LAST: std::sync::Mutex<Option<(std::time::Instant, Vec<u32>)>> = std::sync::Mutex::new(None);
    let mut g = LAST.lock().unwrap_or_else(|e| e.into_inner());
    if g.as_ref().is_some_and(|l| l.0.elapsed().as_millis() < 250) {
        return;
    }
    let Some(data) = player0_datum() else { return };
    let data = data as *const u32;
    let now: Vec<u32> = (0..0x490 / 4).map(|i| data.add(i).read_unaligned()).collect();
    let mut line = String::new();
    let mut n = 0;
    if let Some((_, old)) = g.as_ref() {
        for (i, (a, b)) in old.iter().zip(&now).enumerate() {
            if a != b {
                n += 1;
                if n <= 48 {
                    line += &format!(" +{:#x}:{a:#x}->{b:#x}", i * 4);
                }
            }
        }
    }
    if n > 0 || g.is_none() {
        log!("preview: player0 dump ({n} changed):{line}");
    }
    *g = Some((std::time::Instant::now(), now));
}

/// Facing of the biped when the orbit was installed (radians); yaw 0 = looking at its face.
static FACING: AtomicU32 = AtomicU32::new(0);

/// Debug (game thread): log director 0's camera vtable (TLS +0x188 -> directors of 0x1AC bytes, camera at +8).
pub unsafe fn director_dump() {
    let base = crate::crash::ENGINE_BASE.load(Ordering::Relaxed);
    let idx = ((base + 0xc17b18) as *const u32).read() as usize;
    let tls: *const *const u8;
    core::arch::asm!("mov {}, gs:[0x58]", out(reg) tls);
    let block = *tls.add(idx);
    if block.is_null() {
        return;
    }
    let dirs = *(block.add(0x188) as *const *const u8);
    if dirs.is_null() {
        log!("preview: no directors");
        return;
    }
    let mut line = String::new();
    for i in 0..24 {
        line += &format!(" {:08x}", (dirs.add(i * 4) as *const u32).read_unaligned());
    }
    line += &format!(" | +0x190.. {:08x} {:08x} {:.3} {:.3}", *(dirs.add(0x190) as *const u32), *(dirs.add(0x194) as *const u32), *(dirs.add(0x198) as *const f32), *(dirs.add(0x19c) as *const f32));
    line += &format!(" | cam +0x20.. {:.3} {:.3} {:.3}", *(dirs.add(0x28) as *const f32), *(dirs.add(0x2c) as *const f32), *(dirs.add(0x30) as *const f32));
    let vt = (dirs.add(8) as *const usize).read_unaligned();
    log!("preview: director 0 camera vtable haloreach+{:#x}:{line}", vt.wrapping_sub(base));
}

// ---------------------------------------------------------------------------------------------------------------
// Child process: `reach-host --mcc <dir> --preview <mapping name> --size WxH`, started by the menu (Client below).
// It plays a solo Slayer on PREVIEW_MAP with no HUD, no audio and the orbit camera on its own biped, reads commands
// from stdin and publishes its frames into a shared memory block the menu draws.
// ---------------------------------------------------------------------------------------------------------------

/// Map and game variant of the preview scene (overridable with REACH_PREVIEW_MAP for experiments).
/// Spire: outdoor, its Covenant spire and cliffs behind the biped (the indoor Sword Base boxed the camera in).
const PREVIEW_MAP: &str = "35_island";
const PREVIEW_VARIANT: &str = "slayer_054.bin";
/// Field of view of the preview camera (degrees; narrower than play so the model fills a portrait frame).
pub const PREVIEW_FOV: u32 = 40;
/// Field of view the menu asked for (`view` command's fourth number): the menu background uses a wider one.
static VIEW_FOV: AtomicU32 = AtomicU32::new(PREVIEW_FOV);

/// Shared block: 64-byte header {magic, width, height, seq, state, pid} then two frame buffers of MAX_PIXELS RGBA.
const MAGIC: u32 = 0x4433_5052; // "RP3D"
const HEADER: usize = 64;
const MAX_W: u32 = 1920;
const MAX_H: u32 = 1080;
const BUF_BYTES: usize = (MAX_W * MAX_H * 4) as usize;
const MAP_BYTES: usize = HEADER + 2 * BUF_BYTES;
/// Header state: 0 starting, 1 live (orbit camera on the biped), 2 switching species (respawning, no frames).
const STATE_LIVE: u32 = 1;
const STATE_SWITCHING: u32 = 2;

static SHARED: AtomicUsize = AtomicUsize::new(0);
static LIVE_AT: std::sync::Mutex<Option<std::time::Instant>> = std::sync::Mutex::new(None);
static PENDING_ARMOUR: std::sync::Mutex<Option<crate::ui::settings::Armour>> = std::sync::Mutex::new(None);
static PENDING_HS: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

pub fn is_child() -> bool {
    CHILD.load(Ordering::Relaxed)
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

/// `--preview <name>`: turn this process into the preview child (rewrites the arguments into a hidden solo Slayer).
pub fn apply_args(argv: &mut Vec<String>) -> Result<(), String> {
    let Some(i) = argv.iter().position(|a| a == "--preview") else { return Ok(()) };
    let name = argv.get(i + 1).cloned().ok_or("--preview needs a mapping name")?;
    argv.drain(i..i + 2);
    CHILD.store(true, Ordering::Relaxed);
    let mcc = argv
        .iter()
        .position(|a| a == "--mcc")
        .and_then(|j| argv.get(j + 1).cloned())
        .or_else(|| std::env::var("MCC_PATH").ok())
        .ok_or("--preview needs --mcc")?;
    let map = std::env::var("REACH_PREVIEW_MAP").unwrap_or_else(|_| PREVIEW_MAP.into());
    let gv = std::path::Path::new(&mcc).join("haloreach").join("game_variants").join(PREVIEW_VARIANT);
    argv.extend(["--map".into(), map, "--game-variant".into(), gv.to_string_lossy().into_owned(), "--hidden".into()]);
    if name != "-" {
        unsafe {
            use windows::Win32::System::Memory::*;
            let h = OpenFileMappingW(FILE_MAP_ALL_ACCESS.0, false, windows::core::PCWSTR(wide(&name).as_ptr()))
                .map_err(|e| format!("preview: open mapping {name}: {e}"))?;
            let v = MapViewOfFile(h, FILE_MAP_ALL_ACCESS, 0, 0, MAP_BYTES);
            if v.Value.is_null() {
                return Err(format!("preview: map view of {name} failed"));
            }
            SHARED.store(v.Value as usize, Ordering::Relaxed);
        }
    }
    // Test hook: REACH_PREVIEW_VIEW="yaw pitch dist" (umu-run does not pass stdin through).
    if let Some(v) = std::env::var("REACH_PREVIEW_VIEW").ok().map(|v| v.split_whitespace().filter_map(|x| x.parse::<f32>().ok()).collect::<Vec<_>>()) {
        if v.len() == 3 {
            set_view(v[0], v[1], v[2]);
        }
    }
    log!("preview: child process, scene {} ({}), mapping {name}", argv[argv.len() - 4], PREVIEW_VARIANT);
    start_stdin_reader();
    Ok(())
}

/// Commands from the menu, one per line: `view <yaw> <pitch> <dist>`, `armour <json>`.
/// End of input = the menu is gone: exit.
fn start_stdin_reader() {
    std::thread::spawn(|| {
        use std::io::BufRead;
        let stdin = std::io::stdin();
        for line in stdin.lock().lines() {
            let Ok(line) = line else { break };
            let (cmd, rest) = line.split_once(' ').unwrap_or((line.as_str(), ""));
            match cmd {
                "view" => {
                    let v: Vec<f32> = rest.split_whitespace().filter_map(|x| x.parse().ok()).collect();
                    if v.len() >= 3 {
                        set_view(v[0], v[1], v[2]);
                        if let Some(f) = v.get(3) {
                            VIEW_FOV.store(*f as u32, Ordering::Relaxed);
                        }
                        static FIRST: AtomicBool = AtomicBool::new(true);
                        if FIRST.swap(false, Ordering::Relaxed) {
                            log!("preview: first view command {rest}");
                        }
                    }
                }
                "armour" => match serde_json::from_str::<crate::ui::settings::Armour>(rest) {
                    Ok(a) => *PENDING_ARMOUR.lock().unwrap_or_else(|e| e.into_inner()) = Some(a),
                    Err(e) => log!("preview: bad armour line: {e}"),
                },
                "quit" => break,
                _ => log!("preview: unknown command {line}"),
            }
        }
        log!("preview: menu closed the pipe, exiting");
        crate::ui::hard_exit(0);
    });
}

/// Child, game thread: apply queued armour (profile + engine msg 5) and commands. Also while dead once the first
/// biped was seen, so a respawn uses the latest look; game_tick respawns when the species differs.
unsafe fn apply_pending(spawned: bool) {
    if !spawned && ORBIT_UNIT.load(Ordering::Relaxed) == u32::MAX {
        return;
    }
    if let Some(a) = PENDING_ARMOUR.lock().unwrap_or_else(|e| e.into_inner()).take() {
        crate::profile::write_armour(crate::host::profile_ptr(), &a);
        crate::ui::settings_apply::post_engine(5);
    }
    if !spawned {
        return;
    }
    for e in PENDING_HS.lock().unwrap_or_else(|e| e.into_inner()).drain(..) {
        hs(&e);
    }
}

/// Child: queue a look to show (also used by the `set armour` test-script step).
pub fn queue_armour(a: &crate::ui::settings::Armour) {
    *PENDING_ARMOUR.lock().unwrap_or_else(|e| e.into_inner()) = Some(a.clone());
}

/// Child: hold the engine's render thread to ~30 fps while the menu shows the preview, ~4 fps while it does not
/// (the menu bumps header word 6 on every frame it draws the pane). Saves most of the GPU/CPU the hidden
/// engine would otherwise spend at its 60 fps minimum cap.
unsafe fn pace_render_thread(h: *mut u32) {
    static LAST: std::sync::Mutex<Option<(u32, std::time::Instant, std::time::Instant)>> = std::sync::Mutex::new(None);
    let mut g = LAST.lock().unwrap_or_else(|e| e.into_inner());
    let now = std::time::Instant::now();
    let tick = h.add(6).read_volatile();
    let (seen, seen_at, frame_at) = g.get_or_insert((tick, now, now));
    if *seen != tick {
        *seen = tick;
        *seen_at = now;
    }
    let period = if now.duration_since(*seen_at).as_millis() < 1000 { 33 } else { 250 };
    let due = *frame_at + std::time::Duration::from_millis(period);
    if due > now {
        std::thread::sleep(due - now);
    }
    *frame_at = std::time::Instant::now();
}

fn header() -> Option<*mut u32> {
    let p = SHARED.load(Ordering::Relaxed);
    (p != 0).then_some(p as *mut u32)
}

/// Child, engine render thread (host slot 1, before Present): copy the back buffer into the shared block, about
/// 30 times a second, once the orbit camera has settled. Two staging textures so the map never waits on the GPU.
pub unsafe fn end_frame(sc: *mut std::ffi::c_void) {
    use windows::core::Interface;
    use windows::Win32::Graphics::Direct3D11::*;
    use windows::Win32::Graphics::Dxgi::IDXGISwapChain;
    if !is_child() {
        return;
    }
    let Some(h) = header() else { return };
    pace_render_thread(h);
    let live = LIVE_AT.lock().unwrap_or_else(|e| e.into_inner()).is_some_and(|t| t.elapsed().as_millis() > 400);
    if !live {
        return;
    }
    struct Copier {
        staging: [ID3D11Texture2D; 2],
        size: (u32, u32),
        n: u64,
        last: std::time::Instant,
    }
    static COPIER: std::sync::Mutex<Option<Copier>> = std::sync::Mutex::new(None);
    let mut g = COPIER.lock().unwrap_or_else(|e| e.into_inner());
    if g.as_ref().is_some_and(|c| c.last.elapsed().as_millis() < 30) {
        return;
    }
    let Some(sc) = IDXGISwapChain::from_raw_borrowed(&sc) else { return };
    let Ok(tex) = sc.GetBuffer::<ID3D11Texture2D>(0) else { return };
    let mut desc = D3D11_TEXTURE2D_DESC::default();
    tex.GetDesc(&mut desc);
    let (w, hgt) = (desc.Width.min(MAX_W), desc.Height.min(MAX_H));
    let Ok(dev) = tex.GetDevice() else { return };
    let Ok(ctx) = dev.GetImmediateContext() else { return };
    if g.as_ref().is_none_or(|c| c.size != (desc.Width, desc.Height)) {
        let sdesc = D3D11_TEXTURE2D_DESC { Usage: D3D11_USAGE_STAGING, BindFlags: 0, CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32, MiscFlags: 0, ..desc };
        let mk = || {
            let mut t = None;
            dev.CreateTexture2D(&sdesc, None, Some(&mut t)).ok().and(t)
        };
        let (Some(a), Some(b)) = (mk(), mk()) else { return };
        *g = Some(Copier { staging: [a, b], size: (desc.Width, desc.Height), n: 0, last: std::time::Instant::now() });
    }
    let c = g.as_mut().unwrap();
    c.last = std::time::Instant::now();
    ctx.CopyResource(&c.staging[(c.n % 2) as usize], &tex);
    if c.n > 0 {
        let src = &c.staging[((c.n - 1) % 2) as usize];
        let mut m = D3D11_MAPPED_SUBRESOURCE::default();
        if ctx.Map(src, 0, D3D11_MAP_READ, 0, Some(&mut m)).is_ok() {
            let seq = (h.add(3)).read_volatile();
            let slot = ((seq + 1) % 2) as usize;
            let dst = (h as *mut u8).add(HEADER + slot * BUF_BYTES);
            let bgra = desc.Format.0 == 87 || desc.Format.0 == 91;
            for y in 0..hgt as usize {
                let s = (m.pData as *const u8).add(y * m.RowPitch as usize);
                let d = dst.add(y * w as usize * 4);
                std::ptr::copy_nonoverlapping(s, d, w as usize * 4);
                if bgra {
                    for x in 0..w as usize {
                        std::ptr::swap(d.add(x * 4), d.add(x * 4 + 2));
                    }
                }
                for x in 0..w as usize {
                    *d.add(x * 4 + 3) = 255;
                }
            }
            ctx.Unmap(src, 0);
            h.write_volatile(MAGIC);
            h.add(1).write_volatile(w);
            h.add(2).write_volatile(hgt);
            h.add(4).write_volatile(STATE_LIVE);
            h.add(5).write_volatile(std::process::id());
            std::sync::atomic::fence(Ordering::Release);
            h.add(3).write_volatile(seq + 1);
        }
    }
    c.n += 1;
}

// ---------------------------------------------------------------------------------------------------------------
// Menu side: start / feed / read the child.
// ---------------------------------------------------------------------------------------------------------------

/// MCC folder for the child's `--mcc` (set by main before the menu).
pub static MCC: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
/// Child render size: 16:9 for the full-screen menu background (menu_scene.rs, upscaled to the window); the
/// CUSTOMIZE pane crops its middle. 1280x720 keeps the copy into shared memory cheap (~3.7 MB a frame).
pub const CHILD_SIZE: (u32, u32) = (1280, 720);

struct Client {
    child: std::process::Child,
    stdin: std::process::ChildStdin,
    view: usize,
    _mapping: windows::Win32::Foundation::HANDLE,
    seq: u32,
    tex: Option<egui::TextureHandle>,
    started: std::time::Instant,
    last_view: String,
    last_armour: String,
    /// When the last look was sent / the first frame arrived (settled() for unattended captures).
    armour_at: std::time::Instant,
    view_at: std::time::Instant,
    first_frame: Option<std::time::Instant>,
    failed: Option<String>,
}
unsafe impl Send for Client {}

static CLIENT: std::sync::Mutex<Option<Client>> = std::sync::Mutex::new(None);

pub enum Frame {
    /// Engine still loading: seconds since start.
    Loading(f32),
    Live(egui::TextureId, egui::Vec2),
    Failed(String),
}

/// Start the preview child if it is not running (menu thread). Cheap when already started. The menu calls it at
/// launch, so the model is loaded (~20 s) by the time the player opens CUSTOMIZE; until then the child renders at
/// ~4 fps into the shared block and nobody looks.
pub fn start() {
    let mut g = CLIENT.lock().unwrap_or_else(|e| e.into_inner());
    if g.is_some() {
        return;
    }
    match spawn() {
        Ok(c) => *g = Some(c),
        Err(e) => {
            log!("preview: could not start the preview engine: {e}");
        }
    }
}

fn spawn() -> Result<Client, String> {
    use windows::Win32::System::Memory::*;
    let mcc = MCC.get().ok_or("no MCC folder")?;
    let name = format!("Local\\ReachPreview{}", std::process::id());
    let (mapping, view) = unsafe {
        let h = CreateFileMappingW(
            windows::Win32::Foundation::INVALID_HANDLE_VALUE,
            None,
            PAGE_READWRITE,
            0,
            MAP_BYTES as u32,
            windows::core::PCWSTR(wide(&name).as_ptr()),
        )
        .map_err(|e| format!("CreateFileMapping: {e}"))?;
        let v = MapViewOfFile(h, FILE_MAP_ALL_ACCESS, 0, 0, MAP_BYTES);
        if v.Value.is_null() {
            return Err("MapViewOfFile failed".into());
        }
        (h, v.Value as usize)
    };
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut child = std::process::Command::new(exe)
        .args(["--mcc", &mcc.to_string_lossy(), "--preview", &name, "--size", &format!("{}x{}", CHILD_SIZE.0, CHILD_SIZE.1)])
        .env_remove("REACH_UI_SHOT")
        .env_remove("REACH_CAPTURE_AT")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("spawn: {e}"))?;
    let stdin = child.stdin.take().ok_or("no stdin pipe")?;
    log!("preview: started preview engine (pid {}, mapping {name})", child.id());
    Ok(Client { child, stdin, view, _mapping: mapping, seq: 0, tex: None, started: std::time::Instant::now(), last_view: String::new(), last_armour: String::new(), armour_at: std::time::Instant::now(), view_at: std::time::Instant::now(), first_frame: None, failed: None })
}

/// Stop the child (menu leaves for a game, or quits). Its stdin closes and it exits; kill it to be sure.
pub fn stop() {
    if let Some(mut c) = CLIENT.lock().unwrap_or_else(|e| e.into_inner()).take() {
        let _ = c.child.kill();
        log!("preview: preview engine stopped");
    }
}

fn send(c: &mut Client, line: &str) {
    use std::io::Write;
    if writeln!(c.stdin, "{line}").and_then(|_| c.stdin.flush()).is_err() && c.failed.is_none() {
        c.failed = Some("The preview engine stopped.".into());
    }
}

/// Camera: yaw (radians, 0 = facing the camera), pitch (radians, up positive), distance (world units).
pub fn set_camera(yaw: f32, pitch: f32, dist: f32) {
    set_shot(yaw, pitch, dist, PREVIEW_FOV);
}

/// Camera with a field of view (degrees; a change costs the child an engine message 5).
pub fn set_shot(yaw: f32, pitch: f32, dist: f32, fov: u32) {
    if let Some(c) = CLIENT.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        let line = format!("view {yaw:.3} {pitch:.3} {dist:.3} {fov}");
        if line != c.last_view {
            send(c, &line);
            c.last_view = line;
            c.view_at = std::time::Instant::now();
        }
    }
}

/// Look to show (hover = try on): sent when it changes.
pub fn set_armour(a: &crate::ui::settings::Armour) {
    if let Some(c) = CLIENT.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        let json = serde_json::to_string(a).unwrap_or_default();
        if json != c.last_armour {
            send(c, &format!("armour {json}"));
            c.last_armour = json;
            c.armour_at = std::time::Instant::now();
        }
    }
}


/// Menu thread, every menu frame: notice a child that ended and restart it, also while CUSTOMIZE is not open.
pub fn menu_tick() {
    if let Some(c) = CLIENT.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        poll(c);
    }
}

fn poll(c: &mut Client) {
    if c.failed.is_none() {
        if let Ok(Some(st)) = c.child.try_wait() {
            c.failed = Some(format!("The preview engine stopped ({st})."));
        }
    }
    // A preview that ran fine and then ended (the scene's game hit its time limit) is restarted once a minute at
    // most, keeping the last frame on screen meanwhile.
    if c.failed.is_some() && c.first_frame.is_some() && c.started.elapsed().as_secs() > 60 {
        log!("preview: preview engine ended ({}), restarting", c.failed.as_deref().unwrap_or(""));
        match spawn() {
            Ok(mut n) => {
                n.tex = c.tex.take();
                n.first_frame = c.first_frame;
                n.last_view.clear();
                n.last_armour.clear();
                let _ = c.child.kill();
                *c = n;
            }
            Err(e) => log!("preview: restart failed: {e}"),
        }
    }
}

/// The latest frame as an egui texture (menu thread), or why there is none.
pub fn frame(ctx: &egui::Context) -> Frame {
    let mut g = CLIENT.lock().unwrap_or_else(|e| e.into_inner());
    let Some(c) = g.as_mut() else { return Frame::Failed("The preview engine could not start.".into()) };
    poll(c);
    if let Some(f) = &c.failed {
        return Frame::Failed(f.clone());
    }
    unsafe {
        let h = c.view as *mut u32;
        h.add(6).write_volatile(h.add(6).read_volatile().wrapping_add(1)); // "pane visible" for the child's pacing
        let seq = h.add(3).read_volatile();
        if h.read_volatile() == MAGIC && seq != c.seq {
            std::sync::atomic::fence(Ordering::Acquire);
            let (w, hgt) = (h.add(1).read_volatile().min(MAX_W) as usize, h.add(2).read_volatile().min(MAX_H) as usize);
            let src = std::slice::from_raw_parts((c.view + HEADER + (seq % 2) as usize * BUF_BYTES) as *const u8, w * hgt * 4);
            let img = egui::ColorImage::from_rgba_unmultiplied([w, hgt], src);
            match &mut c.tex {
                Some(t) => t.set(img, egui::TextureOptions::LINEAR),
                None => c.tex = Some(ctx.load_texture("preview3d", img, egui::TextureOptions::LINEAR)),
            }
            c.seq = seq;
            c.first_frame.get_or_insert_with(std::time::Instant::now);
        }
    }
    match &c.tex {
        Some(t) => Frame::Live(t.id(), t.size_vec2()),
        None => Frame::Loading(c.started.elapsed().as_secs_f32()),
    }
}

/// The child is respawning the biped for a Spartan / Elite change (its frames pause; the last one stays up).
pub fn switching() -> bool {
    let g = CLIENT.lock().unwrap_or_else(|e| e.into_inner());
    g.as_ref().is_some_and(|c| c.failed.is_none() && unsafe { (c.view as *const u32).add(4).read_volatile() } == STATE_SWITCHING)
}

/// Seconds since the first frame arrived (0 before), for unattended captures.
pub fn live_secs() -> f32 {
    let g = CLIENT.lock().unwrap_or_else(|e| e.into_inner());
    g.as_ref().and_then(|c| c.first_frame).map_or(0.0, |t| t.elapsed().as_secs_f32())
}

/// Unattended captures: frames are arriving and the last look change had time to show (or the preview failed).
pub fn settled() -> bool {
    let g = CLIENT.lock().unwrap_or_else(|e| e.into_inner());
    g.as_ref().is_none_or(|c| {
        // REACH_PREVIEW_SETTLE_SECS: keep the screen up longer (CPU / memory measurements).
        let settle = std::env::var("REACH_PREVIEW_SETTLE_SECS").ok().and_then(|v| v.parse::<f32>().ok()).unwrap_or(2.0);
        c.failed.is_some() || (c.first_frame.is_some_and(|t| t.elapsed().as_secs_f32() > settle) && c.armour_at.elapsed().as_secs_f32() > 1.5 && c.view_at.elapsed().as_secs_f32() > 0.6
            && unsafe { (c.view as *const u32).add(4).read_volatile() } != STATE_SWITCHING)
    })
}
