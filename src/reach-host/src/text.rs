//! Host font rendering: slot 51 setting 6 = true and slots 61-67 (notes/host-slots.md, "Fonts").
//!
//! MCC does not ship `maps\fonts\font_package.bin`, so the engine's own text path has no glyphs at all (every
//! string drew empty). With setting 6 the engine loads `font_package_icon.bin` for its icon glyphs (U+E000..)
//! and asks the host for everything else: we rasterise glyphs with GDI into 1024x1024 RGBA atlas pages and hand
//! out their metrics; icon bitmaps come to us from the engine (slot 66) and go into the same atlas.

use std::collections::{HashMap, HashSet};
use std::ffi::{c_void, CStr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use windows::core::{Interface, PCWSTR};
use windows::Win32::Foundation::LPARAM;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::IDXGISwapChain;
use windows::Win32::Graphics::Gdi::*;

use crate::log;

const PAGE: usize = 1024; // the engine scales glyph texels by 1/1024 (0x180bb1b48)

#[derive(Clone, Copy)]
struct Glyph {
    left: u16,
    top: u16,
    w: i16,
    h: i16,
    bearing_y: i16,
    bearing_x: i16,
    advance: i16,
    page: u8,
}

struct Font {
    hfont: HFONT,
    ascent: u16,
    descent: u16,
}

struct Page {
    rgba: Vec<u8>,
    dirty: Option<(usize, usize)>, // row range to upload
    tex: Option<ID3D11Texture2D>,
    srv: Option<ID3D11ShaderResourceView>,
}

struct State {
    dc: HDC,
    families: HashSet<String>,
    fonts: HashMap<(String, i32, u32), Font>,
    glyphs: HashMap<(String, i32, u32, u16), Option<Glyph>>,
    pages: Vec<Page>,
    pen: (usize, usize, usize), // x, y, row height on the last page
}

// GDI handles and COM pointers are only touched under the mutex.
unsafe impl Send for State {}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut g = STATE.lock().unwrap_or_else(|e| e.into_inner());
    let s = g.get_or_insert_with(|| unsafe { State::new() });
    f(s)
}

unsafe extern "system" fn enum_family(lf: *const LOGFONTW, _tm: *const TEXTMETRICW, _t: u32, out: LPARAM) -> i32 {
    let set = &mut *(out.0 as *mut HashSet<String>);
    let name = &(*lf).lfFaceName;
    let n = name.iter().position(|&c| c == 0).unwrap_or(name.len());
    set.insert(String::from_utf16_lossy(&name[..n]).to_lowercase());
    1
}

/// Engine face (GetFontFaceName 0x1801ce748) -> installed font to use: the Halo fonts are not on the system, so
/// the closest common ones (Windows first, then Linux fonts that Wine sees through fontconfig).
fn candidates(face: &str) -> (&'static [&'static str], bool) {
    match face {
        "TVNordCondBold" => (&["Bahnschrift SemiBold Condensed", "Bahnschrift Condensed", "Arial Narrow", "Liberation Sans Narrow", "DejaVu Sans Condensed", "Arial", "Tahoma"], true),
        "TVNordCond" => (&["Bahnschrift Condensed", "Arial Narrow", "Liberation Sans Narrow", "DejaVu Sans Condensed", "Arial", "Tahoma"], false),
        "Eurostile" => (&["Eurostile", "Bahnschrift", "Liberation Sans", "DejaVu Sans", "Arial", "Tahoma"], false),
        "Fixedsys" => (&["Consolas", "Lucida Console", "Liberation Mono", "DejaVu Sans Mono", "Courier New"], false),
        _ => (&["Arial", "Liberation Sans", "DejaVu Sans", "Tahoma"], false),
    }
}

