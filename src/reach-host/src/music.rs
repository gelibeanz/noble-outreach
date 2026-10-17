// Main-menu music: Reach's real menu theme, decoded at runtime from the player's own install (nothing is shipped).
// Samples live in `haloreach/fmod/pc/sfx.fsb` (FSB5, Vorbis); the Vorbis setup headers are not in the bank but in
// the player's `haloreach.dll`. Format facts and the DLL table: notes/frontend.md "Menu music".

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::thread::JoinHandle;
use std::time::Duration;

use windows::Win32::Media::Audio::*;

use crate::log;

const RATE: u32 = 48000;
const MENU_PREFIX: &str = "data\\sound\\music\\reach_menu\\track1\\";
const FADE_IN_S: f32 = 2.0;
const FADE_OUT_S: f32 = 1.0;
const BUF_FRAMES: usize = 4096; // 85 ms per waveOut buffer
const N_BUFS: usize = 5;

static STOP: AtomicBool = AtomicBool::new(false);
static THREAD: Mutex<Option<JoinHandle<()>>> = Mutex::new(None);

fn rd32(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}

/// One menu sample: where its packets sit in the bank and which setup header (by CRC) decodes them.
#[derive(Clone, Debug)]
struct Track {
    path: String,
    offset: u64, // from the start of the sample data
    len: usize,
    crc: u32,
}

/// Parses an FSB5 header (everything before the sample data) and returns, per sample, (data offset, crc, name
/// table index) plus the offset where the sample data starts and the total data size.
fn parse_fsb(head: &[u8]) -> Result<(Vec<(u64, u32)>, u64, u64), String> {
    if head.len() < 0x3c || &head[0..4] != b"FSB5" {
        return Err("not an FSB5 bank".into());
    }
    let n = rd32(head, 8) as usize;
    let shs = rd32(head, 12) as usize;
    let nts = rd32(head, 16) as usize;
    let data_size = rd32(head, 20) as u64;
    if head.len() < 0x3c + shs {
        return Err("short FSB header".into());
    }
    let sh = &head[0x3c..0x3c + shs];
    let mut o = 0;
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        if o + 8 > sh.len() {
            return Err("sample header overrun".into());
        }
        let v = u64::from_le_bytes(sh[o..o + 8].try_into().unwrap());
        o += 8;
        let mut more = v & 1 != 0;
        let off = ((v >> 7) & 0x7ff_ffff) * 32;
        let mut crc = 0;
        while more {
            if o + 4 > sh.len() {
                return Err("chunk overrun".into());
            }
            let c = rd32(sh, o);
            o += 4;
            more = c & 1 != 0;
            let size = ((c >> 1) & 0xff_ffff) as usize;
            if c >> 25 == 11 && size >= 4 && o + 4 <= sh.len() {
                crc = rd32(sh, o);
            }
            o += size;
        }
        out.push((off, crc));
    }
    Ok((out, (0x3c + shs + nts) as u64, data_size))
}

/// Full path of sample `i` from `sfx.fsb.info` (0x118-byte entries, path at +0x18).
fn info_path(info: &[u8], i: usize) -> Option<String> {
    let e = info.get(i * 0x118..(i + 1) * 0x118)?;
    let s = &e[0x18..];
    let end = s.iter().position(|&c| c == 0)?;
    Some(String::from_utf8_lossy(&s[..end]).to_lowercase())
}

/// The menu music tracks in bank order: intro first, then the loops.
fn menu_tracks(head: &[u8], info: &[u8]) -> Result<(Vec<Track>, u64), String> {
    let (samples, data_start, data_size) = parse_fsb(head)?;
    if info.len() / 0x118 != samples.len() {
        return Err(format!("sfx.fsb.info has {} entries, bank has {} samples", info.len() / 0x118, samples.len()));
    }
    let mut offs: Vec<u64> = samples.iter().map(|s| s.0).collect();
    offs.sort_unstable();
    let mut v = Vec::new();
    for (i, (off, crc)) in samples.iter().enumerate() {
        let Some(path) = info_path(info, i) else { continue };
        if !path.starts_with(MENU_PREFIX) {
            continue;
        }
        let next = offs.iter().copied().find(|&o| o > *off).unwrap_or(data_size);
        v.push(Track { path, offset: *off, len: (next - off) as usize, crc: *crc });
    }
    Ok((v, data_start))
}

