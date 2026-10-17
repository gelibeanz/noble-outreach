//! Between two games in one window (seamless.rs): a "Loading <map>" card with the map's art from the player's MCC
//! files (mccart.rs), drawn by us while the old engine is gone and the next one has not drawn yet. The flip-model
//! swap chain keeps showing the last presented frame, so one frame is enough until the engine's own load screen.

use windows::Win32::Graphics::Direct3D11::ID3D11DeviceContext;
use windows::Win32::Graphics::Dxgi::{IDXGISwapChain, IDXGISwapChain1, DXGI_PRESENT};

use windows::core::Interface;

use super::theme::{self, ACCENT, DIM, TEXT};
use crate::log;

/// Campaign missions in story order: the texture-pack key is `C<nn>_<NAME>` (see campaign.rs).
const MISSION_ORDER: &[&str] = &["m05", "m10", "m20", "m30", "m35", "m45", "m50", "m52", "m60", "m70", "m70_bonus"];

/// Texture-pack key of a name: capitals without spaces or punctuation (`ONI: Sword Base` → `ONISWORDBASE`).
fn key(name: &str) -> String {
    name.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_uppercase()
}

/// The large art of a map stem, as named in MCC's UI texture packs.
fn art_name(stem: &str) -> String {
    let title = super::menu::map_title(stem);
    if let Some(i) = MISSION_ORDER.iter().position(|m| *m == stem) {
        format!("HREACH_C{:02}_{}_LARGE", i + 1, key(&title))
    } else if stem.starts_with("ff") {
        format!("REACHFIREFIGHT_{}LARGE", key(&title))
    } else {
        format!("HREACHMP_{}LARGE", key(&title))
    }
}

/// The map a game's arguments play (`--map`, `--campaign`, `--firefight`, `--forge`), if they name one.
pub fn map_of(argv: &[String]) -> Option<String> {
    argv.iter()
        .position(|a| matches!(a.as_str(), "--map" | "--campaign" | "--firefight" | "--forge"))
        .and_then(|i| argv.get(i + 1))
        .filter(|m| !m.to_ascii_lowercase().ends_with(".mvar"))
        .cloned()
}

/// Draw the card once and present it. `map`: the next game's map stem (None: just the headline).
pub unsafe fn show(sc: &IDXGISwapChain1, dc: &ID3D11DeviceContext, headline: &str, map: Option<&str>, mcc: &std::path::Path) {
    super::mccart::init(mcc);
    let Ok(sc0) = sc.cast::<IDXGISwapChain>() else { return };
    let (rtv, size) = match super::back_buffer_rtv(&sc0) {
        Ok(v) => v,
        Err(e) => {
            log!("loading: no back buffer: {e}");
            return;
        }
    };
    let title = map.map(super::menu::map_title);
    let art = map.map(art_name);
    dc.ClearState();
    let r = super::draw(dc, &rtv, size, false, |ui| {
        theme::backdrop(ui);
        let full = ui.max_rect();
        let ctx = ui.ctx().clone();
        if let Some(t) = art.as_deref().and_then(|a| super::mccart::texture(&ctx, a)) {
            // Fill the window, keep the aspect, crop the overflow; darken so the text reads.
            let ts = t.size_vec2();
            let s = (full.width() / ts.x).max(full.height() / ts.y);
            let rect = egui::Rect::from_center_size(full.center(), ts * s);
            ui.painter().with_clip_rect(full).image(t.id(), rect, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::from_gray(150));
        }
        let band = egui::Rect::from_min_size(egui::pos2(full.left(), full.bottom() - 150.0), egui::vec2(full.width(), 150.0));
        ui.painter().rect_filled(band, 0.0, egui::Color32::from_black_alpha(170));
        let x = full.left() + 60.0;
        ui.painter().text(egui::pos2(x, band.top() + 30.0), egui::Align2::LEFT_TOP, headline.to_uppercase(), egui::FontId::proportional(18.0), DIM);
        if let Some(t) = &title {
            ui.painter().text(egui::pos2(x, band.top() + 56.0), egui::Align2::LEFT_TOP, t.to_uppercase(), egui::FontId::proportional(38.0), TEXT);
        }
        ui.painter().rect_filled(egui::Rect::from_min_size(egui::pos2(x, band.bottom() - 34.0), egui::vec2(120.0, 3.0)), 0.0, ACCENT);
    });
    if let Err(e) = r {
        log!("loading: draw failed: {e}");
        return;
    }
    drop(rtv);
    dc.Flush();
    // REACH_UI_SHOT includes "loading": keep the card as loading-<n>.bmp (n = the game that ended) for checks.
    if super::shot_steps().iter().any(|s| s == "loading") {
        let n = crate::seamless::GAMES.load(std::sync::atomic::Ordering::Relaxed);
        let path = std::env::current_exe().unwrap_or_default().with_file_name(format!("loading-{n}.bmp"));
        let _ = crate::host::capture_back_buffer(sc.as_raw(), &path);
    }
    let _ = sc.Present(0, DXGI_PRESENT(0));
    dc.ClearState();
    dc.Flush();
    log!("loading: \"{headline}\" {} shown{}", title.as_deref().unwrap_or(""), art.map_or(String::new(), |a| format!(" (art {a})")));
}