impl State {
    unsafe fn new() -> State {
        let dc = CreateCompatibleDC(None);
        let mut families = HashSet::new();
        let lf = LOGFONTW { lfCharSet: DEFAULT_CHARSET, ..Default::default() };
        EnumFontFamiliesExW(dc, &lf, Some(enum_family), LPARAM(&mut families as *mut _ as isize), 0);
        log!("host fonts: {} font families visible to GDI", families.len());
        State { dc, families, fonts: HashMap::new(), glyphs: HashMap::new(), pages: Vec::new(), pen: (0, 0, 0) }
    }

    unsafe fn font(&mut self, face: &str, size: i32, scale: f32) -> &Font {
        let key = (face.to_string(), size, scale.to_bits());
        if !self.fonts.contains_key(&key) {
            let (list, bold) = candidates(face);
            let pick = list.iter().find(|n| self.families.contains(&n.to_lowercase())).copied().unwrap_or("Tahoma");
            let px = ((size as f32) * scale.max(0.25)).round() as i32;
            let wide: Vec<u16> = pick.encode_utf16().chain([0]).collect();
            let hfont = CreateFontW(
                -px, 0, 0, 0, if bold { 700 } else { 400 }, 0, 0, 0, DEFAULT_CHARSET, OUT_TT_PRECIS,
                CLIP_DEFAULT_PRECIS, ANTIALIASED_QUALITY, 0, PCWSTR(wide.as_ptr()),
            );
            SelectObject(self.dc, hfont.into());
            let mut tm = TEXTMETRICW::default();
            let _ = GetTextMetricsW(self.dc, &mut tm);
            log!("host font {face} size {size} scale {scale}: using \"{pick}\" {px}px, ascent {} descent {}", tm.tmAscent, tm.tmDescent);
            self.fonts.insert(key.clone(), Font { hfont, ascent: tm.tmAscent as u16, descent: tm.tmDescent as u16 });
        }
        &self.fonts[&key]
    }

    /// Reserve a w x h cell (1 px gutter) in the atlas; returns (page, x, y).
    fn alloc(&mut self, w: usize, h: usize) -> (usize, usize, usize) {
        let (gw, gh) = (w + 1, h + 1);
        if self.pages.is_empty() {
            self.new_page();
        }
        if self.pen.0 + gw > PAGE {
            self.pen = (0, self.pen.1 + self.pen.2, 0);
        }
        if self.pen.1 + gh > PAGE {
            self.new_page();
        }
        let (x, y) = (self.pen.0, self.pen.1);
        self.pen.0 += gw;
        self.pen.2 = self.pen.2.max(gh);
        (self.pages.len() - 1, x, y)
    }

    fn new_page(&mut self) {
        self.pages.push(Page { rgba: vec![0; PAGE * PAGE * 4], dirty: None, tex: None, srv: None });
        self.pen = (0, 0, 0);
    }

    /// Copy rows of RGBA into the atlas at (x, y) and mark them for upload.
    fn blit(&mut self, page: usize, x: usize, y: usize, w: usize, h: usize, mut px: impl FnMut(usize, usize) -> [u8; 4]) {
        let p = &mut self.pages[page];
        for r in 0..h {
            for c in 0..w {
                let o = ((y + r) * PAGE + x + c) * 4;
                p.rgba[o..o + 4].copy_from_slice(&px(c, r));
            }
        }
        let (a, b) = p.dirty.unwrap_or((y, y + h));
        p.dirty = Some((a.min(y), b.max(y + h)));
    }

