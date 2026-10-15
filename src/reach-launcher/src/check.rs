//! Build check: reach-host is pinned to one haloreach.dll (every engine address depends on it), so the launcher
//! refuses to start anything else. Expected hash = work/mcc/SHA256-frozen.txt.

use std::path::Path;

use sha2::{Digest, Sha256};

pub const FROZEN_SHA256: &str = "738dd2d24ea3aea12e1ee9aa4a61094bf116027d42004c35a19e5048608b0894";
pub const FROZEN_BUILD: &str = "2025.08.16.178512.1";

/// Files reach-host loads from the MCC folder besides the maps (see reach-host main.rs / audio.rs).
pub const NEEDED: [&str; 4] = [
    "haloreach/haloreach.dll",
    "haloreach/maps",
    "MCC/Binaries/Win64/bink2w64.dll",
    "MCC/Binaries/Win64/xaudio2_9redist.dll",
];

#[derive(Clone, Debug, PartialEq)]
pub enum Check {
    Ok { build: String },
    Missing(Vec<String>),
    WrongBuild { build: String, sha: String },
    Error(String),
}

pub fn sha256_file(p: &Path) -> std::io::Result<String> {
    let mut f = std::fs::File::open(p)?;
    let mut h = Sha256::new();
    std::io::copy(&mut f, &mut h)?;
    Ok(format!("{:x}", h.finalize()))
}

/// `build_tag.txt` first token, e.g. "2025.08.16.178512.1-Release" -> "2025.08.16.178512.1".
pub fn build_tag(mcc: &Path) -> String {
    std::fs::read_to_string(mcc.join("build_tag.txt"))
        .ok()
        .and_then(|t| t.split_whitespace().next().map(|s| s.trim_end_matches("-Release").to_string()))
        .unwrap_or_else(|| "unknown".into())
}

pub fn check(mcc: &Path) -> Check {
    let missing: Vec<String> = NEEDED.iter().filter(|f| !mcc.join(f).exists()).map(|f| f.to_string()).collect();
    if !missing.is_empty() {
        return Check::Missing(missing);
    }
    match sha256_file(&mcc.join("haloreach").join("haloreach.dll")) {
        Ok(sha) if sha == FROZEN_SHA256 => Check::Ok { build: build_tag(mcc) },
        Ok(sha) => Check::WrongBuild { build: build_tag(mcc), sha },
        Err(e) => Check::Error(format!("could not read haloreach.dll: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_and_wrong_build() {
        let d = std::env::temp_dir().join(format!("reach-launcher-check-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("haloreach/maps")).unwrap();
        assert!(matches!(check(&d), Check::Missing(m) if m.len() == 3));
        std::fs::create_dir_all(d.join("MCC/Binaries/Win64")).unwrap();
        for f in ["haloreach/haloreach.dll", "MCC/Binaries/Win64/bink2w64.dll", "MCC/Binaries/Win64/xaudio2_9redist.dll"] {
            std::fs::write(d.join(f), b"x").unwrap();
        }
        std::fs::write(d.join("build_tag.txt"), "2026.01.01.1-Release  \nRelease\n").unwrap();
        assert!(matches!(check(&d), Check::WrongBuild { build, .. } if build == "2026.01.01.1"));
        let _ = std::fs::remove_dir_all(&d);
    }
}
