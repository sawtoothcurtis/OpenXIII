//! Improved build only: the original menu sounds for the pages OpenXIII adds that are not made of
//! the game's own buttons (the Mods page, drawn by the game window).
//!
//! The game's menus play them through the player's own PlayMenu:
//! - focus moved: XIIIGUIBaseButton.MouseEnter, `myRoot.GetPlayerOwner().PlayMenu(hMenuCurseur)`
//!   (Sound'XIIIsound.Interface.MnCurseur');
//! - Enter on a page whose help bar shows a validating action: XIIIWindow.InternalOnKeyEvent,
//!   `GetPlayerOwner().PlayMenu(hMenuOn)` (MnValid);
//! - Backspace on a page whose help bar shows Back or Cancel: the same function, `hMenuOff`
//!   (MnAnnul). Escape plays nothing in the original.
//!
//! [`Vm::play_menu_sound`] does the same: the local GUI controller's player owner plays the same
//! sound object, so the volume and the channel are the game's.

#[allow(unused_imports)]
use openxiii_math::SimMath as _;
use crate::value::Value;
use crate::Vm;

/// A sound of the game's menus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuSound {
    /// The focus moved to another element (MnCurseur).
    Tick,
    /// An element validated (MnValid).
    Enter,
    /// Back or Cancel with Backspace (MnAnnul).
    Back,
}

impl MenuSound {
    /// The sound object's path, as the menus' default properties name it.
    pub fn path(self) -> &'static str {
        match self {
            MenuSound::Tick => "XIIIsound.Interface.MnCurseur",
            MenuSound::Enter => "XIIIsound.Interface.MnValid",
            MenuSound::Back => "XIIIsound.Interface.MnAnnul",
        }
    }
}

impl Vm {
    /// Plays `sound` as the game's menus do (the GUI controller's player owner, PlayMenu).
    /// False without a menu or a player.
    pub fn play_menu_sound(&mut self, sound: MenuSound) -> bool {
        let r = self.play_menu_sound_inner(sound);
        r.is_ok()
    }

    fn play_menu_sound_inner(&mut self, sound: MenuSound) -> std::result::Result<(), &'static str> {
        let root = self.gui_local().ok_or("no GUI controller")?;
        let owner = self.call_named(root, "GetPlayerOwner", vec![]).ok().and_then(|v| v.as_object()).ok_or("no player owner")?;
        // The menus' sound properties hold the sound as an asset path (Sound'XIIIsound.Interface.
        // MnCurseur'), which execPlayMenu resolves (natives.rs): the same value here.
        self.call_named(owner, "PlayMenu", vec![Value::Asset(sound.path().to_string())]).map(|_| ()).map_err(|_| "PlayMenu failed")
    }
}

/// The main menu's arrival hit alone: MainTr, the wave AmbianceMenu__hMainPage starts with the
/// main menu's loop (MainLoop) at gain 0.6. Closing an added page drawn over the main menu (Mods,
/// Co-op) plays it instead of hMainPage: the main menu's loop never stopped under those pages
/// (the Co-op page, an XIIIWindowMainMenu, has no sound of its own; the Mods page is not a GUI
/// page), and hMainPage would start it again from its beginning.
pub(crate) const MAIN_ARRIVAL: &str = "OpenXIII.MenuWave.MainTr";
/// The wave hMainPage plays with the loop, in the MapMenu bank.
const MAIN_ARRIVAL_WAVE: &str = "MainTr";
/// The resource made for it (a copy of MainTr's, at hMainPage's gain).
const MAIN_ARRIVAL_ID: openxiii_assets::hx::Cuuid = openxiii_assets::hx::Cuuid(0x0E13_A001, 0x4D61_5472);
/// hMainPage's gain for MainTr.
pub(crate) const MAIN_ARRIVAL_GAIN: f32 = 0.6;

/// The HX volume (in the engine's units: gain = e^(0.15 × volume)) that gives `gain`.
pub(crate) fn volume_for_gain(gain: f32) -> f32 {
    gain.dln() / 0.15
}

/// A PlayMenu of hMainPage while an added page closes back to the main menu: the arrival hit alone.
pub(crate) fn is_main_page(sound: &str) -> bool {
    sound.rsplit('.').next().is_some_and(|n| n.eq_ignore_ascii_case("AmbianceMenu__hMainPage"))
}

impl Vm {
    /// audio_sound_ref of MAIN_ARRIVAL: MainTr's resource copied at hMainPage's gain (once per
    /// loaded bank set). None when the path is another one (Some(None): no such wave loaded).
    pub(crate) fn menu_wave_ref(&mut self, path: &str) -> Option<Option<openxiii_hx::subsystem::SoundRef>> {
        if path != MAIN_ARRIVAL {
            return None;
        }
        Some((|| {
            use openxiii_hx::engine::ResData;
            let hx = self.audio.as_mut()?;
            if !hx.engine.registry.contains_key(&MAIN_ARRIVAL_ID) {
                let mut w = hx.engine.registry.values().find_map(|r| match r {
                    ResData::Wav(w) if w.name.eq_ignore_ascii_case(MAIN_ARRIVAL_WAVE) => Some((**w).clone()),
                    _ => None,
                })?;
                w.cuuid = MAIN_ARRIVAL_ID;
                w.volume += volume_for_gain(MAIN_ARRIVAL_GAIN);
                hx.engine.registry.insert(MAIN_ARRIVAL_ID, ResData::Wav(std::rc::Rc::new(w)));
            }
            Some(openxiii_hx::subsystem::SoundRef { path: path.to_string(), name: MAIN_ARRIVAL_WAVE.to_string(), cuuid: MAIN_ARRIVAL_ID, handler: false })
        })())
    }

