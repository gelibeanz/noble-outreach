// Generates one stub per host-interface slot: load the slot index into rcx and tail-jump to the common
// logger `host_stub_common`, so the Rust call sees the same stack alignment as the original call.
use std::{env, fs, path::Path};

const SLOTS: usize = 192; // keep in sync with HOST_SLOTS in host.rs

fn main() {
    let mut s = String::from(".text\n");
    for i in 0..SLOTS {
        s += &format!(
            ".globl host_stub_{i}\n.p2align 4\nhost_stub_{i}:\n    mov ecx, {i}\n    jmp host_stub_common\n"
        );
    }
    // Event manager vtable: same idea, `evt_stub_N` → `evt_stub_common` (slot index in rcx).
    for i in 0..SLOTS {
        s += &format!(
            ".globl evt_stub_{i}\n.p2align 4\nevt_stub_{i}:\n    mov ecx, {i}\n    jmp evt_stub_common\n"
        );
    }
    s += ".section .rdata,\"dr\"\n.globl HOST_STUBS\n.p2align 3\nHOST_STUBS:\n";
    for i in 0..SLOTS {
        s += &format!("    .quad host_stub_{i}\n");
    }
    s += ".globl EVT_STUBS\n.p2align 3\nEVT_STUBS:\n";
    for i in 0..SLOTS {
        s += &format!("    .quad evt_stub_{i}\n");
    }
    s += ".text\n";
    fs::write(Path::new(&env::var("OUT_DIR").unwrap()).join("stubs.s"), s).unwrap();
    println!("cargo:rerun-if-changed=build.rs");
    // browser::DEFAULT_MASTERS reads it with option_env!.
    println!("cargo:rerun-if-env-changed=NOBLE_DEFAULT_MASTERS");
}