    unsafe fn rasterise(&mut self, face: &str, size: i32, scale: f32, ch: u16) -> Option<Glyph> {
        let hfont = self.font(face, size, scale).hfont;
        SelectObject(self.dc, hfont.into());
        let mat = MAT2 { eM11: FIXED { fract: 0, value: 1 }, eM22: FIXED { fract: 0, value: 1 }, ..Default::default() };
        let mut gm = GLYPHMETRICS::default();
        let len = GetGlyphOutlineW(self.dc, ch as u32, GGO_GRAY8_BITMAP, &mut gm, 0, None, &mat);
        if len == u32::MAX {
            return None;
        }
        let mut g = Glyph {
            left: 0,
            top: 0,
            w: 0,
            h: 0,
            bearing_y: gm.gmptGlyphOrigin.y as i16,
            bearing_x: gm.gmptGlyphOrigin.x as i16,
            advance: gm.gmCellIncX as i16,
            page: 0,
        };
        if len > 0 {
            let mut buf = vec![0u8; len as usize];
            if GetGlyphOutlineW(self.dc, ch as u32, GGO_GRAY8_BITMAP, &mut gm, len, Some(buf.as_mut_ptr() as _), &mat) == u32::MAX {
                return None;
            }
            let (w, h) = (gm.gmBlackBoxX as usize, gm.gmBlackBoxY as usize);
            let pitch = (w + 3) & !3;
            let (page, x, y) = self.alloc(w, h);
            // White glyph, coverage in alpha (0..64 from GDI): the text shader tints it with the string colour.
            self.blit(page, x, y, w, h, |c, r| [255, 255, 255, (buf[r * pitch + c] as u32 * 255 / 64).min(255) as u8]);
            (g.left, g.top, g.w, g.h, g.page) = (x as u16, y as u16, w as i16, h as i16, page as u8);
        }
        Some(g)
    }
}

fn face_str(face: *const i8) -> String {
    if face.is_null() {
        return String::new();
    }
    unsafe { CStr::from_ptr(face) }.to_string_lossy().into_owned()
}

fn note(slot: usize, what: &str) {
    static SEEN: AtomicU64 = AtomicU64::new(0);
    let bit = 1u64 << (slot - 51);
    if SEEN.fetch_or(bit, Ordering::Relaxed) & bit == 0 {
        log!("host slot {slot} ({what}) called (first call)");
    }
}

/// Slot 51 `get_setting(id, bool* out, u64* out2)`. Id 6 = host renders fonts: on (see module docs). Everything
/// else stays false (engine defaults).
pub unsafe extern "system" fn slot_get_setting(_this: *mut c_void, id: i32, out: *mut bool, _out2: *mut u64) -> bool {
    static SEEN: AtomicU64 = AtomicU64::new(0);
    let on = id == 6;
    if (id as u32) < 64 && SEEN.fetch_or(1 << id, Ordering::Relaxed) & (1 << id) == 0 {
        log!("host slot 51 setting id {id} -> {on}");
    }
    if on && !out.is_null() {
        *out = true;
    }
    on
}

/// Slot 61: make sure every character of a zero-terminated string is in the atlas (icons excluded: the engine
/// strips U+E000..U+E2FF and sends those through 64/66).
unsafe extern "system" fn slot_font_precache_string(_this: *mut c_void, s: *const u16, size: i32, scale: f32, face: *const i8) -> bool {
    note(61, "font_precache_string");
    if s.is_null() {
        return true;
    }
    let face = face_str(face);
    if std::env::var_os("REACH_TEXT_LOG").is_some() {
        let n = (0..).take_while(|&i| *s.add(i) != 0).count();
        let text = std::slice::from_raw_parts(s, n);
        log!("precache {face} {size}: {:?} {:x?}", String::from_utf16_lossy(text), text);
    }
    with_state(|st| {
        let mut p = s;
        while *p != 0 {
            let key = (face.clone(), size, scale.to_bits(), *p);
            if !st.glyphs.contains_key(&key) {
                let g = st.rasterise(&face, size, scale, *p);
                st.glyphs.insert(key, g);
            }
            p = p.add(1);
        }
    });
    true
}

