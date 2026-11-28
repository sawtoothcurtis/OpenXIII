//! Display settings added by OpenXIII (not in the original game), kept in [OpenXIII.Video] of
//! XIII.ini ("System" configuration, next to the game's own [WinDrv.WindowsClient] sizes):
//!
//! - `FullscreenMode` = Windowed / Borderless / Exclusive. Absent: the original behaviour
//!   (StartupFullscreen decides, fullscreen is exclusive). The menu writes it together with
//!   StartupFullscreen, so the game's own key stays consistent.
//! - `RenderScale` = whole percentage, 25 to 400, default 100.
//! - `MSAA` = samples, 1 (none), 2, 4 or 8, default 1.
//! - `AnisotropicFiltering` = 1, 2, 4, 8 or 16, default 1 (none, as the original) in the faithful
//!   build; 16 (4 on Android) in the improved build.
//! - `UIScale` = whole percentage, 75 to 150, default 100: size of the improved build's menus
//!   (gui.rs, MenuFrame::of_window), a factor of the size that keeps the menus the share of the
//!   screen they have on the original's 1280-wide modes (1280 x 720, 800, 960, 1024).
//!
//! - `Graphics` = Original (when absent or unknown) or Enhanced: the Video menu's Graphics line.
//!   A first launch writes Enhanced ([`apply_first_launch`]); a profile without the key stays
//!   Original (see [`graphics_of_profile`]). Original turns off every visual improvement at once (see
//!   [`graphics_enhanced`]); the values of the other keys are kept, and apply again with Enhanced.
//!
//! - V-sync has two modes ([`VsyncMode`]): Off and Classic (On: each image waits for the
//!   screen's refresh, the original's).
//!   Original graphics use the game's `UseVSync` in [D3DDrv.D3DRenderDevice] (True: Classic,
//!   False: Off), default True. Enhanced graphics use `VSync` in [OpenXIII.Video]: Off or On
//!   (Classic), default [`VSYNC_DEFAULT`] (Classic). The Video menu keeps the two preferences
//!   independently. OXIII_VSYNC overrides the local display ([`vsync_mode`]).
//!   Ini files written while a third mode (Triple) existed are read as On: `VSync=Triple` is
//!   Classic, and the old `OriginalGraphicsVSync` key is ignored (it was only written with
//!   UseVSync=True); neither is written any more.
//!
//! The resolution stays in the original keys (FullscreenViewportX/Y, WindowedViewportX/Y). With
//! every key absent, the display is exactly the original's.
//!
//! A value written by hand that is not one of these (not a number, out of range, a sample count
//! or anisotropy level not in the list) is read as the key's default, by the menu and by the
//! renderer alike ([`VideoSettings::from_texts`] is the only parser).
//!
//! The lists offered by the menu come from [`render_scale_choices`], [`msaa_choices`] and
//! [`anisotropy_choices`], limited to what the device accepts ([`VideoCaps`], supplied by the
//! renderer).

use crate::{improved::graphics, Vm};

/// Section and configuration file of the keys.
pub const SECTION: &str = "OpenXIII.Video";
pub const FILE: &str = "System";
pub const KEY_FULLSCREEN_MODE: &str = "FullscreenMode";
pub const KEY_RENDER_SCALE: &str = "RenderScale";
pub const KEY_MSAA: &str = "MSAA";
pub const KEY_ANISOTROPY: &str = "AnisotropicFiltering";
pub const KEY_UI_SCALE: &str = "UIScale";
pub const KEY_GRAPHICS: &str = "Graphics";
/// V-sync of the improved build (in [`SECTION`]).
pub const KEY_VSYNC: &str = "VSync";
/// V-sync of the faithful build: the original's key and section (same configuration file).
pub const ORIGINAL_VSYNC_SECTION: &str = "D3DDrv.D3DRenderDevice";
pub const KEY_ORIGINAL_VSYNC: &str = "UseVSync";

/// How the display waits for the screen's refresh.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum VsyncMode {
    /// No wait: images go to the screen as soon as they are drawn (may tear).
    Off,
    /// Each image waits for the screen's refresh (Fifo presentation): the original's V-sync.
    Classic,
}

impl VsyncMode {
    /// The Video menu's order (the combo box's indices): Off, On (Classic).
    pub const ALL: [VsyncMode; 2] = [VsyncMode::Off, VsyncMode::Classic];

    /// Index in [`VsyncMode::ALL`].
    pub const fn index(self) -> usize {
        match self {
            VsyncMode::Off => 0,
            VsyncMode::Classic => 1,
        }
    }

    /// Mode at index `i` of [`VsyncMode::ALL`] (out of range: the nearest end).
    pub fn from_index(i: i32) -> Self {
        Self::ALL[i.clamp(0, Self::ALL.len() as i32 - 1) as usize]
    }
}

/// V-sync when the Enhanced key is absent: Classic in both builds (a single constant per build,
/// improved/graphics.rs and faithful/graphics.rs; the original's default).
pub const VSYNC_DEFAULT: VsyncMode = graphics::VSYNC_DEFAULT;

/// V-sync when the key is absent, for a Graphics profile: Enhanced, [`VSYNC_DEFAULT`]; Original,
/// Classic (the original's UseVSync=True).
pub fn vsync_profile_default(enhanced: bool) -> VsyncMode {
    if enhanced { VSYNC_DEFAULT } else { VsyncMode::Classic }
}

/// V-sync when the key is absent, for this machine's Graphics setting.
pub fn vsync_default() -> VsyncMode {
    vsync_profile_default(graphics_enhanced_local())
}

/// V-sync as last read from the ini or chosen in the Video menu (process-wide, read by the
/// renderer before each frame): an index of [`VsyncMode::ALL`]. The default until the ini is read.
static VSYNC: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(VSYNC_DEFAULT.index() as u8);

fn store_vsync(mode: VsyncMode) {
    VSYNC.store(mode.index() as u8, std::sync::atomic::Ordering::Relaxed);
}

/// OXIII_VSYNC read: 0, Off; anything else (the old triple, mailbox or 3 included), Classic.
pub fn vsync_override_from_text(v: &str) -> VsyncMode {
    match v.trim().to_ascii_lowercase().as_str() {
        "0" => VsyncMode::Off,
        _ => VsyncMode::Classic,
    }
}

/// OXIII_VSYNC when set (read once).
fn vsync_override() -> Option<VsyncMode> {
    static OVERRIDE: std::sync::OnceLock<Option<VsyncMode>> = std::sync::OnceLock::new();
    *OVERRIDE.get_or_init(|| openxiii_package::env::var("OXIII_VSYNC").ok().map(|v| vsync_override_from_text(&v)))
}

/// How the display waits for the screen's refresh: OXIII_VSYNC when set, otherwise the ini's key.
pub fn vsync_mode() -> VsyncMode {
    vsync_override().unwrap_or_else(|| VsyncMode::from_index(VSYNC.load(std::sync::atomic::Ordering::Relaxed) as i32))
}

/// A V-sync key read as the engine reads a bool: True, 1, Yes or On is on, False, 0, No or Off
/// is off (any case); absent or anything else, `default`.
pub fn vsync_from_text(s: Option<&str>, default: bool) -> bool {
    match s.map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        Some("true" | "1" | "yes" | "on") => true,
        Some("false" | "0" | "no" | "off") => false,
        _ => default,
    }
}

/// The improved build's VSync key read: the bool words of [`vsync_from_text`] are Classic (On,
/// True, 1, Yes) or Off; Triple (or Mailbox, 3), written by older versions, is Classic; absent or
/// anything else, `default`.
pub fn vsync_mode_from_text(s: Option<&str>, default: VsyncMode) -> VsyncMode {
    match s.map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        Some("true" | "1" | "yes" | "on" | "triple" | "mailbox" | "3") => VsyncMode::Classic,
        Some("false" | "0" | "no" | "off") => VsyncMode::Off,
        _ => default,
    }
}

/// Ini text of the improved build's VSync key.
pub fn vsync_text(mode: VsyncMode) -> &'static str {
    match mode {
        VsyncMode::Off => "Off",
        VsyncMode::Classic => "On",
    }
}

