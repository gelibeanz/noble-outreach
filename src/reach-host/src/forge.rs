//! Forge: launching the engine's editor, saving the live map variant and the saved-variant library.
//!
//! Forge is not a separate engine mode: it is a multiplayer game whose game variant has engine type 1 ("sandbox",
//! MCC's `00_basic_editing_054.bin`); with it the engine's monitor, object menu and budget all work under our
//! host. Saving: engine message 0x0B makes the game thread snapshot the live map variant into a new object and hand
//! it to host slot 12; the object's vtable slot 8 serialises it to a 0x7329-byte BLF `.mvar` that data-access
//! slot 1 (`--map-variant`) loads again. Details: notes/forge.md.

use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Instant;

use crate::log;

/// MCC's Forge game variant ("Basic Editing", engine type 1).
pub const FORGE_GAME_VARIANT: &str = "00_basic_editing_054.bin";
/// Engine message: snapshot the live map variant and give it to host slot 12.
const ENGINE_MSG_GET_MAP_VARIANT: u32 = 0x0B;
/// Map variant object vtable: slot 8 = serialise as a new saved file `bool (this, u8** data, u32* len)`.
const MAP_VARIANT_SERIALIZE_AS_NEW: usize = 8;

/// The running game is Forge (game variant engine type 1).
pub static ACTIVE: AtomicBool = AtomicBool::new(false);
/// Map stem of the running game (for the sidecar of a saved variant).
static MAP: Mutex<String> = Mutex::new(String::new());
/// Name the next slot 12 delivery is saved under.
static PENDING: Mutex<Option<String>> = Mutex::new(None);
/// Last save result for the overlay: (message, ok, when).
static LAST: Mutex<Option<(String, bool, Instant)>> = Mutex::new(None);

/// One saved variant in `reach-data/map_variants`.
#[derive(Clone, Debug)]
pub struct Saved {
    pub name: String,
    pub path: PathBuf,
    /// Map stem from the sidecar (`forge_halo`), empty if unknown.
    pub map: String,
    pub modified: std::time::SystemTime,
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct Sidecar {
    map: String,
    #[serde(default)]
    name: String,
}

pub fn dir() -> PathBuf {
    let exe = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())).unwrap_or_default();
    exe.join("reach-data").join("map_variants")
}

/// Called from options.rs once the launch options hold the game variant: remembers whether this is Forge.
pub fn note_launch(map: &str, engine_type: u32) {
    let forge = engine_type == 1;
    ACTIVE.store(forge, Ordering::Relaxed);
    *MAP.lock().unwrap_or_else(|e| e.into_inner()) = map.to_string();
    if forge {
        log!("forge: Forge game on {map} (game variant engine type 1); save with the pause menu or `forge save <name>`");
    }
}

/// A game variant file whose content header says engine type 1 (sandbox = Forge): BLF `chdr` byte at file +0x6A.
pub fn is_forge_file(path: &Path) -> bool {
    let name = path.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    if name == FORGE_GAME_VARIANT {
        return true;
    }
    std::fs::read(path).ok().is_some_and(|b| b.len() > 0x6A && &b[0x30..0x34] == b"chdr" && b[0x6A] == 1)
}

/// Saved variants, newest first.
pub fn list() -> Vec<Saved> {
    let mut v: Vec<Saved> = std::fs::read_dir(dir())
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("mvar")))
        .map(|path| {
            let side: Sidecar = std::fs::read_to_string(path.with_extension("json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
            let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let modified = std::fs::metadata(&path).and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH);
            Saved { name: if side.name.is_empty() { stem } else { side.name }, path, map: side.map, modified }
        })
        .collect();
    v.sort_by(|a, b| b.modified.cmp(&a.modified));
    v
}

/// Map stem a saved variant belongs to (sidecar), if known.
pub fn map_of(path: &Path) -> Option<String> {
    let t = std::fs::read_to_string(path.with_extension("json")).ok()?;
    let s: Sidecar = serde_json::from_str(&t).ok()?;
    (!s.map.is_empty()).then_some(s.map)
}

/// A file-name-safe version of a variant name.
fn file_stem(name: &str) -> String {
    let s: String = name.trim().chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == ' ' { c } else { '_' }).collect();
    let s = s.trim().replace(' ', "_");
    if s.is_empty() { "forge_map".into() } else { s.chars().take(48).collect() }
}

/// Suggested name for a new save: "<Map> 1", "<Map> 2", ... not yet used.
pub fn suggest_name() -> String {
    let map = MAP.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let base = crate::ui::menu::map_title(&map);
    let taken: Vec<String> = list().into_iter().map(|s| s.name).collect();
    (1..).map(|n| format!("{base} {n}")).find(|n| !taken.contains(n)).unwrap()
}

/// Ask the engine for the live map variant; slot 12 writes it as `<name>.mvar`. False if no engine is running.
pub fn request_save(name: &str) -> bool {
    *PENDING.lock().unwrap_or_else(|e| e.into_inner()) = Some(name.to_string());
    let ok = crate::ui::settings_apply::post_engine(ENGINE_MSG_GET_MAP_VARIANT);
    log!("forge: save '{name}' requested (engine message {ENGINE_MSG_GET_MAP_VARIANT:#x}, posted {ok})");
    if !ok {
        set_last(format!("Could not save '{name}': no game running"), false);
    }
    ok
}

fn set_last(msg: String, ok: bool) {
    *LAST.lock().unwrap_or_else(|e| e.into_inner()) = Some((msg, ok, Instant::now()));
}

