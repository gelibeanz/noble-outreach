use std::fs::File;
use std::io::Write;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

static START: OnceLock<Instant> = OnceLock::new();
static LOG: Mutex<Option<File>> = Mutex::new(None);

pub fn init(path: &std::path::Path) {
    START.get_or_init(Instant::now);
    *LOG.lock().unwrap() = File::create(path).ok();
}

/// Timestamped line to `reach-host.log` and stdout. Never blocks (exception handler may log mid-fault).
pub fn log(msg: &str) {
    let t = START.get_or_init(Instant::now).elapsed().as_secs_f64();
    let line = format!("[{t:9.3}] {msg}");
    println!("{line}");
    if let Ok(mut g) = LOG.try_lock() {
        if let Some(f) = g.as_mut() {
            let _ = writeln!(f, "{line}");
            let _ = f.flush();
        }
    }
}

#[macro_export]
macro_rules! log {
    ($($a:tt)*) => { $crate::log::log(&format!($($a)*)) };
}
