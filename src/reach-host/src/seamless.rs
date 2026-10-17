//! One window, one process, game after game (Reclaimer style): when a game ends (LEAVE GAME, vote switch, next
//! campaign mission, host left) the engine is shut down and `haloreach.dll` is unloaded and loaded again, and the
//! next game (or the main menu) starts in the same window. The engine is built for this: its game thread holds its
//! own reference on the DLL and leaves with `FreeLibraryAndExitThread`, and `RunGameThreadBody` shuts every game
//! system down and frees its 2 GB arena before the thread ends. Reloading the DLL gives fresh engine globals.
//! Notes: notes/engine-interface.md "Game after game in one process".
//!
//! `REACH_RELAUNCH=1` keeps the old behaviour (a fresh process per game) as a fallback.

use std::ffi::c_void;
use std::sync::atomic::AtomicU32;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HANDLE, HMODULE, WAIT_OBJECT_0};
use windows::Win32::System::Threading::WaitForSingleObject;

use crate::log;

/// What `RequestGameExit` (0x18000d648) does, written from our thread: the first exit reason (0x180c1a110) and its
/// message (0x180c1a108, slot 4 gets both), the flag 0x182b0354f, and the main loop's exit flag (0x18263ea74); the
/// game thread then shuts down and ends. Calling the function itself from the window thread can deadlock: it takes
/// an engine lock (0x180032bd8) that the game thread may hold while it waits on our window (seen once on a client).
const RVA_EXIT_REASON: usize = 0xc1a110;
const RVA_EXIT_MESSAGE: usize = 0xc1a108;
const RVA_EXIT_REQUESTED: usize = 0x2b0354f;
/// Main loop exit flag (byte).
const RVA_EXIT_FLAG: usize = 0x263ea74;
/// Exit reason we report (the engine's own use 3, 4, 7, 9; slot 4 just logs it).
const EXIT_REASON_HOST: i32 = 2;

/// Games played in this process (1 = the first).
pub static GAMES: AtomicU32 = AtomicU32::new(0);

/// What comes after this game, decided while it runs (vote follow, LEAVE GAME, host left): the main loop ends the
/// engine and starts it.
#[derive(Clone, Debug)]
pub enum Next {
    /// Back to the main menu; the notice is shown on the server browser.
    Menu(Option<String>),
    /// Another game with these arguments (process arguments, as `std::env::args().skip(1)`).
    Game(Vec<String>),
}

static NEXT: Mutex<Option<Next>> = Mutex::new(None);
/// The current game's full argument list (the menu's flags included): what a vote follow rewrites.
static ARGV: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// False with `REACH_RELAUNCH=1`: every game in a fresh process, as before.
pub fn enabled() -> bool {
    std::env::var_os("REACH_RELAUNCH").is_none_or(|v| v.is_empty() || v == "0")
}

pub fn set_next(n: Next) {
    let mut g = NEXT.lock().unwrap_or_else(|e| e.into_inner());
    if g.is_none() {
        log!("seamless: next = {n:?}");
        *g = Some(n);
    }
}

pub fn take_next() -> Option<Next> {
    NEXT.lock().unwrap_or_else(|e| e.into_inner()).take()
}

pub fn next_pending() -> bool {
    NEXT.lock().unwrap_or_else(|e| e.into_inner()).is_some()
}

pub fn set_argv(a: &[String]) {
    *ARGV.lock().unwrap_or_else(|e| e.into_inner()) = a.to_vec();
}

