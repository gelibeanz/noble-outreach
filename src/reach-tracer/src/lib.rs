//! Proxy `haloreach.dll` for MCC. MCC loads this in place of the real engine DLL (renamed to
//! `haloreach_real.dll` next to it). Everything is forwarded unchanged; on the way it records:
//!
//! - `SetLibrarySettings` languages,
//! - every engine call MCC makes that we hook (init graphics, initialize game, messages, commands),
//! - the full launch options block passed to `initialize_game` (one `.bin` per launch),
//! - which host-interface slots the engine calls, when each is first called and how often.
//!
//! Output goes to `reach-trace/` next to this DLL. See notes/engine-interface.md.

use std::ffi::{c_void, CStr};
use std::fs::{self, File};
use std::io::Write;
use std::mem::{size_of, transmute};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::HMODULE;
use windows_sys::Win32::System::LibraryLoader::{
    GetModuleFileNameW, GetModuleHandleExW, GetProcAddress, LoadLibraryExW,
    GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
    LOAD_WITH_ALTERED_SEARCH_PATH,
};
use windows_sys::Win32::System::Memory::{VirtualQuery, MEMORY_BASIC_INFORMATION, MEM_COMMIT};

core::arch::global_asm!(include_str!(concat!(env!("OUT_DIR"), "/thunks.s")));

/// Host-interface slots covered by thunks (build.rs generates the same number).
const MGR_SLOTS: usize = 192;
/// Engine-interface slots copied (11 known; the rest is margin).
const ENGINE_SLOTS: usize = 16;
/// Size of the launch options block per libmcc (`s_game_options`).
const GAME_OPTIONS_SIZE: usize = 0x2BF30;

#[no_mangle]
static mut MGR_ORIG: [usize; MGR_SLOTS] = [0; MGR_SLOTS];
#[no_mangle]
static MGR_CALLS: [AtomicU64; MGR_SLOTS] = [const { AtomicU64::new(0) }; MGR_SLOTS];
extern "C" {
    static MGR_THUNKS: [usize; MGR_SLOTS];
}

static mut ENGINE_ORIG: [usize; ENGINE_SLOTS] = [0; ENGINE_SLOTS];
static mut ENGINE_VT: [usize; ENGINE_SLOTS] = [0; ENGINE_SLOTS];
static ENGINE_VT_READY: AtomicBool = AtomicBool::new(false);
static MGR_VT_ORIG: AtomicUsize = AtomicUsize::new(0);
static LAUNCHES: AtomicUsize = AtomicUsize::new(0);

struct Real {
    create_data_access: usize,
    create_game_engine: usize,
    set_library_settings: usize,
    base: usize,
}

static START: OnceLock<Instant> = OnceLock::new();
static OUT_DIR: OnceLock<PathBuf> = OnceLock::new();
static LOG: Mutex<Option<File>> = Mutex::new(None);
static REAL: OnceLock<Option<Real>> = OnceLock::new();

/// Per-process tag for output files (MCC can load the DLL more than once, e.g. again while quitting).
fn run_tag() -> &'static str {
    static TAG: OnceLock<String> = OnceLock::new();
    TAG.get_or_init(|| {
        let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        format!("{secs}-{}", std::process::id())
    })
}

fn log(msg: &str) {
    let t = START.get_or_init(Instant::now).elapsed().as_secs_f64();
    if let Ok(mut g) = LOG.lock() {
        if g.is_none() {
            if let Some(dir) = OUT_DIR.get() {
                *g = File::create(dir.join(format!("trace-{}.log", run_tag()))).ok();
            }
        }
        if let Some(f) = g.as_mut() {
            let _ = writeln!(f, "[{t:9.3}] {msg}");
            let _ = f.flush();
        }
    }
}

/// Directory holding this DLL (and the real one).
unsafe fn own_dir() -> PathBuf {
    let mut h: HMODULE = std::ptr::null_mut();
    GetModuleHandleExW(
        GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
        own_dir as *const u16,
        &mut h,
    );
    let mut buf = [0u16; 1024];
    let n = GetModuleFileNameW(h, buf.as_mut_ptr(), buf.len() as u32) as usize;
    let p = PathBuf::from(String::from_utf16_lossy(&buf[..n]));
    p.parent().map(|d| d.to_path_buf()).unwrap_or_default()
}

