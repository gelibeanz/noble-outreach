//! Lazy engine start for dedicated servers. An empty server waits "dormant": no window, no D3D device, no
//! haloreach.dll, only our UDP socket (net.rs), the info query (query.rs, players 0, flag "dormant") and the master
//! heartbeat. The first engine datagram from a client, or a wake datagram (`RWAK`, sent by clients before they load
//! their own engine), wakes it: main.rs then loads the engine as before. Engine datagrams are dropped until the
//! woken engine reaches launch state 8 (the engine's join retries, and the client's wait in `wait_for_host`, carry
//! the join through). Once the last player leaves the process exits and the supervisor restarts it dormant again.
//! Wire format and timings: notes/networking.md "Wake", notes/dedicated.md "Lazy engine start".

use std::net::{SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::log;

/// Wake datagram (16 bytes): magic `RWAK`, version 1, kind 1 (wake), 2 reserved, u64 sender network id (0 = none).
pub const WAKE_MAGIC: [u8; 4] = *b"RWAK";
const WAKE_VERSION: u8 = 1;
const WAKE_KIND: u8 = 1;

/// Server phases: 0 not lazy (engine loads at once, or a client), 1 dormant, 2 woken (engine loading), 3 ready
/// (launch state 8 reached once: engine datagrams flow).
pub static PHASE: AtomicU8 = AtomicU8::new(0);
pub const DORMANT: u8 = 1;
pub const WAKING: u8 = 2;
pub const READY: u8 = 3;
/// `--no-lazy`: load the engine at once (old behaviour).
pub static DISABLED: AtomicBool = AtomicBool::new(false);
static WOKE_AT: Mutex<Option<Instant>> = Mutex::new(None);

pub fn phase() -> u8 {
    PHASE.load(Ordering::Relaxed)
}

/// Query reply flag bits: 8 = dormant (no engine), 16 = engine starting (not accepting joins yet).
pub fn query_flags() -> u8 {
    match phase() {
        DORMANT => 8,
        WAKING => 16,
        _ => 0,
    }
}

fn wake(why: &str, from: SocketAddr) {
    if PHASE.compare_exchange(DORMANT, WAKING, Ordering::Relaxed, Ordering::Relaxed).is_ok() {
        *WOKE_AT.lock().unwrap() = Some(Instant::now());
        log!("dormant: woken by {why} from {from}; starting the engine");
    }
}

/// net.rs receive thread: `RWAK` datagrams (true = consumed). Only a lazy server acts on them; others drop them.
pub fn handle(pkt: &[u8], from: SocketAddr) -> bool {
    if pkt.len() < 8 || pkt[..4] != WAKE_MAGIC {
        return false;
    }
    if pkt[4] == WAKE_VERSION && pkt[5] == WAKE_KIND {
        wake("wake packet", from);
    }
    true
}

/// net.rs receive thread, for every engine datagram: true = drop it (engine not loaded / not in state 8 yet).
pub fn gate_engine_datagram(from: SocketAddr) -> bool {
    match phase() {
        DORMANT => {
            wake("engine packet", from);
            true
        }
        WAKING => true,
        _ => false,
    }
}

/// main.rs on a lazy dedicated server, after networking is up: block until woken. Logs the process's own CPU and
/// RAM every 30 s (what an idle server costs).
pub fn wait() {
    PHASE.store(DORMANT, Ordering::Relaxed);
    log!("dormant: waiting for the first player without the engine (wake on a join or RWAK packet)");
    let t0 = Instant::now();
    let mut last = Instant::now();
    let mut cpu0 = crate::process_cpu_100ns();
    while phase() == DORMANT {
        std::thread::sleep(Duration::from_millis(50));
        if last.elapsed() >= Duration::from_secs(30) {
            let cpu = crate::process_cpu_100ns();
            let pct = (cpu - cpu0) as f64 / 1e7 / last.elapsed().as_secs_f64() * 100.0;
            log!("dormant: {:.0} s, cpu {pct:.1} % of one core, working set {} MB", t0.elapsed().as_secs_f32(), working_set_mb());
            cpu0 = cpu;
            last = Instant::now();
        }
    }
}

/// main.rs loop (dedicated): note the engine reaching launch state 8 (joins can be answered from now on).
pub fn tick(launch_state: u32) {
    if phase() == WAKING && launch_state == 8 {
        PHASE.store(READY, Ordering::Relaxed);
        let woke = WOKE_AT.lock().unwrap().map_or(0.0, |t| t.elapsed().as_secs_f32());
        log!("dormant: engine ready for joins (launch state 8) {woke:.1} s after the wake");
    }
}

/// Seconds since the wake (None when the server was not lazily started).
pub fn since_wake() -> Option<Duration> {
    WOKE_AT.lock().unwrap().map(|t| t.elapsed())
}

pub fn working_set_mb() -> u64 {
    use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    use windows::Win32::System::Threading::GetCurrentProcess;
    unsafe {
        let mut c = PROCESS_MEMORY_COUNTERS { cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32, ..Default::default() };
        if GetProcessMemoryInfo(GetCurrentProcess(), &mut c, c.cb).is_ok() {
            return c.WorkingSetSize as u64 >> 20;
        }
    }
    0
}

// ---- client -------------------------------------------------------------------------------------------------

fn wake_packet() -> [u8; 16] {
    let mut p = [0u8; 16];
    p[..4].copy_from_slice(&WAKE_MAGIC);
    p[4] = WAKE_VERSION;
    p[5] = WAKE_KIND;
    p
}

/// Client, before loading its own engine: wake a dormant server so both load in parallel. 3 sends, harmless to
/// servers that are awake (dropped) or older builds (an unknown datagram on an unread port).
pub fn send_wake(host: SocketAddr) {
    let Ok(sock) = UdpSocket::bind(SocketAddr::from(([0, 0, 0, 0], 0))) else { return };
    for _ in 0..3 {
        let _ = sock.send_to(&wake_packet(), host);
    }
    log!("dormant: wake sent to {host}");
}

/// Client, just before starting its engine's join: if the server reports dormant/starting, wait until it is ready
/// (up to `limit`), re-sending the wake. No reply (older server, lost packets) = go ahead at once.
pub fn wait_for_host(host: SocketAddr, limit: Duration) {
    let t0 = Instant::now();
    let mut said = false;
    loop {
        let Some(r) = crate::query::query_many(&[host], Duration::from_millis(1000)).into_iter().next().flatten() else { return };
        if !r.waking {
            if said {
                log!("dormant: server ready after {:.1} s", t0.elapsed().as_secs_f32());
            }
            return;
        }
        if !said {
            said = true;
            log!("dormant: server waking, please wait (its engine is starting)");
        }
        if t0.elapsed() >= limit {
            log!("dormant: server still starting after {} s; joining anyway", limit.as_secs());
            return;
        }
        send_wake_quiet(host);
        std::thread::sleep(Duration::from_millis(500));
    }
}

fn send_wake_quiet(host: SocketAddr) {
    if let Ok(sock) = UdpSocket::bind(SocketAddr::from(([0, 0, 0, 0], 0))) {
        let _ = sock.send_to(&wake_packet(), host);
    }
}