/// First launch of the improved build's game (the player's folder has no XIII.ini yet), set by
/// the viewer before the game loads, with the screen's native size when it is known by then
/// (Windows); otherwise (Linux, Android) the viewer gives it once the window is open
/// ([`take_first_launch_size_pending`], [`write_first_launch_size`]).
static FIRST_LAUNCH: std::sync::OnceLock<Option<(u32, u32)>> = std::sync::OnceLock::new();
/// First launch whose native size is still to come from the window.
static FIRST_LAUNCH_SIZE_PENDING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The game a player starts has no XIII.ini of its own yet: its first configuration will be the
/// original game's display defaults, in fullscreen at the screen's native size `native` (None:
/// not known before the window, given later), with Graphics = Enhanced.
pub fn set_first_launch(native: Option<(u32, u32)>) {
    let native = native.filter(|&(w, h)| w > 0 && h > 0);
    if FIRST_LAUNCH.set(native).is_ok() && native.is_none() {
        FIRST_LAUNCH_SIZE_PENDING.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

/// First launch whose native size was not known at load: true once (the viewer then opens the
/// fullscreen at the screen's native size and writes it with [`write_first_launch_size`]).
pub fn take_first_launch_size_pending() -> bool {
    FIRST_LAUNCH_SIZE_PENDING.swap(false, std::sync::atomic::Ordering::Relaxed)
}

/// The fullscreen size of a first launch, known once the window is open (Linux, Android).
pub fn write_first_launch_size(c: &mut crate::config::ConfigCache, w: u32, h: u32) {
    if w > 0 && h > 0 {
        let client = "WinDrv.WindowsClient";
        c.set_string(client, "FullscreenViewportX", &w.to_string(), FILE);
        c.set_string(client, "FullscreenViewportY", &h.to_string(), FILE);
    }
}

/// Display sections of the game's ini taken from the original's defaults (Default.ini) on a
/// first launch.
const FIRST_LAUNCH_SECTIONS: [&str; 2] = ["WinDrv.WindowsClient", "D3DDrv.D3DRenderDevice"];

/// First launch (see [`set_first_launch`]): the display settings are the original game's
/// defaults (its Default.ini, whatever the installation's XIII.ini holds), with two changes:
/// fullscreen at the screen's native size, and Graphics = Enhanced. The other OpenXIII keys stay
/// absent (their defaults). In memory, written into the player's folder at the next flush. False
/// when this is not a first launch.
pub fn apply_first_launch(c: &mut crate::config::ConfigCache) -> bool {
    let Some(&native) = FIRST_LAUNCH.get() else { return false };
    apply_first_launch_size(c, native);
    true
}

fn apply_first_launch_size(c: &mut crate::config::ConfigCache, native: Option<(u32, u32)>) {
    for section in FIRST_LAUNCH_SECTIONS {
        if !c.section_pairs(section, "Default").is_empty() {
            c.reset_section(section, FILE, "Default");
        }
    }
    if let Some((w, h)) = native {
        write_first_launch_size(c, w, h);
    }
    c.set_string("WinDrv.WindowsClient", "StartupFullscreen", "True", FILE);
    c.set_string(SECTION, KEY_GRAPHICS, graphics_text(true), FILE);
}

/// Graphics = Enhanced, as last read from the ini or chosen in the Video menu (process-wide, so
/// that the renderer and the free functions of the VM see it without a Vm).
pub(crate) static ENHANCED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Are the visual improvements on (Video menu, Graphics: Enhanced)? Never in the faithful build.
/// Original (the default) gives the original game's look in the improved build: the menu offers
/// only the original's resolutions, no wide-screen display (display_format.rs: view, menus' frame
/// and size, HUD scale), render scale 100 %, no MSAA, no anisotropic filtering, the original's
/// sizes of the shadow targets, comic panels and screen-effect texture, no fix of the effects
/// that disappear with the view angle (effects_angle.rs), the help bar in the original's place.
/// Every other improvement that changes the look must ask this too.
pub fn graphics_enhanced() -> bool {
    SessionGraphics::default().enhanced()
}

/// This machine's own Graphics setting, whatever a session's: the renderer's quality (render
/// scale, MSAA, the targets' sizes), which is a video setting and never travels.
pub fn graphics_enhanced_local() -> bool {
    graphics::local()
}

/// The host's graphics preference for the simulation, including menus between steps.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SessionGraphics {
    pub(crate) host: Option<bool>,
    pub(crate) in_step: bool,
}

impl SessionGraphics {
    pub fn enhanced(self) -> bool {
        graphics::effective(self)
    }
}

/// Sets what [`graphics_enhanced`] returns (Vm::refresh_graphics, the Video menu, tests).
pub fn set_graphics_enhanced(on: bool) {
    ENHANCED.store(on, std::sync::atomic::Ordering::Relaxed);
}

pub(crate) fn graphics_override() -> Option<bool> {
    None
}

/// Ini text of the Graphics key.
pub fn graphics_text(enhanced: bool) -> &'static str {
    if enhanced { "Enhanced" } else { "Original" }
}

/// Graphics key read: Enhanced only when it says so (any case); absent or anything else, Original.
pub fn graphics_from_text(s: Option<&str>) -> bool {
    s.is_some_and(|s| s.trim().eq_ignore_ascii_case("Enhanced"))
}

/// The keys only the Video page of the improved build writes (always with Graphics since that
/// line exists): one of them without a Graphics key is a profile made before the line.
pub const ENHANCED_KEYS: [&str; 7] = [KEY_FULLSCREEN_MODE, KEY_RENDER_SCALE, KEY_MSAA, KEY_ANISOTROPY, KEY_UI_SCALE, KEY_SHADOW_QUALITY, KEY_CHARACTER_EDGES];

/// Graphics of a profile: the key when it is there (any value, explicit choice); absent, Enhanced
/// for a profile made before the Graphics line (`older_video_keys`: one of [`ENHANCED_KEYS`] is
/// set, its player saw the improvements), Original for a new one.
pub fn graphics_of_profile(graphics: Option<&str>, older_video_keys: bool) -> bool {
    match graphics {
        Some(_) => graphics_from_text(graphics),
        None => older_video_keys,
    }
}

/// The original game's resolutions (the 4:3 list GetAvailableRes gives) when `list` holds the
/// monitor's modes: those of `list` whose width is 4/3 of their height.
pub fn original_resolutions(list: &[(u32, u32)]) -> Vec<(u32, u32)> {
    list.iter().copied().filter(|&(w, h)| w * 3 == h * 4).collect()
}

/// The entry of `list` to select for a window of `current` size: the same size, otherwise the
/// one of the nearest height (the smaller on a tie); None for an empty list.
pub fn nearest_resolution(list: &[(u32, u32)], current: (u32, u32)) -> Option<usize> {
    if let Some(i) = list.iter().position(|&r| r == current) {
        return Some(i);
    }
    list.iter().enumerate().min_by_key(|(_, &(_, h))| (h.abs_diff(current.1), h)).map(|(i, _)| i)
}

/// Bounds of RenderScale (percent).
pub const RENDER_SCALE_MIN: u32 = 25;
pub const RENDER_SCALE_MAX: u32 = 400;
pub const RENDER_SCALE_DEFAULT: u32 = 100;
pub const MSAA_DEFAULT: u32 = 1;
/// Bounds and default of UIScale (percent).
pub const UI_SCALE_MIN: u32 = 75;
pub const UI_SCALE_MAX: u32 = 150;
pub const UI_SCALE_DEFAULT: u32 = 100;
/// Default anisotropic filtering: none (1) in the faithful build, whose textures are filtered as
/// the original's; 16 on PC and 4 on Android in the improved build. A value set in the menu or the
/// ini is kept in both.
pub const ANISOTROPY_DEFAULT: u32 = graphics::ANISOTROPY_DEFAULT;

/// How fullscreen is shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FullscreenMode {
    /// A window (StartupFullscreen=False).
    Windowed,
    /// A borderless window covering the monitor at the desktop's resolution.
    Borderless,
    /// The monitor switched to the chosen video mode (the original game's fullscreen).
    Exclusive,
}

impl FullscreenMode {
    /// The Video page's Display mode line, in its order: Fullscreen, Borderless, Window (the ini
    /// keeps each mode's name, whatever this order).
    pub const ALL: [FullscreenMode; 3] = [FullscreenMode::Exclusive, FullscreenMode::Borderless, FullscreenMode::Windowed];

    pub fn as_str(self) -> &'static str {
        match self {
            FullscreenMode::Windowed => "Windowed",
            FullscreenMode::Borderless => "Borderless",
            FullscreenMode::Exclusive => "Exclusive",
        }
    }

    /// Ini text (any case); None if unknown.
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.as_str().eq_ignore_ascii_case(s.trim()))
    }

    pub fn is_fullscreen(self) -> bool {
        self != FullscreenMode::Windowed
    }
}

/// The values of [OpenXIII.Video], checked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VideoSettings {
    /// None when the key is absent (original behaviour).
    pub fullscreen_mode: Option<FullscreenMode>,
    pub render_scale: u32,
    pub msaa: u32,
    pub anisotropy: u32,
    /// Menus' size, percent (UIScale).
    pub ui_scale: u32,
}

impl Default for VideoSettings {
    fn default() -> Self {
        VideoSettings { fullscreen_mode: None, render_scale: RENDER_SCALE_DEFAULT, msaa: MSAA_DEFAULT, anisotropy: ANISOTROPY_DEFAULT, ui_scale: UI_SCALE_DEFAULT }
    }
}

/// What the display device accepts, supplied by the renderer (`Vm::video_caps`). The default
/// accepts everything (no renderer yet, tests).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VideoCaps {
    /// MSAA sample counts the device supports (1 included).
    pub msaa: Vec<u32>,
    /// Largest render scale (percent) for the current frame size.
    pub max_render_scale: u32,
    /// Largest anisotropy level (1 when the device has no anisotropic filtering).
    pub max_anisotropy: u32,
}

impl Default for VideoCaps {
    fn default() -> Self {
        VideoCaps { msaa: msaa_choices(), max_render_scale: RENDER_SCALE_MAX, max_anisotropy: 16 }
    }
}

impl VideoCaps {
    /// RenderScale values of the menu the device accepts (at least the smallest).
    pub fn render_scale_choices(&self) -> Vec<u32> {
        keep_some(render_scale_choices(), |v| v <= self.max_render_scale)
    }

    /// MSAA values of the menu the device accepts (at least 1).
    pub fn msaa_choices(&self) -> Vec<u32> {
        keep_some(msaa_choices(), |v| v == 1 || self.msaa.contains(&v))
    }

    /// Anisotropy values of the menu the device accepts (at least 1).
    pub fn anisotropy_choices(&self) -> Vec<u32> {
        keep_some(anisotropy_choices(), |v| v <= self.max_anisotropy.max(1))
    }
}

/// `all` filtered by `keep`, or its first value when nothing is left.
fn keep_some(all: Vec<u32>, keep: impl Fn(u32) -> bool) -> Vec<u32> {
    let first = all[0];
    let kept: Vec<u32> = all.into_iter().filter(|&v| keep(v)).collect();
    if kept.is_empty() { vec![first] } else { kept }
}