fn wide(s: &std::path::Path) -> Vec<u16> {
    s.to_string_lossy().encode_utf16().chain(Some(0)).collect()
}

unsafe fn real() -> Option<&'static Real> {
    REAL.get_or_init(|| {
        START.get_or_init(Instant::now);
        let dir = own_dir();
        let out = dir.join("reach-trace");
        let _ = fs::create_dir_all(&out);
        let _ = OUT_DIR.set(out);
        let path = dir.join("haloreach_real.dll");
        let h = LoadLibraryExW(wide(&path).as_ptr(), std::ptr::null_mut(), LOAD_WITH_ALTERED_SEARCH_PATH);
        if h.is_null() {
            log(&format!("FATAL: cannot load {}", path.display()));
            return None;
        }
        let get = |name: &[u8]| GetProcAddress(h, name.as_ptr()).map(|f| f as usize).unwrap_or(0);
        let r = Real {
            create_data_access: get(b"CreateDataAccess\0"),
            create_game_engine: get(b"CreateGameEngine\0"),
            set_library_settings: get(b"SetLibrarySettings\0"),
            base: h as usize,
        };
        log(&format!(
            "loaded {} at {:#x}; exports CDA=+{:#x} CGE=+{:#x} SLS=+{:#x}",
            path.display(),
            r.base,
            r.create_data_access.wrapping_sub(r.base),
            r.create_game_engine.wrapping_sub(r.base),
            r.set_library_settings.wrapping_sub(r.base)
        ));
        Some(r)
    })
    .as_ref()
}

/// Address as an RVA into the real DLL when it lies inside it, else absolute.
fn where_(addr: usize) -> String {
    match REAL.get().and_then(|r| r.as_ref()) {
        Some(r) if addr >= r.base && addr < r.base + 0x0600_0000 => format!("haloreach+{:#x}", addr - r.base),
        _ => format!("{addr:#x}"),
    }
}

/// Number of pointer-sized entries readable from `p` (up to `max`) without leaving committed memory.
unsafe fn readable_words(p: *const usize, max: usize) -> usize {
    let mut mbi: MEMORY_BASIC_INFORMATION = std::mem::zeroed();
    if VirtualQuery(p as *const c_void, &mut mbi, size_of::<MEMORY_BASIC_INFORMATION>()) == 0
        || mbi.State != MEM_COMMIT
    {
        return 0;
    }
    let end = mbi.BaseAddress as usize + mbi.RegionSize;
    ((end - p as usize) / 8).min(max)
}

// ---------------------------------------------------------------- exports

#[no_mangle]
pub unsafe extern "system" fn CreateDataAccess(out: *mut *mut c_void) -> u64 {
    let Some(r) = real() else { return 1 };
    let f: unsafe extern "system" fn(*mut *mut c_void) -> u64 = transmute(r.create_data_access);
    let rc = f(out);
    log(&format!("CreateDataAccess -> rc={rc:#x} obj={:?}", if out.is_null() { std::ptr::null_mut() } else { *out }));
    rc
}

#[no_mangle]
pub unsafe extern "system" fn SetLibrarySettings(settings: *const u16) -> u64 {
    let Some(r) = real() else { return 1 };
    if !settings.is_null() {
        let field = |i: usize| {
            let s = std::slice::from_raw_parts(settings.add(i * 85), 85);
            let n = s.iter().position(|&c| c == 0).unwrap_or(85);
            String::from_utf16_lossy(&s[..n])
        };
        log(&format!("SetLibrarySettings audio={:?} text={:?} text2={:?}", field(0), field(1), field(2)));
    }
    let f: unsafe extern "system" fn(*const u16) -> u64 = transmute(r.set_library_settings);
    let rc = f(settings);
    log(&format!("SetLibrarySettings -> {rc:#x}"));
    rc
}

