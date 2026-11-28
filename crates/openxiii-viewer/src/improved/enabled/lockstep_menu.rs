impl crate::App {
    fn set_lockstep_notice(&mut self, why: &str) {
        self.viewer.lockstep_notice = Some(lockstep_reason(why));
    }

    /// Once a frame: a reason waiting to be told (`lockstep_fail`), shown once the menus allow it.
    pub(crate) fn lockstep_notice_frame(&mut self) {
        let Some(reason) = self.viewer.lockstep_notice else { return };
        if let Some(g) = self.game.as_mut() {
            if g.vm.lockstep_show_notice(reason) {
                self.viewer.lockstep_notice = None;
            }
        }
    }

    /// See above (improved build: the added texts).
    pub(crate) fn lockstep_sign(&mut self, w: f32, h: f32) -> Vec<openxiii_vm::canvas::HudDraw> {
        use openxiii_vm::added_text::{text, Text};
        let meeting = self.lockstep_meeting.as_ref().map(|m| (m.guest && !m.meeting.met(), m.meeting.progress()));
        let waiting = self.lockstep.as_ref().is_some_and(|r| r.waiting > 0.25);
        // A difference between the two games is a tool for us, not a message for the players.
        let differ = crate::lockstep_play::differ_shown() && self.lockstep.as_ref().is_some_and(|r| r.reported);
        let Some(g) = self.game.as_mut() else { return Vec::new() };
        let lang = g.vm.text_language().to_string();
        let line = match meeting {
            Some((true, _)) => text(&lang, Text::LockstepJoining).to_string(),
            Some((false, (got, total))) if total > 0 => format!("{} {}%", text(&lang, Text::LockstepWaiting), got * 100 / total),
            Some(_) => text(&lang, Text::LockstepWaiting).to_string(),
            None if waiting => text(&lang, Text::LockstepWaiting).to_string(),
            None if differ => text(&lang, Text::LockstepDiffer).to_string(),
            None => return Vec::new(),
        };
        // Dots that move: the game is not frozen.
        let dots = (self.started.elapsed().as_millis() / 300 % 4) as usize;
        let line = format!("{}{}", line.trim_end_matches('.'), ".".repeat(dots));
        let scale = (h / 540.0).max(1.0);
        sign_box(g, &line, w, h * 0.08, scale)
    }

    /// The gamepad buttons down now (skipping the New game video of a met session).
    pub(crate) fn lockstep_pad_buttons(&mut self) -> u32 {
        self.recap_pad_buttons()
    }

    fn note_departing_player(&mut self, by: Option<usize>, local: Option<usize>) {
        if by.is_some() && local.is_some() && by != local {
            self.viewer.lockstep_notice = Some(openxiii_vm::added_text::Text::LockstepOtherLeft);
        }
    }
}

/// The waiting sign: the text on a dark box, centred across `w`, from height `y0`.
fn sign_box(g: &mut game::Game, line: &str, w: f32, y0: f32, scale: f32) -> Vec<openxiii_vm::canvas::HudDraw> {
    let (mut glyphs, tw, th) = g.vm.free_text("XIIIFonts.PoliceF16", 0.0, 0.0, line, [255, 255, 255, 255]);
    let (bw, bh) = (tw * scale + 24.0, th * scale + 16.0);
    let x0 = (w - bw) * 0.5;
    let mut out = vec![openxiii_vm::canvas::HudDraw::Tile {
        x1: x0,
        y1: y0,
        x2: x0 + bw,
        y2: y0 + bh,
        uv: [0.0, 0.0, 1.0, 1.0],
        material: std::sync::Arc::from("Engine.WhiteSquareTexture"),
        color: [0, 0, 0, 170],
        point: false,
        enlarged: false,
        glyph: false,
    }];
    for d in glyphs.iter_mut() {
        if let openxiii_vm::canvas::HudDraw::Tile { x1, y1, x2, y2, enlarged, .. } = d {
            *x1 = x0 + 12.0 + *x1 * scale;
            *x2 = x0 + 12.0 + *x2 * scale;
            *y1 = y0 + 8.0 + *y1 * scale;
            *y2 = y0 + 8.0 + *y2 * scale;
            *enlarged = scale > 1.0;
        }
    }
    out.extend(glyphs);
    out
}

/// The on-screen reason for the session's own reasons (Meeting, lockstep_travel, lockstep_steps).
fn lockstep_reason(why: &str) -> openxiii_vm::added_text::Text {
    use openxiii_vm::added_text::Text;
    let w = why.to_ascii_lowercase();
    if w.contains("no guest came") {
        Text::LockstepNoGuest
    } else if w.contains("does not answer") {
        Text::LockstepNoHost
    } else if w.contains("other game version") {
        Text::LockstepOtherVersion
    } else if w.contains("other game files") {
        Text::LockstepOtherFiles
    } else if w.contains("map") {
        Text::LockstepOtherMap
    } else if w.contains("language") {
        Text::LockstepLanguage
    } else if w.contains("address is not valid") {
        Text::LockstepBadAddress
    } else if w.starts_with("port ") || w.contains("10048") || w.contains("in use") {
        Text::LockstepPortBusy
    } else if w.contains("other player left") {
        Text::LockstepOtherLeft
    } else if w.contains("went apart") {
        Text::LockstepApart
    } else if w.contains("stopped answering") {
        Text::LockstepLinkLost
    } else {
        Text::LockstepFailed
    }
}

fn character_selection_travel(url: &mut openxiii_vm::travel::Url) -> bool {
    if url.has_option(openxiii_vm::character_screen::SCREEN_OPTION) {
        url.options.retain(|o| {
            let key = o.split('=').next().unwrap_or("");
            !key.eq_ignore_ascii_case("Listen") && !key.eq_ignore_ascii_case("Port")
        });
        return true;
    }

    false
}

fn local_graphics_profile_value() -> &'static str {
    if openxiii_vm::video_settings::graphics_enhanced_local() {
        "1"
    } else {
        "0"
    }
}

/// A co-op host waiting for its guest is listed on the Join page of the other machines
/// (lockstep_find.rs): its player's co-op name and its map.
fn list_coop_host(meeting: &mut Meeting, vm: &mut Vm, map: &str) {
    meeting.list_as(&vm.coop_chat_name(), map);
}

/// This build and the checksum of its game files and mods, as a host's meeting compares them
/// (the Join page's list of the games found on the network).
pub(crate) fn own_game(game: &std::path::Path) -> (String, u64) {
    (BUILD_ID.to_string(), data_checksum(game, &mods_in_use()))
}