/// RenderScale values offered by the menu.
pub fn render_scale_choices() -> Vec<u32> {
    vec![25, 50, 75, 100, 125, 150, 200, 300, 400]
}

/// MSAA values offered by the menu (1 = none).
pub fn msaa_choices() -> Vec<u32> {
    vec![1, 2, 4, 8]
}

/// Anisotropic filtering values offered by the menu (1 = none).
pub fn anisotropy_choices() -> Vec<u32> {
    vec![1, 2, 4, 8, 16]
}

/// Key of the shadow quality: Auto (default, or anything else), Low, High or VeryHigh.
pub const KEY_SHADOW_QUALITY: &str = "ShadowQuality";

/// Shadow quality values offered by the menu, as the side of the shadow targets: 0 is Auto (the
/// side follows the height of the world as drawn), 256 the original's, then 1024 and 2048 (not
/// on Android).
pub fn shadow_quality_choices() -> Vec<u32> {
    if cfg!(target_os = "android") { vec![0, 256, 1024] } else { vec![0, 256, 1024, 2048] }
}

/// Ini text of a shadow quality value.
pub fn shadow_quality_text(v: u32) -> &'static str {
    match v {
        256 => "Low",
        1024 => "High",
        2048 => "VeryHigh",
        _ => "Auto",
    }
}

/// Shadow quality of an ini text (None: absent); anything unknown, or not offered here, is Auto.
pub fn shadow_quality_from_text(s: Option<&str>) -> u32 {
    let s = s.map_or("", str::trim);
    shadow_quality_choices().into_iter().find(|&v| shadow_quality_text(v).eq_ignore_ascii_case(s)).unwrap_or(0)
}

/// UIScale values offered by the menu: smaller or larger than the original's 1280-wide modes.
/// Above about 120 % on a 16:9 screen the menus' 800 x 600 page already fills the height and the
/// size is held there (MenuFrame::of_window).
pub fn ui_scale_choices() -> Vec<u32> {
    vec![75, 85, 100, 110, 120, 135, 150]
}

/// Key of the former Edges line of the Video page. The menus' characters always have their edge
/// smoothed with Graphics: Enhanced (menu_figures.rs), so the key is no longer read; it stays in
/// [`ENHANCED_KEYS`] because a profile that has it was made with the improvements on.
pub const KEY_CHARACTER_EDGES: &str = "CharacterEdges";

/// Index in `choices` of the value closest to `v` (the first on a tie).
pub fn closest_index(choices: &[u32], v: u32) -> usize {
    choices.iter().enumerate().min_by_key(|(_, &c)| c.abs_diff(v)).map_or(0, |(i, _)| i)
}

impl VideoSettings {
    /// From the ini texts of the keys (None: absent); invalid or out-of-range values fall back to
    /// the key's default.
    pub fn from_texts(mode: Option<&str>, scale: Option<&str>, msaa: Option<&str>, anisotropy: Option<&str>, ui_scale: Option<&str>) -> Self {
        let d = Self::default();
        let int = |s: Option<&str>| s.and_then(|s| s.trim().parse::<i64>().ok());
        VideoSettings {
            fullscreen_mode: mode.and_then(FullscreenMode::parse),
            render_scale: int(scale).filter(|v| (RENDER_SCALE_MIN as i64..=RENDER_SCALE_MAX as i64).contains(v)).map_or(d.render_scale, |v| v as u32),
            msaa: int(msaa).filter(|v| [1, 2, 4, 8].contains(v)).map_or(d.msaa, |v| v as u32),
            anisotropy: int(anisotropy).filter(|v| [1, 2, 4, 8, 16].contains(v)).map_or(d.anisotropy, |v| v as u32),
            ui_scale: int(ui_scale).filter(|v| (UI_SCALE_MIN as i64..=UI_SCALE_MAX as i64).contains(v)).map_or(d.ui_scale, |v| v as u32),
        }
    }
}

/// Resolutions offered by the improved build's Video menu: every video mode of the monitor
/// (24 or 32 bits, or of unknown depth), at least 640 x 480, without duplicates, from the
/// smallest. The original 4:3 resolutions are among them whenever the monitor has them.
pub fn screen_resolutions(modes: &[(u32, u32, u32)]) -> Vec<(u32, u32)> {
    let mut out: Vec<(u32, u32)> = modes.iter().filter(|&&(w, h, bits)| (bits == 0 || bits >= 24) && w >= 640 && h >= 480).map(|&(w, h, _)| (w, h)).collect();
    out.sort_unstable();
    out.dedup();
    out
}

/// `list` without the resolutions larger than the screen's native one (`native`, its preferred
/// mode) in width or height: a driver can list scaled modes above the panel's own size (2560 x 1440
/// on a 1920 x 1200 panel), which the screen shrinks, making every text soft. Unknown native size:
/// the list unchanged.
pub fn up_to_native(mut list: Vec<(u32, u32)>, native: Option<(u32, u32)>) -> Vec<(u32, u32)> {
    if let Some((nw, nh)) = native.filter(|&(w, h)| w > 0 && h > 0) {
        list.retain(|&(w, h)| w <= nw && h <= nh);
    }
    list
}

/// Android: the 4:3 modes a monitor lists (the ones the original's GetAvailableRes keeps).
const ANDROID_4_3: [(u32, u32); 6] = [(640, 480), (800, 600), (1024, 768), (1152, 864), (1280, 960), (1600, 1200)];
/// Android: heights of the modes in the screen's own shape.
const ANDROID_WIDE_HEIGHTS: [u32; 8] = [480, 540, 600, 720, 900, 1080, 1200, 1440];
/// Android: height of the activity's image before the game asks for its resolution (the
/// surface GameActivityXiii.java gives the window while the game loads), in the screen's shape.
/// Never what a first launch records ([`android_first_launch_size`]).
pub const ANDROID_DEFAULT_HEIGHT: u32 = 600;

/// Android, first launch (no XIII.ini of the player's yet): the resolution recorded and applied,
/// as on PC the screen's native size, the largest of the Video menu's list
/// ([`android_modes`]), not the activity's first 600-line image.
pub fn android_first_launch_size(screen: (u32, u32)) -> (u32, u32) {
    android_modes(screen).iter().map(|&(w, h, _)| (w, h)).max_by_key(|&(w, h)| (u64::from(w) * u64::from(h), h)).unwrap_or(screen)
}

/// Android: width of an image `h` lines high in the shape of the screen `screen` (landscape),
/// rounded as the activity rounds its first image size (GameActivityXiii.java).
pub fn android_wide_width(screen: (u32, u32), h: u32) -> u32 {
    (screen.0 as f32 * h as f32 / screen.1.max(1) as f32).round() as u32
}

/// Android has no display modes: the game image is the surface's size, which the screen scales
/// up. The modes offered are those a monitor would list for a screen of `screen` pixels
/// (landscape): the 4:3 standards and sizes in the screen's own shape, all up to the screen's
/// size, plus the screen's own size; 32 bits, from the smallest.
pub fn android_modes(screen: (u32, u32)) -> Vec<(u32, u32, u32)> {
    let (sw, sh) = screen;
    if sw == 0 || sh == 0 {
        return Vec::new();
    }
    let mut out: Vec<(u32, u32)> = ANDROID_4_3.iter().copied().filter(|&(w, h)| w <= sw && h <= sh).collect();
    out.extend(ANDROID_WIDE_HEIGHTS.iter().filter(|&&h| h <= sh).map(|&h| (android_wide_width(screen, h).min(sw), h)));
    out.push((sw, sh));
    out.sort_unstable();
    out.dedup();
    out.into_iter().map(|(w, h)| (w, h, 32)).collect()
}

/// Android: the surface size for a resolution `wanted` (the ini's, or the Video page's): the
/// same mode of `modes`, otherwise the nearest in height then in width (an ini written on
/// another phone). None without modes.
pub fn android_surface_for(modes: &[(u32, u32, u32)], wanted: (u32, u32)) -> Option<(u32, u32)> {
    modes
        .iter()
        .map(|&(w, h, _)| (w, h))
        .min_by_key(|&(w, h)| (h.abs_diff(wanted.1), w.abs_diff(wanted.0)))
}

/// Android: a finger at `p` (pixels of the full-screen touch view) on the game image, shown in
/// the rectangle `rect` (left, top, width, height) of that view and `image` pixels large: its
/// place on the image (outside it on the black bars beside it, at the same scale: a finger there
/// still turns the camera in game).
pub fn android_touch_to_image(rect: (f32, f32, f32, f32), image: (u32, u32), p: (f32, f32)) -> (f32, f32) {
    let (left, top, vw, vh) = rect;
    if vw <= 0.0 || vh <= 0.0 || image.0 == 0 || image.1 == 0 {
        return p;
    }
    let (iw, ih) = (image.0 as f32, image.1 as f32);
    ((p.0 - left) * iw / vw, (p.1 - top) * ih / vh)
}

/// Android: a finger point `p` mapped with [`android_touch_to_image`] for the image `old`
/// (its rectangle on the screen and its size), mapped again for the image `new`. The game image
/// changes size while a finger is down when the activity applies the ini's resolution just as
/// the first menu appears (the 600-line startup image, then the ini's): a finger kept in the old
/// image's pixels pointed elsewhere on the new image, and its tap clicked nothing.
pub fn android_touch_remap(old: ((f32, f32, f32, f32), (u32, u32)), new: ((f32, f32, f32, f32), (u32, u32)), p: (f32, f32)) -> (f32, f32) {
    let ((left, top, vw, vh), image) = old;
    let screen = if vw <= 0.0 || vh <= 0.0 || image.0 == 0 || image.1 == 0 {
        p
    } else {
        (left + p.0 * vw / image.0 as f32, top + p.1 * vh / image.1 as f32)
    };
    android_touch_to_image(new.0, new.1, screen)
}

