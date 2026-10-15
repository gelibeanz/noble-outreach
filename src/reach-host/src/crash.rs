//! Vectored exception handler: our main debugging tool. Logs code, faulting address and a short stack
//! walk, then lets the exception continue (to the engine's own handlers or the crash).

use std::sync::atomic::{AtomicUsize, Ordering};

use windows::Win32::Foundation::HMODULE;
use windows::Win32::System::Diagnostics::Debug::{
    AddVectoredExceptionHandler, RtlCaptureStackBackTrace, EXCEPTION_POINTERS,
};
use windows::Win32::System::LibraryLoader::{
    GetModuleFileNameW, GetModuleHandleExW, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
    GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
};

use crate::log;

pub static ENGINE_BASE: AtomicUsize = AtomicUsize::new(0);
const ENGINE_SIZE: usize = 0x0600_0000;

/// `haloreach+0xRVA`, `module.dll+0xRVA`, or the bare address.
pub fn describe(addr: usize) -> String {
    let base = ENGINE_BASE.load(Ordering::Relaxed);
    if base != 0 && addr >= base && addr < base + ENGINE_SIZE {
        return format!("haloreach+{:#x}", addr - base);
    }
    unsafe {
        let mut h = HMODULE::default();
        if GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            windows::core::PCWSTR(addr as *const u16),
            &mut h,
        )
        .is_ok()
        {
            let mut buf = [0u16; 260];
            let n = GetModuleFileNameW(Some(h), &mut buf) as usize;
            let path = String::from_utf16_lossy(&buf[..n]);
            let name = path.rsplit(['\\', '/']).next().unwrap_or("?");
            return format!("{name}+{:#x}", addr - h.0 as usize);
        }
    }
    format!("{addr:#x}")
}

unsafe extern "system" fn handler(info: *mut EXCEPTION_POINTERS) -> i32 {
    const CONTINUE_SEARCH: i32 = 0;
    let rec = &*(*info).ExceptionRecord;
    let code = rec.ExceptionCode.0 as u32;
    // Skip C++ throws, debug-print and thread-name noise; keep errors and breakpoints.
    let interesting = (code >> 28) == 0xC || matches!(code, 0x8000_0001 | 0x8000_0003 | 0x8000_0004);
    if !interesting || code == 0xE06D_7363 {
        return CONTINUE_SEARCH;
    }
    let addr = rec.ExceptionAddress as usize;
    let mut extra = String::new();
    if code == 0xC000_0005 && rec.NumberParameters >= 2 {
        let kind = match rec.ExceptionInformation[0] {
            0 => "read",
            1 => "write",
            8 => "execute",
            _ => "?",
        };
        extra = format!(" access violation: {kind} at {:#x}", rec.ExceptionInformation[1]);
    }
    log!("EXCEPTION {code:#010x} at {}{extra}", describe(addr));
    dump_engine_globals();
    let mut frames = [std::ptr::null_mut::<std::ffi::c_void>(); 24];
    let n = RtlCaptureStackBackTrace(0, &mut frames, None) as usize;
    for (i, f) in frames[..n].iter().enumerate() {
        log!("  #{i:02} {}", describe(*f as usize));
    }
    CONTINUE_SEARCH
}

/// Engine globals worth seeing at a crash (RVAs into haloreach.dll, see notes/engine-interface.md).
const WATCHED: &[(&str, usize, usize)] = &[
    ("gfx.device (init_graphics arg)", 0xc1a0f8, 8),
    ("gfx.swapchain (init_graphics arg)", 0xc1a0f0, 8),
    ("gfx.hwnd (EnumWindows)", 0xc1a188, 8),
    ("rasterizer.initialized", 0x4e38860, 1),
    ("rasterizer.swapchain", 0x4e38868, 8),
    ("rasterizer.context1", 0x4e38870, 8),
    ("rasterizer.device1", 0x4e38878, 8),
    ("rasterizer.dxgi_factory", 0x4e38858, 8),
    ("rasterizer.pending_w/h", 0x4e38890, 8),
    ("render.width/height", 0xb43a90, 8),
    ("engine message queue ready", 0x4e2fca0, 1),
];

fn dump_engine_globals() {
    let base = ENGINE_BASE.load(Ordering::Relaxed);
    if base == 0 {
        return;
    }
    for &(name, rva, size) in WATCHED {
        let p = (base + rva) as *const u8;
        let v = unsafe {
            match size {
                1 => *p as u64,
                _ => (p as *const u64).read_unaligned(),
            }
        };
        log!("  [{name}] haloreach+{rva:#x} = {v:#x}");
    }
}

/// Load-screen and game-loop globals logged by the frame heartbeat (name, RVA, size: 1 byte, 4 = u32, 5 = f32).
const HEARTBEAT: &[(&str, usize, usize)] = &[
    ("load_screen.active", 0xc519c8, 1),
    ("load_screen.finish_tick (-1 = not at 90%)", 0xc519cc, 4),
    ("load.progress", 0x46f0e04, 5),
    ("load.in_progress", 0x46f0de4, 1),
    ("render: 0x1829f5798", 0x29f5798, 1),
    ("render: fade 0x182a0b2d8", 0x2a0b2d8, 5),
    ("0x184e2fe78 (game mode copy)", 0x4e2fe78, 4),
    ("engine paused 0x180c1a0e2", 0xc1a0e2, 1),
];

pub fn dump_heartbeat() {
    let base = ENGINE_BASE.load(Ordering::Relaxed);
    if base == 0 {
        return;
    }
    let mut line = String::new();
    for &(name, rva, size) in HEARTBEAT {
        let p = (base + rva) as *const u8;
        let v = unsafe {
            match size {
                1 => format!("{:#x}", *p),
                4 => format!("{:#x}", (p as *const u32).read_unaligned()),
                5 => format!("{}", (p as *const f32).read_unaligned()),
                _ => format!("{:#x}", (p as *const u64).read_unaligned()),
            }
        };
        line += &format!(" [{name}]={v}");
    }
    log!("  globals:{line}");
    // Per-thread game globals (engine TLS slot, index at 0x180c17b18): +0xa0 -> {u8 active, u16 pause bits}.
    unsafe {
        let idx = ((base + 0xc17b18) as *const u32).read() as usize;
        let tls: *const *const u8;
        core::arch::asm!("mov {}, gs:[0x58]", out(reg) tls);
        if tls.is_null() {
            return;
        }
        let block = *tls.add(idx);
        if block.is_null() {
            log!("  tls: no engine block on this thread");
            return;
        }
        let g = *(block.add(0xa0) as *const *const u8);
        if g.is_null() {
            log!("  tls+0xa0: null");
        } else {
            log!("  tls+0xa0 game time: active {} pause bits {:#06x}", *g, (g.add(2) as *const u16).read_unaligned());
        }
        // Firefight: tls+0x20 survival globals, +0x190 = waves begun (survival_mode_begin_new_wave increments it).
        let s = *(block.add(0x20) as *const *const u8);
        if !s.is_null() {
            log!("  tls+0x20 survival: waves begun {}", (s.add(0x190) as *const i32).read_unaligned());
        }
    }
}

pub fn install() {
    unsafe {
        AddVectoredExceptionHandler(1, Some(handler));
    }
}