/// Maps file offsets of a PE image to virtual addresses and back: (image base, [(va, raw offset, raw size)]).
fn pe_sections(d: &[u8]) -> Option<(u64, Vec<(u64, usize, usize)>)> {
    if d.len() < 0x40 || &d[0..2] != b"MZ" {
        return None;
    }
    let pe = rd32(d, 0x3c) as usize;
    if d.get(pe..pe + 4)? != b"PE\0\0" {
        return None;
    }
    let nsec = u16::from_le_bytes(d[pe + 6..pe + 8].try_into().ok()?) as usize;
    let opt = u16::from_le_bytes(d[pe + 20..pe + 22].try_into().ok()?) as usize;
    let base = u64::from_le_bytes(d.get(pe + 48..pe + 56)?.try_into().ok()?);
    let mut secs = Vec::new();
    for i in 0..nsec {
        let o = pe + 24 + opt + 40 * i;
        let h = d.get(o..o + 40)?;
        let (va, rs, ra) = (rd32(h, 12) as u64, rd32(h, 16) as usize, rd32(h, 20) as usize);
        secs.push((base + va, ra, rs));
    }
    Some((base, secs))
}

/// Finds the complete Vorbis setup header for `crc` in the DLL image. FMOD keeps a table of 0x20-byte rows
/// {u64 data VA, u32 size, u32 crc, u64 base VA, u32 patch offset, u32 patch length}. Base 0: `data` is the
/// complete header (`\x05vorbis...`, `size` bytes). Otherwise the header is `size` bytes copied from `base`, with
/// `patch length` bytes at `patch offset` replaced from `data` (FMOD's fsbvorbis codec does exactly this).
fn find_setup(dll: &[u8], crc: u32) -> Option<Vec<u8>> {
    let (_, secs) = pe_sections(dll)?;
    let va2f = |va: u64| secs.iter().find(|s| va >= s.0 && va < s.0 + s.2 as u64).map(|s| (va - s.0) as usize + s.1);
    let bytes = |va: u64, n: usize| -> Option<&[u8]> {
        let f = va2f(va)?;
        dll.get(f..f + n)
    };
    for &(_, ra, rs) in &secs {
        let mut o = ra.div_ceil(0x20) * 0x20;
        while o + 0x20 <= (ra + rs).min(dll.len()) {
            let e = &dll[o..o + 0x20];
            let data = u64::from_le_bytes(e[0..8].try_into().unwrap());
            let size = rd32(e, 8) as usize;
            if rd32(e, 12) == crc && (data >> 32) == 1 && (16..=0x8000).contains(&size) {
                let base = u64::from_le_bytes(e[16..24].try_into().unwrap());
                let (poff, plen) = (rd32(e, 24) as usize, rd32(e, 28) as usize);
                let hdr = if base == 0 {
                    bytes(data, size).map(|b| b.to_vec())
                } else if poff + plen <= size {
                    bytes(base, size).zip(bytes(data, plen)).map(|(b, p)| {
                        let mut h = b.to_vec();
                        h[poff..poff + plen].copy_from_slice(p);
                        h
                    })
                } else {
                    None
                };
                if let Some(h) = hdr.filter(|h| h.starts_with(b"\x05vorbis")) {
                    return Some(h);
                }
            }
            o += 0x20;
        }
    }
    None
}

/// Decodes one track's `{u16 size, packet}` stream, a block of interleaved stereo i16 at a time.
struct Stream {
    data: Vec<u8>,
    pos: usize,
    id: lewton::header::IdentHeader,
    setup: lewton::header::SetupHeader,
    prev: lewton::audio::PreviousWindowRight,
    packets: u32,
    errors: u32,
}

fn ident_header() -> Result<lewton::header::IdentHeader, String> {
    let mut h = vec![1u8];
    h.extend_from_slice(b"vorbis");
    h.extend_from_slice(&0u32.to_le_bytes());
    h.push(2);
    h.extend_from_slice(&RATE.to_le_bytes());
    h.extend_from_slice(&[0u8; 12]);
    h.push(0xb8); // blocksizes 256 / 2048
    h.push(1);
    lewton::header::read_header_ident(&h).map_err(|e| format!("ident header: {e:?}"))
}