pub fn argv() -> Vec<String> {
    ARGV.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Ask the running engine to end its game (any thread). Returns false when an exit was already requested.
/// `why` must be a static NUL-terminated string (the engine keeps the pointer).
pub unsafe fn request_exit(base: usize, why: &'static [u8]) -> bool {
    let flag = (base + RVA_EXIT_FLAG) as *mut u8;
    if flag.read_volatile() != 0 {
        return false;
    }
    let reason = (base + RVA_EXIT_REASON) as *mut i32;
    if reason.read_volatile() == 0 {
        ((base + RVA_EXIT_MESSAGE) as *mut *const u8).write_volatile(why.as_ptr());
        reason.write_volatile(EXIT_REASON_HOST);
    }
    ((base + RVA_EXIT_REQUESTED) as *mut u8).write_volatile(1);
    flag.write_volatile(1);
    log!("seamless: game exit requested (reason {}, \"{}\")", reason.read_volatile(), String::from_utf8_lossy(&why[..why.len() - 1]));
    true
}

/// Wait for the game thread to end, pumping window messages so the window stays responsive.
pub unsafe fn wait_thread(thread: HANDLE, limit: Duration) -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{DispatchMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE, WM_QUIT};
    let t0 = Instant::now();
    let mut msg = MSG::default();
    loop {
        if WaitForSingleObject(thread, 10) == WAIT_OBJECT_0 {
            log!("seamless: game thread ended {:.2} s after the exit request", t0.elapsed().as_secs_f32());
            return true;
        }
        while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
            if msg.message == WM_QUIT {
                crate::net::send_bye();
                crate::ui::hard_exit(0);
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        if t0.elapsed() >= limit {
            log!("seamless: game thread still running {} s after the exit request", limit.as_secs());
            return false;
        }
    }
}

/// Threads of this process that started inside [base, base + size) (engine workers still alive).
pub unsafe fn engine_threads(base: usize, size: usize) -> Vec<(u32, usize)> {
    #[repr(C)]
    struct ThreadEntry32 {
        size: u32,
        usage: u32,
        tid: u32,
        owner_pid: u32,
        base_pri: i32,
        delta_pri: i32,
        flags: u32,
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn CreateToolhelp32Snapshot(flags: u32, pid: u32) -> *mut c_void;
        fn Thread32First(snap: *mut c_void, e: *mut ThreadEntry32) -> i32;
        fn Thread32Next(snap: *mut c_void, e: *mut ThreadEntry32) -> i32;
        fn OpenThread(access: u32, inherit: i32, tid: u32) -> *mut c_void;
        fn CloseHandle(h: *mut c_void) -> i32;
    }
    #[link(name = "ntdll")]
    extern "system" {
        fn NtQueryInformationThread(h: *mut c_void, class: u32, out: *mut c_void, len: u32, ret: *mut u32) -> i32;
    }
    const TH32CS_SNAPTHREAD: u32 = 4;
    const THREAD_QUERY_INFORMATION: u32 = 0x40;
    const THREAD_QUERY_SET_WIN32_START_ADDRESS: u32 = 9;
    let pid = std::process::id();
    let mut out = Vec::new();
    let snap = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
    if snap.is_null() || snap as isize == -1 {
        return out;
    }
    let mut e = ThreadEntry32 { size: std::mem::size_of::<ThreadEntry32>() as u32, usage: 0, tid: 0, owner_pid: 0, base_pri: 0, delta_pri: 0, flags: 0 };
    let mut ok = Thread32First(snap, &mut e) != 0;
    while ok {
        if e.owner_pid == pid {
            let h = OpenThread(THREAD_QUERY_INFORMATION, 0, e.tid);
            if !h.is_null() {
                let mut start = 0usize;
                if NtQueryInformationThread(h, THREAD_QUERY_SET_WIN32_START_ADDRESS, &mut start as *mut usize as *mut c_void, 8, std::ptr::null_mut()) == 0
                    && start >= base
                    && start < base + size
                {
                    out.push((e.tid, start - base));
                }
                CloseHandle(h);
            }
        }
        ok = Thread32Next(snap, &mut e) != 0;
    }
    CloseHandle(snap);
    out
}

/// Size of the loaded image (PE SizeOfImage).
pub unsafe fn image_size(base: usize) -> usize {
    let pe = base + *((base + 0x3c) as *const u32) as usize;
    *((pe + 0x50) as *const u32) as usize
}

/// Release our reference(s) on the engine DLL until Windows unloads it. Returns true when it is gone.
pub unsafe fn unload(module: HMODULE, path: &std::path::Path) -> bool {
    use windows::Win32::Foundation::FreeLibrary;
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    let wide: Vec<u16> = path.to_string_lossy().encode_utf16().chain(Some(0)).collect();
    for i in 0..4 {
        if FreeLibrary(module).is_err() {
            break;
        }
        if GetModuleHandleW(windows::core::PCWSTR(wide.as_ptr())).is_err() {
            log!("seamless: haloreach.dll unloaded after {} FreeLibrary call(s)", i + 1);
            return true;
        }
    }
    log!("seamless: haloreach.dll is still loaded after FreeLibrary");
    false
}

/// The engine does not release every Direct3D object it made on our device (the device's reference count grows by
/// 20-40 per game, and the working set left after an unload by roughly 25-300 MB per game, most for campaign
/// missions). Past this working set (MB, `REACH_SEAMLESS_MAX_MB`, default 3072) the next game starts in a fresh
/// process instead, which frees it all.
pub fn over_memory_limit() -> bool {
    let limit = std::env::var("REACH_SEAMLESS_MAX_MB").ok().and_then(|v| v.parse::<usize>().ok()).unwrap_or(3072);
    working_set() >> 20 > limit
}

fn working_set() -> usize {
    use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    use windows::Win32::System::Threading::GetCurrentProcess;
    let mut c = PROCESS_MEMORY_COUNTERS { cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32, ..Default::default() };
    unsafe {
        let _ = GetProcessMemoryInfo(GetCurrentProcess(), &mut c, c.cb);
    }
    c.WorkingSetSize
}

/// Working set and private bytes of this process, for the leak check across games.
pub fn mem_line() -> String {
    use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX};
    use windows::Win32::System::Threading::GetCurrentProcess;
    let mut c = PROCESS_MEMORY_COUNTERS_EX { cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32, ..Default::default() };
    unsafe {
        let _ = GetProcessMemoryInfo(GetCurrentProcess(), &mut c as *mut _ as *mut _, c.cb);
    }
    format!("working set {} MB, private {} MB, peak working set {} MB",
        c.WorkingSetSize >> 20, c.PrivateUsage >> 20, c.PeakWorkingSetSize >> 20)
}
