//! `--probe rva[,rva...]`: breakpoints on engine functions for RE. Each RVA gets an `int3`; the first
//! `MAX_HITS` hits log the call stack, rcx/rdx/r8/r9 and the first 16 bytes at rcx. The original byte is put
//! back, one instruction is single-stepped, then the `int3` is re-armed.

use std::sync::Mutex;

use windows::Win32::System::Diagnostics::Debug::{
    AddVectoredExceptionHandler, RtlCaptureStackBackTrace, EXCEPTION_POINTERS,
};
use windows::Win32::System::Memory::{VirtualProtect, PAGE_EXECUTE_READWRITE, PAGE_PROTECTION_FLAGS};

use crate::log;

const MAX_HITS: u32 = 12;

/// (address, original byte, hits)
static ARMED: Mutex<Vec<(usize, u8, u32)>> = Mutex::new(Vec::new());
thread_local! {
    static STEPPING: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

unsafe fn patch(addr: usize, byte: u8) {
    let mut old = PAGE_PROTECTION_FLAGS(0);
    if VirtualProtect(addr as _, 1, PAGE_EXECUTE_READWRITE, &mut old).is_ok() {
        *(addr as *mut u8) = byte;
        let _ = VirtualProtect(addr as _, 1, old, &mut old);
    }
}

unsafe extern "system" fn handler(info: *mut EXCEPTION_POINTERS) -> i32 {
    const BREAKPOINT: i32 = 0x80000003u32 as i32;
    const SINGLE_STEP: i32 = 0x80000004u32 as i32;
    let rec = &*(*info).ExceptionRecord;
    let ctx = &mut *(*info).ContextRecord;
    if rec.ExceptionCode.0 == SINGLE_STEP {
        let a = STEPPING.with(|s| s.replace(0));
        if a == 0 {
            return 0;
        }
        patch(a, 0xCC);
        return -1;
    }
    if rec.ExceptionCode.0 != BREAKPOINT {
        return 0;
    }
    let addr = rec.ExceptionAddress as usize;
    let mut armed = ARMED.lock().unwrap();
    let Some(i) = armed.iter().position(|(a, _, _)| *a == addr) else { return 0 };
    armed[i].2 += 1;
    let (_, orig, hits) = armed[i];
    patch(addr, orig);
    ctx.Rip = addr as u64;
    if hits < MAX_HITS {
        ctx.EFlags |= 0x100;
        STEPPING.with(|s| s.set(addr));
    } else {
        armed.remove(i);
    }
    drop(armed);
    let at_rcx = if ctx.Rcx > 0x10000 {
        let p = ctx.Rcx as *const u32;
        format!(" [rcx]={:#x} {:#x} {:#x} {:#x}", *p, *p.add(1), *p.add(2), *p.add(3))
    } else {
        String::new()
    };
    log!("probe hit #{hits} {} rcx={:#x} rdx={:#x} r8={:#x} r9={:#x}{at_rcx}", crate::crash::describe(addr), ctx.Rcx, ctx.Rdx, ctx.R8, ctx.R9);
    // The return address of the probed function is at [rsp]; then the walk from our own frame.
    let ret = *(ctx.Rsp as *const usize);
    let mut frames = [std::ptr::null_mut::<std::ffi::c_void>(); 24];
    let n = RtlCaptureStackBackTrace(0, &mut frames, None) as usize;
    let trace: Vec<String> = frames[..n].iter().map(|f| crate::crash::describe(*f as usize)).filter(|s| s.starts_with("haloreach")).collect();
    log!("  return to {}; stack {}", crate::crash::describe(ret), trace.join(" <- "));
    -1 // EXCEPTION_CONTINUE_EXECUTION
}

/// Arm the probes; call after the engine DLL is loaded and before `initialize_game`.
pub fn arm(base: usize, rvas: &[usize]) {
    if rvas.is_empty() {
        return;
    }
    unsafe {
        AddVectoredExceptionHandler(1, Some(handler));
        let mut armed = ARMED.lock().unwrap();
        for &rva in rvas {
            let a = base + rva;
            armed.push((a, *(a as *const u8), 0));
            patch(a, 0xCC);
            log!("probe armed at haloreach+{rva:#x}");
        }
    }
}