impl Stream {
    fn new(data: Vec<u8>, setup_bytes: &[u8]) -> Result<Self, String> {
        let id = ident_header()?;
        let setup = lewton::header::read_header_setup(setup_bytes, 2, (id.blocksize_0, id.blocksize_1)).map_err(|e| format!("setup header: {e:?}"))?;
        Ok(Stream { data, pos: 0, id, setup, prev: lewton::audio::PreviousWindowRight::new(), packets: 0, errors: 0 })
    }

    /// Next decoded block (interleaved L R), or None at the end of the track.
    fn next(&mut self) -> Option<Vec<i16>> {
        loop {
            if self.pos + 2 > self.data.len() {
                return None;
            }
            let sz = u16::from_le_bytes([self.data[self.pos], self.data[self.pos + 1]]) as usize;
            self.pos += 2;
            if sz == 0 || self.pos + sz > self.data.len() {
                return None;
            }
            let pkt = &self.data[self.pos..self.pos + sz];
            self.pos += sz;
            self.packets += 1;
            let r: Result<Vec<Vec<i16>>, _> = lewton::audio::read_audio_packet(&self.id, &self.setup, pkt, &mut self.prev);
            match r {
                Ok(p) if p.len() >= 2 && !p[0].is_empty() => {
                    let mut out = Vec::with_capacity(p[0].len() * 2);
                    for k in 0..p[0].len() {
                        out.push(p[0][k]);
                        out.push(p[1][k]);
                    }
                    return Some(out);
                }
                Ok(_) => {}
                Err(_) => self.errors += 1,
            }
        }
    }
}

struct Source {
    file: std::fs::File,
    data_start: u64,
    tracks: Vec<Track>,
    setups: Vec<(u32, Vec<u8>)>,
    cur: Option<Stream>,
    cur_idx: usize,
    started: bool,
    rng: u64,
    packets: u32,
    errors: u32,
}

impl Source {
    fn open(mcc: &Path) -> Result<Source, String> {
        let dir = mcc.join("haloreach");
        let fsb = dir.join("fmod").join("pc").join("sfx.fsb");
        let mut file = std::fs::File::open(&fsb).map_err(|e| format!("{}: {e}", fsb.display()))?;
        let info = std::fs::read(dir.join("fmod").join("pc").join("sfx.fsb.info")).map_err(|e| format!("sfx.fsb.info: {e}"))?;
        let mut h = [0u8; 0x3c];
        file.read_exact(&mut h).map_err(|e| e.to_string())?;
        let shs = rd32(&h, 12) as usize;
        let nts = rd32(&h, 16) as usize;
        let mut head = h.to_vec();
        head.resize(0x3c + shs + nts, 0);
        file.read_exact(&mut head[0x3c..]).map_err(|e| e.to_string())?;
        let (tracks, data_start) = menu_tracks(&head, &info)?;
        let intro = tracks.iter().filter(|t| t.path.contains("\\in\\")).count();
        let loops = tracks.iter().filter(|t| t.path.contains("\\loop\\")).count();
        if intro == 0 && loops == 0 {
            return Err("no menu music tracks in sfx.fsb".into());
        }
        let dllp = dir.join("haloreach.dll");
        let dll = std::fs::read(&dllp).map_err(|e| format!("{}: {e}", dllp.display()))?;
        let mut setups: Vec<(u32, Vec<u8>)> = Vec::new();
        for t in &tracks {
            if !setups.iter().any(|s| s.0 == t.crc) {
                let s = find_setup(&dll, t.crc).ok_or_else(|| format!("no Vorbis setup header for crc {:08x} in haloreach.dll (different build?)", t.crc))?;
                setups.push((t.crc, s));
            }
        }
        log!("music: {} tracks ({intro} intro, {loops} loops), {} setup header(s) from haloreach.dll", tracks.len(), setups.len());
        let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_nanos() as u64) | 1;
        Ok(Source { file, data_start, tracks, setups, cur: None, cur_idx: usize::MAX, started: false, rng: seed, packets: 0, errors: 0 })
    }

    fn rand(&mut self, n: usize) -> usize {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng % n as u64) as usize
    }

    /// Intro once, then random loops, never the same one twice in a row.
    fn pick(&mut self) -> usize {
        if !self.started {
            self.started = true;
            if let Some(i) = self.tracks.iter().position(|t| t.path.contains("\\in\\")) {
                return i;
            }
        }
        let loops: Vec<usize> = (0..self.tracks.len()).filter(|&i| self.tracks[i].path.contains("\\loop\\") && i != self.cur_idx).collect();
        if loops.is_empty() {
            return self.cur_idx.min(self.tracks.len() - 1);
        }
        loops[self.rand(loops.len())]
    }

    fn start_track(&mut self) -> Result<(), String> {
        let i = self.pick();
        let t = self.tracks[i].clone();
        let mut data = vec![0u8; t.len];
        self.file.seek(SeekFrom::Start(self.data_start + t.offset)).map_err(|e| e.to_string())?;
        self.file.read_exact(&mut data).map_err(|e| e.to_string())?;
        let setup = &self.setups.iter().find(|s| s.0 == t.crc).ok_or("setup missing")?.1;
        self.cur = Some(Stream::new(data, setup)?);
        self.cur_idx = i;
        log!("music: playing {}", t.path.rsplit('\\').take(2).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("/"));
        Ok(())
    }

    /// Next block of interleaved stereo samples; moves on to the next track at the end of one (gapless).
    fn next(&mut self) -> Option<Vec<i16>> {
        for _ in 0..4 {
            if let Some(s) = &mut self.cur {
                if let Some(b) = s.next() {
                    return Some(b);
                }
                self.packets += s.packets;
                self.errors += s.errors;
            }
            if let Err(e) = self.start_track() {
                log!("music: {e}");
                return None;
            }
        }
        None
    }
}

