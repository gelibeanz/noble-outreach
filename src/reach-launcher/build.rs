// Embeds the launcher build version (REACH_LAUNCHER_VERSION from scripts/make-launcher.sh, else date + short commit)
// so the self-update can compare it with the master's manifest.
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=REACH_LAUNCHER_VERSION");
    println!("cargo:rerun-if-env-changed=NOBLE_UPDATE_SOURCE"); // update::BUILT_SOURCE
    let v = std::env::var("REACH_LAUNCHER_VERSION").ok().filter(|v| !v.trim().is_empty()).unwrap_or_else(|| {
        let run = |c: &str, a: &[&str]| Command::new(c).args(a).output().ok().filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());
        let date = run("date", &["+%Y.%m.%d"]).unwrap_or_else(|| "dev".into());
        let rev = run("git", &["rev-parse", "--short", "HEAD"]).unwrap_or_else(|| "nogit".into());
        format!("{date}-{rev}")
    });
    println!("cargo:rustc-env=REACH_LAUNCHER_BUILD={v}");

    // Exe icon on Windows builds (assets/icon.ico, rendered from assets/icon.svg by scripts/make-icon.sh).
    println!("cargo:rerun-if-changed=assets/icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        if let Err(e) = res.compile() {
            panic!("embedding the exe icon failed ({e}); install mingw windres and its preprocessor (binutils-mingw-w64-x86-64, gcc-mingw-w64-x86-64)");
        }
    }
}