/// Slot 62: glyph record (26 bytes): +0 u16 char, +4 f32 scale, +8 i16 atlas x, +0xA atlas y, +0xC width,
/// +0xE height, +0x10 bearing y (top above baseline), +0x12 bearing x, +0x14 advance, +0x18 i8 atlas page,
/// +0x19 u8 valid. All in texels of the scaled font.
unsafe extern "system" fn slot_font_get_glyph(_this: *mut c_void, ch: u16, out: *mut u8, size: i32, scale: f32, face: *const i8) -> bool {
    note(62, "font_get_glyph");
    let face = face_str(face);
    let g = with_state(|st| {
        let key = (face.clone(), size, scale.to_bits(), ch);
        if let Some(g) = st.glyphs.get(&key) {
            return *g;
        }
        if (0xE000..0xF900).contains(&ch) {
            return None; // icons only exist once the engine handed them over (slot 66)
        }
        let g = st.rasterise(&face, size, scale, ch);
        st.glyphs.insert(key, g);
        g
    });
    let Some(g) = g else { return false };
    if out.is_null() {
        return true;
    }
    std::ptr::write_bytes(out, 0, 26);
    let put = |o: usize, v: i16| (out.add(o) as *mut i16).write_unaligned(v);
    put(0, ch as i16);
    (out.add(4) as *mut f32).write_unaligned(scale);
    put(8, g.left as i16);
    put(0xA, g.top as i16);
    put(0xC, g.w);
    put(0xE, g.h);
    put(0x10, g.bearing_y);
    put(0x12, g.bearing_x);
    put(0x14, g.advance);
    *out.add(0x18) = g.page;
    *out.add(0x19) = 1;
    true
}

/// Slot 63: shader view of atlas page `page`; uploads rows changed since the last call. Called on the render
/// thread right before the text batch is drawn, so the immediate context is ours to use here.
unsafe extern "system" fn slot_font_get_texture(_this: *mut c_void, page: i32) -> *mut c_void {
    note(63, "font_get_texture");
    with_state(|st| {
        let Some(p) = st.pages.get_mut(page as usize) else { return std::ptr::null_mut() };
        let sc = crate::host::SWAPCHAIN.load(Ordering::Relaxed) as *mut c_void;
        let Some(sc) = IDXGISwapChain::from_raw_borrowed(&sc) else { return std::ptr::null_mut() };
        let Ok(dev) = sc.GetDevice::<ID3D11Device>() else { return std::ptr::null_mut() };
        if p.tex.is_none() {
            let desc = D3D11_TEXTURE2D_DESC {
                Width: PAGE as u32,
                Height: PAGE as u32,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
                ..Default::default()
            };
            let mut tex = None;
            if let Err(e) = dev.CreateTexture2D(&desc, None, Some(&mut tex)) {
                log!("font page {page}: CreateTexture2D failed: {e}");
                return std::ptr::null_mut();
            }
            let tex = tex.unwrap();
            let mut srv = None;
            if let Err(e) = dev.CreateShaderResourceView(&tex, None, Some(&mut srv)) {
                log!("font page {page}: CreateShaderResourceView failed: {e}");
                return std::ptr::null_mut();
            }
            p.tex = Some(tex);
            p.srv = srv;
            p.dirty = Some((0, PAGE));
            log!("font atlas page {page} created");
        }
        if let (Some((a, b)), Some(tex)) = (p.dirty.take(), p.tex.as_ref()) {
            if let Ok(ctx) = dev.GetImmediateContext() {
                let bx = D3D11_BOX { left: 0, top: a as u32, front: 0, right: PAGE as u32, bottom: b as u32, back: 1 };
                ctx.UpdateSubresource(tex, 0, Some(&bx), p.rgba[a * PAGE * 4..].as_ptr() as _, (PAGE * 4) as u32, 0);
            }
        }
        p.srv.as_ref().map_or(std::ptr::null_mut(), |s| s.as_raw())
    })
}