fn write_wav(path: &str, pcm: &[i16]) {
    let dl = (pcm.len() * 2) as u32;
    let mut w = Vec::new();
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + dl).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    for v in [16u32, 0x0002_0001, RATE, RATE * 4, 0x0010_0004] {
        w.extend_from_slice(&v.to_le_bytes());
    }
    w.extend_from_slice(b"data");
    w.extend_from_slice(&dl.to_le_bytes());
    for s in pcm {
        w.extend_from_slice(&s.to_le_bytes());
    }
    let _ = std::fs::write(path, w);
}

/// Starts the menu music on its own thread (no-op if already running). Safe to call when the files are missing.
pub fn start(mcc: &Path) {
    let mut g = THREAD.lock().unwrap_or_else(|e| e.into_inner());
    if g.is_some() {
        return;
    }
    STOP.store(false, Ordering::Relaxed);
    let mcc: PathBuf = mcc.to_path_buf();
    *g = Some(std::thread::spawn(move || {
        let mut src = match Source::open(&mcc) {
            Ok(s) => s,
            Err(e) => {
                log!("music: none ({e})");
                return;
            }
        };
        if let Ok(p) = std::env::var("REACH_MUSIC_DUMP") {
            // Test hook: decode the first ~30 s to a WAV instead of playing.
            if std::env::var_os("REACH_MUSIC_TRACKS").is_some() {
                for i in 0..src.tracks.len() {
                    src.cur_idx = i;
                    let t = src.tracks[i].clone();
                    let mut data = vec![0u8; t.len];
                    src.file.seek(SeekFrom::Start(src.data_start + t.offset)).unwrap();
                    src.file.read_exact(&mut data).unwrap();
                    let mut st = Stream::new(data, &src.setups.iter().find(|s| s.0 == t.crc).unwrap().1).unwrap();
                    let (mut n, mut e1, mut e2, mut clip) = (0usize, 0f64, 0f64, 0usize);
                    let mut last = 0i16;
                    while let Some(b) = st.next() {
                        for k in (0..b.len()).step_by(2) {
                            e1 += (b[k] as f64).powi(2);
                            e2 += ((b[k] as i32 - last as i32) as f64).powi(2);
                            clip += (b[k].unsigned_abs() >= 32767) as usize;
                            last = b[k];
                            n += 1;
                        }
                    }
                    log!("music: track {i} {} crc {:08x}: {:.1}s rms {:.0} diff/rms {:.2} clipped {clip} errs {}", t.path, t.crc, n as f64 / RATE as f64, (e1 / n.max(1) as f64).sqrt(), (e2 / e1.max(1.0)).sqrt(), st.errors);
                }
                return;
            }
            let mut pcm = Vec::new();
            while pcm.len() < RATE as usize * 2 * 30 {
                let Some(b) = src.next() else { break };
                pcm.extend(b);
            }
            let rms = (pcm.iter().map(|&s| (s as f64).powi(2)).sum::<f64>() / pcm.len().max(1) as f64).sqrt();
            log!("music: dumped {} frames to {p}, rms {rms:.0}, packets {}, errors {}", pcm.len() / 2, src.packets, src.errors);
            write_wav(&p, &pcm);
            return;
        }
        unsafe { play(&mut src) };
    }));
}

