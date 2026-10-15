//! Menu art from the player's own MCC install (never shipped with us): MCC's UI texture packs in
//! `Data\UI\texturepacks`. Each `<name>.perm.bin` is an index of 0x1E0-byte entries (ASCII name at +0x44, pixel
//! format at +0x6C: 1 = BC1, 3 = BC3, u16 width/height at +0x74/+0x76, u64 byte size at +0x88 and offset at +0x90
//! into the matching `<name>.temp.bin`, one mip, no tiling). We decode BC1/BC3 to RGBA ourselves and hand egui a
//! texture. Names used by the CAMPAIGN screen: notes/frontend.md "Campaign screen".

use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use egui::{ColorImage, TextureHandle, TextureOptions};

use crate::log;

const ENTRY: usize = 0x1E0;
/// Packs we look in, in this order (the first entry of a name wins).
const PACKS: &[&str] = &["levelstexturepack", "mainmenuandcampaigntexturepack", "skullstexturepack", "globaluitexturepack", "spmappreviewtexturepack"];

#[derive(Clone)]
struct Item {
    temp: PathBuf,
    bc: u32,
    w: usize,
    h: usize,
    size: usize,
    off: u64,
}

#[derive(Default)]
struct Art {
    index: HashMap<String, Item>,
    textures: HashMap<String, Option<TextureHandle>>,
}

static ART: Mutex<Option<Art>> = Mutex::new(None);

/// Index the packs once (a few hundred KB of entry tables). Missing folder = no art; the screens fall back to text.
pub fn init(mcc: &Path) {
    let mut g = ART.lock().unwrap();
    if g.is_some() {
        return;
    }
    let dir = mcc.join("Data").join("UI").join("texturepacks");
    let mut art = Art::default();
    for p in PACKS {
        let Ok(b) = std::fs::read(dir.join(format!("{p}.perm.bin"))) else { continue };
        let temp = dir.join(format!("{p}.temp.bin"));
        for e in b.chunks_exact(ENTRY) {
            let name = String::from_utf8_lossy(e[0x44..0x68].split(|&c| c == 0).next().unwrap_or_default()).into_owned();
            let u32_at = |o: usize| u32::from_le_bytes(e[o..o + 4].try_into().unwrap());
            let u16_at = |o: usize| u16::from_le_bytes(e[o..o + 2].try_into().unwrap()) as usize;
            let u64_at = |o: usize| u64::from_le_bytes(e[o..o + 8].try_into().unwrap());
            let item = Item { temp: temp.clone(), bc: u32_at(0x6C), w: u16_at(0x74), h: u16_at(0x76), size: u64_at(0x88) as usize, off: u64_at(0x90) };
            if !name.is_empty() && matches!(item.bc, 1 | 3) && item.w > 0 && item.h > 0 {
                art.index.entry(name).or_insert(item);
            }
        }
    }
    log!("mccart: {} UI textures indexed in {}", art.index.len(), dir.display());
    *g = Some(art);
}

/// The texture `name` (e.g. `HREACH_C02_WINTERCONTINGENCY_LARGE`), decoded on first use; None when the player's
/// install does not have it.
pub fn texture(ctx: &egui::Context, name: &str) -> Option<TextureHandle> {
    let mut g = ART.lock().unwrap();
    let art = g.as_mut()?;
    if let Some(t) = art.textures.get(name) {
        return t.clone();
    }
    let t = art.index.get(name).cloned().and_then(|it| match load(&it) {
        Ok(img) => Some(ctx.load_texture(format!("mcc:{name}"), img, TextureOptions::LINEAR)),
        Err(e) => {
            log!("mccart: {name}: {e}");
            None
        }
    });
    art.textures.insert(name.to_string(), t.clone());
    t
}

fn load(it: &Item) -> Result<ColorImage, String> {
    let block = if it.bc == 1 { 8 } else { 16 };
    let (bw, bh) = (it.w.div_ceil(4), it.h.div_ceil(4));
    if bw * bh * block > it.size {
        return Err(format!("{}x{} BC{} needs {} bytes, entry has {}", it.w, it.h, it.bc, bw * bh * block, it.size));
    }
    let mut f = std::fs::File::open(&it.temp).map_err(|e| format!("{}: {e}", it.temp.display()))?;
    f.seek(SeekFrom::Start(it.off)).map_err(|e| e.to_string())?;
    let mut data = vec![0u8; bw * bh * block];
    f.read_exact(&mut data).map_err(|e| e.to_string())?;
    let mut rgba = vec![0u8; it.w * it.h * 4];
    for by in 0..bh {
        for bx in 0..bw {
            let b = &data[(by * bw + bx) * block..][..block];
            let mut px = [[0u8; 4]; 16];
            if it.bc == 1 {
                colour_block(b, &mut px, true);
            } else {
                colour_block(&b[8..], &mut px, false);
                alpha_block(&b[..8], &mut px);
            }
            for (i, p) in px.iter().enumerate() {
                let (x, y) = (bx * 4 + i % 4, by * 4 + i / 4);
                if x < it.w && y < it.h {
                    rgba[(y * it.w + x) * 4..][..4].copy_from_slice(p);
                }
            }
        }
    }
    Ok(ColorImage::from_rgba_unmultiplied([it.w, it.h], &rgba))
}

fn rgb565(c: u16) -> [u8; 4] {
    let (r, g, b) = ((c >> 11) & 31, (c >> 5) & 63, c & 31);
    [(r * 255 / 31) as u8, (g * 255 / 63) as u8, (b * 255 / 31) as u8, 255]
}

/// BC1 colour block (also the colour half of BC3, which always uses the four-colour mode).
fn colour_block(b: &[u8], px: &mut [[u8; 4]; 16], bc1: bool) {
    let (c0, c1) = (u16::from_le_bytes([b[0], b[1]]), u16::from_le_bytes([b[2], b[3]]));
    let (a, z) = (rgb565(c0), rgb565(c1));
    let mix = |p: u16, q: u16, d: u16| -> [u8; 4] {
        let f = |i: usize| ((a[i] as u16 * p + z[i] as u16 * q) / d) as u8;
        [f(0), f(1), f(2), 255]
    };
    let pal = if !bc1 || c0 > c1 { [a, z, mix(2, 1, 3), mix(1, 2, 3)] } else { [a, z, mix(1, 1, 2), [0, 0, 0, 0]] };
    let bits = u32::from_le_bytes([b[4], b[5], b[6], b[7]]);
    for (i, p) in px.iter_mut().enumerate() {
        *p = pal[(bits >> (i * 2) & 3) as usize];
    }
}

fn alpha_block(b: &[u8], px: &mut [[u8; 4]; 16]) {
    let (a0, a1) = (b[0] as u16, b[1] as u16);
    let mut pal = [0u16; 8];
    pal[0] = a0;
    pal[1] = a1;
    if a0 > a1 {
        for i in 1..7 {
            pal[i + 1] = ((7 - i as u16) * a0 + i as u16 * a1) / 7;
        }
    } else {
        for i in 1..5 {
            pal[i + 1] = ((5 - i as u16) * a0 + i as u16 * a1) / 5;
        }
        pal[6] = 0;
        pal[7] = 255;
    }
    let bits = b[2..8].iter().rev().fold(0u64, |acc, &x| acc << 8 | x as u64);
    for (i, p) in px.iter_mut().enumerate() {
        p[3] = pal[(bits >> (i * 3) & 7) as usize] as u8;
    }
}