#[no_mangle]
pub unsafe extern "system" fn CreateGameEngine(out: *mut *mut c_void) -> u64 {
    let Some(r) = real() else { return 1 };
    let f: unsafe extern "system" fn(*mut *mut c_void) -> u64 = transmute(r.create_game_engine);
    let rc = f(out);
    let obj = if out.is_null() { std::ptr::null_mut() } else { *out };
    log(&format!("CreateGameEngine -> rc={rc:#x} engine={obj:?}"));
    if !obj.is_null() {
        wrap_engine(obj);
    }
    rc
}

// ---------------------------------------------------------------- engine interface

unsafe fn wrap_engine(obj: *mut c_void) {
    let vptr = obj as *mut *const usize;
    let vt = *vptr;
    let ours = std::ptr::addr_of!(ENGINE_VT) as *const usize;
    if vt == ours {
        return;
    }
    if !ENGINE_VT_READY.swap(true, Ordering::SeqCst) {
        let n = readable_words(vt, ENGINE_SLOTS);
        let slots: Vec<String> = (0..n).map(|i| where_(*vt.add(i))).collect();
        log(&format!("engine vtable {} ({n} slots read): {}", where_(vt as usize), slots.join(" ")));
        for i in 0..n {
            ENGINE_ORIG[i] = *vt.add(i);
            ENGINE_VT[i] = *vt.add(i);
        }
        ENGINE_VT[1] = hook_initialize_graphics as *const () as usize;
        ENGINE_VT[2] = hook_initialize_game as *const () as usize;
        ENGINE_VT[3] = hook_post_message as *const () as usize;
        ENGINE_VT[5] = hook_preload_level_begin as *const () as usize;
        ENGINE_VT[9] = hook_post_command as *const () as usize;
    }
    *vptr = ours;
    log(&format!("engine {obj:?} wrapped"));
}

unsafe extern "system" fn hook_initialize_graphics(
    this: *mut c_void,
    device: *mut c_void,
    context: *mut c_void,
    swap1: *mut c_void,
    swap2: *mut c_void,
) {
    log(&format!("engine.initialize_graphics device={device:?} context={context:?} swapchain={swap1:?} swapchain2={swap2:?}"));
    let f: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut c_void, *mut c_void, *mut c_void) =
        transmute(ENGINE_ORIG[1]);
    f(this, device, context, swap1, swap2)
}

unsafe extern "system" fn hook_initialize_game(this: *mut c_void, mgr: *mut c_void, options: *const u8) -> *mut c_void {
    let k = LAUNCHES.fetch_add(1, Ordering::SeqCst);
    log(&format!("engine.initialize_game #{k} host={mgr:?} options={options:?}"));
    if !options.is_null() {
        let n = readable_words(options as *const usize, GAME_OPTIONS_SIZE / 8) * 8;
        let bytes = std::slice::from_raw_parts(options, n);
        if let Some(dir) = OUT_DIR.get() {
            let path = dir.join(format!("game-options-{}-{k}.bin", run_tag()));
            let _ = fs::write(&path, bytes);
            log(&format!("  wrote {n:#x} bytes to {}", path.display()));
        }
        // A few fields we already expect (libmcc layout): flags, game mode, map ids.
        let rd = |o: usize| u32::from_le_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]]);
        if n >= 0x20 {
            log(&format!(
                "  flags={:#06x} game_mode={} legacy_map_id={} map_id_word0={:#x}",
                u16::from_le_bytes([bytes[0], bytes[1]]),
                rd(0x0C),
                rd(0x10),
                rd(0x14)
            ));
        }
    }
    if !mgr.is_null() {
        wrap_host(mgr);
    }
    let f: unsafe extern "system" fn(*mut c_void, *mut c_void, *const u8) -> *mut c_void = transmute(ENGINE_ORIG[2]);
    let thread = f(this, mgr, options);
    log(&format!("engine.initialize_game #{k} -> game thread handle {thread:?}"));
    thread
}