/// Fades the music out (about a second) and waits for the thread to end.
pub fn stop() {
    STOP.store(true, Ordering::Relaxed);
    let h = THREAD.lock().unwrap_or_else(|e| e.into_inner()).take();
    if let Some(h) = h {
        let _ = h.join();
    }
}

unsafe fn play(src: &mut Source) {
    let fmt = WAVEFORMATEX { wFormatTag: WAVE_FORMAT_PCM as u16, nChannels: 2, nSamplesPerSec: RATE, nAvgBytesPerSec: RATE * 4, nBlockAlign: 4, wBitsPerSample: 16, cbSize: 0 };
    let mut hwo = HWAVEOUT::default();
    let r = waveOutOpen(Some(&mut hwo), WAVE_MAPPER, &fmt, None, None, CALLBACK_NULL);
    if r != 0 {
        log!("music: waveOutOpen failed ({r})");
        return;
    }
    log!("music: waveOut opened ({RATE} Hz stereo)");
    let mut bufs: Vec<Vec<i16>> = (0..N_BUFS).map(|_| vec![0i16; BUF_FRAMES * 2]).collect();
    let mut hdrs: Vec<WAVEHDR> = (0..N_BUFS).map(|_| WAVEHDR::default()).collect();
    for i in 0..N_BUFS {
        hdrs[i].lpData = windows::core::PSTR(bufs[i].as_mut_ptr() as *mut u8);
        hdrs[i].dwBufferLength = (BUF_FRAMES * 4) as u32;
        waveOutPrepareHeader(hwo, &mut hdrs[i], size_of::<WAVEHDR>() as u32);
        hdrs[i].dwFlags |= WHDR_DONE; // free
    }
    let mut gain = 0.0f32;
    let mut pending: Vec<i16> = Vec::new();
    let mut ppos = 0;
    let mut paused = false;
    let mut vol = crate::ui::settings::get().audio.menu_music.clamp(0.0, 1.0);
    let mut last_vol = std::time::Instant::now();
    let mut played = 0u64;
    loop {
        let stopping = STOP.load(Ordering::Relaxed);
        if last_vol.elapsed() > Duration::from_millis(150) {
            vol = crate::ui::settings::get().audio.menu_music.clamp(0.0, 1.0);
            last_vol = std::time::Instant::now();
        }
        let target = if stopping { 0.0 } else { vol * vol }; // squared: a more natural slider
        if stopping && gain <= 0.0 {
            break;
        }
        if target <= 0.0 && gain <= 0.0 {
            if !paused {
                waveOutPause(hwo);
                paused = true;
            }
            std::thread::sleep(Duration::from_millis(100));
            continue;
        }
        if paused {
            waveOutRestart(hwo);
            paused = false;
        }
        let mut idle = true;
        for i in 0..N_BUFS {
            if hdrs[i].dwFlags & WHDR_DONE == 0 {
                continue;
            }
            idle = false;
            let buf = &mut bufs[i];
            for f in 0..BUF_FRAMES {
                if ppos >= pending.len() {
                    match src.next() {
                        Some(b) => {
                            pending = b;
                            ppos = 0;
                        }
                        None => {
                            pending.clear();
                            ppos = 0;
                        }
                    }
                }
                let (l, r) = if ppos + 1 < pending.len() { (pending[ppos], pending[ppos + 1]) } else { (0, 0) };
                ppos += 2;
                if gain < target {
                    gain = (gain + 1.0 / (FADE_IN_S * RATE as f32)).min(target);
                } else if gain > target {
                    gain = (gain - 1.0 / (FADE_OUT_S * RATE as f32)).max(target);
                }
                buf[f * 2] = (l as f32 * gain) as i16;
                buf[f * 2 + 1] = (r as f32 * gain) as i16;
            }
            hdrs[i].dwFlags &= !WHDR_DONE;
            waveOutWrite(hwo, &mut hdrs[i], size_of::<WAVEHDR>() as u32);
            played += 1;
        }
        if idle {
            std::thread::sleep(Duration::from_millis(8));
        }
    }
    waveOutReset(hwo);
    for h in hdrs.iter_mut() {
        waveOutUnprepareHeader(hwo, h, size_of::<WAVEHDR>() as u32);
    }
    let _ = waveOutClose(hwo);
    log!("music: stopped ({played} buffers played, {} packets decoded, {} errors)", src.packets + src.cur.as_ref().map_or(0, |s| s.packets), src.errors);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fsb_header_and_menu_filter() {
        // Two samples: #0 plain, #1 with a Vorbis chunk (crc 0xad11d38d); offsets 0 and 64.
        let mut sh = Vec::new();
        sh.extend_from_slice(&(0u64 | (0 << 7) | (100u64 << 34)).to_le_bytes());
        sh.extend_from_slice(&(1u64 | (2 << 7) | (100u64 << 34)).to_le_bytes());
        sh.extend_from_slice(&(((11u32) << 25) | (4 << 1)).to_le_bytes());
        sh.extend_from_slice(&0xad11d38du32.to_le_bytes());
        let mut head = b"FSB5".to_vec();
        head.resize(0x3c, 0);
        head[8..12].copy_from_slice(&2u32.to_le_bytes());
        head[12..16].copy_from_slice(&(sh.len() as u32).to_le_bytes());
        head[20..24].copy_from_slice(&200u32.to_le_bytes());
        head.extend_from_slice(&sh);
        let mut info = vec![0u8; 0x118 * 2];
        info[0x18..0x18 + 3].copy_from_slice(b"abc");
        let p = format!("{MENU_PREFIX}in\\02_menu_in");
        info[0x118 + 0x18..0x118 + 0x18 + p.len()].copy_from_slice(p.as_bytes());
        let (t, start) = menu_tracks(&head, &info).unwrap();
        assert_eq!(start, 0x3c + sh.len() as u64);
        assert_eq!(t.len(), 1);
        assert_eq!((t[0].offset, t[0].len, t[0].crc), (64, 136, 0xad11d38d));
    }

    #[test]
    fn setup_scan_uses_base_header() {
        // Minimal PE: one section at VA 0x1000, raw 0x200, size 0x400; image base 0x180000000.
        let mut d = vec![0u8; 0x600];
        d[0..2].copy_from_slice(b"MZ");
        d[0x3c..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        d[0x80..0x84].copy_from_slice(b"PE\0\0");
        d[0x86..0x88].copy_from_slice(&1u16.to_le_bytes());
        d[0x94..0x96].copy_from_slice(&0xf0u16.to_le_bytes());
        d[0x80 + 48..0x80 + 56].copy_from_slice(&0x1_8000_0000u64.to_le_bytes());
        let so = 0x80 + 24 + 0xf0;
        d[so + 12..so + 16].copy_from_slice(&0x1000u32.to_le_bytes());
        d[so + 16..so + 20].copy_from_slice(&0x400u32.to_le_bytes());
        d[so + 20..so + 24].copy_from_slice(&0x200u32.to_le_bytes());
        let hdr = b"\x05vorbisXXXXXXXXXX"; // 17 bytes at VA +0x100 (file 0x300)
        d[0x300..0x300 + hdr.len()].copy_from_slice(hdr);
        let ent = |d: &mut Vec<u8>, o: usize, data: u64, size: u32, crc: u32, base: u64| {
            d[o..o + 8].copy_from_slice(&data.to_le_bytes());
            d[o + 8..o + 12].copy_from_slice(&size.to_le_bytes());
            d[o + 12..o + 16].copy_from_slice(&crc.to_le_bytes());
            d[o + 16..o + 24].copy_from_slice(&base.to_le_bytes());
        };
        ent(&mut d, 0x220, 0x1_8000_1100, 17, 0x1111, 0); // complete entry
        ent(&mut d, 0x240, 0x1_8000_1180, 17, 0xad11d38d, 0x1_8000_1100); // derived: base copy + patch
        d[0x240 + 24..0x240 + 28].copy_from_slice(&8u32.to_le_bytes()); // patch offset 8, length 2
        d[0x240 + 28..0x240 + 32].copy_from_slice(&2u32.to_le_bytes());
        d[0x380..0x382].copy_from_slice(b"ZZ");
        let mut want = hdr.to_vec();
        want[8..10].copy_from_slice(b"ZZ");
        assert_eq!(find_setup(&d, 0x1111).as_deref(), Some(&hdr[..]));
        assert_eq!(find_setup(&d, 0xad11d38d).as_deref(), Some(&want[..]));
        assert!(find_setup(&d, 0x9999).is_none());
    }
}
