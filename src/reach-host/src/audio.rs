// Engine audio needs MCC's XAudio2 redistributable. The engine loads it with
// `LoadLibraryExA("xaudio2_9redist.dll", 0, LOAD_LIBRARY_SEARCH_DEFAULT_DIRS)` (0x180176030) and also
// delay-imports it for XAudio2Create. Default dirs = exe dir + System32 (not SetDllDirectory, not cwd), so
// from our own exe folder it is never found and the engine runs silent. Preloading it by full path makes the
// by-name load return the already-loaded module.

use std::path::Path;

use windows::core::PCWSTR;
use windows::Win32::System::LibraryLoader::LoadLibraryW;

use crate::log;

pub unsafe fn preload(mcc: &Path) {
    let dll = mcc.join("MCC").join("Binaries").join("Win64").join("xaudio2_9redist.dll");
    let w: Vec<u16> = dll.to_string_lossy().encode_utf16().chain(Some(0)).collect();
    match LoadLibraryW(PCWSTR(w.as_ptr())) {
        Ok(_) => log!("audio: preloaded {}", dll.display()),
        Err(e) => log!("audio: could not preload {}: {e} (no game audio)", dll.display()),
    }
}

/// Applies the saved volumes as soon as the engine has its mastering voice, and logs the audio init state once the
/// game has had time to start it.
///
/// Why the host sets the volume itself: the engine copies the profile volumes into its globals only inside
/// `RebuildPlayerInputBindings` (0x1802c8dfc), and hands master to the mastering voice through a queue that an audio
/// pump drains at irregular times (0x180066678). Measured 2026-10-15 (Firefight, master 0): the global was 0 but the
/// voice still played at 1.0 for the first ~15 s, and a settings change alone (no control field changed) does not
/// trigger that rebuild at all.
pub fn watch(base: usize) {
    std::thread::spawn(move || {
        let rd = |a: usize| unsafe { *((base + a - 0x180000000) as *const u64) };
        let start = std::time::Instant::now();
        while rd(MASTERING_VOICE) == 0 && start.elapsed().as_secs() < 120 {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        if rd(MASTERING_VOICE) != 0 {
            apply_volume(&crate::ui::settings::get().audio);
            log_volume("mastering voice created");
        }
        std::thread::sleep(std::time::Duration::from_secs(12).saturating_sub(start.elapsed()));
        let load_hr = rd(0x180b019e0) as u32; // HRESULT of the xaudio2_9redist load + X3DAudio lookups
        let system = rd(0x180b88718); // audio system handle (0 = init failed)
        let master = rd(MASTERING_VOICE);
        let rate = rd(0x180b88764) as u32;
        let channels = rd(0x180b88760) as u32;
        log!(
            "audio: xaudio2 load hr={load_hr:#010x} system={system:#x} mastering_voice={master:#x} \
             channels={channels} rate={rate}"
        );
        log_volume("startup");
        if std::env::var_os("REACH_VOLUME_WATCH").is_some() {
            std::thread::spawn(|| {
                for t in [30, 50, 70] {
                    std::thread::sleep(std::time::Duration::from_secs(if t == 30 { 18 } else { 20 }));
                    log_volume(&format!("{t}s"));
                }
            });
        }
        // Test hook: REACH_TEST_VOLUME=<master> changes the master volume live (in memory, not saved), like the
        // Settings screen does, then logs the engine's effective volume again.
        if let Some(v) = std::env::var("REACH_TEST_VOLUME").ok().and_then(|v| v.parse::<f32>().ok()) {
            std::thread::sleep(std::time::Duration::from_secs(8));
            let old = crate::ui::settings::get();
            let mut new = old.clone();
            new.audio.master = v;
            log!("audio: test: master {} -> {v} live", old.audio.master);
            crate::ui::settings_apply::apply(&old, &new);
            std::thread::sleep(std::time::Duration::from_secs(2));
            log_volume("after live change");
        }
    });
}

/// Engine volume globals (f32, 0..1). Master goes to the mastering voice; music / sfx multiply every source voice's
/// volume each audio update (0x180178385 / 0x18017838f), so writing them takes effect live.
pub const MASTER_VOL: usize = 0x180afc108;
pub const MUSIC_VOL: usize = 0x180afc10c;
pub const SFX_VOL: usize = 0x180afc110;
/// IXAudio2MasteringVoice* (0 until the engine's audio init has run).
const MASTERING_VOICE: usize = 0x180b88730;

fn engine_addr(a: usize) -> Option<usize> {
    let base = crate::crash::ENGINE_BASE.load(std::sync::atomic::Ordering::Relaxed);
    (base != 0).then(|| base + a - 0x180000000)
}

/// Writes master / music / sfx into the engine's globals and sets the mastering voice's volume (0 = silent).
/// Safe to call from any thread (XAudio2 voice calls are thread-safe); does nothing before the engine is loaded.
pub fn apply_volume(a: &crate::ui::settings::Audio) {
    let Some(voice_at) = engine_addr(MASTERING_VOICE) else { return };
    let (m, mu, fx) = (a.master.clamp(0.0, 1.0), a.music.clamp(0.0, 1.0), a.sfx.clamp(0.0, 1.0));
    unsafe {
        for (g, v) in [(MASTER_VOL, m), (MUSIC_VOL, mu), (SFX_VOL, fx)] {
            *(engine_addr(g).unwrap() as *mut f32) = v;
        }
        let voice = *(voice_at as *const *mut *const usize);
        if !voice.is_null() {
            // IXAudio2Voice::SetVolume(float volume, UINT32 operation_set), vtable +0x60
            let set: unsafe extern "system" fn(*mut *const usize, f32, u32) -> i32 = std::mem::transmute(*(*voice).add(12));
            set(voice, m, 0);
        }
    }
}

/// Logs the engine's volume globals and the mastering voice's actual volume (IXAudio2Voice::GetVolume, vtable +0x68).
pub fn log_volume(tag: &str) {
    let Some(voice_at) = engine_addr(MASTERING_VOICE) else { return };
    unsafe {
        let [m, mu, fx] = [MASTER_VOL, MUSIC_VOL, SFX_VOL].map(|a| *(engine_addr(a).unwrap() as *const f32));
        let voice = *(voice_at as *const *mut *const usize);
        let mut real = f32::NAN;
        if !voice.is_null() {
            let get: unsafe extern "system" fn(*mut *const usize, *mut f32) = std::mem::transmute(*(*voice).add(13));
            get(voice, &mut real);
        }
        log!("audio: volume ({tag}): engine master {m} music {mu} sfx {fx}, mastering voice {real}");
    }
}