unsafe extern "system" fn hook_post_message(this: *mut c_void, msg: u32, param: *const u8) -> *mut c_void {
    // Bytes the engine copies per message id (EnginePostMessage).
    let len = match msg {
        0x10 | 0x11 | 0x13 | 0x18 => 8,
        0x12 => 16,
        0x19 => 48,
        _ => 0,
    };
    let p = if param.is_null() || len == 0 {
        "-".to_string()
    } else {
        std::slice::from_raw_parts(param, len).iter().map(|b| format!("{b:02x}")).collect()
    };
    log(&format!("engine.post_message msg={msg} param={p}"));
    let f: unsafe extern "system" fn(*mut c_void, u32, *const u8) -> *mut c_void = transmute(ENGINE_ORIG[3]);
    f(this, msg, param)
}

unsafe extern "system" fn hook_preload_level_begin(this: *mut c_void, map_id: i32) {
    log(&format!("engine.preload_level_begin map_id={map_id}"));
    let f: unsafe extern "system" fn(*mut c_void, i32) = transmute(ENGINE_ORIG[5]);
    f(this, map_id)
}

unsafe extern "system" fn hook_post_command(this: *mut c_void, cmd: *const i8) -> *mut c_void {
    let s = if cmd.is_null() { "<null>".into() } else { CStr::from_ptr(cmd).to_string_lossy().into_owned() };
    log(&format!("engine.post_command {s:?}"));
    let f: unsafe extern "system" fn(*mut c_void, *const i8) -> *mut c_void = transmute(ENGINE_ORIG[9]);
    f(this, cmd)
}

// ---------------------------------------------------------------- host interface (MCC's game manager)

unsafe fn wrap_host(mgr: *mut c_void) {
    let vptr = mgr as *mut *const usize;
    let vt = *vptr;
    let thunks = std::ptr::addr_of!(MGR_THUNKS) as *const usize;
    if vt == thunks {
        return;
    }
    let prev = MGR_VT_ORIG.swap(vt as usize, Ordering::SeqCst);
    if prev != 0 && prev != vt as usize {
        log(&format!("WARNING: host vtable changed from {prev:#x} to {vt:?}; counters now mix both"));
    }
    let n = readable_words(vt, MGR_SLOTS);
    let orig = std::ptr::addr_of_mut!(MGR_ORIG) as *mut usize;
    for i in 0..n {
        *orig.add(i) = *vt.add(i);
    }
    log(&format!("host {mgr:?} vtable {vt:?} wrapped ({n} slots readable)"));
    if let Some(dir) = OUT_DIR.get() {
        let lines: Vec<String> = (0..n).map(|i| format!("{i:3} +{:#05x} {:#x}", i * 8, *vt.add(i))).collect();
        let _ = fs::write(dir.join(format!("host-vtable-{}.txt", run_tag())), lines.join("\n") + "\n");
    }
    *vptr = thunks;
    start_watcher();
}

/// Logs each host slot's first call (to ~50 ms) and rewrites `host-calls.txt` with totals every 2 s.
fn start_watcher() {
    static STARTED: AtomicBool = AtomicBool::new(false);
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(|| {
        let mut seen = [0u64; MGR_SLOTS];
        let mut tick = 0u32;
        loop {
            std::thread::sleep(Duration::from_millis(50));
            for i in 0..MGR_SLOTS {
                let c = MGR_CALLS[i].load(Ordering::Relaxed);
                if seen[i] == 0 && c != 0 {
                    log(&format!("host slot {i} (+{:#x}) first called", i * 8));
                }
                seen[i] = c;
            }
            tick += 1;
            if tick % 40 == 0 {
                if let Some(dir) = OUT_DIR.get() {
                    let lines: Vec<String> = (0..MGR_SLOTS)
                        .filter(|&i| seen[i] != 0)
                        .map(|i| format!("{i:3} +{:#05x} {}", i * 8, seen[i]))
                        .collect();
                    let _ = fs::write(dir.join(format!("host-calls-{}.txt", run_tag())), lines.join("\n") + "\n");
                }
            }
        }
    });
}