/// The Resolution line's list with the window's current size in it: a window of a size the
/// monitor has no mode for (a launcher's forced size such as 1024x576) is added in its sorted
/// place, so that the line shows it and accepting the page keeps it (otherwise the first entry,
/// 640x480, was selected and applied). An empty list stays empty (the page's own list).
pub fn resolutions_with_current(mut list: Vec<(u32, u32)>, current: (u32, u32)) -> Vec<(u32, u32)> {
    if !list.is_empty() && current.0 > 0 && current.1 > 0 && !list.contains(&current) {
        let at = list.partition_point(|&r| r < current);
        list.insert(at, current);
    }
    list
}

impl Vm {
    fn video_ini(&mut self, key: &str) -> Option<String> {
        self.config.as_mut()?.get_string(SECTION, key, FILE)
    }

    /// The values of [OpenXIII.Video] (defaults for absent or invalid keys).
    pub fn video_settings(&mut self) -> VideoSettings {
        let mode = self.video_ini(KEY_FULLSCREEN_MODE);
        let scale = self.video_ini(KEY_RENDER_SCALE);
        let msaa = self.video_ini(KEY_MSAA);
        let aniso = self.video_ini(KEY_ANISOTROPY);
        let ui = self.video_ini(KEY_UI_SCALE);
        VideoSettings::from_texts(mode.as_deref(), scale.as_deref(), msaa.as_deref(), aniso.as_deref(), ui.as_deref())
    }

    /// UIScale of [OpenXIII.Video] as a factor (1.0 = 100 %, the default; read like the other
    /// video settings, an invalid value is the default). The improved build's menus are drawn at
    /// MenuFrame::of_window's scale times this factor; a page that scales itself multiplies its
    /// scale by it.
    ///
    /// The Video page no longer offers it: the menus and the HUD are always at 100 %, the
    /// original's layout on the screen of its 1280-wide mode at any resolution. A value saved
    /// by an earlier version (75 to 150) is ignored. With all the weapons the HUD's weapon row,
    /// anchored at the bottom right, went off the screen over the life block above 100 %.
    pub fn ui_scale(&mut self) -> f32 {
        1.0
    }

    /// Writes RenderScale, MSAA, AnisotropicFiltering and UIScale, and FullscreenMode when set (with
    /// StartupFullscreen of [WinDrv.WindowsClient]). Written to disk at the next flush.
    pub fn set_video_settings(&mut self, s: &VideoSettings) {
        let Some(c) = self.config.as_mut() else { return };
        c.set_string(SECTION, KEY_RENDER_SCALE, &s.render_scale.to_string(), FILE);
        c.set_string(SECTION, KEY_MSAA, &s.msaa.to_string(), FILE);
        c.set_string(SECTION, KEY_ANISOTROPY, &s.anisotropy.to_string(), FILE);
        c.set_string(SECTION, KEY_UI_SCALE, &s.ui_scale.to_string(), FILE);
        if let Some(m) = s.fullscreen_mode {
            c.set_string(SECTION, KEY_FULLSCREEN_MODE, m.as_str(), FILE);
            c.set_string("WinDrv.WindowsClient", "StartupFullscreen", if m.is_fullscreen() { "True" } else { "False" }, FILE);
        }
    }

    /// Graphics of [OpenXIII.Video]: Enhanced (true) or Original (false, the default for a new
    /// profile; a profile with older Video keys and no Graphics key is Enhanced, graphics_of_profile).
    pub fn graphics_setting(&mut self) -> bool {
        if let Some(on) = graphics_override() {
            return on;
        }
        let graphics = self.video_ini(KEY_GRAPHICS);
        let older = graphics.is_none() && ENHANCED_KEYS.iter().any(|k| self.video_ini(k).is_some());
        graphics_of_profile(graphics.as_deref(), older)
    }

    /// Writes Graphics (to disk at the next flush) and applies it at once.
    pub fn set_graphics_setting(&mut self, enhanced: bool) {
        if let Some(c) = self.config.as_mut() {
            c.set_string(SECTION, KEY_GRAPHICS, graphics_text(enhanced), FILE);
        }
        set_graphics_enhanced(enhanced);
        self.refresh_vsync();
    }

    /// [`graphics_enhanced`] brought to the ini's Graphics (read by the display before each frame).
    pub fn refresh_graphics(&mut self) {
        let on = self.graphics_setting();
        graphics::refresh(on, self.engine.client.session_graphics);
    }

    /// Presentation follows this display's graphics mode, including during a network session.
    pub fn vsync_setting(&mut self) -> VsyncMode {
        self.profile_vsync(graphics::local())
    }

    pub(crate) fn profile_vsync(&mut self, enhanced: bool) -> VsyncMode {
        let (section, key) = graphics::vsync_ini(enhanced);
        let v = self.config.as_mut().and_then(|c| c.get_string(section, key, FILE));
        if key != KEY_ORIGINAL_VSYNC {
            return vsync_mode_from_text(v.as_deref(), vsync_profile_default(enhanced));
        }
        // The original's bool (default True).
        if vsync_from_text(v.as_deref(), true) { VsyncMode::Classic } else { VsyncMode::Off }
    }

    /// Write the current display mode's preference and apply it from the next frame.
    pub fn set_vsync_setting(&mut self, mode: VsyncMode) {
        self.write_profile_vsync(graphics::local(), mode);
        store_vsync(mode);
    }

    pub(crate) fn write_profile_vsync(&mut self, enhanced: bool, mode: VsyncMode) {
        let (section, key) = graphics::vsync_ini(enhanced);
        let text = match (key, mode) {
            (KEY_ORIGINAL_VSYNC, VsyncMode::Off) => "False",
            (KEY_ORIGINAL_VSYNC, _) => "True",
            _ => vsync_text(mode),
        };
        if let Some(c) = self.config.as_mut() {
            c.set_string(section, key, text, FILE);
        }
    }

    /// [`vsync_mode`] brought to the ini's key (read by the display before each frame).
    pub fn refresh_vsync(&mut self) {
        let mode = self.vsync_setting();
        store_vsync(mode);
    }

    /// ShadowQuality of [OpenXIII.Video]: side of the shadow targets, 0 for Auto.
    pub fn shadow_quality(&mut self) -> u32 {
        shadow_quality_from_text(self.video_ini(KEY_SHADOW_QUALITY).as_deref())
    }

    /// Writes ShadowQuality (to disk at the next flush).
    pub fn set_shadow_quality(&mut self, v: u32) {
        if let Some(c) = self.config.as_mut() {
            c.set_string(SECTION, KEY_SHADOW_QUALITY, shadow_quality_text(v), FILE);
        }
    }

    /// Resolutions of the improved build's Video menu (the GetAvailableRes native keeps the
    /// original's 4:3 list): the monitor's modes ([`screen_resolutions`]) up to its native
    /// resolution ([`up_to_native`]); on Android, the modes of [`android_modes`] the viewer puts
    /// in `display_modes`. With Graphics: Original the menu keeps the original's 4:3 ones
    /// ([`original_resolutions`], video_menu.rs).
    pub fn menu_resolutions(&self) -> Vec<(u32, u32)> {
        up_to_native(screen_resolutions(&self.display_modes), self.screen_native)
    }

    /// Fullscreen is borderless ([OpenXIII.Video] FullscreenMode=Borderless, with Graphics:
    /// Enhanced); otherwise it is exclusive, as in the original. The original had no borderless
    /// mode: with Graphics: Original a Borderless key is exclusive fullscreen in the original's
    /// resolution, as from exclusive fullscreen; the key stays, for Enhanced again.
    pub fn borderless_fullscreen(&mut self) -> bool {
        self.borderless_key() && self.graphics_setting()
    }

    /// Improved build: the fullscreen size for `size` (the FullscreenViewportX/Y keys, or the
    /// window's size): with Graphics: Original a Borderless key runs exclusive fullscreen at the
    /// original's resolution, the nearest of GetAvailableRes's 4:3 list as the Video page's
    /// Resolution line shows it, never at a wide size the keys kept from Enhanced. Otherwise, and
    /// in the faithful build, `size` as it is.
    pub fn fullscreen_size(&mut self, size: (u32, u32)) -> (u32, u32) {
        if self.original_fullscreen() {
            original_fullscreen_size(&self.display_modes, size)
        } else {
            size
        }
    }

    /// Improved build: Graphics: Original with a Borderless key, whose fullscreen is exclusive at
    /// the original's resolution ([`Self::fullscreen_size`]); always false in the faithful build.
    pub fn original_fullscreen(&mut self) -> bool {
        crate::improved::navigation::original_fullscreen(self)
    }

    /// FullscreenMode=Borderless in [OpenXIII.Video], whatever Graphics.
    pub(crate) fn borderless_key(&mut self) -> bool {
        self.video_ini(KEY_FULLSCREEN_MODE).and_then(|s| FullscreenMode::parse(&s)) == Some(FullscreenMode::Borderless)
    }

    /// The mode the window is in now.
    pub fn current_fullscreen_mode(&mut self) -> FullscreenMode {
        if !self.fullscreen {
            FullscreenMode::Windowed
        } else if self.borderless_fullscreen() {
            FullscreenMode::Borderless
        } else {
            FullscreenMode::Exclusive
        }
    }

    /// The Display mode line's value (a line of Graphics: Enhanced only): the window's mode with
    /// Enhanced, a Borderless key staying Borderless while Original runs it exclusive.
    pub fn menu_fullscreen_mode(&mut self) -> FullscreenMode {
        if !self.fullscreen {
            FullscreenMode::Windowed
        } else if self.borderless_key() {
            FullscreenMode::Borderless
        } else {
            FullscreenMode::Exclusive
        }
    }
}