/// Slot 64: does the host already hold icon glyph `ch` for this font? False makes the engine decode it from
/// font_package_icon and pass it to slot 66.
unsafe extern "system" fn slot_font_has_icon(_this: *mut c_void, ch: u16, size: i32, scale: f32, face: *const i8) -> bool {
    note(64, "font_has_icon");
    let face = face_str(face);
    with_state(|st| matches!(st.glyphs.get(&(face, size, scale.to_bits(), ch)), Some(Some(_))))
}

/// Slot 65: kerning between `ch` and `next` (texels). None: GDI pair kerning is not worth it at HUD sizes.
unsafe extern "system" fn slot_font_get_kerning(_this: *mut c_void, _ch: u16, _next: u16, _size: i32, _scale: f32, _face: *const i8) -> i32 {
    note(65, "font_get_kerning");
    0
}

/// Slot 66: icon glyph from font_package_icon, `w` x `h` RGBA8 (pitch bytes per row), `ascent` = the package's
/// per-glyph ascent. The engine draws icons top-aligned to the line ascent and in their own colours.
unsafe extern "system" fn slot_font_add_icon(
    _this: *mut c_void, ch: u16, size: i32, scale: f32, face: *const i8, w: i32, h: i32, ascent: i32, data: *const u8, pitch: i32,
) -> bool {
    note(66, "font_add_icon");
    let face = face_str(face);
    if data.is_null() || w <= 0 || h <= 0 || w as usize > PAGE || h as usize > PAGE {
        return false;
    }
    let (w, h) = (w as usize, h as usize);
    let src = std::slice::from_raw_parts(data, pitch as usize * h);
    with_state(|st| {
        let (page, x, y) = st.alloc(w, h);
        st.blit(page, x, y, w, h, |c, r| {
            let o = r * pitch as usize + c * 4;
            [src[o], src[o + 1], src[o + 2], src[o + 3]]
        });
        let g = Glyph {
            left: x as u16,
            top: y as u16,
            w: w as i16,
            h: h as i16,
            bearing_y: ascent as i16,
            bearing_x: 0,
            advance: w as i16,
            page: page as u8,
        };
        st.glyphs.insert((face, size, scale.to_bits(), ch), Some(g));
    });
    true
}

/// Slot 67: line ascent / descent (texels, both positive) for a font; line height = their sum.
unsafe extern "system" fn slot_font_get_line_metrics(_this: *mut c_void, size: i32, scale: f32, face: *const i8, asc: *mut u16, desc: *mut u16) {
    note(67, "font_get_line_metrics");
    let face = face_str(face);
    let (a, d) = with_state(|st| {
        let f = st.font(&face, size, scale);
        (f.ascent, f.descent)
    });
    if !asc.is_null() {
        *asc = a;
    }
    if !desc.is_null() {
        *desc = d;
    }
}

/// Extra back-buffer captures by wall clock for screens before spawn (pre-game scoreboard, loadout picker):
/// `REACH_CAPTURE_AT=14,16` saves `at-014s.bmp`, `at-016s.bmp` (seconds after the first frame).
pub fn timed_capture(secs: u64) -> Option<String> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let list = std::env::var("REACH_CAPTURE_AT").ok()?;
    let done = NEXT.load(Ordering::Relaxed);
    let due = list.split(',').filter_map(|t| t.trim().parse::<u64>().ok()).filter(|&t| t <= secs && t + 1 > done).max()?;
    NEXT.store(due + 1, Ordering::Relaxed);
    Some(format!("at-{due:03}s.bmp"))
}

pub fn overrides() -> Vec<(usize, usize)> {
    vec![
        (51, slot_get_setting as *const () as usize),
        (61, slot_font_precache_string as *const () as usize),
        (62, slot_font_get_glyph as *const () as usize),
        (63, slot_font_get_texture as *const () as usize),
        (64, slot_font_has_icon as *const () as usize),
        (65, slot_font_get_kerning as *const () as usize),
        (66, slot_font_add_icon as *const () as usize),
        (67, slot_font_get_line_metrics as *const () as usize),
    ]
}