/// Last save message for 6 s after it happened: (text, ok, alpha).
pub fn last_status() -> Option<(String, bool, f32)> {
    let g = LAST.lock().unwrap_or_else(|e| e.into_inner());
    let (m, ok, at) = g.as_ref()?;
    let t = at.elapsed().as_secs_f32();
    (t < 6.0).then(|| (m.clone(), *ok, if t > 5.0 { 6.0 - t } else { 1.0 }))
}

/// Host slot 12 (`set_map_variant(obj)`): the engine's answer to message 0x0B, a fresh map variant object
/// (0x14CF0 bytes, engine heap, ours now; leaked, ~85 KB per save).
pub unsafe extern "system" fn slot_set_map_variant(_this: *mut c_void, obj: *mut c_void) {
    let name = PENDING.lock().unwrap_or_else(|e| e.into_inner()).take();
    log!("host slot 12 set_map_variant({obj:?}) for {:?}", name);
    let Some(name) = name else { return };
    if obj.is_null() {
        set_last(format!("Could not save '{name}': the engine sent no map variant"), false);
        return;
    }
    // Raw variant at obj +0x08; its content header (+0x00..+0x2B0) holds the name (UTF-16, 128 chars) at +0x80
    // and the description at +0x180 (as in MCC's own .mvar files at file +0xC0 / +0x1C0).
    let raw = (obj as *mut u8).add(8);
    let title: Vec<u16> = name.encode_utf16().take(127).chain(std::iter::once(0)).collect();
    std::ptr::write_bytes(raw.add(0x80), 0, 0x100);
    std::ptr::copy_nonoverlapping(title.as_ptr() as *const u8, raw.add(0x80), title.len() * 2);
    let desc: Vec<u16> = "Made in Forge with Noble Outreach".encode_utf16().chain(std::iter::once(0)).collect();
    std::ptr::write_bytes(raw.add(0x180), 0, 0x100);
    std::ptr::copy_nonoverlapping(desc.as_ptr() as *const u8, raw.add(0x180), desc.len() * 2);
    let ser: unsafe extern "system" fn(*mut c_void, *mut *const u8, *mut u32) -> bool =
        std::mem::transmute(*(*(obj as *const *const usize)).add(MAP_VARIANT_SERIALIZE_AS_NEW));
    let (mut data, mut len) = (std::ptr::null::<u8>(), 0u32);
    if !ser(obj, &mut data, &mut len) || data.is_null() || len == 0 {
        log!("forge: map variant serialise failed");
        set_last(format!("Could not save '{name}'"), false);
        return;
    }
    let bytes = std::slice::from_raw_parts(data, len as usize);
    match save_file(&name, bytes) {
        Ok(p) => {
            log!("forge: saved map variant '{name}' ({len} bytes) to {}", p.display());
            set_last(format!("Saved '{name}'"), true);
        }
        Err(e) => {
            log!("forge: save failed: {e}");
            set_last(format!("Could not save '{name}': {e}"), false);
        }
    }
}

fn save_file(name: &str, bytes: &[u8]) -> Result<PathBuf, String> {
    let d = dir();
    std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
    let p = d.join(format!("{}.mvar", file_stem(name)));
    std::fs::write(&p, bytes).map_err(|e| e.to_string())?;
    let map = MAP.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let side = Sidecar { map, name: name.to_string() };
    let _ = std::fs::write(p.with_extension("json"), serde_json::to_string_pretty(&side).unwrap_or_default());
    Ok(p)
}

/// Test-script command `forge save <name>`.
pub fn script_command(a: &[String]) {
    match a.first().map(String::as_str) {
        Some("save") => {
            let name = if a.len() > 1 { a[1..].join(" ") } else { suggest_name() };
            request_save(&name);
        }
        Some("count") => unsafe { log_object_count() },
        _ => log!("input script: forge {a:?}: expected `forge save <name>` or `forge count`"),
    }
}

/// Test hook `forge count`: number of live objects in the engine's object table (game thread only), to check that
/// an object placed on one machine shows up on another. Object table: TLS +0x10 -> data array: +0x20 entry size
/// (0x18), +0x28 signature `d@t@`, +0x2C capacity (0x800), +0x40/+0x44/+0x48 allocation counters (rise by one per
/// placed object), +0x50 entries; u16 salt at entry +0 (0 = free).
unsafe fn log_object_count() {
    let Some(block) = crate::input::engine_tls_block() else { return log!("forge count: no engine TLS (not the game thread)") };
    let objs = *(block.add(0x10) as *const *const u8);
    if objs.is_null() {
        return log!("forge count: no object table");
    }
    let hdr: Vec<String> = (0x20..0x50).step_by(4).map(|o| format!("{:#x}", (objs.add(o) as *const u32).read_unaligned())).collect();
    let max = ((objs.add(0x2C) as *const i32).read_unaligned()).clamp(0, 8192) as usize;
    let table = *(objs.add(0x50) as *const *const u8);
    if table.is_null() {
        return log!("forge count: empty table");
    }
    let used = (0..max).filter(|i| (table.add(i * 0x18) as *const u16).read_unaligned() != 0).count();
    log!("forge count: {used} live objects of {max} (header +0x20..: {})", hdr.join(" "));
}

/// Host slot overrides owned by Forge.
pub fn overrides() -> Vec<(usize, usize)> {
    vec![(12, slot_set_map_variant as *const () as usize)]
}
