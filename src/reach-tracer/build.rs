// Generates one counting thunk per host-interface slot. Each thunk bumps its call counter and
// tail-jumps to the original method, leaving every argument register (integer and float) untouched.
use std::{env, fs, path::Path};

const SLOTS: usize = 192; // keep in sync with MGR_SLOTS in lib.rs

fn main() {
    let mut s = String::from(".text\n");
    for i in 0..SLOTS {
        let o = i * 8;
        s += &format!(
            ".globl mgr_thunk_{i}\n.p2align 4\nmgr_thunk_{i}:\n    lock inc qword ptr [rip + MGR_CALLS + {o}]\n    jmp qword ptr [rip + MGR_ORIG + {o}]\n"
        );
    }
    s += ".section .rdata,\"dr\"\n.globl MGR_THUNKS\n.p2align 3\nMGR_THUNKS:\n";
    for i in 0..SLOTS {
        s += &format!("    .quad mgr_thunk_{i}\n");
    }
    s += ".text\n";
    fs::write(Path::new(&env::var("OUT_DIR").unwrap()).join("thunks.s"), s).unwrap();
    println!("cargo:rerun-if-changed=build.rs");
}