    /// Runs `f` (an added page closing back to the main menu) with the main menu's hMainPage
    /// replaced by its arrival hit alone.
    pub(crate) fn with_main_arrival_only<R>(&mut self, f: impl FnOnce(&mut Vm) -> R) -> R {
        let before = std::mem::replace(&mut self.improved.menu_boxes.arrival_only, true);
        let r = f(self);
        self.improved.menu_boxes.arrival_only = before;
        r
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arrival_volume_gives_the_programs_gain() {
        let v = volume_for_gain(MAIN_ARRIVAL_GAIN);
        assert!(((v * 0.15).dexp() - 0.6).abs() < 1e-5, "{v}");
        // Negative control: full gain is volume 0.
        assert!(volume_for_gain(1.0).abs() < 1e-6);
        assert!(is_main_page("XIIIsound.Interface__AmbianceMenu.AmbianceMenu__hMainPage"));
        assert!(!is_main_page("XIIIsound.Interface__AmbianceMenu.AmbianceMenu__hOption"));
        assert!(!is_main_page("XIIIsound.Interface.MnCurseur"));
    }

    /// The arrival hit is MainTr's own resource at the program's gain, made once; another path is
    /// left to the sound objects.
    #[test]
    fn arrival_wave_is_main_tr_at_the_programs_gain() {
        let game = match std::env::var("OXIII_GAME") {
            Ok(g) => std::path::PathBuf::from(g),
            Err(_) => panic!("OXIII_GAME must point to the game; a skipped test does not count"),
        };
        let mut vm = Vm::new();
        let mut hx = openxiii_hx::subsystem::HxAudio::new(openxiii_hx::Engine::new(), openxiii_hx::subsystem::Config::default());
        hx.engine.load_banks(&game, &["MapMenu", "Fix"]);
        vm.audio = Some(hx);
        assert!(vm.menu_wave_ref("XIIIsound.Interface.MnCurseur").is_none());
        let r = vm.menu_wave_ref(MAIN_ARRIVAL).expect("handled").expect("MainTr loaded");
        assert!(!r.handler);
        let reg = &vm.audio.as_ref().unwrap().engine.registry;
        let wav = |id| match reg.get(&id) {
            Some(openxiii_hx::engine::ResData::Wav(w)) => (**w).clone(),
            _ => panic!("no wave"),
        };
        let made = wav(r.cuuid);
        let orig = reg.values().find_map(|x| match x { openxiii_hx::engine::ResData::Wav(w) if w.name == "MainTr" && w.cuuid != r.cuuid => Some((**w).clone()), _ => None }).expect("MainTr");
        assert_eq!(made.wave, orig.wave);
        assert!(((made.volume - orig.volume) - volume_for_gain(0.6)).abs() < 1e-5);
        // Negative control: without the MapMenu bank there is no MainTr to play.
        let mut vm = Vm::new();
        let mut hx = openxiii_hx::subsystem::HxAudio::new(openxiii_hx::Engine::new(), openxiii_hx::subsystem::Config::default());
        hx.engine.load_banks(&game, &["Fix"]);
        vm.audio = Some(hx);
        assert!(vm.menu_wave_ref(MAIN_ARRIVAL).expect("handled").is_none());
    }

    /// hMainPage played when an added page closes: its arrival hit alone with Graphics: Enhanced,
    /// the whole sound with Graphics: Original (the original's). Negative control: the same
    /// PlayMenu without arrival_only plays hMainPage with either setting.
    #[test]
    fn arrival_hit_alone_only_with_graphics_enhanced() {
        let Some(game) = openxiii_package::env::var_os("OXIII_GAME") else {
            eprintln!("Set OXIII_GAME to run the menu sound test");
            return;
        };
        let system = openxiii_package::system_dir(std::path::Path::new(&game));
        if !system.is_dir() {
            eprintln!("Missing game scripts: {}", system.display());
            return;
        }
        let mut vm = Vm::new();
        vm.load_dir(&system).expect("load game scripts");
        let actor = vm.class("Actor").expect("Actor class");
        let this = vm.spawn(actor);
        let main = "XIIIsound.Interface__AmbianceMenu.AmbianceMenu__hMainPage";
        let mut played = Vec::new();
        for (enhanced, arrival_only) in [(true, true), (false, true), (true, false)] {
            crate::video_settings::set_graphics_enhanced(enhanced);
            vm.improved.menu_boxes.arrival_only = arrival_only;
            vm.sounds.clear();
            let mut args = vec![crate::value::Value::Asset(main.into())];
            crate::natives::call(&mut vm, "PlayMenu", this, &mut args, &[true]).unwrap();
            played.push(match vm.sounds.last() {
                Some(crate::canvas::AudioCall::Play { sound, .. }) => sound.clone(),
                other => format!("{other:?}"),
            });
        }
        assert_eq!(played, [MAIN_ARRIVAL, main, main]);
    }
}
