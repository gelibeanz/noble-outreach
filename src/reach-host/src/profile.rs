//! Look-control fields of the slot 34 profile (0xACC bytes). A zeroed profile left the zoomed mouse look at
//! 0 (no turning while scoped) and the pad look sensitivity at its lowest index. Values are the engine's own
//! defaults (`FUN_1802c87d0`, which initialises its default player profile).

/// Write MCC/engine default look settings into the profile at `p`.
pub unsafe fn write_look_defaults(p: *mut u8) {
    // Look sensitivity index 0..18 (vertical, horizontal): engine default 4. Copied to engine profile +0x50/+0x4C.
    *p.add(0x1b5) = 4;
    *p.add(0x1b6) = 4;
    // Look acceleration index 0..9: 0.
    *p.add(0x1b7) = 0;
    let put = |off: usize, v: f32| (p.add(off) as *mut f32).write_unaligned(v);
    // Stick axial / radial dead zone: 0.2.
    put(0x1b8, 0.2);
    put(0x1bc, 0.2);
    // Zoomed mouse look multiplier (engine profile +0x60). While zoomed, keyboard/mouse look is scaled by
    // this / zoom magnification (`GetZoomLookSensitivityScale` 0x1800618f0); 0 froze the crosshair.
    put(0x1c0, 1.0);
    // Mouse look multiplier in a vehicle seat (engine profile +0x64, same function).
    put(0x1c4, 1.0);
}

/// Slot 88's optional name buffer (`wchar_t*`, capacity in r9; the engine passes 0x40 for its 0x40-byte
/// per-user slots at 0x184e1b3b0): the gamertag shown on nameplates, the scoreboard and the loadout screen.
/// Left empty, every name drew blank. `REACH_NAME` sets it, else "Player <n>" from the xuid.
pub unsafe fn write_player_name(buf: *mut u16, cap: u32, xuid: u64) {
    if buf.is_null() || cap < 2 {
        return;
    }
    let name = std::env::var("REACH_NAME").ok().filter(|n| !n.trim().is_empty()).unwrap_or_else(|| format!("Player {}", xuid & 0xffff));
    let max = (cap as usize / 2).min(16) - 1; // Reach gamertags: 15 characters + terminator
    let wide: Vec<u16> = name.encode_utf16().take(max).collect();
    std::ptr::copy_nonoverlapping(wide.as_ptr(), buf, wide.len());
    *buf.add(wide.len()) = 0;
    static LOGGED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if !LOGGED.swap(true, std::sync::atomic::Ordering::Relaxed) {
        crate::log!("local player name \"{name}\" (slot 88 buffer, capacity {cap})");
    }
}

/// Writes the player's settings (ui/settings.rs) into the slot 34 profile. The engine reads these fields when it
/// applies a local player's settings (`ApplyProfileLookSettingsToPlayer` 0x1802eda08, FOV and volumes elsewhere);
/// engine message 5 re-runs that for every local player (notes/host-slots.md, slot 32/34).
pub unsafe fn write_settings(p: *mut u8, s: &crate::ui::settings::Settings) {
    let put_i = |off: usize, v: i32| (p.add(off) as *mut i32).write_unaligned(v);
    let put_f = |off: usize, v: f32| (p.add(off) as *mut f32).write_unaligned(v);
    let put_b = |off: usize, v: bool| *p.add(off) = v as u8;
    put_i(0x14, s.game.fov.clamp(60, 120));
    put_i(0x18, s.game.vehicle_fov.clamp(60, 120));
    put_b(0x1c, s.game.crosshair_lowered);
    put_b(0x1d, s.pad.invert_look);
    put_b(0x1e, false); // mouse look inversion is done by the host (input.rs), which owns mouse look
    put_b(0x1f, !s.pad.vibration);
    put_b(0x21, s.pad.invert_flight);
    put_b(0x22, s.mouse.invert_flight);
    put_b(0x23, s.pad.auto_center);
    put_b(0x24, s.pad.crouch_lock);
    put_b(0x26, s.pad.clench_protection);
    put_b(0x27, s.game.female_voice);
    *p.add(0x1b5) = s.pad.look_sens.min(18);
    *p.add(0x1b6) = s.pad.look_sens.min(18);
    put_f(0x1b8, s.pad.axial_dead_zone.clamp(0.0, 0.2));
    put_f(0x1bc, s.pad.radial_dead_zone.clamp(0.0, 0.2));
    *p.add(0x1c8) = s.pad.button_preset.min(13);
    *p.add(0x1c9) = s.pad.stick_preset.min(3);
    put_b(0x1d7, s.pad.swap_triggers);
    put_f(0xa5c, s.audio.master.clamp(0.0, 1.0));
    put_f(0xa60, s.audio.music.clamp(0.0, 1.0));
    put_f(0xa64, s.audio.sfx.clamp(0.0, 1.0));
    crate::input::write_profile_bindings(p, &s.mouse);
    write_armour(p, &s.armour);
}

/// Appearance fields (MCC customisation item ids; notes/customization.md). `ApplyProfileLookSettingsToPlayer` converts
/// each id to the engine's own index (`ConvertHostCustomizationItemToEngineIndex` 0x18003b2f4) into the local player's
/// settings record, and the engine's player data carries it to the other machines.
pub unsafe fn write_armour(p: *mut u8, a: &crate::ui::settings::Armour) {
    let put = |off: usize, v: u32| (p.add(off) as *mut u32).write_unaligned(v);
    *p.add(0x38) = a.elite as u8;
    put(0x40, a.helmet);
    put(0x44, a.left_shoulder);
    put(0x48, a.right_shoulder);
    put(0x4c, a.chest);
    put(0x50, a.wrist);
    put(0x54, a.utility);
    put(0x64, a.body);
    put(0x68, a.effect);
    put(0x6c, a.knees);
    put(0x70, a.visor);
    put(0x8c, a.elite_armour);
    put(0x90, a.elite_effect);
    put(0x94, a.voice);
    put(0x98, a.primary);
    put(0x9c, a.secondary);
    // MCC also keeps the palette position at +0x2C / +0x30; Reach reads only +0x98 / +0x9C.
    put(0x2c, a.primary.wrapping_sub(0xca));
    put(0x30, a.secondary.wrapping_sub(0xca));
    static LOGGED: std::sync::Mutex<Option<crate::ui::settings::Armour>> = std::sync::Mutex::new(None);
    let mut last = LOGGED.lock().unwrap_or_else(|e| e.into_inner());
    if last.as_ref() != Some(a) {
        crate::log!(
            "armour: {} helmet {:#x} shoulders {:#x}/{:#x} chest {:#x} colours {:#x}/{:#x} (profile +0x38..+0x9C)",
            if a.elite { "elite" } else { "spartan" },
            a.helmet,
            a.left_shoulder,
            a.right_shoulder,
            a.chest,
            a.primary,
            a.secondary
        );
        *last = Some(a.clone());
    }
}

/// Forces the engine to rebuild player 0's key bindings on the next settings sync. `RebuildPlayerInputBindings`
/// only runs when one of the fields `ApplyProfileLookSettingsToPlayer` compares has changed (flag 0x800); key codes
/// are not among them, so flip theater thrust (+0x1D2, used only in Theater) between 0 and 1.
pub unsafe fn nudge_binding_rebuild(p: *mut u8) {
    *p.add(0x1d2) ^= 1;
}