/// The original's fullscreen resolution for `size`: `size` when GetAvailableRes's 4:3 list of
/// `display_modes` holds it, else the list's nearest (Vm::fullscreen_size); `size` when the list
/// is empty.
pub fn original_fullscreen_size(display_modes: &[(u32, u32, u32)], size: (u32, u32)) -> (u32, u32) {
    let list = crate::video::available_res_of(display_modes);
    nearest_resolution(&list, size).map_or(size, |i| list[i])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lockstep co-op: in a session the game's Graphics is the host's, between steps too (the HUD
    /// and menus drawn, laid out with it), while the renderer's quality keeps this machine's; out
    /// of a session, this machine's.
    #[cfg(feature = "improved")]
    #[test]
    fn session_graphics_is_the_hosts_between_steps() {
        let mine = ENHANCED.load(std::sync::atomic::Ordering::Relaxed);
        set_graphics_enhanced(false);
        let shared = SessionGraphics { host: Some(true), in_step: false };
        let (game, local) = (shared.enhanced(), graphics_enhanced_local());
        let after = graphics_enhanced();
        set_graphics_enhanced(mine);
        {
            assert!(game, "the host's Enhanced between steps");
            assert!(!local, "this machine's Original for its renderer");
            assert!(!after, "no session: this machine's");
        }
    }

    #[test]
    fn vsync_key_read_and_written() {
        for default in [true, false] {
            assert_eq!(vsync_from_text(None, default), default);
            assert_eq!(vsync_from_text(Some("garbage"), default), default);
            for on in ["True", "1", " on ", "Yes"] {
                assert!(vsync_from_text(Some(on), default), "{on}");
            }
            for off in ["False", "false", " 0 ", "No", "OFF"] {
                assert!(!vsync_from_text(Some(off), default), "{off}");
            }
        }
        for default in VsyncMode::ALL {
            assert_eq!(vsync_mode_from_text(None, default), default);
            assert_eq!(vsync_mode_from_text(Some("garbage"), default), default);
            // On is Classic, and so is the Triple of older versions.
            for on in ["On", "True", "1", " yes ", "Triple", " TRIPLE ", "Mailbox", "3"] {
                assert_eq!(vsync_mode_from_text(Some(on), default), VsyncMode::Classic, "{on}");
            }
            for off in ["Off", "False", "0", "NO"] {
                assert_eq!(vsync_mode_from_text(Some(off), default), VsyncMode::Off, "{off}");
            }
            for mode in VsyncMode::ALL {
                assert_eq!(vsync_mode_from_text(Some(vsync_text(mode)), default), mode);
            }
        }
        assert_eq!(vsync_text(VsyncMode::Classic), "On");
        assert_eq!(vsync_text(VsyncMode::Off), "Off");
        // Classic by default in both builds (the original's).
        assert_eq!(VSYNC_DEFAULT, VsyncMode::Classic);
        assert_eq!(vsync_profile_default(false), VsyncMode::Classic);
    }

    #[test]
    fn vsync_modes_in_menu_order() {
        assert_eq!(VsyncMode::ALL, [VsyncMode::Off, VsyncMode::Classic]);
        for (i, m) in VsyncMode::ALL.into_iter().enumerate() {
            assert_eq!(m.index(), i);
            assert_eq!(VsyncMode::from_index(i as i32), m);
        }
        assert_eq!(VsyncMode::from_index(-1), VsyncMode::Off);
        assert_eq!(VsyncMode::from_index(7), VsyncMode::Classic);
    }

    #[test]
    fn vsync_override_values() {
        assert_eq!(vsync_override_from_text("0"), VsyncMode::Off);
        assert_eq!(vsync_override_from_text(" 0 "), VsyncMode::Off);
        for t in ["1", "on", "yes", "", "2", "triple", "Triple", "MAILBOX", "3"] {
            assert_eq!(vsync_override_from_text(t), VsyncMode::Classic, "{t}");
        }
    }

    #[cfg(feature = "improved")]
    #[test]
    fn vsync_preferences_follow_graphics_and_preserve_the_other_key() {
        let mut vm = Vm::new();
        let mut config = crate::config::ConfigCache::new(std::env::temp_dir());
        config.replace_file("XIII.ini", "[D3DDrv.D3DRenderDevice]\nUseVSync=False\n[OpenXIII.Video]\nVSync=On\n");
        vm.config = Some(config);
        assert_eq!(vm.profile_vsync(false), VsyncMode::Off);
        assert_eq!(vm.profile_vsync(true), VsyncMode::Classic);
        vm.write_profile_vsync(false, VsyncMode::Classic);
        assert_eq!(vm.profile_vsync(true), VsyncMode::Classic);
        assert_eq!(vm.profile_vsync(false), VsyncMode::Classic);
        vm.write_profile_vsync(true, VsyncMode::Off);
        assert_eq!(vm.profile_vsync(false), VsyncMode::Classic);
        assert_eq!(vm.profile_vsync(true), VsyncMode::Off);
        vm.write_profile_vsync(true, VsyncMode::Classic);
        assert_eq!(vm.profile_vsync(true), VsyncMode::Classic);
        assert_eq!(vm.profile_vsync(false), VsyncMode::Classic);
        let c = vm.config.as_mut().unwrap();
        assert_eq!(c.get_string(ORIGINAL_VSYNC_SECTION, KEY_ORIGINAL_VSYNC, FILE).as_deref(), Some("True"));
        assert_eq!(c.get_string(SECTION, KEY_VSYNC, FILE).as_deref(), Some("On"));
    }

    /// Improved build: ini files of older versions with Triple are read as On and rewritten
    /// without it (VSync=Triple; OriginalGraphicsVSync=Triple beside UseVSync=True).
    #[cfg(feature = "improved")]
    #[test]
    fn vsync_old_triple_keys_read_as_on() {
        let mut vm = Vm::new();
        let mut config = crate::config::ConfigCache::new(std::env::temp_dir());
        config.replace_file("XIII.ini", "[D3DDrv.D3DRenderDevice]\nUseVSync=True\n[OpenXIII.Video]\nGraphics=Original\nVSync=Triple\nOriginalGraphicsVSync=Triple\n");
        vm.config = Some(config);
        assert_eq!(vm.profile_vsync(false), VsyncMode::Classic);
        assert_eq!(vm.profile_vsync(true), VsyncMode::Classic);
        let get = |vm: &mut Vm, s: &str, k: &str| vm.config.as_mut().unwrap().get_string(s, k, FILE);
        vm.write_profile_vsync(true, VsyncMode::Classic);
        assert_eq!(get(&mut vm, SECTION, KEY_VSYNC).as_deref(), Some("On"));
        vm.write_profile_vsync(false, VsyncMode::Off);
        assert_eq!(vm.profile_vsync(false), VsyncMode::Off);
        assert_eq!(get(&mut vm, ORIGINAL_VSYNC_SECTION, KEY_ORIGINAL_VSYNC).as_deref(), Some("False"));
        // The old key is left as it was, never written again.
        assert_eq!(get(&mut vm, SECTION, "OriginalGraphicsVSync").as_deref(), Some("Triple"));
    }

    /// Faithful build: only the original's UseVSync, read as a bool; a Triple key is ignored.
    #[cfg(not(feature = "improved"))]
    #[test]
    fn vsync_faithful_reads_only_usevsync() {
        let mut vm = Vm::new();
        let mut config = crate::config::ConfigCache::new(std::env::temp_dir());
        config.replace_file("XIII.ini", "[OpenXIII.Video]\nVSync=Triple\nOriginalGraphicsVSync=Triple\n");
        vm.config = Some(config);
        for enhanced in [false, true] {
            assert_eq!(vm.profile_vsync(enhanced), VsyncMode::Classic);
        }
        vm.write_profile_vsync(false, VsyncMode::Off);
        assert_eq!(vm.profile_vsync(false), VsyncMode::Off);
        let c = vm.config.as_mut().unwrap();
        assert_eq!(c.get_string(ORIGINAL_VSYNC_SECTION, KEY_ORIGINAL_VSYNC, FILE).as_deref(), Some("False"));
        assert_eq!(c.get_string(SECTION, "OriginalGraphicsVSync", FILE).as_deref(), Some("Triple"));
    }

    /// V-sync without its key: Classic with either Graphics setting, in both builds.
    #[test]
    fn vsync_default_follows_the_graphics_setting() {
        let mine = ENHANCED.load(std::sync::atomic::Ordering::Relaxed);
        set_graphics_enhanced(false);
        let original = vsync_default();
        set_graphics_enhanced(true);
        let enhanced = vsync_default();
        set_graphics_enhanced(mine);
        assert_eq!(original, VsyncMode::Classic, "Graphics: Original");
        assert_eq!(enhanced, VsyncMode::Classic, "Graphics: Enhanced");
    }

    #[test]
    fn resolutions_sorted_without_duplicates() {
        // Two refresh rates of 1920x1080, a 16-bit mode, modes below 640x480, 16:10 and 4:3 ones.
        let modes = [
            (1920, 1080, 32),
            (800, 600, 32),
            (1920, 1080, 32),
            (640, 480, 16),
            (640, 480, 32),
            (320, 240, 32),
            (1280, 800, 32),
            (1024, 768, 24),
            (2560, 1440, 32),
            (720, 400, 32),
        ];
        assert_eq!(screen_resolutions(&modes), vec![(640, 480), (800, 600), (1024, 768), (1280, 800), (1920, 1080), (2560, 1440)]);
        // Unknown depth (0) is kept, 16 bits alone is not.
        assert_eq!(screen_resolutions(&[(1366, 768, 0), (1600, 900, 16)]), vec![(1366, 768)]);
        assert!(screen_resolutions(&[]).is_empty());
    }

    #[test]
    fn nothing_above_the_native_resolution() {
        // A 1920 x 1200 panel whose driver lists scaled modes up to 2560 x 1600 and a 4K factor.
        let modes = [(1280, 720, 32), (1920, 1080, 32), (1920, 1200, 32), (2560, 1440, 32), (2560, 1600, 32), (3840, 2400, 32), (1600, 1200, 32), (2048, 1152, 32)];
        let list = up_to_native(screen_resolutions(&modes), Some((1920, 1200)));
        assert_eq!(list, vec![(1280, 720), (1600, 1200), (1920, 1080), (1920, 1200)]);
        // A 2560 x 1440 screen keeps 2560 x 1440 but not 2560 x 1600 nor 4K.
        assert_eq!(up_to_native(screen_resolutions(&modes), Some((2560, 1440))), vec![(1280, 720), (1600, 1200), (1920, 1080), (1920, 1200), (2048, 1152), (2560, 1440)]);
        // Unknown native size: every mode.
        assert_eq!(up_to_native(screen_resolutions(&modes), None).len(), 8);
        assert_eq!(up_to_native(screen_resolutions(&modes), Some((0, 0))).len(), 8);
        // The window's current size stays offered even above the native one (a launcher's size).
        assert_eq!(resolutions_with_current(list, (2560, 1440)).last(), Some(&(2560, 1440)));
    }

    #[test]
    fn resolutions_hold_the_current_window() {
        let monitor = vec![(640, 480), (800, 600), (1024, 768), (1280, 720), (1920, 1080)];
        // A forced window the monitor has no mode for: added in its place, so it is the selection.
        let with = resolutions_with_current(monitor.clone(), (1024, 576));
        assert_eq!(with, vec![(640, 480), (800, 600), (1024, 576), (1024, 768), (1280, 720), (1920, 1080)]);
        assert_ne!(with.iter().position(|&r| r == (1024, 576)), Some(0), "the first entry (640x480) must not be taken");
        let small = resolutions_with_current(monitor.clone(), (854, 480));
        assert_eq!(small[2], (854, 480));
        // Already in the list, unknown size, or no list: unchanged.
        assert_eq!(resolutions_with_current(monitor.clone(), (1280, 720)), monitor);
        assert_eq!(resolutions_with_current(monitor.clone(), (0, 0)), monitor);
        assert!(resolutions_with_current(Vec::new(), (1024, 576)).is_empty());
    }

    #[test]
    fn resolutions_keep_original_4_3_modes() {
        // Every 4:3 resolution of the original list (640x480 to 1600x1200) the monitor has is offered.
        let original = [(640, 480), (800, 600), (1024, 768), (1152, 864), (1280, 960), (1600, 1200)];
        let mut modes: Vec<(u32, u32, u32)> = original.iter().map(|&(w, h)| (w, h, 32)).collect();
        modes.extend([(1920, 1080, 32), (1280, 720, 32), (3840, 2160, 32)]);
        let r = screen_resolutions(&modes);
        for o in original {
            assert!(r.contains(&o), "{o:?}");
        }
        assert_eq!(r.len(), original.len() + 3);
        assert!(r.windows(2).all(|p| p[0] < p[1]));
    }

    #[test]
    fn graphics_of_older_and_new_profiles() {
        assert!(!graphics_of_profile(None, false), "new profile: Original");
        assert!(graphics_of_profile(None, true), "profile with older Video keys: Enhanced");
        for older in [false, true] {
            assert!(!graphics_of_profile(Some("Original"), older), "explicit Original kept");
            assert!(graphics_of_profile(Some("Enhanced"), older), "explicit Enhanced kept");
            assert!(!graphics_of_profile(Some("nonsense"), older), "unknown value: Original");
        }
    }

    /// The same through the game's configuration: each older key alone makes a profile Enhanced.
    #[test]
    fn graphics_of_an_ini_without_the_key() {
        for (i, key) in std::iter::once(None).chain(ENHANCED_KEYS.iter().map(Some)).enumerate() {
            let dir = std::env::temp_dir().join(format!("oxiii-graphics-profile-{}-{i}", std::process::id()));
            let system = dir.join("System");
            std::fs::create_dir_all(&system).unwrap();
            let mut ini = String::from("[WinDrv.WindowsClient]\r\nStartupFullscreen=False\r\n\r\n");
            if let Some(k) = key {
                ini += &format!("[{SECTION}]\r\n{k}=100\r\n\r\n");
            }
            std::fs::write(system.join("XIII.ini"), ini).unwrap();
            let mut vm = Vm::new();
            vm.config = Some(crate::config::ConfigCache::with_user_dir(system.clone(), Some(dir.join("player"))));
            assert_eq!(vm.graphics_setting(), key.is_some(), "{key:?}");
            if key.is_some() {
                vm.config.as_mut().unwrap().set_string(SECTION, KEY_GRAPHICS, "Original", FILE);
                assert!(!vm.graphics_setting(), "{key:?}: an explicit Original wins");
            }
            let _ = std::fs::remove_dir_all(&dir);
        }
    }

    #[test]
    fn graphics_texts() {
        assert!(!graphics_from_text(None), "absent: Original");
        assert!(!graphics_from_text(Some("Original")));
        assert!(!graphics_from_text(Some("nonsense")));
        assert!(graphics_from_text(Some(" enhanced ")));
        for on in [false, true] {
            assert_eq!(graphics_from_text(Some(graphics_text(on))), on);
        }
    }

    #[test]
    fn original_resolution_list() {
        let monitor = [(640, 480), (800, 600), (1024, 768), (1280, 720), (1280, 960), (1366, 768), (1920, 1080), (1600, 1200)];
        let o = original_resolutions(&monitor);
        assert_eq!(o, vec![(640, 480), (800, 600), (1024, 768), (1280, 960), (1600, 1200)]);
        assert_eq!(nearest_resolution(&o, (1024, 768)), Some(2));
        // 1920 x 1080: the 4:3 mode nearest in height (1280 x 960 and 1600 x 1200 both 120 away: the smaller).
        assert_eq!(nearest_resolution(&o, (1920, 1080)), Some(3));
        assert_eq!(nearest_resolution(&o, (1280, 720)), Some(2));
        assert_eq!(nearest_resolution(&[], (1280, 720)), None);
    }

    #[test]
    fn shadow_quality_texts() {
        assert_eq!(shadow_quality_from_text(None), 0);
        assert_eq!(shadow_quality_from_text(Some("auto")), 0);
        assert_eq!(shadow_quality_from_text(Some("nonsense")), 0);
        assert_eq!(shadow_quality_from_text(Some(" low ")), 256);
        assert_eq!(shadow_quality_from_text(Some("High")), 1024);
        assert_eq!(shadow_quality_from_text(Some("veryhigh")), if cfg!(target_os = "android") { 0 } else { 2048 });
        for v in shadow_quality_choices() {
            assert_eq!(shadow_quality_from_text(Some(shadow_quality_text(v))), v);
        }
        assert_eq!(shadow_quality_choices()[0], 0);
    }

    #[test]
    fn defaults() {
        let d = VideoSettings::default();
        assert_eq!(d, VideoSettings { fullscreen_mode: None, render_scale: 100, msaa: 1, anisotropy: ANISOTROPY_DEFAULT, ui_scale: 100 });
        assert_eq!(VideoSettings::from_texts(None, None, None, None, None), d);
        assert!(ui_scale_choices().contains(&UI_SCALE_DEFAULT));
        assert!(ui_scale_choices().iter().all(|v| (UI_SCALE_MIN..=UI_SCALE_MAX).contains(v)));
        assert!(render_scale_choices().contains(&RENDER_SCALE_DEFAULT));
        assert!(msaa_choices().contains(&MSAA_DEFAULT));
        assert!(anisotropy_choices().contains(&ANISOTROPY_DEFAULT));
        assert_eq!(ANISOTROPY_DEFAULT, if !cfg!(feature = "improved") { 1 } else if cfg!(target_os = "android") { 4 } else { 16 });
    }

    #[test]
    fn parse_and_check_values() {
        let s = VideoSettings::from_texts(Some("borderless"), Some(" 150 "), Some("4"), Some("8"), Some(" 90"));
        assert_eq!(s, VideoSettings { fullscreen_mode: Some(FullscreenMode::Borderless), render_scale: 150, msaa: 4, anisotropy: 8, ui_scale: 90 });
        // Out of range or invalid: the defaults.
        let s = VideoSettings::from_texts(Some("Fullscreen"), Some("1000"), Some("3"), Some("32"), Some("200"));
        assert_eq!(s, VideoSettings::default());
        assert_eq!(VideoSettings::from_texts(None, Some("10"), Some("x"), Some(""), Some("big")), VideoSettings::default());
        assert_eq!(VideoSettings::from_texts(None, Some("25"), None, None, None).render_scale, 25);
        assert_eq!(VideoSettings::from_texts(None, Some("400"), None, None, None).render_scale, 400);
        assert_eq!(VideoSettings::from_texts(None, None, None, None, Some("75")).ui_scale, 75);
        assert_eq!(VideoSettings::from_texts(None, None, None, None, Some("150")).ui_scale, 150);
        assert_eq!(VideoSettings::from_texts(None, None, None, None, Some("74")).ui_scale, 100);
        assert_eq!(VideoSettings::from_texts(None, Some("-100"), Some("16"), Some("0"), Some("151")), VideoSettings::default());
        for m in FullscreenMode::ALL {
            assert_eq!(FullscreenMode::parse(m.as_str()), Some(m));
        }
        // The Display mode line's order; the ini's names stay those of the earlier versions.
        assert_eq!(FullscreenMode::ALL, [FullscreenMode::Exclusive, FullscreenMode::Borderless, FullscreenMode::Windowed]);
        assert_eq!(FullscreenMode::ALL.map(FullscreenMode::as_str), ["Exclusive", "Borderless", "Windowed"]);
        assert_eq!(FullscreenMode::parse("windowed"), Some(FullscreenMode::Windowed));
        assert!(!FullscreenMode::Windowed.is_fullscreen());
        assert!(FullscreenMode::Borderless.is_fullscreen() && FullscreenMode::Exclusive.is_fullscreen());
    }

    #[test]
    fn choices_follow_the_device() {
        let all = VideoCaps::default();
        assert_eq!(all.render_scale_choices(), render_scale_choices());
        assert_eq!(all.msaa_choices(), msaa_choices());
        assert_eq!(all.anisotropy_choices(), anisotropy_choices());
        let small = VideoCaps { msaa: vec![1, 4], max_render_scale: 160, max_anisotropy: 1 };
        assert_eq!(small.render_scale_choices(), vec![25, 50, 75, 100, 125, 150]);
        assert_eq!(small.msaa_choices(), vec![1, 4]);
        assert_eq!(small.anisotropy_choices(), vec![1]);
        let none = VideoCaps { msaa: vec![], max_render_scale: 0, max_anisotropy: 0 };
        assert_eq!(none.render_scale_choices(), vec![25]);
        assert_eq!(none.msaa_choices(), vec![1]);
        assert_eq!(none.anisotropy_choices(), vec![1]);
    }

    #[test]
    fn closest_choice() {
        let c = render_scale_choices();
        assert_eq!(c[closest_index(&c, 100)], 100);
        assert_eq!(c[closest_index(&c, 110)], 100);
        assert_eq!(c[closest_index(&c, 260)], 300);
        assert_eq!(c[closest_index(&c, 25)], 25);
        assert_eq!(closest_index(&[], 7), 0);
    }

    /// First launch: the installation's XIII.ini was changed (a window, another size, another
    /// brightness); the player's first configuration is the original's defaults (Default.ini)
    /// in fullscreen at the native size, written into the player's folder only. Negative
    /// control: without the first-launch step the installation's values are read.
    #[test]
    fn first_launch_is_original_defaults_at_native_size_enhanced() {
        let dir = std::env::temp_dir().join(format!("oxiii-first-launch-{}", std::process::id()));
        let system = dir.join("System");
        let player = dir.join("player");
        let _ = std::fs::remove_dir_all(&player);
        std::fs::create_dir_all(&system).unwrap();
        std::fs::write(
            system.join("Default.ini"),
            "[WinDrv.WindowsClient]\r\nWindowedViewportX=800\r\nWindowedViewportY=600\r\nFullscreenViewportX=800\r\nFullscreenViewportY=600\r\nBrightness=0.500000\r\nStartupFullscreen=True\r\n\r\n[D3DDrv.D3DRenderDevice]\r\nUseVSync=True\r\n\r\n",
        )
        .unwrap();
        std::fs::write(
            system.join("XIII.ini"),
            "[WinDrv.WindowsClient]\r\nWindowedViewportX=640\r\nWindowedViewportY=480\r\nFullscreenViewportX=1024\r\nFullscreenViewportY=768\r\nBrightness=0.900000\r\nStartupFullscreen=False\r\n\r\n[D3DDrv.D3DRenderDevice]\r\nUseVSync=False\r\n\r\n[Engine.Engine]\r\nLanguage=frt\r\n\r\n",
        )
        .unwrap();
        let get = |c: &mut crate::config::ConfigCache, s: &str, k: &str| c.get_string(s, k, FILE);
        let mut before = crate::config::ConfigCache::with_user_dir(system.clone(), Some(player.clone()));
        assert_eq!(get(&mut before, "WinDrv.WindowsClient", "StartupFullscreen").as_deref(), Some("False"), "negative control");
        let mut c = crate::config::ConfigCache::with_user_dir(system.clone(), Some(player.clone()));
        apply_first_launch_size(&mut c, Some((2560, 1080)));
        let w = "WinDrv.WindowsClient";
        assert_eq!(get(&mut c, w, "StartupFullscreen").as_deref(), Some("True"));
        assert_eq!(get(&mut c, w, "FullscreenViewportX").as_deref(), Some("2560"));
        assert_eq!(get(&mut c, w, "FullscreenViewportY").as_deref(), Some("1080"));
        assert_eq!(get(&mut c, w, "WindowedViewportX").as_deref(), Some("800"));
        assert_eq!(get(&mut c, w, "Brightness").as_deref(), Some("0.500000"));
        assert_eq!(get(&mut c, "D3DDrv.D3DRenderDevice", "UseVSync").as_deref(), Some("True"));
        // Other sections stay the installation's; Graphics: Enhanced, the other OpenXIII keys absent.
        assert_eq!(get(&mut c, "Engine.Engine", "Language").as_deref(), Some("frt"));
        assert!(get(&mut before, SECTION, KEY_GRAPHICS).is_none(), "negative control");
        assert_eq!(get(&mut c, SECTION, KEY_GRAPHICS).as_deref(), Some("Enhanced"));
        assert!(get(&mut c, SECTION, KEY_UI_SCALE).is_none());
        c.flush();
        let written = std::fs::read_to_string(player.join("XIII.ini")).unwrap();
        assert!(written.contains("FullscreenViewportX=2560") && written.contains("StartupFullscreen=True") && written.contains("Graphics=Enhanced"), "{written}");
        assert!(std::fs::read_to_string(system.join("XIII.ini")).unwrap().contains("FullscreenViewportX=1024"), "the game's folder is never written");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Ini round trip through the game's configuration (XIII.ini in a temporary System folder).
    /// Graphics: Original with a Borderless key: the fullscreen size (launch, Alt+Enter, a display
    /// mode asked without a size) is the original's 4:3 resolution nearest to the keys', never a
    /// wide size kept from Enhanced (a 1920 x 1200 monitor: 1600 x 1200); Enhanced, an Exclusive
    /// key, and the faithful build keep the size.
    #[test]
    fn original_fullscreen_is_the_originals_resolution() {
        let dir = std::env::temp_dir().join(format!("oxiii-video-original-fullscreen-{}", std::process::id()));
        let system = dir.join("System");
        std::fs::create_dir_all(&system).unwrap();
        std::fs::write(system.join("XIII.ini"), "[WinDrv.WindowsClient]
StartupFullscreen=True

").unwrap();
        let mut vm = Vm::new();
        vm.config = Some(crate::config::ConfigCache::with_user_dir(system.clone(), Some(dir.join("player"))));
        // The modes of a 1920 x 1200 monitor (4:3: 640 x 480 to 1600 x 1200; no 1280 x 960).
        let sizes = [(640, 480), (800, 600), (1024, 768), (1280, 720), (1280, 800), (1280, 1024), (1440, 1080), (1600, 1200), (1680, 1050), (1920, 1080), (1920, 1200)];
        vm.display_modes = sizes.iter().map(|&(w, h)| (w, h, 32)).collect();
        assert_eq!(original_fullscreen_size(&vm.display_modes, (1920, 1200)), (1600, 1200));
        assert_eq!(original_fullscreen_size(&vm.display_modes, (1280, 720)), (1024, 768));
        assert_eq!(original_fullscreen_size(&vm.display_modes, (800, 600)), (800, 600));
        assert_eq!(original_fullscreen_size(&[], (1920, 1200)), (1920, 1200));
        let set = |vm: &mut Vm, mode: FullscreenMode, enhanced: bool| {
            vm.set_video_settings(&VideoSettings { fullscreen_mode: Some(mode), ..VideoSettings::default() });
            vm.set_graphics_setting(enhanced);
        };
        let mut seen = Vec::new();
        for (mode, enhanced) in [(FullscreenMode::Borderless, false), (FullscreenMode::Borderless, true), (FullscreenMode::Exclusive, false), (FullscreenMode::Exclusive, true)] {
            set(&mut vm, mode, enhanced);
            seen.push((vm.original_fullscreen(), vm.fullscreen_size((1920, 1200)), vm.fullscreen_size((1280, 720)), vm.fullscreen_size((1600, 1200))));
        }
        set_graphics_enhanced(false);
        let _ = std::fs::remove_dir_all(&dir);
        let kept = (false, (1920, 1200), (1280, 720), (1600, 1200));
        if cfg!(feature = "improved") {
            assert_eq!(seen[0], (true, (1600, 1200), (1024, 768), (1600, 1200)), "Original, Borderless key: the original's resolution");
        } else {
            assert_eq!(seen[0], kept, "faithful build: the keys' size");
        }
        assert_eq!(&seen[1..], &[kept; 3], "Enhanced or an Exclusive key: the keys' size");
    }

    #[test]
    fn ini_read_write() {
        let dir = std::env::temp_dir().join(format!("oxiii-video-settings-{}", std::process::id()));
        let system = dir.join("System");
        std::fs::create_dir_all(&system).unwrap();
        std::fs::write(
            system.join("XIII.ini"),
            "[WinDrv.WindowsClient]\r\nStartupFullscreen=True\r\nFullscreenViewportX=1024\r\nFullscreenViewportY=768\r\n\r\n",
        )
        .unwrap();
        let mut vm = Vm::new();
        vm.config = Some(crate::config::ConfigCache::with_user_dir(system.clone(), Some(dir.join("player"))));
        // Nothing written yet: defaults, exclusive fullscreen as in the original, Graphics: Original.
        assert_eq!(vm.video_settings(), VideoSettings::default());
        assert!(!vm.graphics_setting());
        vm.set_graphics_setting(true);
        assert!(vm.graphics_setting());
        assert_eq!(vm.config.as_mut().unwrap().get_string(SECTION, KEY_GRAPHICS, FILE).as_deref(), Some("Enhanced"));
        vm.set_graphics_setting(false);
        assert!(!vm.graphics_setting() && !graphics_enhanced());
        assert!(!vm.borderless_fullscreen());
        vm.fullscreen = true;
        assert_eq!(vm.current_fullscreen_mode(), FullscreenMode::Exclusive);
        let s = VideoSettings { fullscreen_mode: Some(FullscreenMode::Borderless), render_scale: 75, msaa: 8, anisotropy: 2, ui_scale: 85 };
        vm.set_video_settings(&s);
        assert_eq!(vm.video_settings(), s);
        // Graphics: Original has no borderless mode: exclusive fullscreen, as the original.
        assert!(!vm.borderless_fullscreen());
        assert_eq!(vm.current_fullscreen_mode(), FullscreenMode::Exclusive);
        assert_eq!(vm.menu_fullscreen_mode(), FullscreenMode::Borderless);
        vm.set_graphics_setting(true);
        assert!(vm.borderless_fullscreen());
        assert_eq!(vm.current_fullscreen_mode(), FullscreenMode::Borderless);
        vm.set_graphics_setting(false);
        vm.set_video_settings(&VideoSettings { fullscreen_mode: Some(FullscreenMode::Windowed), ..s });
        let c = vm.config.as_mut().unwrap();
        assert_eq!(c.get_string("WinDrv.WindowsClient", "StartupFullscreen", "System").as_deref(), Some("False"));
        assert_eq!(c.get_string(SECTION, KEY_RENDER_SCALE, FILE).as_deref(), Some("75"));
        assert_eq!(c.get_string(SECTION, KEY_MSAA, FILE).as_deref(), Some("8"));
        assert_eq!(c.get_string(SECTION, KEY_ANISOTROPY, FILE).as_deref(), Some("2"));
        assert_eq!(c.get_string(SECTION, KEY_UI_SCALE, FILE).as_deref(), Some("85"));
        assert_eq!(c.get_string(SECTION, KEY_FULLSCREEN_MODE, FILE).as_deref(), Some("Windowed"));
        // The game's own resolution keys are untouched.
        assert_eq!(c.get_string("WinDrv.WindowsClient", "FullscreenViewportX", "System").as_deref(), Some("1024"));
        // Written to disk by the flush (into the player's folder, as in the game), read back by a new configuration.
        c.flush();
        let mut vm2 = Vm::new();
        vm2.config = Some(crate::config::ConfigCache::with_user_dir(system.clone(), Some(dir.join("player"))));
        assert_eq!(vm2.video_settings(), VideoSettings { fullscreen_mode: Some(FullscreenMode::Windowed), ..s });
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn android_modes_of_a_wide_phone() {
        let modes = android_modes((2400, 1080));
        let sizes: Vec<(u32, u32)> = modes.iter().map(|&(w, h, _)| (w, h)).collect();
        assert_eq!(
            sizes,
            [(640, 480), (800, 600), (1024, 768), (1067, 480), (1152, 864), (1200, 540), (1280, 960), (1333, 600), (1600, 720), (2000, 900), (2400, 1080)]
        );
        assert!(modes.iter().all(|m| m.2 == 32));
        // A first launch records and applies the screen's native size, the list's largest, as
        // on PC; never the activity's first 600-line image (control).
        assert_eq!(android_first_launch_size((2400, 1080)), (2400, 1080));
        assert_eq!(sizes.last(), Some(&android_first_launch_size((2400, 1080))));
        assert_ne!(android_first_launch_size((2400, 1080)), (android_wide_width((2400, 1080), ANDROID_DEFAULT_HEIGHT), ANDROID_DEFAULT_HEIGHT));
        assert_eq!(android_first_launch_size((1920, 1080)), (1920, 1080));
        assert_eq!(android_first_launch_size((1280, 720)), (1280, 720));
        // The activity's first image (600 lines in the screen's shape) is one of them.
        assert_eq!(android_wide_width((2400, 1080), ANDROID_DEFAULT_HEIGHT), 1333);
        assert_eq!(android_surface_for(&modes, (1333, 600)), Some((1333, 600)));
        // The Video menu: every mode in Enhanced, the original's GetAvailableRes 4:3 list in
        // Original (the same path as on PC).
        let mut vm = Vm::new();
        vm.display_modes = modes.clone();
        vm.screen_native = Some((2400, 1080));
        assert_eq!(vm.menu_resolutions(), sizes);
        let original = [(640, 480), (800, 600), (1024, 768), (1152, 864), (1280, 960)];
        assert_eq!(vm.available_res(), original);
        assert_eq!(original_resolutions(&vm.menu_resolutions()), original);
        // A smaller screen: nothing larger than it.
        let small: Vec<(u32, u32)> = android_modes((1600, 720)).iter().map(|&(w, h, _)| (w, h)).collect();
        assert_eq!(small, [(640, 480), (800, 600), (1067, 480), (1200, 540), (1333, 600), (1600, 720)]);
        assert_eq!(original_resolutions(&small), [(640, 480), (800, 600)]);
        assert!(android_modes((0, 0)).is_empty());
    }

    #[test]
    fn android_surface_of_another_phones_ini() {
        let modes = android_modes((2400, 1080));
        assert_eq!(android_surface_for(&modes, (800, 600)), Some((800, 600)));
        // 1280 x 576 (another phone's 600-line-like size): the nearest height, then width.
        assert_eq!(android_surface_for(&modes, (1280, 580)), Some((1333, 600)));
        assert_eq!(android_surface_for(&modes, (3200, 1440)), Some((2400, 1080)));
        assert_eq!(android_surface_for(&[], (800, 600)), None);
    }

    #[test]
    fn android_touch_with_black_bars() {
        // 2400 x 1080 screen, 800 x 600 image shown 1440 x 1080 in the middle (480 px bars).
        let rect = (480.0, 0.0, 1440.0, 1080.0);
        let map = |p| android_touch_to_image(rect, (800, 600), p);
        assert_eq!(map((480.0, 0.0)), (0.0, 0.0));
        assert_eq!(map((1200.0, 540.0)), (400.0, 300.0));
        assert_eq!(map((1920.0, 1080.0)), (800.0, 600.0));
        // On a bar: outside the image, at the same scale (a look finger keeps turning there).
        assert_eq!(map((120.0, 540.0)), (-200.0, 300.0));
        assert_eq!(map((2280.0, 540.0)), (1000.0, 300.0));
        // Full screen (no bars): the plain scale of before.
        assert_eq!(android_touch_to_image((0.0, 0.0, 2400.0, 1080.0), (1333, 600), (1200.0, 540.0)), (666.5, 300.0));
        // No view size yet: unchanged.
        assert_eq!(android_touch_to_image((0.0, 0.0, 0.0, 0.0), (800, 600), (5.0, 6.0)), (5.0, 6.0));
        // Negative check: the mapping without the bar offset (the old one) misses the button.
        let old = |p: (f32, f32)| (p.0 * 800.0 / 1440.0, p.1 * 600.0 / 1080.0);
        let mut differ = 0;
        for i in 0..200 {
            let p = (480.0 + i as f32 * 7.1, (i * 37 % 1080) as f32);
            let new_p = map(p);
            let expect = ((p.0 - 480.0) * 800.0 / 1440.0, p.1 * 600.0 / 1080.0);
            assert!((new_p.0 - expect.0).abs() < 1e-3 && (new_p.1 - expect.1).abs() < 1e-3, "{p:?}");
            if (old(p).0 - new_p.0).abs() > 1.0 {
                differ += 1;
            }
        }
        assert_eq!(differ, 200);
    }

    #[test]
    fn android_finger_follows_an_image_resize() {
        // Startup: 1333 x 600 image over the whole 2400 x 1080 screen, then the ini's resolution
        // (2400 x 1080 full screen, or 800 x 600 between 480 px bars) while the finger is down.
        let start = ((0.0, 0.0, 2400.0, 1080.0), (1333, 600));
        let targets = [((0.0, 0.0, 2400.0, 1080.0), (2400, 1080)), ((480.0, 0.0, 1440.0, 1080.0), (800, 600)), ((0.0, 0.0, 2400.0, 1080.0), (1333, 600))];
        let mut stale_misses = 0;
        for (n, &target) in targets.iter().enumerate() {
            for i in 0..150 {
                let screen = ((i * 97 % 2400) as f32 + 0.5, (i * 53 % 1080) as f32 + 0.25);
                let kept = android_touch_to_image(start.0, start.1, screen);
                let moved = android_touch_remap(start, target, kept);
                // Where a finger put down after the resize is.
                let fresh = android_touch_to_image(target.0, target.1, screen);
                assert!((moved.0 - fresh.0).abs() < 1e-2 && (moved.1 - fresh.1).abs() < 1e-2, "{n} {screen:?}");
                // Negative check: the point kept unchanged (before the fix) is elsewhere on the new
                // image, unless the image did not change.
                if (kept.0 - fresh.0).abs() > 2.0 || (kept.1 - fresh.1).abs() > 2.0 {
                    stale_misses += 1;
                }
            }
        }
        assert!(stale_misses >= 290, "{stale_misses}");
        // No view rectangle yet (points in screen pixels): mapped from the screen.
        assert_eq!(android_touch_remap(((0.0, 0.0, 0.0, 0.0), (1333, 600)), targets[0], (1200.0, 540.0)), (1200.0, 540.0));
    }
}
