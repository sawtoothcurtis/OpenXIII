//! Improved build only: the co-op character screen.
//!
//! The last step before a co-op game starts: each player chooses who they play. The screen is a
//! real level of the game (by default Hual02, a storeroom of the base) loaded
//! with the [`SCREEN_OPTION`] URL option: the level is paused as soon as it starts (nothing in it
//! moves or thinks), each player's view is a fixed camera on their own spot, and on that spot
//! stands the character they chose, playing a pose, lit and shadowed by the level. The menu page
//! is drawn over it.
//!
//! Every character of the roster (and every variant) is spawned once, hidden, when the screen
//! opens: the renderer loads their models and skins then. Choosing another one only hides the
//! previous actor and shows the next (no loading while the screen is shown).
//!
//! Where the cameras and spots are is one small table, [`Place`] (defaults in [`PLACES`]),
//! which a text file can replace without rebuilding the game ([`set_place_file`]).

#[allow(unused_imports)]
use openxiii_math::SimMath as _;
use crate::value::{NameStruct, ObjId, Value};
use crate::{NativeField as Field, Vm};

/// URL option of the character screen's map ("Hual02?OXIIIChoose").
pub const SCREEN_OPTION: &str = "OXIIIChoose";

/// One player's camera and spot on the screen's map.
#[derive(Clone, Debug, PartialEq)]
pub struct Place {
    /// Camera position, and its rotation when the place file gives one (pitch, yaw, roll in Unreal
    /// units: 65536 = a full turn); None: the camera looks at the character by itself.
    pub camera: [f32; 3],
    pub camera_rotation: Option<[i32; 3]>,
    /// The recorded view's vertical field of view (degrees; None: the game's own alone on the
    /// screen, [`SOLO_VFOV`]). The live-view panel shows that height of picture.
    pub fov: Option<f32>,
    /// Where the character stands (its centre, as the map's own characters are placed) and the
    /// direction it faces (yaw).
    pub spot: [f32; 3],
    pub facing: i32,
}

/// The screen's map and the players' places (player 1, player 2).
#[derive(Clone, Debug, PartialEq)]
pub struct Places {
    pub map: String,
    pub players: [Place; 2],
    /// How the page is laid out alone on the screen (one player), and in each half of a split
    /// screen.
    pub layout: Layout,
    pub split_layout: Layout,
    /// The characters face their camera; this turns them further (degrees, to the character's
    /// left for a positive value), for a three-quarter view.
    pub spot_turn: f32,
    /// With a recorded camera direction: the camera moves back along its own line of sight
    /// until the character fits head to feet (otherwise the view is exactly the recorded one).
    pub fit: bool,
    /// The outfit of a character with several, written on the live view: not at all (default),
    /// or in small discreet letters (OutfitLabel=small).
    pub outfit_label: bool,
}

/// The game's vertical field of view alone on the screen: FovAngle 85 across a 16:9 window.
pub const SOLO_VFOV: f32 = 54.52;

/// The default layouts: a compact block, as narrow as the game's own menus, centred on the crates
/// picture, the help bar a little wider than the panels; a little larger alone on the screen (one
/// player on this PC, a network game) than in each half of a split screen. The earlier, wider
/// ones stay reachable from the place file (Layout=wide, BlockWidth, ListShare...).
pub fn default_layouts() -> (Layout, Layout) {
    (
        // Alone (and a network game): the same family, more rows, the view magnified and raised.
        Layout { width: 0.42, aspect: 1.28, list: 0.44, gap: 0.018, rows: 8, bar: 0.044, scale: 0.985, margin: 0.005, text: 14.0, ink: 0.8, zoom: 1.07, shift: [0.0, -0.01], ..Layout::of(LayoutKind::Block) },
        // Split screen: the proportions of the game's other menus (chosen at 640 x 480, the same
        // at every size): a smaller text and block, no margin, a finer ink.
        Layout { width: 0.402, aspect: 1.29, list: 0.44, gap: 0.018, rows: 5, bar: 0.04, scale: 0.98, margin: 0.0, text: 14.0, ink: 0.8, ..Layout::of(LayoutKind::Block) },
    )
}

/// The page's two layouts: a compact block the size of the game's large menus, centred (the
/// roster's panel and a live view of about the same width), or the player's whole part of the
/// screen (a wide roster panel, a narrower live view).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutKind {
    Block,
    Wide,
}

/// The roster panel's background: the game's multi_splitter picture or the Co-op box's frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanelPicture {
    Splitter,
    Coop,
}

/// The game's picture behind the roster (the multiplayer rules page's, read from the game's own
/// package at run time).
pub const SPLITTER: &str = "XIIIMenuStart.Multi_rules.multi_splitter";

/// A player's page: the block (panels and help bar) and what is in it. All shares and sizes
/// can be changed in the place file (see [`places`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    pub kind: LayoutKind,
    /// The block's width, as a share of the player's part of the screen.
    pub width: f32,
    /// Its widest shape (width / height; 0: as high as the player's part allows).
    pub aspect: f32,
    /// The roster panel's share of the block's width (it is widened if the longest line of the
    /// roster in the game's language would not fit).
    pub list: f32,
    /// The gap between the two panels, as a share of the block's width.
    pub gap: f32,
    /// Rows of the roster shown at once (at most; fewer when the panel is too low).
    pub rows: usize,
    /// Room around the character in the live view (1: head and feet at its edges).
    pub room: f32,
    pub picture: PanelPicture,
    /// How far the help bar reaches past the panels on each side (share of the block's width).
    pub bar: f32,
    /// Each player's live view a little wider on the right (share of the block's width): the
    /// panels of a comic page are rarely all of a size.
    pub view_extra: [f32; 2],
    /// The block's size multiplied by this, after it is made as large as the view allows.
    pub scale: f32,
    /// The margin around the block, as a share of the view's height and width.
    pub margin: f32,
    /// The text's height at the reference size (pixels); rows, gaps and paddings follow it.
    pub text: f32,
    /// The panels' ink border multiplied by this.
    pub ink: f32,
    /// The recorded view zoomed in the live-view panel, around its centre (1: as recorded).
    pub zoom: f32,
    /// The recorded view moved in the live-view panel, as a share of its width and height
    /// (positive: right and down), by an off-centre projection.
    pub shift: [f32; 2],
}

/// The sizes every layout starts from: the reference text and margin, no zoom or shift.
const LAYOUT_SIZES: Layout = Layout {
    kind: LayoutKind::Block,
    width: 0.83,
    aspect: 1.74,
    list: 0.44,
    gap: 0.012,
    rows: 5,
    room: 1.12,
    picture: PanelPicture::Splitter,
    bar: 0.04,
    view_extra: [0.0; 2],
    scale: 1.0,
    // The full page's text: the menu font's height at 1280 x 720.
    margin: 0.04,
    text: 23.0,
    ink: 1.0,
    zoom: 1.0,
    shift: [0.0; 2],
};

impl Layout {
    pub fn of(kind: LayoutKind) -> Layout {
        match kind {
            LayoutKind::Block => Layout { kind, width: 0.83, aspect: 1.74, list: 0.44, gap: 0.012, rows: 5, room: 1.12, picture: PanelPicture::Splitter, bar: 0.04, view_extra: [0.0; 2], ..LAYOUT_SIZES },
            LayoutKind::Wide => Layout { kind, width: 0.94, aspect: 0.0, list: 0.6, gap: 0.012, rows: 5, room: 1.12, picture: PanelPicture::Coop, bar: 0.0, view_extra: [0.0; 2], ..LAYOUT_SIZES },
        }
    }
}

/// Where and how the character screen's live view is drawn (client.rs): its panel (pixels within
/// the viewport), the horizontal field of view across it, the picture's move (normalised device
/// units, x right, y up) and the wider field of view the scene's visibility is worked out for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ScreenScene {
    pub panel: [i32; 4],
    pub fov: f32,
    pub lens: [f32; 2],
    pub cull_fov: f32,
}

/// The panel shows the recorded view's height of picture (vertical field of view `vfov`)
/// magnified by ViewZoom around its centre, then moved by ViewShiftX/Y of the panel's width and
/// height (positive: right and down) by an off-centre projection: nothing turns.
fn screen_scene(panel: [i32; 4], vfov: f32, layout: Layout) -> ScreenScene {
    let tan_v = (vfov.to_radians() / 2.0).dtan() / layout.zoom.max(0.01);
    let tan_h = tan_v * panel[2] as f32 / panel[3] as f32;
    let hfov = |t: f32| 2.0 * t.datan().to_degrees();
    // A device unit is half the panel: a share s of it is 2·s units (y up on the device).
    let lens = [2.0 * layout.shift[0], -2.0 * layout.shift[1]];
    let reach = 1.0 + lens[0].abs().max(lens[1].abs());
    ScreenScene { panel, fov: hfov(tan_h), lens, cull_fov: hfov(tan_h * reach).min(170.0) }
}

/// What the other players have (`per_player[me]` left out).
fn others<T: Copy>(per_player: &[Vec<T>], me: usize) -> Vec<T> {
    per_player.iter().enumerate().filter(|&(p, _)| p != me).flat_map(|(_, v)| v.iter().copied()).collect()
}

/// Default map of the screen.
pub const MAP: &str = "Hual02";

/// The soft hum looping over the screen (DM_Pal's room tone).
pub const SCREEN_AMBIENT: &str = "XIIIsound.Ambient.PalaceHum";

/// Default places: in Hual02, the storeroom with the crates (the owner's chosen spot, its camera
/// set as far from the character, as high and as tilted as in the daylight room he also liked),
/// the character facing the camera, the view exactly as recorded (the field of view of the game
/// alone on the screen). In split screen the second player has the same place: each half shows
/// only its own player's character.
pub fn default_places() -> Places {
    Places {
        map: MAP.to_string(),
        layout: default_layouts().0,
        split_layout: default_layouts().1,
        spot_turn: 0.0,
        fit: false,
        outfit_label: false,
        players: [
            Place { camera: [-7778.0, 3628.0, -83.0], camera_rotation: Some([-2892, -22471, 0]), fov: None, spot: [-7870.0, 3483.0, -143.0], facing: 10487 },
            Place { camera: [-7778.0, 3628.0, -83.0], camera_rotation: Some([-2892, -22471, 0]), fov: None, spot: [-7870.0, 3483.0, -143.0], facing: 10487 },
        ],
    }
}

/// File replacing the default places (set by the game window; None: the defaults).
static PLACE_FILE: std::sync::Mutex<Option<std::path::PathBuf>> = std::sync::Mutex::new(None);

/// Where the screen's places can be changed without rebuilding (see [`read_places`]).
pub fn set_place_file(path: Option<std::path::PathBuf>) {
    if let Ok(mut f) = PLACE_FILE.lock() {
        *f = path;
    }
}

fn place_file() -> Option<std::path::PathBuf> {
    PLACE_FILE.lock().ok().and_then(|f| f.clone())
}

/// `text` (a place file, possibly empty) with these keys set: a line already there is replaced
/// where it is (whatever its case), a new one is added at the end; comments and other lines stay.
pub fn set_place_lines(text: &str, entries: &[(&str, String)]) -> String {
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    if !lines.iter().any(|l| l.trim().eq_ignore_ascii_case("[CharacterScreen]")) {
        lines.insert(0, "[CharacterScreen]".into());
    }
    for (key, value) in entries {
        let line = format!("{key}={value}");
        let found = lines.iter().position(|l| l.split(';').next().unwrap_or("").split_once('=').is_some_and(|(k, _)| k.trim().eq_ignore_ascii_case(key)));
        match found {
            Some(i) => lines[i] = line,
            None => lines.push(line),
        }
    }
    let mut out = lines.join("\r\n");
    out.push_str("\r\n");
    out
}

/// A rotator component (Unreal units) between -32768 and 32767.
fn signed_angle(a: i32) -> i32 {
    ((a + 32768).rem_euclid(65536)) - 32768
}

/// What the recording keys save (improved build, [`Vm::character_place_key`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaceKey {
    /// F3: where the character stands and the way they face (the player's own).
    Spot,
    /// F4: the camera (the player's eyes and the way they look).
    Camera,
    /// F8: both forgotten for this map, and the preview character removed.
    Remove,
}

/// IK_F8: free in XIII too (DefUser.ini binds F1 and F12; F2 opens the console; F5, F6, F7, F11
/// are the test builds' cheats, F10 the window's menu).
pub const REMOVE_KEY: u8 = 0x77;

/// `text` (a place file) without the lines of these keys (whatever their case).
pub fn remove_place_lines(text: &str, keys: &[&str]) -> String {
    let mut out: String = text
        .lines()
        .filter(|l| !l.split(';').next().unwrap_or("").split_once('=').is_some_and(|(k, _)| keys.iter().any(|key| k.trim().eq_ignore_ascii_case(key))))
        .collect::<Vec<_>>()
        .join("\r\n");
    out.push_str("\r\n");
    out
}

/// IK_F3 and IK_F4: free in XIII (DefUser.ini binds nothing there; F5, F6, F7, F11 are the test
/// builds' cheats).
pub const SPOT_KEY: u8 = 0x72;
pub const CAMERA_KEY: u8 = 0x73;

/// The lines a recording key writes, for both players (each half of a split screen shows only
/// its own player's character, so both can stand on the same spot).
pub fn place_entries(map: &str, what: PlaceKey, location: [f32; 3], rotation: [i32; 3], vfov: f32) -> Vec<(&'static str, String)> {
    let [x, y, z] = location.map(|v| v.round() as i32);
    let value = match what {
        PlaceKey::Spot => format!("{x},{y},{z}"),
        PlaceKey::Camera => format!("{x},{y},{z},{},{},{:.1}", signed_angle(rotation[0]), signed_angle(rotation[1]), vfov),
        PlaceKey::Remove => return Vec::new(),
    };
    let (one, two) = match what {
        PlaceKey::Spot => ("Spot1", "Spot2"),
        _ => ("Camera1", "Camera2"),
    };
    vec![("Map", map.to_string()), (one, value.clone()), (two, value)]
}

/// The places: the defaults, with what the place file gives instead. The file is an ini text:
///
/// ```text
/// [CharacterScreen]
/// Map=Hual02
/// Camera1=-7778,3628,-83               ; x, y, z (it looks at the character by itself),
///                                       ; or x, y, z, pitch, yaw[, vertical field of view]
///                                       ; (exactly that view, as F4 records it)
/// Spot1=-7870,3483,-143                 ; x, y, z (the character faces the camera)
/// Camera2=-7778,3628,-83,-2892,-22471   ; (the default: its pitch and yaw as well)
/// Spot2=-7870,3483,-143
/// SpotTurn=0                            ; then turned by this (degrees, + to its left)
/// Fit=0                                 ; 1: a recorded view moves back to fit the character
/// Layout=block                          ; block (compact, centred) or wide (the whole part)
/// BlockWidth=0.83                       ; the block's width (share of the player's part),
/// BlockAspect=1.74                      ; its widest shape (width / height, 0: full height),
/// ListShare=0.44                        ; the roster panel's share of the block's width,
/// PanelGap=0.012                        ; the gap between the panels (share of the width),
/// Rows=5                                ; roster rows shown at once,
/// ViewRoom=1.12                         ; room around the character in the live view,
/// Picture=splitter                      ; the roster panel's picture: splitter or coop
/// BarOverhang=0.049                     ; the help bar past the panels on each side,
/// ViewExtra1=0                          ; and each player's live view a little wider
/// ViewExtra2=0                          ;   (shares of the block's width)
/// BlockScale=1                          ; the block's size, after it fills the view, times this
/// Margin=0.04                           ; the margin around it (share of the view)
/// TextSize=23                           ; the text's height at the reference size (pixels)
/// InkBorder=1                           ; the panels' ink border times this
/// ViewZoom=1                            ; the recorded view zoomed in its panel,
/// ViewShiftX=0                          ;   and moved (shares of the panel's width and
/// ViewShiftY=0                          ;   height, right and down)
/// ```
///
/// Layout gives the other keys their defaults for that layout; the keys after it change them.
/// A layout key applies alone on the screen and in split screen; "Solo." or "Split." before it
/// applies it to one only (Split.Layout=wide, Solo.ListShare=0.4...). By default: Layout=block
/// alone, Layout=wide with Picture=splitter in split screen.
///
/// Without a rotation, the camera looks at the character and comes nearer or goes farther along
/// its line to it so that the character fills the window; with one (as F4 records it, see
/// [`Vm::character_place_key`]), it is exactly the recorded view. The live-view panel shows
/// the height of picture of the recorded view (its vertical field of view, by default the
/// game's alone on the screen), its middle band across: what was framed is what is seen,
/// split screen or not (the co-op views' own field of view does not apply here).
pub fn places() -> Places {
    let mut p = default_places();
    if let Some(text) = place_file().and_then(|p| std::fs::read_to_string(p).ok()) {
        read_places(&text, &mut p);
    }
    face_cameras(&mut p);
    p
}

/// Each character faces its camera (yaw only, from the spot to the camera's position), turned
/// by SpotTurn: a facing written after a spot is not used.
pub fn face_cameras(p: &mut Places) {
    let turn = (p.spot_turn * 65536.0 / 360.0).round() as i32;
    for place in &mut p.players {
        place.facing = signed_angle(look_at(place.spot, place.camera)[1] + turn);
    }
}

/// The point at `distance` from `from` towards `to` (the camera's place on its line to the
/// character: the file's camera gives the direction and height, the framing the distance).
pub fn along(from: [f32; 3], to: [f32; 3], distance: f32) -> [f32; 3] {
    let d = [to[0] - from[0], to[1] - from[1], to[2] - from[2]];
    let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt().max(0.001);
    [0, 1, 2].map(|i| from[i] + d[i] * distance / len)
}

/// The direction a camera with this pitch and yaw (Unreal units) looks.
pub fn forward([pitch, yaw]: [i32; 2]) -> [f32; 3] {
    let units = std::f32::consts::TAU / 65536.0;
    let (p, y) = (pitch as f32 * units, yaw as f32 * units);
    [p.dcos() * y.dcos(), p.dcos() * y.dsin(), p.dsin()]
}

/// Pitch and yaw (Unreal units) of a camera at `from` looking at `to`.
pub fn look_at(from: [f32; 3], to: [f32; 3]) -> [i32; 2] {
    let (dx, dy, dz) = (to[0] - from[0], to[1] - from[1], to[2] - from[2]);
    let units = 65536.0 / std::f32::consts::TAU;
    let yaw = dy.datan2(dx) * units;
    let pitch = dz.datan2((dx * dx + dy * dy).sqrt()) * units;
    [pitch.round() as i32, yaw.round() as i32]
}

/// Lines of a place file applied to `p` (unknown or malformed lines are left out).
pub fn read_places(text: &str, p: &mut Places) {
    // The layouts first: their defaults, then the keys that change them, wherever they are.
    for line in text.lines() {
        let line = line.split(';').next().unwrap_or("").trim();
        let Some((key, value)) = line.split_once('=') else { continue };
        let key = key.trim().to_ascii_lowercase();
        let (solo, split, key) = layout_target(&key);
        if key == "layout" {
            let kind = match value.trim().to_ascii_lowercase().as_str() {
                "block" => LayoutKind::Block,
                "wide" => LayoutKind::Wide,
                _ => continue,
            };
            if solo {
                p.layout = Layout::of(kind);
            }
            if split {
                p.split_layout = Layout::of(kind);
            }
        }
    }
    for line in text.lines() {
        let line = line.split(';').next().unwrap_or("").trim();
        let Some((key, value)) = line.split_once('=') else { continue };
        let (key, value) = (key.trim().to_ascii_lowercase(), value.trim());
        let numbers: Vec<f32> = value.split(',').filter_map(|v| v.trim().parse().ok()).collect();
        let player = |k: &str| k.chars().last().and_then(|c| c.to_digit(10)).filter(|&n| (1..=2).contains(&n)).map(|n| n as usize - 1);
        match key.as_str() {
            "map" if !value.is_empty() => p.map = value.to_string(),
            k if k.starts_with("camera") && numbers.len() >= 3 => {
                if let Some(i) = player(k) {
                    p.players[i].camera = [numbers[0], numbers[1], numbers[2]];
                    // A position alone: the camera looks at the character (not the default's way).
                    p.players[i].camera_rotation = (numbers.len() >= 5).then(|| [numbers[3] as i32, numbers[4] as i32, 0]);
                    p.players[i].fov = numbers.get(5).copied().filter(|f| (5.0..=170.0).contains(f));
                }
            }
            k if layout_target(k).2 != "layout" && set_layout_key(&mut Layout::of(LayoutKind::Block), layout_target(k).2, value, &numbers) => {
                let (solo, split, k) = layout_target(k);
                if solo {
                    set_layout_key(&mut p.layout, k, value, &numbers);
                }
                if split {
                    set_layout_key(&mut p.split_layout, k, value, &numbers);
                }
            }
            "fit" if numbers.len() == 1 => p.fit = numbers[0] != 0.0,
            "outfitlabel" => p.outfit_label = value.eq_ignore_ascii_case("small"),
            "spotturn" if numbers.len() == 1 && (-180.0..=180.0).contains(&numbers[0]) => p.spot_turn = numbers[0],
            k if k.starts_with("spot") && numbers.len() >= 3 => {
                if let Some(i) = player(k) {
                    p.players[i].spot = [numbers[0], numbers[1], numbers[2]];
                    if let Some(&yaw) = numbers.get(3) {
                        p.players[i].facing = yaw as i32;
                    }
                }
            }
            _ => {}
        }
    }
}

/// Which layouts a key of the place file is for (alone on the screen, in split screen) and the
/// key without its "solo." or "split." prefix.
fn layout_target(key: &str) -> (bool, bool, &str) {
    if let Some(k) = key.strip_prefix("solo.") {
        (true, false, k)
    } else if let Some(k) = key.strip_prefix("split.") {
        (false, true, k)
    } else {
        (true, true, key)
    }
}

/// A layout key (lower case, without prefix) applied to `l`; false when it is not one or its
/// value is out of range.
fn set_layout_key(l: &mut Layout, key: &str, value: &str, numbers: &[f32]) -> bool {
    let one = (numbers.len() == 1).then(|| numbers[0]);
    match (key, one) {
        ("blockwidth", Some(v)) if v > 0.2 && v <= 1.0 => l.width = v,
        ("blockaspect", Some(v)) if v == 0.0 || (0.5..=8.0).contains(&v) => l.aspect = v,
        ("listshare", Some(v)) if (0.15..=0.85).contains(&v) => l.list = v,
        ("panelgap", Some(v)) if (0.0..=0.2).contains(&v) => l.gap = v,
        ("rows", Some(v)) if (1.0..=20.0).contains(&v) => l.rows = v as usize,
        ("viewroom", Some(v)) if (0.8..=4.0).contains(&v) => l.room = v,
        ("baroverhang", Some(v)) if (0.0..=0.2).contains(&v) => l.bar = v,
        ("viewextra1", Some(v)) if (0.0..=0.2).contains(&v) => l.view_extra[0] = v,
        ("viewextra2", Some(v)) if (0.0..=0.2).contains(&v) => l.view_extra[1] = v,
        ("blockscale", Some(v)) if (0.3..=1.0).contains(&v) => l.scale = v,
        ("margin", Some(v)) if (0.0..=0.2).contains(&v) => l.margin = v,
        ("textsize", Some(v)) if (6.0..=64.0).contains(&v) => l.text = v,
        ("inkborder", Some(v)) if (0.0..=4.0).contains(&v) => l.ink = v,
        ("viewzoom", Some(v)) if (0.25..=4.0).contains(&v) => l.zoom = v,
        ("viewshiftx", Some(v)) if (-1.0..=1.0).contains(&v) => l.shift[0] = v,
        ("viewshifty", Some(v)) if (-1.0..=1.0).contains(&v) => l.shift[1] = v,
        ("picture", _) => match value.to_ascii_lowercase().as_str() {
            "splitter" => l.picture = PanelPicture::Splitter,
            "coop" => l.picture = PanelPicture::Coop,
            _ => return false,
        },
        _ => return false,
    }
    true
}

/// One way a character can look: the model, its skin (None: the model's own), the pose played on
/// the screen and the one played once the player confirms their choice, the first-person arms
/// recipe (name of an `[arms.<Name>]` table; None: XIII's own arms), and the variant's name (key
/// of added_text::variant_name; empty for a character with a single look).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Variant {
    pub mesh: &'static str,
    pub skin: Option<&'static str>,
    pub pose: Pose,
    pub confirm: Pose,
    pub arms: Option<&'static str>,
    pub name: &'static str,
}

/// A character of the roster and their variants (outfits of the same person, or other models of
/// the same kind: each killer, each policeman).
#[derive(Clone, Copy, Debug)]
pub struct Character {
    /// Stable identifier (kept between runs, sent over the network).
    pub id: &'static str,
    pub variants: &'static [Variant],
}

const fn v(mesh: &'static str, skin: Option<&'static str>, pose: Pose, confirm: Pose, arms: Option<&'static str>, name: &'static str) -> Variant {
    Variant { mesh, skin, pose, confirm, arms, name }
}

/// How a character stands on the screen. Every field but the sequence is optional; a plain
/// looped sequence is what the screen has always done.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    /// An animation file linked after the model's own (LinkSkelAnim; the model's common file
    /// stays first: the engine takes the first sequence of a name), e.g. XIIIPersos.CARRINGTONSpeA.
    pub anims: Option<&'static str>,
    /// The pose's sequence and its rate (1: as made, 30 frames a second).
    pub sequence: &'static str,
    pub rate: f32,
    /// Played once and held on its last frame (otherwise looped).
    pub hold: bool,
    /// Played first, then blended into the pose.
    pub entry: Option<Entry>,
    /// An object in the character's hand (or on another bone) while the pose plays.
    pub prop: Option<Prop>,
    /// A weapon held while the pose plays: the game's weapon class (XIII.Beretta), held as the
    /// game holds it on a character (its third-person attachment, bone and offsets).
    pub weapon: Option<&'static str>,
    /// The character turns this far while in the pose (degrees, + to its left).
    pub turn: f32,
    /// Played up to this share of the sequence, then held there (None: all of it).
    pub stop: Option<f32>,
    /// The weapon comes into the hand only once this sequence (the pose's way in or the pose
    /// itself) reaches this share of it: a draw from the holster.
    pub draw: Option<(&'static str, f32)>,
    /// Played on the upper body only (from "X Spine", as the game layers its weapon animations
    /// over the walk): the legs keep the pose shown before.
    pub upper: bool,
    /// With an upper-body pose: a bone outside the spine's branch played with it (and its own
    /// branch), on a second channel: the policemen's baton hangs from the pelvis.
    pub upper_also: Option<&'static str>,
    /// Only this part of the sequence (shares of it), played in a loop (a pose played once:
    /// from its start share, held at its end).
    pub segment: Option<(f32, f32)>,
    /// With an upper-body pose: the bone it starts from (None: "X Spine").
    pub upper_root: Option<&'static str>,
    /// With a draw: the weapon in the hand until the draw is reached (the one put away).
    pub put_away: Option<&'static str>,
    /// The blend into this pose, when not the screen's own (seconds).
    pub blend: Option<f32>,
    /// The sequence is taken from the pose's file even when the model's own file has one of that
    /// name (the engine takes the first it finds).
    pub own_file: bool,
}

/// A pose's way in: `sequence` (looped) for `seconds`, then `blend` seconds into the pose.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Entry {
    pub sequence: &'static str,
    pub seconds: f32,
    pub blend: f32,
    /// The file the way in is in (linked like the pose's), when not the model's or the pose's.
    pub anims: Option<&'static str>,
    /// Entered at this share of its sequence (its still start skipped).
    pub start: f32,
}

/// A static mesh on a bone of the character (e.g. XIDCine.CineCigareSM on "X R Hand").
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Prop {
    pub mesh: &'static str,
    pub bone: &'static str,
}

impl Pose {
    /// `sequence` looped at its own rate, nothing else.
    pub const fn looped(sequence: &'static str) -> Pose {
        Pose { anims: None, sequence, rate: 1.0, hold: false, entry: None, prop: None, weapon: None, turn: 0.0, stop: None, draw: None, upper: false, upper_also: None, segment: None, upper_root: None, put_away: None, blend: None, own_file: false }
    }

    /// `sequence` played once and held on its last frame.
    pub const fn once(sequence: &'static str) -> Pose {
        Pose { hold: true, ..Pose::looped(sequence) }
    }

    /// The sequence is in this animation file (linked after the model's own).
    pub const fn from(self, anims: &'static str) -> Pose {
        Pose { anims: Some(anims), ..self }
    }

    /// With this weapon in the hand.
    pub const fn holding(self, weapon: &'static str) -> Pose {
        Pose { weapon: Some(weapon), ..self }
    }

    /// Turned this far (degrees, + to the character's left).
    pub const fn turned(self, degrees: f32) -> Pose {
        Pose { turn: degrees, ..self }
    }

    /// Played up to `share` of the sequence, then held.
    pub const fn until(self, share: f32) -> Pose {
        Pose { stop: Some(share), ..self }
    }

    /// `sequence` played once first (`seconds`), then blended into the pose.
    pub const fn after(self, sequence: &'static str, seconds: f32, blend: f32) -> Pose {
        Pose { entry: Some(Entry { sequence, seconds, blend, anims: None, start: 0.0 }), ..self }
    }

    /// `sequence` of the file `anims` played once first (`seconds`), then blended into the pose.
    pub const fn after_from(self, anims: &'static str, sequence: &'static str, seconds: f32, blend: f32) -> Pose {
        Pose { entry: Some(Entry { sequence, seconds, blend, anims: Some(anims), start: 0.0 }), ..self }
    }

    /// The way in entered at `share` of its sequence.
    pub const fn entered_at(self, share: f32) -> Pose {
        match self.entry {
            Some(e) => Pose { entry: Some(Entry { start: share, ..e }), ..self },
            None => self,
        }
    }

    /// At this rate (1: as made).
    pub const fn at_rate(self, rate: f32) -> Pose {
        Pose { rate, ..self }
    }

    /// With this static mesh (or cut-scene prop class) in the right hand.
    pub const fn with_prop(self, mesh: &'static str) -> Pose {
        Pose { prop: Some(Prop { mesh, bone: "X R Hand" }), ..self }
    }

    /// The weapon comes into the hand once `sequence` reaches `share` of it.
    pub const fn drawn_at(self, sequence: &'static str, share: f32) -> Pose {
        Pose { draw: Some((sequence, share)), ..self }
    }

    /// Only the part of the sequence between these shares, looped.
    pub const fn segment(self, from: f32, to: f32) -> Pose {
        Pose { segment: Some((from, to)), ..self }
    }

    /// The sequence of the pose's file, not the model's of the same name.
    pub const fn from_its_file(self) -> Pose {
        Pose { own_file: true, ..self }
    }

    /// Blended into over `seconds` (instead of the screen's blend).
    pub const fn blended(self, seconds: f32) -> Pose {
        Pose { blend: Some(seconds), ..self }
    }

    /// With a draw: `weapon` stays in the hand until then (put away for the new one).
    pub const fn putting_away(self, weapon: &'static str) -> Pose {
        Pose { put_away: Some(weapon), ..self }
    }

    /// On the upper body only, over the pose shown before.
    pub const fn upper_body(self) -> Pose {
        Pose { upper: true, ..self }
    }

    /// Only from `bone` up (e.g. "X Neck": the head alone), over the pose shown before.
    pub const fn from_bone(self, bone: &'static str) -> Pose {
        Pose { upper: true, upper_root: Some(bone), ..self }
    }

    /// On the upper body and on `bone`'s branch (a bone outside the spine's).
    pub const fn upper_body_and(self, bone: &'static str) -> Pose {
        Pose { upper: true, upper_also: Some(bone), ..self }
    }

    /// The pose as it is taken back to after a confirmation: without its way in.
    pub const fn without_entry(self) -> Pose {
        Pose { entry: None, ..self }
    }
}

/// How long a confirmation (or its taking back) blends in from the pose before (seconds).
pub const CONFIRM_BLEND: f32 = 0.5;

/// The channels a pose plays on.
fn screen_pose_channels(pose: &Pose) -> Vec<usize> {
    match (pose.upper, pose.upper_also) {
        (false, _) => vec![0],
        (true, None) => vec![UPPER_CHANNEL as usize],
        (true, Some(_)) => vec![UPPER_CHANNEL as usize, UPPER_ALSO_CHANNEL as usize],
    }
}

/// The channel of the poses played on the upper body only, and of the bone played with them.
const UPPER_CHANNEL: i32 = 1;
const UPPER_ALSO_CHANNEL: i32 = 2;

/// DrawType of a weapon in the hand (DT_StaticMesh) and of one put away (DT_None).
const DT_STATIC_MESH: u8 = 8;
const DT_NONE: u8 = 0;

/// The game's handgun (Jones, Kim, Amos) and assault rifle (the SPADS, the GI, McCall).
pub const PISTOL: &str = "XIII.Beretta";
pub const RIFLE: &str = "XIII.M16";
/// The Magnum (XIII, the killers, the policemen).
pub const MAGNUM: &str = "XIII.Magnum";

/// Two worked examples of the richer poses (not the roster's choices): Carrington smoking his
/// cigar, coming from standing still; Jones holding a long weapon.
pub const POSE_EXAMPLES: [(&str, &str, Pose); 2] = [
    ("Carrington", "", Pose::looped("Fume1").from("XIIIPersos.CARRINGTONSpeA").with_prop(CIGAR).after("WaitNeutre", 1.2, 0.4)),
    ("Jones", "blouse", Pose::looped("WaitFusil").with_prop("MeshArmesPickup.M16").after("WaitNeutre", 1.0, 0.35)),
];

pub fn screen_pose(character: &str, variant: &Variant) -> Pose {
    let _ = character;
    variant.pose
}

pub fn screen_confirm_pose(variant: &Variant) -> Pose {
    variant.confirm
}

/// How long the game waits at most once everyone confirmed before it starts (seconds): the
/// owner's choice, 14/11: 1 s at most; it starts sooner once every confirmation animation has
/// finished (confirm_pose_finished).
pub const CONFIRM_START_DELAY: f64 = 1.0;

/// When the game starts after the last confirmation at `clock` (screen time, seconds).
pub fn start_time(clock: f64) -> f64 {
    clock + CONFIRM_START_DELAY
}

/// What a confirmation's ending needs to know of one animation channel: its sequence, frame,
/// last frame and blend (the channel's AnimSequence, AnimFrame, AnimLast, blend alpha).
#[derive(Clone, Copy, Debug)]
pub struct ChannelView<'a> {
    pub sequence: &'a str,
    pub frame: f32,
    pub last: f32,
    pub alpha: f32,
}

/// The confirmation `pose` has finished on its channels (`channels`, as screen_pose_channels
/// lists them; None: a channel that cannot be read): played once and held at its end (or at
/// its stop), its way in over, an upper-body pose fully blended in. A looped pose never
/// finishes; a channel that cannot be read, or shows another sequence, counts as not finished.
pub fn confirm_pose_finished(pose: &Pose, channels: &[Option<ChannelView>]) -> bool {
    if !(pose.hold || pose.stop.is_some()) || channels.is_empty() {
        return false;
    }
    channels.iter().all(|c| {
        c.is_some_and(|c| {
            let end = match (pose.stop, pose.segment) {
                (Some(stop), _) => c.frame >= stop - 1e-3,
                (None, Some((_, to))) => c.frame >= to - 1e-3,
                (None, None) => c.last <= c.frame,
            };
            c.sequence.eq_ignore_ascii_case(pose.sequence) && c.frame >= 0.0 && end && (!pose.upper || c.alpha >= 0.99)
        })
    })
}

/// A yaw (Unreal units) moved towards `target` by at most `step`, the short way round.
pub fn turn_towards(yaw: i32, target: i32, step: i32) -> i32 {
    let d = signed_angle(target - yaw);
    signed_angle(yaw + d.clamp(-step, step))
}

/// A turn in degrees (+ to the character's left) in Unreal yaw units.
pub fn turn_units(degrees: f32) -> i32 {
    (degrees * 65536.0 / 360.0).round() as i32
}

/// Pose of the screen when a model has none of its own: standing still, the game's idle.
pub const DEFAULT_POSE: &str = "WaitNeutre";

// The roster's poses: every sequence is one of the game's own (its characters' common files,
// XIIIPersosG.MigA for the men and XIIIPersosG.JonesMajA for Jones and Kim, or one of their
// cut-scene files, XIIIPersos.*speA, linked on the screen).

/// Standing still, the game's idle.
const STILL: Pose = Pose::looped(DEFAULT_POSE);
/// The cut-scenes' cigar (the game has no cigarette model; its smoking soldiers of SPADS02a play
/// these sequences with nothing in the hand).
pub const CIGAR: &str = "XIDCine.CineCigareSM";
/// Smoking: Carrington's Fume1 (CARRINGTONSpeA) with the cigar in the hand.
const SMOKING: Pose = Pose::looped("Fume1").from("XIIIPersos.CARRINGTONSpeA").with_prop(CIGAR);
/// Handgun held low (the game's guards' stance), and aimed with both hands: the start of the
/// game's handgun draw (PistoletRetour), held where it points ahead.
const PISTOL_ALERT: Pose = Pose::once("PistoletRetour").holding(PISTOL).until(0.4).turned(AIM_TURN);
/// Assault rifle held across the body, and aimed.
const RIFLE_WAIT: Pose = Pose::looped("WaitFusil").holding(RIFLE);
const RIFLE_TAKEN: Pose = RIFLE_WAIT.drawn_at("SearchGun", 0.25).after("SearchGun", 0.45, 0.25).blended(0.35);
/// The military salute of the base's officers and soldiers, held with the hand at the brow.
const fn salute(anims: &'static str) -> Pose {
    Pose::once("salut").from(anims).until(0.55)
}
/// McCall's handgun draw (MCCALLspeA "degaine"): the hand to the holster on the belt, the gun
/// out and aimed with one hand, held there. The gun comes into the hand once the hand is behind
/// the hip, hidden by the body (frame 24 of 51).
const DRAW_AT: f32 = 0.47;
/// The draw's first frames are still (the hand rests): it is entered where the hand starts moving
/// (frame 9 of 51), faded in quickly over the whole body.
const DRAW_START: f32 = 0.18;
const fn draw(weapon: &'static str) -> Pose {
    Pose::once("degaine").from("XIIIPersos.MCCALLspeA").holding(weapon).drawn_at("degaine", DRAW_AT).segment(DRAW_START, 1.0).from_bone("X").blended(0.15)
}
/// XIII (every outfit): arms crossed (standing still in the colonel's uniform and in torn
/// clothes); confirmed, he
/// draws the Magnum (McCall's draw, the gun appearing as his hand is behind him at the holster)
/// and aims with both hands, slightly to the side.
const XIII_IDLE: Pose = Pose::looped("WaitNeutreBrasCroises");
const XIII_CONFIRM: Pose = Pose::once("PistoletRetour")
    .from("XIIIPersos.MCCALLspeA")
    .holding(MAGNUM)
    .until(0.4)
    .turned(SMALL_TURN)
    .drawn_at("degaine", DRAW_AT)
    .after("degaine", 0.55, 0.25)
    .entered_at(DRAW_START)
    .from_bone("X")
    .blended(0.15);
/// Jones shown: her handgun held low (WaitPistolet). Confirmed: the handgun put away and the assault rifle taken
/// out as the player changes weapon (SearchGun: the gun changes when the hand is down), then held
/// (WaitFusil).
const JONES_IDLE: Pose = Pose::looped("WaitNeutre");
/// Confirmed: the assault rifle taken from her back as the player switches to a long gun
/// (SearchGun: the hand goes over her shoulder, the rifle appears there), then held. The
/// sequence starts and ends holding the previous gun: it is entered where the hand starts going
/// up (frame 3 of 21), faded in on a channel over the whole body (root included: her dress
/// pose leans the root) so that nothing jumps, quickly: the hand must be up behind her shoulder,
/// whatever her idle, when the rifle appears.
const JONES_M16: Pose = Pose::once("SearchGun").holding(RIFLE).drawn_at("SearchGun", 0.38).segment(0.14, 1.0).from_bone("X").turned(JONES_TURN).blended(0.15);
/// Confirmed, Jones turns between front and three-quarter view (her best side).
const JONES_TURN: f32 = -22.0;
/// A two-handed aim turned this far stays in the picture.
const SMALL_TURN: f32 = -15.0;
/// Who aims does it slightly to the side, never at the camera: the way Amos's draw ends (one
/// hand, the arm to the character's right). A long gun is turned that way.
const AIM_TURN: f32 = -30.0;
/// The killers draw a Magnum as Amos draws.
/// Both arms raised wide, the robe spread: the start of the Klan's speech (KKKspeA
/// dialogue1KKK, from just before the arms rise: the slow start cut), held there. Played on a
/// channel over the whole body (from the pelvis: the root stays where the idle has it) that fades
/// in: starting inside the sequence
/// would otherwise jump.
const KLAN_ARMS: Pose = Pose::once("dialogue1KKK").from("XIIIPersos.KKKspeA").until(0.33).segment(0.04, 0.33).from_bone("X Pelvis");
/// The FBI agents' hand to the earpiece (FbiOreillette), held there.
const EARPIECE: Pose = Pose::once("FbiOreillette").from("XIIIPersos.FBIspeA").until(0.4);
/// His boxing taunt, fists up.
const FISTS: Pose = Pose::looped("MigBoxeProvoc");
/// The policemen: standing with their baton (the baton is part of their model); confirmed, they
/// draw a Magnum.
const BATON_WAIT: Pose = Pose::looped("WaitMatrak").from("XIIIPersos.GardienSpeA");
/// Kept for testing: only the taps of Matrak3, looped, on the upper body and the baton.
#[allow(dead_code)]
const BATON: Pose = Pose::looped("Matrak3").from("XIIIPersos.GardienSpeMatrakA").upper_body_and("X Matrac").segment(0.43, 0.56);

/// Every character whose first-person arms were made (coop-arms/*.toml), XIII first. A
/// variant without its own arms recipe is here only when its sleeves are those of an existing
/// recipe of the same character (XIII's bandages: his own jacket; Jones's evening dress: her
/// bare arms).
pub const ROSTER: &[Character] = &[
    Character {
        id: "XIII",
        variants: &[
            v("XIIIPersos.XIIIM", None, XIII_IDLE, XIII_CONFIRM, None, "jacket"),
            v("XIIIPersos.XIIIBandagesM", None, XIII_IDLE, XIII_CONFIRM, None, "bandages"),
            // His two other models of the game: the military one (multiplayer) and his disguise as
            // a general (SSH101a), who salutes.
            v("XIIIPersos.XIIIMilitM", None, XIII_IDLE, XIII_CONFIRM, None, "military"),
            v("XIIIPersos.XIIIGalM", None, STILL, XIII_CONFIRM, None, "general"),
            // A model of his the game never uses (no map or script names it), clothes torn.
            v("XIIIPersos.XIIITatooM", Some("XIIIPersos.Skins.XIIITatooTex"), STILL, XIII_CONFIRM, None, "torn"),
        ],
    },
    Character {
        id: "Jones",
        variants: &[
            v("XIIIPersos.JonesM", Some("XIIIPersos.Skins.JonesTex"), JONES_IDLE, JONES_M16, Some("Jones"), "blouse"),
            // Showing off her dress (SoireeSpeA Jolierobe, her line about it), ending arms crossed.
            v("XIIIPersos.SoireeM", Some("XIIIPersos.Skins.JonesSoireeTex"), Pose::once("Jolierobe").from("XIIIPersos.SoireeSpeA"), JONES_M16, Some("Jones"), "evening_dress"),
            // Her camouflage, the outfit of most of her levels (Amos01, Hual04c, SSH102b, Toits01):
            // shown first (DEFAULT_VARIANTS); last in the list so that the earlier codes stay.
            v("XIIIPersosG.JonesMajM", Some("XIIIPersosG.Skins.JonesMajTex"), JONES_IDLE, JONES_M16, Some("Jones"), "camouflage"),
        ],
    },
    Character { id: "Carrington", variants: &[v("XIIIPersos.CarringtonM", Some("XIIIPersos.Skins.CarringtonTex"), SMOKING, FISTS, Some("Carrington"), "")] },
    Character {
        id: "Mongoose",
        // His doubt; confirmed, he bursts out laughing (Johansson's eclatrire, also played by
        // killers in Kello01a).
        variants: &[v(
            "XIIIPersos.MangousteM",
            Some("XIIIPersos.Skins.MangousteTex"),
            Pose::looped("Doute").from("XIIIPersos.MANGOUSTEspeA"),
            Pose::once("eclatrire").from("XIIIPersos.JOHANSSONspeA"),
            Some("Mongoose"),
            "",
        )],
    },
    // Her own wait (kimspeA); confirmed, her handgun aimed with both hands (the women's files
    // have no draw from a holster).
    Character { id: "Kim", variants: &[v("XIIIPersos.KimM", Some("XIIIPersos.Skins.KimTex"), Pose::looped("KimWait").from("XIIIPersos.kimspeA"), PISTOL_ALERT, Some("Kim"), "")] },
    Character {
        id: "Amos",
        variants: &[
            v("XIIIPersos.AmosM", Some("XIIIPersos.Skins.AmosTex"), Pose::looped("Wait").from("XIIIPersos.AMOSspeA"), draw(PISTOL), Some("Amos"), "burgundy_suit"),
            v("XIIIPersos.AmosM", Some("XIIIPersos.Skins.AmosChicTEX"), Pose::looped("Wait").from("XIIIPersos.AMOSspeA"), draw(PISTOL), Some("AmosCream"), "cream_suit"),
        ],
    },
    // His own wait, hands at the belt; confirmed, the assault rifle taken out and held as the
    // SPADS do (the owner's choice, 27/11: the M16 rather than the M60).
    Character { id: "McCall", variants: &[v("XIIIPersos.McCallM", Some("XIIIPersos.Skins.McCallTex"), Pose::looped("Wait").from("XIIIPersos.MCCALLspeA"), RIFLE_TAKEN, Some("McCall"), "")] },
    Character {
        id: "Sheridan",
        variants: &[
            v("XIIIPersos.SheridanM", Some("XIIIPersos.Skins.SheridanTex"), STILL, FISTS, Some("SheridanGrey"), "grey_suit"),
            v("XIIIPersos.SheridanM", Some("XIIIPersos.Skins.SheridanChicTex"), STILL, FISTS, Some("Sheridan"), "tuxedo"),
        ],
    },
    Character {
        id: "Standwell",
        variants: &[
            v("XIIIPersos.StandwellM", Some("XIIIPersos.Skins.StandwellTex"), STILL, salute("XIIIPersos.STANDWELLspeA"), Some("Standwell"), "uniform"),
            // In the Klan's robe he does as the Conspirator.
            v("XIIIPersos.StandKM", Some("XIIIPersos.Skins.StandKTex"), STILL, KLAN_ARMS, Some("StandwellRobe"), "robe"),
        ],
    },
    Character { id: "Admiral", variants: &[v("XIIIPersos.PachaM", Some("XIIIPersos.Skins.PachaTex"), STILL, salute("XIIIPersos.STANDWELLspeA"), Some("Admiral"), "")] },
    Character {
        id: "Spads",
        variants: &[
            // The skins the first-person arms were made from: the model's own (grey sleeves) and
            // the second jungle skin (SPADS02a, SPADS02b, USA01).
            v("XIIIPersos.SpadsM", None, STILL, RIFLE_TAKEN, Some("Spads"), "standard"),
            v("XIIIPersos.SpadsM", Some("XIIIPersos.Skins.SpadsJungleSkin2TEX"), STILL, RIFLE_TAKEN, Some("SpadsJungle"), "jungle"),
        ],
    },
    // The GI salutes, held; confirmed, back to standing, then his rifle taken out and held.
    Character { id: "GI", variants: &[v("XIIIPersos.GiM", Some("XIIIPersos.Skins.GITex"), salute("XIIIPersos.GIspeA"), RIFLE_TAKEN, Some("GI"), "")] },
    // A hand to the earpiece; confirmed, the handgun drawn and aimed.
    Character {
        id: "FBI",
        variants: &[
            v("XIIIPersos.FbiM", Some("XIIIPersos.Skins.FBITex"), EARPIECE, draw(PISTOL), Some("FBI"), "jacket"),
            v("XIIIPersos.FBI2M", Some("XIIIPersos.Skins.FBI2Tex"), EARPIECE, draw(PISTOL), Some("FBI2"), "rolled_sleeves"),
        ],
    },
    Character {
        id: "Killer",
        variants: &[
            v("XIIIPersos.Tueur1M", Some("XIIIPersos.Skins.Tueur1Tex"), STILL, draw(MAGNUM), Some("Killer1"), "killer1"),
            v("XIIIPersos.Tueur2M", Some("XIIIPersos.Skins.Tueur2Tex"), STILL, draw(MAGNUM), Some("Killer2"), "killer2"),
            v("XIIIPersos.Tueur3M", Some("XIIIPersos.Skins.Tueur3Tex"), STILL, draw(MAGNUM), Some("Killer3"), "killer3"),
            v("XIIIPersos.Tueur5M", Some("XIIIPersos.Skins.Tueur5Tex"), STILL, draw(MAGNUM), Some("Killer5"), "killer4"),
        ],
    },
    // Confirmed: both arms raised wide, his robe spread (the start of his speech in the Klan's
    // cut-scene, held there).
    Character {
        id: "Conspirator",
        variants: &[v("XIIIPersos.KKKM", Some("XIIIPersos.Skins.KKKTex"), STILL, KLAN_ARMS, Some("Conspirator"), "")],
    },
    Character {
        id: "Policeman",
        variants: &[
            v("XIIIPersos.GardienM", Some("XIIIPersos.Skins.PolicierTex"), BATON_WAIT, draw(MAGNUM), Some("ThinPoliceman"), "cop"),
            v("XIIIPersos.GroflicM", Some("XIIIPersos.Skins.GroflicTex"), BATON_WAIT, draw(MAGNUM), Some("Policeman"), "fat_cop"),
        ],
    },
];

/// The roster entries whose variants are different people the game names (its policemen: "Flic",
/// "Gros flic"): the row is named after the variant. The killers have no names in the game: all
/// four are "Tueur". The others' variants are outfits of the same
/// person: the row keeps the person's name and the outfit is named on the character's picture.
pub const PEOPLE_GROUPS: &[&str] = &["Policeman"];

/// The order the left/right keys show these roster entries' variants in, the first one by
/// default (the others: their list order). The list order itself never changes: a choice's code
/// (the network's, the saved one's) is the variant's place in the list.
pub const VARIANT_ORDER: &[(&str, &[&str])] =
    &[("XIII", &["jacket", "military", "general", "bandages", "torn"]), ("Jones", &["camouflage", "blouse", "evening_dress"])];

/// The variants of roster entry `character` (places in its list) in the order they are shown.
pub fn variant_order(character: usize) -> Vec<usize> {
    let Some(c) = ROSTER.get(character) else { return Vec::new() };
    let mut order: Vec<usize> = VARIANT_ORDER
        .iter()
        .find(|(id, _)| *id == c.id)
        .map_or(Vec::new(), |(_, names)| names.iter().filter_map(|n| c.variants.iter().position(|v| v.name == *n)).collect());
    for i in 0..c.variants.len() {
        if !order.contains(&i) {
            order.push(i);
        }
    }
    order
}

/// The variant each roster entry shows before the player changes it.
pub fn default_variants() -> Vec<usize> {
    (0..ROSTER.len()).map(|i| variant_order(i).first().copied().unwrap_or(0)).collect()
}

/// True when the variants of roster entry `character` are different people (see [`PEOPLE_GROUPS`]).
pub fn variants_are_people(character: usize) -> bool {
    ROSTER.get(character).is_some_and(|c| PEOPLE_GROUPS.contains(&c.id))
}

/// Index in [`ROSTER`] and variant of a choice.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Choice {
    pub character: usize,
    pub variant: usize,
}

impl Choice {
    pub fn variant_data(self) -> Option<&'static Variant> {
        ROSTER.get(self.character)?.variants.get(self.variant)
    }

    /// "Id" or "Id.2" (second variant), as kept between runs and sent to a host.
    pub fn code(self) -> String {
        let id = ROSTER.get(self.character).map_or("XIII", |c| c.id);
        if self.variant == 0 {
            id.to_string()
        } else {
            format!("{id}.{}", self.variant + 1)
        }
    }

    pub fn from_code(code: &str) -> Option<Choice> {
        let (id, variant) = match code.trim().split_once('.') {
            Some((id, n)) => (id, n.parse::<usize>().ok()?.checked_sub(1)?),
            None => (code.trim(), 0),
        };
        let character = ROSTER.iter().position(|c| c.id.eq_ignore_ascii_case(id))?;
        (variant < ROSTER[character].variants.len()).then_some(Choice { character, variant })
    }
}

/// The screen's actors: one per roster variant and per player, all spawned when it opens.
#[derive(Default)]
pub struct Screen {
    /// On for the map being played (its URL has the option).
    pub on: bool,
    pub places: Option<Places>,
    /// Set up (actors spawned, level paused) on the first frame where the players exist.
    ready: bool,
    /// Per player: actor of each (character, variant), in roster order.
    actors: Vec<Vec<Option<ObjId>>>,
    /// Per player: the choice shown.
    pub shown: Vec<Choice>,
    /// Time taken to set the screen up (spawning every actor), in ms.
    pub setup_ms: f64,
    /// Time since the screen opened (frames' time, also while the level is paused).
    pub clock: f64,
    /// Per player: the screen's menu page, and the player whose page is being opened.
    pub(crate) pages: Vec<Option<ObjId>>,
    pub(crate) opening: Option<usize>,
    /// Per player: confirmed ("ready"), and the variant last shown of each character.
    pub(crate) confirmed: Vec<bool>,
    pub(crate) variants: Vec<Vec<usize>>,
    /// Per player: where the page drew its rows and buttons (pixels), for the mouse.
    pub(crate) hits: Vec<Vec<([f32; 4], crate::character_page::Hit)>>,
    /// Everyone confirmed: the held travel goes (character_page.rs).
    pub(crate) go: bool,
    /// The first player went back: to the main menu's Co-op page.
    pub(crate) back: bool,
    /// Kept from one map to the next: the game's own travel held while the screen is shown, and
    /// the Co-op page to open again on the main menu after going back.
    pub(crate) held: Option<crate::character_page::Held>,
    pub(crate) reopen_coop: Option<crate::local_play::CoopMode>,
    /// The save slot just read (Continue, Load): its co-op characters (coop_save.rs), Some(None)
    /// for a slot without them; taken by the travel that follows.
    pub(crate) slot_characters: Option<Option<[Choice; 2]>>,
    /// Everyone confirmed: when the held travel goes ([`start_time`]).
    pub(crate) finish_at: Option<f64>,
    /// Host of a network co-op game: the character each remote player chose (LOGIN option).
    pub(crate) remote: std::collections::HashMap<ObjId, Choice>,
    /// The menu cursor of each page's GUI controller, kept to be given back when the screen
    /// is left (the controllers outlive the map); the cursor is shown only on the first
    /// player's page while the mouse is in use.
    pub(crate) saved_cursors: Vec<(ObjId, Value)>,
    /// Per player: their live-view panel (pixels within their viewport) and the room around
    /// the character, from the page's last paint (character_page.rs).
    pub(crate) panels: Vec<Option<([f32; 4], f32)>>,
    /// The menu's white texture, for the page around the window onto the scene.
    pub(crate) white: Option<Value>,
    /// The left panel's picture (aspect it was cut to, texture) and the number of pictures made.
    pub(crate) picture: Option<(f32, Value)>,
    pub(crate) picture_serial: u32,
    /// F3's preview character in the level being played (improved build), and the way it faces
    /// once the camera is recorded (before: towards the player).
    pub(crate) preview: Option<ObjId>,
    pub(crate) preview_facing: Option<i32>,
    /// The page's text scale while it is drawn (character_page.rs).
    pub(crate) text_scale: f32,
    /// The roster panel's splitter picture, loaded once.
    pub(crate) splitter: Option<Value>,
    pub(crate) mouse_in_use: bool,
    last_mouse: Option<(i32, i32)>,
    /// The first player's mouse (or finger) over the Valider button at the last drawing (its
    /// hover look and the focus tick when it comes over it).
    pub(crate) confirm_hovered: bool,
    /// Co-op players' first-person meshes drawn with their character's arms: actor -> mesh.
    /// Each co-op player's pawn and the arms recipe of their character (none: XIII's own).
    pub(crate) arms_by_pawn: std::collections::HashMap<ObjId, &'static str>,
    /// The arms mesh of each (recipe, first-person mesh) pair, resolved once when first drawn
    /// (None: that mesh has no arms of the recipe, XIII's stay).
    pub(crate) arms_names: std::cell::RefCell<std::collections::HashMap<(&'static str, String), Option<&'static str>>>,
    pub(crate) arms_gaps: std::cell::RefCell<Vec<String>>,
    pub(crate) arms_report: std::collections::HashMap<ObjId, String>,
    /// The props of the roster's characters (character -> prop), and per player the pose waiting
    /// for its entry to end (when, which character, the pose).
    pub(crate) props: std::collections::HashMap<(ObjId, &'static str), ObjId>,
    /// Per player: a weapon waiting for its draw (character, sequence, share of it, the pose).
    pub(crate) drawing: Vec<Option<(ObjId, &'static str, f32, Pose)>>,
    pub(crate) pending_poses: Vec<Option<(f64, ObjId, Pose)>>,
    /// The weapons the roster's characters hold in their poses: (character, weapon class) -> the
    /// weapon's attachment in the character's hand (spawned hidden with the roster).
    pub(crate) weapons: std::collections::HashMap<(ObjId, &'static str), ObjId>,
    /// Per player: the pose their character plays (for its stop), the yaw it faces on its spot and
    /// the yaw it turns to.
    pub(crate) playing: Vec<Option<(ObjId, Pose)>>,
    pub(crate) yaws: Vec<Option<(ObjId, i32, i32)>>,
}

impl Screen {
    /// Flat index of a choice in a player's actor list.
    fn slot(choice: Choice) -> Option<usize> {
        let mut n = 0;
        for (i, c) in ROSTER.iter().enumerate() {
            if i == choice.character {
                return (choice.variant < c.variants.len()).then_some(n + choice.variant);
            }
            n += c.variants.len();
        }
        None
    }
}

impl Vm {
    /// The map being loaded is the character screen when its URL has the option (improved build).
    pub fn character_screen_from_url(&mut self, url: &crate::travel::Url) {
        let on = url.has_option(SCREEN_OPTION);
        // The menu cursors the screen hid, given back.
        for (root, cursors) in std::mem::take(&mut self.improved.character_screen.saved_cursors) {
            if self.objects.get(root).is_some_and(|o| o.is_some()) {
                self.set_prop(root, "MouseCursors", cursors);
            }
        }
        let held = self.improved.character_screen.held.take();
        let reopen_coop = self.improved.character_screen.reopen_coop.take();
        self.improved.character_screen = Screen { on, places: on.then(places), held, reopen_coop, ..Default::default() };
        // Two players choose side by side, at every screen shape (client.rs, viewport_rect).
        self.engine.client.character_screen_side_by_side = on;
    }

    /// Once per level tick (improved build): the preview character faces the player until the
    /// camera is recorded, then the camera.
    pub(crate) fn character_preview_frame(&mut self) {
        let Some(preview) = self.improved.character_screen.preview.filter(|&a| self.objects.get(a).is_some_and(|o| o.is_some())) else { return };
        let yaw = match self.improved.character_screen.preview_facing {
            Some(yaw) => yaw,
            None => {
                let Some(pawn) = self.local_viewport().and_then(|v| self.propr(v, "Actor").as_object()).and_then(|pc| self.propr(pc, "Pawn").as_object()) else { return };
                let from = self.propr(preview, "Location").as_vector();
                look_at(from, self.propr(pawn, "Location").as_vector())[1]
            }
        };
        self.set_prop(preview, "Rotation", Value::Rotator([0, yaw, 0]));
    }

    /// The character screen is shown on this map.
    pub fn character_screen_on(&self) -> bool {
        self.improved.character_screen.on
    }

    /// The page's layout: the split screen's with two players, else the full page's.
    pub(crate) fn character_layout(&self) -> Layout {
        let split = self.improved.character_screen.pages.len() > 1;
        match self.improved.character_screen.places.as_ref() {
            Some(pl) if split => pl.split_layout,
            Some(pl) => pl.layout,
            None if split => default_layouts().1,
            None => default_layouts().0,
        }
    }

    /// Camera of a viewport on the character screen (location, rotation).
    pub fn character_screen_view(&self, slot: u8) -> Option<([f32; 3], [i32; 3])> {
        let s = &self.improved.character_screen;
        if !s.on {
            return None;
        }
        let places = s.places.as_ref()?;
        let i = self.engine.client.viewports.iter().position(|&v| v == slot).unwrap_or(0).min(1);
        let p = &places.players[i];
        let room = s.panels.get(i).copied().flatten().map_or(1.2, |(_, room)| room);
        // The distance at which the character, head to feet with the layout's room, fills the
        // height of picture.
        let vfov = p.fov.unwrap_or(SOLO_VFOV);
        let fit = crate::character_page::CHARACTER_HALF_HEIGHT * room / (vfov.to_radians() / 2.0).dtan().max(0.01);
        if let Some([pitch, yaw, _]) = p.camera_rotation {
            // The recorded view, exactly; or moved back along its own line of sight.
            let ahead = forward([pitch, yaw]);
            let d: f32 = (0..3).map(|k| (p.spot[k] - p.camera[k]) * ahead[k]).sum();
            let back = if places.fit { (fit - d).max(0.0) } else { 0.0 };
            let camera = [0, 1, 2].map(|k| p.camera[k] - ahead[k] * back);
            return Some((camera, [pitch, yaw, 0]));
        }
        let camera = along(p.spot, p.camera, fit);
        let [pitch, yaw] = look_at(camera, p.spot);
        Some((camera, [pitch, yaw, 0]))
    }

    /// Improved build, client.rs: where the scene of viewport `slot` is drawn on the character
    /// screen (its live-view panel, pixels within the viewport) and the horizontal field of view
    /// across it, so that the panel shows the recorded view's height of picture.
    pub(crate) fn character_screen_scene(&self, slot: u8) -> Option<ScreenScene> {
        let s = &self.improved.character_screen;
        if !s.on {
            return None;
        }
        let i = self.engine.client.viewports.iter().position(|&v| v == slot).unwrap_or(0).min(1);
        let (panel, _) = s.panels.get(i).copied().flatten()?;
        let vfov = s.places.as_ref().and_then(|p| p.players[i].fov).unwrap_or(SOLO_VFOV);
        let r = panel.map(|v| v.round() as i32);
        if r[2] < 2 || r[3] < 2 {
            return None;
        }
        Some(screen_scene(r, vfov, self.character_layout()))
    }

    /// Player controllers of the local viewports, in viewport order.
    pub(crate) fn screen_players(&mut self) -> Vec<ObjId> {
        let slots = self.engine.client.viewports.clone();
        let viewports: Vec<ObjId> = slots.into_iter().filter_map(|s| self.viewport_object(s)).collect();
        viewports.into_iter().filter_map(|v| self.propr(v, "Actor").as_object()).filter(|&pc| self.objects.get(pc).is_some_and(|o| o.is_some())).collect()
    }

    /// Every player on the screen has finished their confirmation animation (confirm_pose_finished):
    /// no way in still to come, no weapon still to draw, no turn still under way, the pose
    /// played being their character's confirmation. A player whose animation cannot be read
    /// has not finished.
    fn confirm_animations_finished(&self) -> bool {
        let s = &self.improved.character_screen;
        if s.confirmed.is_empty() {
            return false;
        }
        (0..s.confirmed.len()).all(|p| {
            if s.pending_poses.get(p).is_none_or(|w| w.is_some()) || s.drawing.get(p).is_none_or(|w| w.is_some()) {
                return false;
            }
            if s.yaws.get(p).copied().flatten().is_some_and(|(_, yaw, target)| yaw != target) {
                return false;
            }
            let Some((pawn, pose)) = s.playing.get(p).copied().flatten() else { return false };
            let confirm = s.shown.get(p).and_then(|c| c.variant_data()).map(screen_confirm_pose);
            if !confirm.is_some_and(|c| c.sequence.eq_ignore_ascii_case(pose.sequence)) {
                return false;
            }
            let Some(inst) = self.mesh_insts.get(&pawn) else { return false };
            let views: Vec<Option<ChannelView>> = screen_pose_channels(&pose)
                .into_iter()
                .map(|ch| inst.channels.get(ch).map(|c| ChannelView { sequence: &c.sequence, frame: c.frame, last: c.last, alpha: c.blend_alpha }))
                .collect();
            confirm_pose_finished(&pose, &views)
        })
    }

    /// Once per level tick: sets the screen up on the first frame where the players exist.
    pub(crate) fn character_screen_frame(&mut self, dt: f32) {
        if !self.improved.character_screen.on {
            self.character_preview_frame();
            return;
        }
        self.improved.character_screen.clock += f64::from(dt);
        self.screen_pose_frame(dt);
        // Everyone confirmed and the short delay over, or every player's confirmation animation
        // already finished: the game starts (taken back in the meantime: it waits for everyone
        // again).
        let due = self.improved.character_screen.finish_at.is_some_and(|t| self.improved.character_screen.clock >= t);
        if due || (self.improved.character_screen.finish_at.is_some() && self.confirm_animations_finished()) {
            if !due {
                let left = self.improved.character_screen.finish_at.map_or(0.0, |t| t - self.improved.character_screen.clock);
                self.log.push(format!("Log: Character screen: the confirmations have finished, the game starts {left:.2} s early"));
            }
            self.improved.character_screen.finish_at = None;
            if !self.improved.character_screen.confirmed.is_empty() && self.improved.character_screen.confirmed.iter().all(|&c| c) {
                self.character_screen_finish();
            }
        }
        if !self.improved.character_screen.ready {
            let players = self.screen_players();
            if players.is_empty() || players.iter().any(|&pc| self.prop(pc, "Pawn").as_object().is_none()) {
                return;
            }
            let started = std::time::Instant::now();
            self.character_screen_setup(&players);
            self.improved.character_screen.setup_ms = started.elapsed().as_secs_f64() * 1000.0;
            self.log.push(format!("Log: Character screen: ready in {:.1} ms", self.improved.character_screen.setup_ms));
        }
        // The players' views stay still and nothing of the game is drawn over them.
        for pc in self.screen_players() {
            self.screen_quiet_player(pc);
        }
        self.character_screen_cursors();
    }

    /// Only the first player has a mouse: the menu cursor is drawn on their page only, and only
    /// while the mouse is in use (it moved or clicked since the last key or gamepad press).
    /// Done through each page's own GUI controller's MouseCursors, given back when the screen
    /// is left.
    fn character_screen_cursors(&mut self) {
        let pages = self.improved.character_screen.pages.clone();
        for (p, page) in pages.into_iter().enumerate() {
            let Some(root) = page.and_then(|pg| self.propr(pg, "Controller").as_object()) else { continue };
            if !self.improved.character_screen.saved_cursors.iter().any(|(r, _)| *r == root) {
                let cursors = self.prop(root, "MouseCursors");
                self.improved.character_screen.saved_cursors.push((root, cursors));
            }
            if p == 0 {
                let pos = (self.propr(root, "MouseX").as_int(), self.propr(root, "MouseY").as_int());
                if self.improved.character_screen.last_mouse.is_some_and(|l| l != pos) {
                    self.improved.character_screen.mouse_in_use = true;
                }
                self.improved.character_screen.last_mouse = Some(pos);
            }
            let show = p == 0 && self.improved.character_screen.mouse_in_use;
            let cursors = if show {
                self.improved.character_screen.saved_cursors.iter().find(|(r, _)| *r == root).map(|(_, v)| v.clone()).unwrap_or(Value::Array(Vec::new()))
            } else {
                Value::Array(Vec::new())
            };
            self.set_prop(root, "MouseCursors", cursors);
        }
    }

    /// The players' pawns hidden, their HUD hidden.
    fn screen_quiet_player(&mut self, pc: ObjId) {
        if let Some(pawn) = self.prop(pc, "Pawn").as_object() {
            if !self.native_prop(pawn, Field::Hidden).as_bool() {
                self.set_prop(pawn, "bHidden", Value::Bool(true));
                let _ = self.call_named(pawn, "RefreshDisplaying", vec![]);
            }
            if let Some(w) = self.prop(pawn, "Weapon").as_object() {
                self.set_prop(w, "bHidden", Value::Bool(true));
            }
        }
        if let Some(hud) = self.prop(pc, "myHUD").as_object() {
            self.set_prop(hud, "bHideHUD", Value::Bool(true));
        }
    }

    fn character_screen_setup(&mut self, players: &[ObjId]) {
        self.improved.character_screen.ready = true;
        let places = self.improved.character_screen.places.clone().unwrap_or_else(default_places);
        // Everything of the level stops (ULevel::Tick while paused: only the players' input and
        // the actors that always tick, our characters).
        if let (Some(level), Some(&pc)) = (self.level, players.first()) {
            let pri = self.prop(pc, "PlayerReplicationInfo");
            self.set_prop(level, "Pauser", pri);
        }
        // The map's own characters (Carrington in his cell, the guards) and what can be picked up
        // (medkits, armour, weapons, ammo, keys, objects of the mission) leave the screen: only
        // the decor stays. The map is not saved: nothing changes when it is really played.
        for a in self.actors.clone() {
            if self.objects.get(a).is_some_and(|o| o.is_some()) && (self.is_a(a, "Pawn") || self.is_a(a, "Pickup")) && !self.native_prop(a, Field::Hidden).as_bool() {
                self.set_prop(a, "bHidden", Value::Bool(true));
                if let Some(shadow) = self.prop(a, "Shadow").as_object() {
                    self.set_prop(shadow, "bHidden", Value::Bool(true));
                }
                let _ = self.call_named(a, "RefreshDisplaying", vec![]);
            }
        }
        let n = players.len().min(2);
        self.improved.character_screen.actors = vec![Vec::new(); n];
        self.improved.character_screen.shown = vec![Choice::default(); n];
        for (p, place) in places.players.iter().enumerate().take(n) {
            let mut list = Vec::new();
            for c in ROSTER {
                for variant in c.variants {
                    let pawn = self.screen_spawn(variant, place);
                    if let Some(pawn) = pawn {
                        self.screen_prepare_pose(pawn, &screen_pose(c.id, variant));
                        self.screen_prepare_pose(pawn, &screen_confirm_pose(variant));
                    }
                    list.push(pawn);
                }
            }
            let made = list.iter().filter(|a| a.is_some()).count();
            self.log.push(format!("Log: Character screen: player {} gets {made} of {} characters", p + 1, list.len()));
            self.improved.character_screen.actors[p] = list;
            self.screen_show(p, Choice::default());
        }
        // Each player's page over their own view (character_page.rs).
        self.improved.character_screen.confirmed = vec![false; n];
        self.improved.character_screen.variants = vec![default_variants(); n];
        self.improved.character_screen.hits = vec![Vec::new(); n];
        self.improved.character_screen.pages = vec![None; n];
        self.improved.character_screen.panels = vec![None; n];
        self.improved.character_screen.pending_poses = vec![None; n];
        self.improved.character_screen.drawing = vec![None; n];
        self.improved.character_screen.playing = vec![None; n];
        self.improved.character_screen.yaws = vec![None; n];
        let last = crate::character_page::last_choices();
        for (p, &pc) in players.iter().enumerate().take(n) {
            let choice = last[p.min(1)];
            self.improved.character_screen.variants[p][choice.character] = choice.variant;
            self.screen_show(p, choice);
            self.character_page_open(p, pc);
        }
        if let Some(&pc) = players.first() {
            self.screen_ambient(pc);
        }
    }

    /// The screen's ambient loop, heard the same everywhere (not positioned), from the first
    /// player's controller: the room shown has no ambient sound of its own. Its bank is another
    /// map's (loaded with the mods' banks, unloaded with the map's when the screen is left).
    fn screen_ambient(&mut self, pc: ObjId) {
        #[cfg(feature = "mods")]
        if let Err(why) = self.mod_ensure_sound(SCREEN_AMBIENT, "character screen") {
            self.log.push(format!("Log: Character screen: no ambient sound ({why})"));
            return;
        }
        let Some(sound) = self.audio_sound_ref(SCREEN_AMBIENT) else { return };
        let Some(mut state) = self.audio_actor_state(pc) else { return };
        state.has_position = false;
        state.ambient_sound = Some(sound);
        if let Some(hx) = self.audio.as_mut() {
            hx.restart_ambient_actor(&state);
        }
    }

    /// Before viewport `slot` is drawn: only its own player's character is shown (both players
    /// may stand on the same spot, each sees their own).
    pub(crate) fn character_screen_for_view(&mut self, slot: u8) {
        if !self.improved.character_screen.on || self.improved.character_screen.actors.len() < 2 {
            return;
        }
        let me = self.engine.client.viewports.iter().position(|&v| v == slot).unwrap_or(0);
        for p in 0..self.improved.character_screen.actors.len() {
            let Some(pawn) =
                self.improved.character_screen.shown.get(p).and_then(|&c| Screen::slot(c)).and_then(|s| self.improved.character_screen.actors[p].get(s).copied().flatten())
            else {
                continue;
            };
            let hidden = p != me;
            if self.native_prop(pawn, Field::Hidden).as_bool() != hidden {
                self.set_prop(pawn, "bHidden", Value::Bool(hidden));
                if let Some(shadow) = self.prop(pawn, "Shadow").as_object() {
                    self.set_prop(shadow, "bHidden", Value::Bool(hidden));
                }
                let _ = self.call_named(pawn, "RefreshDisplaying", vec![]);
            }
        }
    }

    /// The shadows viewport `slot` must not draw: those of the characters the other players
    /// show (all stand on the same spot; their shadow projectors stay attached to the floor
    /// whatever their bHidden when this view is drawn).
    pub(crate) fn character_screen_hidden_shadows(&self, slot: u8) -> Vec<ObjId> {
        let s = &self.improved.character_screen;
        if !s.on || s.actors.len() < 2 {
            return Vec::new();
        }
        let me = self.engine.client.viewports.iter().position(|&v| v == slot).unwrap_or(0);
        // Every roster actor of each player, the one shown and the others: a swap may leave the
        // previous one's shadow attached for a frame.
        let shadows: Vec<Vec<ObjId>> = s.actors.iter().map(|a| a.iter().flatten().filter_map(|&pawn| self.prop(pawn, "Shadow").as_object()).collect()).collect();
        others(&shadows, me)
    }

    /// One roster actor, hidden, standing on its player's spot.
    fn screen_spawn(&mut self, variant: &Variant, place: &Place) -> Option<ObjId> {
        let Some(class) = self.class("XIIIPawn") else {
            self.log.push("Log: Character screen: no XIIIPawn class".to_string());
            return None;
        };
        // Placed even where something stands (the map's own character on the same spot):
        // the screen's characters never collide.
        let spawned =
            self.spawn_actor(class, Value::Object(None), None, Value::Vector(place.spot), Value::Rotator([0, place.facing, 0]), Value::Object(None), true);
        let Some(pawn) = spawned.as_ref().ok().copied().flatten() else {
            self.log.push(format!("Log: Character screen: {} not spawned ({:?})", variant.mesh, spawned.err()));
            return None;
        };
        let _ = self.call_named(pawn, "SetCollision", vec![Value::Bool(false), Value::Bool(false), Value::Bool(false)]);
        let _ = self.call_named(pawn, "SetPhysics", vec![Value::Byte(0)]);
        self.set_prop(pawn, "bPhysicsAnimUpdate", Value::Bool(false));
        self.set_prop(pawn, "Velocity", Value::vector(0.0, 0.0, 0.0));
        // The model and skin by their path, as a map's own characters have them.
        self.set_prop(pawn, "Mesh", Value::Asset(variant.mesh.to_string()));
        if let Some(skin) = variant.skin {
            self.set_prop(pawn, "Skins", Value::Array(vec![Value::Asset(skin.to_string())]));
        }
        self.screen_set_shown(pawn, false);
        Some(pawn)
    }

    /// Shows or hides a roster actor and its shadow; only the shown one ticks (its pose plays).
    fn screen_set_shown(&mut self, pawn: ObjId, shown: bool) {
        self.set_prop(pawn, "bHidden", Value::Bool(!shown));
        self.set_prop(pawn, "bAlwaysTick", Value::Bool(shown));
        if let Some(shadow) = self.prop(pawn, "Shadow").as_object() {
            self.set_prop(shadow, "bHidden", Value::Bool(!shown));
            self.set_prop(shadow, "bAlwaysTick", Value::Bool(shown));
        }
        let _ = self.call_named(pawn, "RefreshDisplaying", vec![]);
        // Its weapons show with the pose that holds them (screen_hold).
        if !shown {
            self.screen_hold(pawn, None, None);
        }
    }

    /// The weapon and the prop a roster character holds (none: every one of its weapons, or
    /// props, hidden).
    fn screen_hold(&mut self, pawn: ObjId, weapon: Option<&'static str>, prop: Option<&'static str>) {
        let mine: Vec<(&'static str, ObjId)> = self.improved.character_screen.weapons.iter().filter(|((c, _), _)| *c == pawn).map(|((_, w), &a)| (*w, a)).collect();
        let props: Vec<(&'static str, ObjId)> = self.improved.character_screen.props.iter().filter(|((c, _), _)| *c == pawn).map(|((_, m), &a)| (*m, a)).collect();
        for (m, a) in props {
            if self.objects.get(a).is_some_and(|o| o.is_some()) {
                let draw = if prop == Some(m) { DT_STATIC_MESH } else { DT_NONE };
                if self.native_prop(a, Field::DrawType).as_byte() != draw {
                    self.set_prop(a, "DrawType", Value::Byte(draw));
                }
            }
        }
        // An actor on a bone is drawn with its base whatever its bHidden (FDynamicActor::Render
        // draws a base's attachments, then hides them from the render loop): a weapon put away
        // is drawn as nothing.
        for (w, a) in mine {
            if self.objects.get(a).is_some_and(|o| o.is_some()) {
                let draw = if weapon == Some(w) { DT_STATIC_MESH } else { DT_NONE };
                if self.native_prop(a, Field::DrawType).as_byte() != draw {
                    self.set_prop(a, "DrawType", Value::Byte(draw));
                }
            }
        }
    }

    /// Player `p` confirms their choice (their character plays its confirmation) or takes it
    /// back (back to its pose, without its way in).
    pub(crate) fn screen_confirm(&mut self, p: usize, on: bool) {
        let Some(choice) = self.improved.character_screen.shown.get(p).copied() else { return };
        let Some(pawn) = Screen::slot(choice).and_then(|s| self.improved.character_screen.actors.get(p).and_then(|l| l.get(s).copied().flatten())) else { return };
        let Some(variant) = choice.variant_data() else { return };
        let pose = if on { screen_confirm_pose(variant) } else { screen_pose(ROSTER[choice.character].id, variant).without_entry() };
        self.screen_begin(p, pawn, pose, CONFIRM_BLEND);
    }

    /// A pose played on player `p`'s character: its weapon in the hand, blended in over `tween`
    /// seconds, the character turned as the pose says.
    fn screen_play_pose(&mut self, p: usize, pawn: ObjId, pose: &Pose, tween: f32) {
        self.screen_start_pose(pawn, pose, tween);
        // The pose's own file's sequence (same name, same length: only the file changes).
        if let (true, Some(file)) = (pose.own_file, pose.anims) {
            if let Some(inst) = self.mesh_insts.get_mut(&pawn) {
                let found = inst.anims.iter().position(|a| a.path.eq_ignore_ascii_case(file) && a.seq(pose.sequence).is_some());
                if let Some(ai) = found {
                    for ch in screen_pose_channels(pose) {
                        if let Some(c) = inst.channels.get_mut(ch).filter(|c| c.sequence.eq_ignore_ascii_case(pose.sequence)) {
                            c.anim_index = ai as i32;
                        }
                    }
                }
            }
        }
        // A segment starts at its beginning (an upper-body pose fades in over the tween).
        if let (Some((from, _)), Some(inst)) = (pose.segment, self.mesh_insts.get_mut(&pawn)) {
            for ch in screen_pose_channels(pose) {
                if let Some(c) = inst.channels.get_mut(ch) {
                    c.frame = from;
                }
            }
        }
        self.screen_hold_for(p, pawn, pose);
        if let Some(w) = self.improved.character_screen.playing.get_mut(p) {
            *w = Some((pawn, *pose));
        }
        let facing = self.improved.character_screen.places.as_ref().map_or(0, |pl| pl.players[p.min(1)].facing);
        if let Some(w) = self.improved.character_screen.yaws.get_mut(p) {
            let now = match *w {
                Some((a, yaw, _)) if a == pawn => yaw,
                _ => facing,
            };
            *w = Some((pawn, now, signed_angle(facing + turn_units(pose.turn))));
        }
    }

    /// Player `p` now shows `choice`: the previous actor hidden, this one shown in its pose.
    pub fn screen_show(&mut self, p: usize, choice: Choice) {
        let Some(slot) = Screen::slot(choice) else { return };
        let Some(list) = self.improved.character_screen.actors.get(p).cloned() else { return };
        if let Some(prev) = self.improved.character_screen.shown.get(p).and_then(|&c| Screen::slot(c)).and_then(|s| list.get(s).copied().flatten()) {
            self.screen_set_shown(prev, false);
        }
        self.improved.character_screen.shown[p] = choice;
        let Some(pawn) = list.get(slot).copied().flatten() else { return };
        self.screen_set_shown(pawn, true);
        // A new character faces the camera straight away.
        let facing = self.improved.character_screen.places.as_ref().map_or(0, |pl| pl.players[p.min(1)].facing);
        self.set_prop(pawn, "Rotation", Value::Rotator([0, facing, 0]));
        if let Some(w) = self.improved.character_screen.yaws.get_mut(p) {
            *w = Some((pawn, facing, facing));
        }
        let pose = choice.variant_data().map_or(Pose::looped(DEFAULT_POSE), |v| screen_pose(ROSTER[choice.character].id, v));
        self.screen_begin(p, pawn, pose, 0.0);
    }

    /// Player `p`'s character starts `pose`: its way in first if it has one (the pose follows
    /// in screen_pose_frame), blended in over `tween` seconds. A pose waiting for its way in is
    /// dropped.
    fn screen_begin(&mut self, p: usize, pawn: ObjId, pose: Pose, tween: f32) {
        let tween = if tween > 0.0 { pose.blend.unwrap_or(tween) } else { tween };
        if let Some(w) = self.improved.character_screen.pending_poses.get_mut(p) {
            *w = None;
        }
        match pose.entry {
            Some(entry) if pose.upper => {
                // On the channels the pose fades in on, from its start share: the pose then
                // follows on them, blended from the way in (one chain, nothing waits).
                let way_in = Pose { sequence: entry.sequence, hold: true, rate: 1.0, stop: None, segment: None, entry: None, ..pose };
                self.screen_start_pose(pawn, &way_in, tween);
                if let Some(inst) = self.mesh_insts.get_mut(&pawn) {
                    for ch in screen_pose_channels(&pose) {
                        if let Some(c) = inst.channels.get_mut(ch).filter(|c| c.sequence.eq_ignore_ascii_case(entry.sequence)) {
                            c.frame = entry.start;
                        }
                    }
                }
                self.screen_hold_for(p, pawn, &pose);
                if let Some(w) = self.improved.character_screen.playing.get_mut(p) {
                    *w = None;
                }
                let at = self.improved.character_screen.clock + f64::from(entry.seconds);
                if let Some(w) = self.improved.character_screen.pending_poses.get_mut(p) {
                    *w = Some((at, pawn, pose));
                }
            }
            Some(entry) => {
                // Played once (held on its last frame if the pose starts later), on the whole
                // body, the pose's weapon and prop in the hand (or the weapon at its draw).
                self.screen_upper_off(pawn);
                self.screen_play(pawn, entry.sequence, 1.0, tween, false);
                self.screen_hold_for(p, pawn, &pose);
                if let Some(w) = self.improved.character_screen.playing.get_mut(p) {
                    *w = None;
                }
                let at = self.improved.character_screen.clock + f64::from(entry.seconds);
                if let Some(w) = self.improved.character_screen.pending_poses.get_mut(p) {
                    *w = Some((at, pawn, pose));
                }
            }
            None => self.screen_play_pose(p, pawn, &pose, tween),
        }
    }

    /// What a character holds in a pose: its prop, and its weapon now or (a draw) once the draw
    /// is reached (screen_pose_frame).
    fn screen_hold_for(&mut self, p: usize, pawn: ObjId, pose: &Pose) {
        let prop = pose.prop.map(|pr| pr.mesh);
        let drawing = pose.draw.filter(|_| pose.weapon.is_some()).filter(|(seq, share)| !self.screen_drawn(pawn, seq, *share));
        if let Some(w) = self.improved.character_screen.drawing.get_mut(p) {
            *w = drawing.map(|(seq, share)| (pawn, seq, share, *pose));
        }
        self.screen_hold(pawn, if drawing.is_some() { pose.put_away } else { pose.weapon }, prop);
    }

    /// The draw is reached: `sequence` played at `share` of it or further, or not played any more
    /// (the draw is over).
    fn screen_drawn(&self, pawn: ObjId, sequence: &str, share: f32) -> bool {
        // On whichever channel plays it (the base one, or one a pose fades in on).
        self.mesh_insts
            .get(&pawn)
            .and_then(|i| i.channels.iter().enumerate().find(|(k, c)| c.sequence.eq_ignore_ascii_case(sequence) && (*k == 0 || c.blend_alpha > 0.0 || c.timed_blend)).map(|(_, c)| c))
            .is_none_or(|c| c.frame >= share)
    }

    /// The upper-body channel faded out: the whole body plays the base channel again.
    fn screen_upper_off(&mut self, pawn: ObjId) {
        if let Some(inst) = self.mesh_insts.get_mut(&pawn) {
            for ch in [UPPER_CHANNEL, UPPER_ALSO_CHANNEL] {
                if inst.channels.len() > ch as usize {
                    inst.fade_to(ch, 0.0, 0.2);
                }
            }
        }
    }

    /// A sequence played once (or looped) on a roster character (the game's idle, looped, if
    /// its model lacks it).
    fn screen_play(&mut self, pawn: ObjId, sequence: &str, rate: f32, tween: f32, looped: bool) {
        let has = self.call_named(pawn, "HasAnim", vec![Value::Name(NameStruct::from(sequence.to_string()))]).is_ok_and(|v| v.as_bool());
        if looped || !has {
            self.screen_loop(pawn, sequence, rate, tween);
        } else {
            let _ = self.call_named(pawn, "PlayAnim", vec![Value::Name(NameStruct::from(sequence.to_string())), Value::Float(rate), Value::Float(tween)]);
            self.screen_quiet_channel(pawn);
        }
    }

    /// A sequence looped on a roster character (the game's idle if its model lacks it).
    fn screen_loop(&mut self, pawn: ObjId, sequence: &str, rate: f32, tween: f32) {
        let has = self.call_named(pawn, "HasAnim", vec![Value::Name(NameStruct::from(sequence.to_string()))]).is_ok_and(|v| v.as_bool());
        let sequence = if has { sequence } else { DEFAULT_POSE };
        let _ = self.call_named(pawn, "LoopAnim", vec![Value::Name(NameStruct::from(sequence.to_string())), Value::Float(rate), Value::Float(tween)]);
        self.screen_quiet_channel(pawn);
    }

    /// The screen's poses are its own: the end of a loop or of a sequence played once tells the
    /// character's script nothing (its AnimEnd would put it back to the game's idle).
    fn screen_quiet_channel(&mut self, pawn: ObjId) {
        if let Some(ch) = self.mesh_insts.get_mut(&pawn).and_then(|i| i.channels.first_mut()) {
            ch.notify_ignore = true;
        }
    }

    /// The pose itself, blended in over `tween` seconds; its prop shows.
    fn screen_start_pose(&mut self, pawn: ObjId, pose: &Pose, tween: f32) {
        let has = self.call_named(pawn, "HasAnim", vec![Value::Name(NameStruct::from(pose.sequence.to_string()))]).is_ok_and(|v| v.as_bool());
        if pose.upper {
            // Over the base channel's pose, from the spine up (the channels the game's pawns
            // layer their weapon animations on start at "X Spine"), faded in over the tween.
            let layers = [(UPPER_CHANNEL, Some(pose.upper_root.unwrap_or("X Spine"))), (UPPER_ALSO_CHANNEL, pose.upper_also)];
            for (channel, bone) in layers {
                let Some(bone) = bone else { continue };
                if let Some(inst) = self.mesh_insts.get_mut(&pawn) {
                    // A channel already shown (a way in on it) stays shown: the new sequence
                    // tweens from its pose.
                    let shown = inst.channels.get(channel as usize).map_or(0.0, |c| c.blend_alpha);
                    let _ = inst.params_blend(channel, shown, 0.0, 0.0, bone);
                    inst.fade_to(channel, 1.0, tween.max(0.05));
                }
                self.play_anim(pawn, channel, if has { pose.sequence } else { DEFAULT_POSE }, pose.rate, tween, !(pose.hold && has));
                if let Some(ch) = self.mesh_insts.get_mut(&pawn).and_then(|i| i.channels.get_mut(channel as usize)) {
                    ch.notify_ignore = true;
                }
            }
            if pose.upper_also.is_none() {
                if let Some(inst) = self.mesh_insts.get_mut(&pawn) {
                    if inst.channels.len() > UPPER_ALSO_CHANNEL as usize {
                        inst.fade_to(UPPER_ALSO_CHANNEL, 0.0, 0.2);
                    }
                }
            }
            return;
        }
        self.screen_upper_off(pawn);
        if pose.hold && has {
            let _ = self.call_named(pawn, "PlayAnim", vec![Value::Name(NameStruct::from(pose.sequence.to_string())), Value::Float(pose.rate), Value::Float(tween)]);
            self.screen_quiet_channel(pawn);
        } else {
            self.screen_loop(pawn, pose.sequence, pose.rate, tween);
        }
    }

    /// Once the screen is up, each frame: a pose whose way in has ended starts.
    fn screen_pose_frame(&mut self, dt: f32) {
        let now = self.improved.character_screen.clock;
        // A pose played to a share of its sequence stops there; a character turns to its pose's
        // direction (a quarter turn a second).
        for p in 0..self.improved.character_screen.playing.len() {
            if let Some((pawn, pose)) = self.improved.character_screen.playing[p] {
                if let (Some(stop), Some(inst)) = (pose.stop, self.mesh_insts.get_mut(&pawn)) {
                    let channel = if pose.upper { UPPER_CHANNEL as usize } else { 0 };
                    if pose.upper_also.is_some() {
                        if let Some(ch) = inst.channels.get_mut(UPPER_ALSO_CHANNEL as usize).filter(|c| c.sequence.eq_ignore_ascii_case(pose.sequence) && c.frame >= stop && c.rate != 0.0) {
                            ch.frame = stop;
                            ch.rate = 0.0;
                            ch.looping = false;
                        }
                    }
                    if let Some(ch) = inst.channels.get_mut(channel).filter(|c| c.sequence.eq_ignore_ascii_case(pose.sequence) && c.frame >= stop && c.rate != 0.0) {
                        ch.frame = stop;
                        ch.rate = 0.0;
                        ch.looping = false;
                    }
                }
            }
        }
        // A segment plays again from its beginning once past its end.
        for p in 0..self.improved.character_screen.playing.len() {
            if let Some((pawn, pose)) = self.improved.character_screen.playing[p] {
                if let (Some((from, to)), Some(inst)) = (pose.segment, self.mesh_insts.get_mut(&pawn)) {
                    for ch in screen_pose_channels(&pose) {
                        if let Some(c) = inst.channels.get_mut(ch).filter(|c| c.sequence.eq_ignore_ascii_case(pose.sequence)) {
                            if (c.frame >= to && !pose.hold) || (c.frame >= 0.0 && c.frame < from) {
                                c.frame = from;
                            }
                        }
                    }
                }
            }
        }
        for p in 0..self.improved.character_screen.drawing.len() {
            if let Some((pawn, seq, share, pose)) = self.improved.character_screen.drawing[p] {
                if self.screen_drawn(pawn, seq, share) {
                    self.improved.character_screen.drawing[p] = None;
                    self.screen_hold(pawn, pose.weapon, pose.prop.map(|pr| pr.mesh));
                }
            }
        }
        for p in 0..self.improved.character_screen.yaws.len() {
            if let Some((pawn, yaw, target)) = self.improved.character_screen.yaws[p] {
                if yaw != target {
                    let step = ((16384.0 * dt) as i32).max(1);
                    let next = turn_towards(yaw, target, step);
                    self.improved.character_screen.yaws[p] = Some((pawn, next, target));
                    self.set_prop(pawn, "Rotation", Value::Rotator([0, next, 0]));
                }
            }
        }
        // The level is paused: its time stands still, and a skeletal pose is computed once per
        // level time (GetFrame's cache). The shown characters' poses are computed again each
        // frame, so that their animations play (this screen only; the game's pause is unchanged).
        for p in 0..self.improved.character_screen.shown.len() {
            let Some(pawn) = Screen::slot(self.improved.character_screen.shown[p]).and_then(|s| self.improved.character_screen.actors.get(p).and_then(|l| l.get(s).copied().flatten())) else { continue };
            if let Some(inst) = self.mesh_insts.get_mut(&pawn) {
                inst.pose.last_tick = None;
            }
            // Its shadow is drawn again each frame too (the paused level's frame count does not
            // move: the game's schedule would keep the first silhouette, before the pose).
            if let Some(sp) = self.prop(pawn, "Shadow").as_object().filter(|&o| self.objects.get(o).is_some_and(|x| x.is_some())) {
                self.projectors.shadow_valid.remove(&sp);
                self.set_prop(sp, "UpdateCount", Value::Int(0));
            }
        }
        for p in 0..self.improved.character_screen.pending_poses.len() {
            if let Some((at, pawn, pose)) = self.improved.character_screen.pending_poses[p] {
                if now >= at {
                    self.improved.character_screen.pending_poses[p] = None;
                    let blend = pose.entry.map_or(0.0, |e| e.blend);
                    self.screen_play_pose(p, pawn, &pose, blend);
                }
            }
        }
    }

    /// When the roster is spawned: a pose's animation file linked and its prop attached (hidden),
    /// so that showing the character later loads nothing.
    fn screen_prepare_pose(&mut self, pawn: ObjId, pose: &Pose) {
        for anims in [pose.anims, pose.entry.and_then(|e| e.anims)].into_iter().flatten() {
            // The model's own animation file first (an instance only receives it when it is
            // first asked for a sequence), then the pose's.
            let _ = self.call_named(pawn, "HasAnim", vec![Value::Name(NameStruct::from(DEFAULT_POSE.to_string()))]);
            let linked = self.mesh_insts.get(&pawn).is_some_and(|i| i.anims.iter().any(|a| a.path.eq_ignore_ascii_case(anims)));
            if !linked {
                let _ = self.call_named(pawn, "LinkSkelAnim", vec![Value::Asset(anims.to_string())]);
            }
        }
        for weapon in [pose.weapon, pose.put_away].into_iter().flatten() {
            if self.improved.character_screen.weapons.contains_key(&(pawn, weapon)) {
                continue;
            }
            match self.screen_weapon(pawn, weapon) {
                Some(a) => {
                    self.improved.character_screen.weapons.insert((pawn, weapon), a);
                }
                None => self.log.push(format!("Log: Character screen: weapon {weapon} not held")),
            }
        }
        let Some(prop) = pose.prop.filter(|pr| !self.improved.character_screen.props.contains_key(&(pawn, pr.mesh))) else { return };
        let known = self.library.as_mut().and_then(|lib| lib.resolve(prop.mesh).map(|(lp, i)| lp.package.export_class(i).to_string()));
        // A prop class of the game's cut-scenes (XIDCine.CineCigareSM) as it is; a static mesh on
        // an attachment the game spawns on bones (its silencer), as the mods' models are.
        let own_class = if known.as_deref() == Some("Class") { self.class(prop.mesh) } else { None };
        let Some(class) = own_class.or_else(|| self.class("XIII.BerettaSilencer")) else { return };
        let (l, r) = (self.prop(pawn, "Location"), self.prop(pawn, "Rotation"));
        let Ok(Some(a)) = self.spawn_actor(class, Value::Object(Some(pawn)), None, l, r, Value::Object(None), true) else {
            self.log.push(format!("Log: Character screen: prop {} not spawned", prop.mesh));
            return;
        };
        let _ = self.call_named(a, "SetCollision", vec![Value::Bool(false), Value::Bool(false), Value::Bool(false)]);
        if own_class.is_none() {
            self.set_prop(a, "StaticMesh", Value::Asset(prop.mesh.to_string()));
            self.set_prop(a, "DrawType", Value::Byte(8));
        }
        // Put away (drawn as nothing, see screen_hold) until a pose holds it.
        self.set_prop(a, "DrawType", Value::Byte(DT_NONE));
        let mut args = [Value::Object(Some(a)), Value::Name(NameStruct::from(prop.bone.to_string()))];
        let attached = crate::natives::call(self, "AttachToBone", pawn, &mut args, &[true, true]).is_ok_and(|v| v.as_bool());
        // In the hand as the game's own cut-scene props are (XIII.SMAttached.AttachTo).
        if attached && prop.bone.eq_ignore_ascii_case("X R Hand") {
            let _ = self.call_named(a, "SetRelativeLocation", vec![Value::vector(6.0, -7.0, 0.5)]);
            let _ = self.call_named(a, "SetRelativeRotation", vec![Value::rotator(-200, 0, 18432)]);
        }
        if !attached {
            self.log.push(format!("Log: Character screen: prop {} not attached to {}", prop.mesh, prop.bone));
        }
        self.improved.character_screen.props.insert((pawn, prop.mesh), a);
    }

    /// A weapon in a roster character's hand, as the game holds one on a character
    /// (Inventory.AttachToPawn): the weapon's third-person attachment (its AttachmentClass) on the
    /// character's weapon bone (XIIIPawn.GetWeaponBoneFor: "X R Hand") at the weapon's
    /// ThirdPersonRelativeLocation and ThirdPersonRelativeRotation. The weapon itself is only
    /// spawned to read them, then removed (a weapon nobody carries puts its attachment away). The
    /// attachment, hidden; None when the class or its attachment is missing.
    fn screen_weapon(&mut self, pawn: ObjId, class: &str) -> Option<ObjId> {
        let class = self.class(class)?;
        let (l, r) = (self.prop(pawn, "Location"), self.prop(pawn, "Rotation"));
        let weapon = self.spawn_actor(class, Value::Object(Some(pawn)), None, l.clone(), r.clone(), Value::Object(Some(pawn)), true).ok().flatten()?;
        let attachment_class = self.prop(weapon, "AttachmentClass");
        let (rel_l, rel_r) = (self.prop(weapon, "ThirdPersonRelativeLocation"), self.prop(weapon, "ThirdPersonRelativeRotation"));
        let _ = self.call_named(weapon, "Destroy", vec![]);
        let attachment_class = match attachment_class {
            Value::Class(Some(c)) => c,
            _ => return None,
        };
        let attachment = self.spawn_actor(attachment_class, Value::Object(Some(pawn)), None, l, r, Value::Object(Some(pawn)), true).ok().flatten()?;
        let _ = self.call_named(attachment, "SetCollision", vec![Value::Bool(false), Value::Bool(false), Value::Bool(false)]);
        let mut args = [Value::Object(Some(attachment)), Value::Name(NameStruct::from("X R Hand".to_string()))];
        if !crate::natives::call(self, "AttachToBone", pawn, &mut args, &[true, true]).is_ok_and(|v| v.as_bool()) {
            return None;
        }
        let _ = self.call_named(attachment, "SetRelativeLocation", vec![rel_l]);
        let _ = self.call_named(attachment, "SetRelativeRotation", vec![rel_r]);
        self.set_prop(attachment, "DrawType", Value::Byte(DT_NONE));
        Some(attachment)
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choice_codes_round_trip() {
        for (i, c) in ROSTER.iter().enumerate() {
            for v in 0..c.variants.len() {
                let choice = Choice { character: i, variant: v };
                assert_eq!(Choice::from_code(&choice.code()), Some(choice));
            }
        }
        assert_eq!(Choice::from_code("killer.3"), Some(Choice { character: 13, variant: 2 }));
        assert_eq!(Choice::from_code("Killer.9"), None);
        assert_eq!(Choice::from_code("Nobody"), None);
        assert_eq!(Choice::default().code(), "XIII");
    }

    #[test]
    fn poses_are_data() {
        let pose = |id: &str, name: &str| {
            let c = ROSTER.iter().find(|c| c.id == id).unwrap();
            *c.variants.iter().find(|v| v.name == name).unwrap()
        };
        for c in ROSTER {
            for v in c.variants {
                for p in [v.pose, v.confirm] {
                    // The game's own files, sequences, weapons and props only.
                    assert!(p.anims.is_none_or(|a| (a.starts_with("XIIIPersos.") || a.starts_with("XIIIPersosG.")) && a.ends_with('A')), "{} {}", c.id, p.sequence);
                    assert!(p.weapon.is_none_or(|w| w.starts_with("XIII.")), "{}", c.id);
                    assert!(p.prop.is_none_or(|pr| pr.mesh == CIGAR), "{}", c.id);
                    assert!(p.stop.is_none_or(|s| s > 0.0 && s < 1.0), "{}", c.id);
                    assert!(p.turn.abs() <= 90.0);
                    // A draw names the sequence it happens in, and a weapon to draw.
                    assert!(p.draw.is_none_or(|(seq, share)| p.weapon.is_some() && share > 0.0 && share < 1.0 && (seq == p.sequence || p.entry.is_some_and(|e| e.sequence == seq))), "{}", c.id);
                }
                // The idle and the confirmation are not mimed: a smoker holds his cigar.
                for p in [v.pose, v.confirm] {
                    assert_eq!(p.sequence == "Fume1", p.prop.is_some(), "{}", c.id);
                }
            }
        }
        // What the owner asked for.
        // XIII: the same in every outfit (standing still in the colonel's uniform).
        for v in ROSTER[0].variants {
            assert_eq!((v.pose, v.confirm), (if ["general", "torn"].contains(&v.name) { STILL } else { XIII_IDLE }, XIII_CONFIRM), "{}", v.name);
        }
        assert_eq!((XIII_CONFIRM.entry.map(|e| e.sequence), XIII_CONFIRM.draw, XIII_CONFIRM.weapon), (Some("degaine"), Some(("degaine", DRAW_AT)), Some(MAGNUM)));
        assert_eq!(XIII_CONFIRM.stop, Some(0.4), "the aim held, no raise to the shoulder");
        let order: Vec<&str> = variant_order(0).into_iter().map(|i| ROSTER[0].variants[i].name).collect();
        assert_eq!(order, ["jacket", "military", "general", "bandages", "torn"]);
        // Jones: the handgun taken out, then put away for the rifle.
        assert_eq!(pose("Jones", "camouflage").pose, JONES_IDLE);
        assert_eq!((JONES_M16.weapon, JONES_M16.draw, JONES_M16.turn), (Some(RIFLE), Some(("SearchGun", 0.38)), JONES_TURN));
        assert_eq!(pose("Mongoose", "").confirm.sequence, "eclatrire");
        for (id, name) in [("Jones", "blouse"), ("Jones", "camouflage")] {
            assert_eq!((pose(id, name).pose, pose(id, name).confirm), (JONES_IDLE, JONES_M16), "{id} {name}");
        }
        let jones = pose("Jones", "camouflage");
        assert!(jones.confirm == JONES_M16 && jones.pose == JONES_IDLE, "standing, then the rifle from her back");
        assert_eq!(pose("Carrington", "").pose, SMOKING);
        for (id, name) in [("Spads", "standard"), ("Spads", "jungle")] {
            assert_eq!((pose(id, name).pose, pose(id, name).confirm), (STILL, RIFLE_TAKEN), "{id} {name}");
        }
        for (id, name) in [("Carrington", ""), ("Sheridan", "grey_suit"), ("Sheridan", "tuxedo")] {
            assert_eq!(pose(id, name).confirm.sequence, "MigBoxeProvoc", "{id} {name}");
        }
        assert_eq!(pose("Mongoose", "").pose.sequence, "Doute");
        assert_eq!(pose("Kim", "").pose.sequence, "KimWait");
        assert_eq!(pose("Amos", "burgundy_suit").confirm.draw, Some(("degaine", DRAW_AT)));
        // McCall and the GI: the assault rifle taken out as the SPADS do.
        for id in ["McCall", "GI"] {
            assert_eq!(pose(id, "").confirm, RIFLE_TAKEN, "{id}");
        }
        assert_eq!(pose("Jones", "evening_dress").pose.sequence, "Jolierobe");

        assert_eq!(pose("Standwell", "robe").confirm, pose("Conspirator", "").confirm);
        for name in ["jacket", "rolled_sleeves"] {
            assert_eq!((pose("FBI", name).pose, pose("FBI", name).confirm), (EARPIECE, draw(PISTOL)), "{name}");
        }
        // Who aims does it to the side: Amos's draw, or turned that way.
        for c in ROSTER {
            for v in c.variants {
                let p = v.confirm;
                if ["PistoletRetour", "WaitSniperAlerte", "GunRetour"].contains(&p.sequence) && p.segment.is_none() && p.stop.is_none_or(|s| s < 0.5) {
                    assert!(p.turn != 0.0, "{} {} aims at the camera", c.id, v.name);
                }
            }
        }
        assert!(ROSTER.iter().find(|c| c.id == "Killer").unwrap().variants.iter().all(|v| v.confirm == draw(MAGNUM)), "every killer draws a Magnum");
        assert_eq!(pose("GI", "").pose.sequence, "salut");
        for (id, name) in [("Spads", "standard"), ("Spads", "jungle")] {
            assert_eq!(pose(id, name).confirm.weapon, Some(RIFLE), "{id} {name}");
        }
        assert!(KLAN_ARMS.segment.is_some_and(|(a, b)| a > 0.0 && Some(b) == KLAN_ARMS.stop), "the arms rise at once");
        for (id, name) in [("Standwell", "uniform"), ("Admiral", "")] {
            let c = pose(id, name).confirm;
            assert_eq!((c.sequence, c.hold, c.weapon, c.stop), ("salut", true, None, Some(0.55)), "{id} {name}");
        }
        // The policemen draw a Magnum; their baton taps are kept for testing.
        let cop = pose("Policeman", "cop");
        assert_eq!(cop.confirm, draw(MAGNUM));
        assert!(BATON.upper && BATON.upper_also == Some("X Matrac") && BATON.segment.is_some_and(|(a, b)| 0.0 < a && a < b && b < 1.0));
        assert!(ROSTER.iter().flat_map(|c| c.variants).filter(|v| v.confirm.upper || v.pose.upper).all(|v| ["dialogue1KKK", "SearchGun", "degaine", "PistoletRetour"].contains(&v.confirm.sequence) && !v.pose.upper));
        assert_eq!(jones.pose.without_entry().entry, None);
        // The examples name roster entries.
        for (c, name, pose) in POSE_EXAMPLES {
            let ch = ROSTER.iter().find(|r| r.id == c).expect("a roster character");
            assert!(ch.variants.iter().any(|v| v.name == name), "{c} {name}");
            assert!(pose.entry.is_some_and(|e| e.seconds > 0.0 && e.blend > 0.0));
        }
    }

    #[test]
    /// The game starts before the second when every confirmation animation has finished: a pose
    /// played once and at its end, a pose stopped at its share. Negative controls: a channel
    /// still before its end (one player still animating), a looped pose, an unreadable or
    /// other channel, an upper-body pose still blending in all keep the wait.
    fn confirmation_animations_end_the_wait() {
        let mut once = Pose::looped("degaine");
        once.hold = true;
        let at = |frame: f32| Some(ChannelView { sequence: "degaine", frame, last: 0.97, alpha: 1.0 });
        assert!(confirm_pose_finished(&once, &[at(0.98)]));
        assert!(!confirm_pose_finished(&once, &[at(0.5)]), "still animating");
        assert!(!confirm_pose_finished(&once, &[at(-0.2)]), "still in its way in");
        assert!(!confirm_pose_finished(&once, &[None]), "unreadable");
        assert!(!confirm_pose_finished(&once, &[Some(ChannelView { sequence: "WaitNeutre", frame: 1.0, last: 0.97, alpha: 1.0 })]), "another sequence");
        assert!(!confirm_pose_finished(&once, &[]));
        assert!(!confirm_pose_finished(&Pose::looped("degaine"), &[at(0.98)]), "a looped pose never ends");
        let mut stopped = Pose::looped("degaine");
        stopped.stop = Some(0.6);
        assert!(confirm_pose_finished(&stopped, &[at(0.6)]));
        assert!(!confirm_pose_finished(&stopped, &[at(0.4)]));
        let mut upper = once;
        upper.upper = true;
        assert!(!confirm_pose_finished(&upper, &[Some(ChannelView { sequence: "degaine", frame: 1.0, last: 0.97, alpha: 0.5 })]), "blending in");
        // Two players: one still animating keeps everyone waiting.
        let players = [vec![at(0.98)], vec![at(0.3)]];
        assert!(!players.iter().all(|c| confirm_pose_finished(&once, c)));
    }

    #[test]
    fn the_game_starts_one_second_after_the_last_confirmation() {
        assert_eq!(CONFIRM_START_DELAY, 1.0);
        assert_eq!(start_time(12.25), 13.25);
    }

    #[test]
    fn a_character_turns_the_short_way() {
        assert_eq!(turn_units(90.0), 16384);
        assert_eq!(turn_towards(0, 7282, 1000), 1000);
        assert_eq!(turn_towards(6900, 7282, 1000), 7282, "never past its target");
        // Across the half turn: the short way.
        assert_eq!(turn_towards(32000, -32000, 500), signed_angle(32500));
        assert_eq!(turn_towards(-100, 100, 1000), 100);
    }

    #[test]
    fn jones_shows_her_camouflage_first_and_the_codes_stay() {
        let jones = ROSTER.iter().position(|c| c.id == "Jones").unwrap();
        let d = default_variants();
        assert_eq!(ROSTER[jones].variants[d[jones]].name, "camouflage");
        assert!(d.iter().enumerate().all(|(i, &v)| i == jones || v == 0), "the others show their first variant");
        // The earlier codes keep their outfits (network, saved choices); the camouflage has its own.
        assert_eq!(Choice::from_code("Jones").map(|c| ROSTER[c.character].variants[c.variant].name), Some("blouse"));
        assert_eq!(Choice::from_code("Jones.2").map(|c| ROSTER[c.character].variants[c.variant].name), Some("evening_dress"));
        assert_eq!(Choice { character: jones, variant: d[jones] }.code(), "Jones.3");
        // Shown: camouflage, blouse, evening dress.
        let names: Vec<&str> = variant_order(jones).iter().map(|&v| ROSTER[jones].variants[v].name).collect();
        assert_eq!(names, ["camouflage", "blouse", "evening_dress"]);
        // XIII's new outfits come after his earlier ones (their codes unchanged).
        let xiii = ROSTER.iter().position(|c| c.id == "XIII").unwrap();
        assert_eq!(Choice::from_code("XIII.2").map(|c| ROSTER[c.character].variants[c.variant].name), Some("bandages"));
        assert_eq!(variant_order(xiii).len(), ROSTER[xiii].variants.len());
    }

    #[test]
    fn the_layout_and_its_keys() {
        let mut p = default_places();
        // The keys change the layout's defaults, before or after the Layout line.
        read_places("ViewRoom=1.3\nLayout=wide\nPicture=splitter\nRows=7\nBlockAspect=0\nListShare=2", &mut p);
        let wide = Layout::of(LayoutKind::Wide);
        assert_eq!(p.layout, Layout { room: 1.3, picture: PanelPicture::Splitter, rows: 7, ..wide });
        assert_eq!(p.split_layout, p.layout, "without a prefix: both");
        assert_eq!(p.layout.list, wide.list, "a share out of range is left out");
        let mut p = default_places();
        read_places("Layout=nothing\nPicture=coop", &mut p);
        assert_eq!(p.layout, Layout { picture: PanelPicture::Coop, ..default_layouts().0 }, "an unknown layout keeps the default");
        // Alone and in split screen apart; the defaults: compact blocks on the crates picture.
        let mut p = default_places();
        assert_eq!((p.layout.kind, p.split_layout.kind, p.split_layout.picture), (LayoutKind::Block, LayoutKind::Block, PanelPicture::Splitter));
        assert!(p.split_layout.bar > 0.0 && p.split_layout.width < 0.5, "a compact centred block, the bar wider than the panels");
        read_places("Split.Layout=block\nSplit.ListShare=0.5\nSolo.Rows=6", &mut p);
        assert_eq!(p.split_layout, Layout { list: 0.5, ..Layout::of(LayoutKind::Block) });
        assert_eq!(p.layout, Layout { rows: 6, ..default_layouts().0 });
        // The sizes: split screen's own defaults (the game's other menus), and their keys.
        let split = default_layouts().1;
        assert_eq!((split.width, split.aspect, split.gap, split.bar, split.scale, split.margin, split.text, split.ink), (0.402, 1.29, 0.018, 0.04, 0.98, 0.0, 14.0, 0.8));
        let solo = default_layouts().0;
        assert_eq!((solo.width, solo.rows, solo.margin, solo.zoom, solo.shift), (0.42, 8, 0.005, 1.07, [0.0, -0.01]));
        let mut p = default_places();
        read_places("Split.BlockScale=0.9
Split.Margin=0.02
TextSize=16
Solo.InkBorder=1.5
ViewZoom=1.2
ViewShiftX=-0.1
ViewShiftY=0.05
BlockScale=2", &mut p);
        assert_eq!((p.split_layout.scale, p.split_layout.margin, p.split_layout.text, p.split_layout.ink), (0.9, 0.02, 16.0, 0.8));
        assert_eq!((p.layout.scale, p.layout.text, p.layout.ink, p.layout.zoom, p.layout.shift), (0.985, 16.0, 1.5, 1.2, [-0.1, 0.05]), "BlockScale=2 is out of range");
    }

    #[test]
    fn each_view_leaves_out_the_other_players_shadows() {
        let shadows = vec![vec![10, 11], vec![20, 21, 22]];
        assert_eq!(others(&shadows, 0), vec![20, 21, 22]);
        assert_eq!(others(&shadows, 1), vec![10, 11]);
        assert!(others(&shadows[..1], 0).is_empty(), "alone: nothing left out");
    }

    #[test]
    fn the_live_view_is_magnified_and_moved_without_turning() {
        let plain = Layout { zoom: 1.0, shift: [0.0; 2], ..default_layouts().0 };
        let v = screen_scene([0, 0, 400, 300], 60.0, plain);
        // As recorded: 60 degrees high, the width from the panel's shape, no move.
        let tan_h = 30f32.to_radians().dtan() * 400.0 / 300.0;
        assert!((v.fov - 2.0 * tan_h.datan().to_degrees()).abs() < 1e-3 && v.lens == [0.0; 2] && v.cull_fov == v.fov);
        // Twice the zoom: half the tangent; the picture up by a tenth of the panel = 0.2 units up.
        let v = screen_scene([0, 0, 400, 300], 60.0, Layout { zoom: 2.0, shift: [0.05, -0.1], ..plain });
        assert!((v.fov - 2.0 * (tan_h / 2.0).datan().to_degrees()).abs() < 1e-3);
        assert_eq!(v.lens, [0.1, 0.2]);
        assert!((v.cull_fov - 2.0 * (tan_h / 2.0 * 1.2).datan().to_degrees()).abs() < 1e-3, "visibility over the moved picture");
    }

    #[test]
    fn recording_keys_write_the_place_file() {
        let spot = place_entries("Plage01", PlaceKey::Spot, [10.4, -20.6, 30.0], [0, 49152, 0], SOLO_VFOV);
        assert_eq!(spot, vec![("Map", "Plage01".to_string()), ("Spot1", "10,-21,30".to_string()), ("Spot2", "10,-21,30".to_string())]);
        let camera = place_entries("Plage01", PlaceKey::Camera, [1.0, 2.0, 3.0], [64536, 100, 0], 54.52);
        assert_eq!(camera[1], ("Camera1", "1,2,3,-1000,100,54.5".to_string()));
        // A new file, then an existing one: lines replaced where they are, the rest kept.
        let text = set_place_lines("", &spot);
        assert_eq!(text, "[CharacterScreen]\r\nMap=Plage01\r\nSpot1=10,-21,30\r\nSpot2=10,-21,30\r\n");
        let text = set_place_lines("[CharacterScreen]\nmap=Hual02 ; old\nViewX=0.7\nCamera1=0,0,0\n", &camera);
        assert_eq!(text, "[CharacterScreen]\r\nMap=Plage01\r\nViewX=0.7\r\nCamera1=1,2,3,-1000,100,54.5\r\nCamera2=1,2,3,-1000,100,54.5\r\n");
        // F8: this map's lines forgotten, the rest kept.
        let both = set_place_lines(&set_place_lines("ViewX=1\n", &spot), &camera);
        assert_eq!(remove_place_lines(&both, &["Map", "Spot1", "Spot2", "Camera1", "Camera2"]), "[CharacterScreen]\r\nViewX=1\r\n");
        assert!(place_entries("Plage01", PlaceKey::Remove, [0.0; 3], [0; 3], SOLO_VFOV).is_empty());
        // Read back: the camera keeps its recorded direction, the character faces the recorded way.
        let mut p = default_places();
        read_places(&set_place_lines(&set_place_lines("", &spot), &camera), &mut p);
        assert_eq!(p.map, "Plage01");
        assert_eq!(p.players[1].camera_rotation, Some([-1000, 100, 0]));
        assert_eq!(p.players[1].fov, Some(54.5), "the recorded view's height of picture");
        assert_eq!(p.players[0].spot, [10.0, -21.0, 30.0]);
        // The character faces the camera (1, 2, 3 seen from 10, -21: towards +y, -x), turned by SpotTurn.
        face_cameras(&mut p);
        assert_eq!(p.players[0].facing, look_at([10.0, -21.0, 30.0], [1.0, 2.0, 3.0])[1]);
        let mut q = p.clone();
        q.spot_turn = 90.0;
        face_cameras(&mut q);
        assert_eq!(q.players[0].facing, signed_angle(p.players[0].facing + 16384));
        // The defaults: the character facing the camera.
        let mut d = default_places();
        let before = d.players[0].facing;
        face_cameras(&mut d);
        assert_eq!(d.players[0].facing, before);
    }

    #[test]
    fn place_file_replaces_what_it_gives() {
        let mut p = default_places();
        read_places("[CharacterScreen]\nMap=Hual01b\nCamera2=1,2,3,4,5 ; comment\nSpot1=10,20,30,16384\nSpot3=0,0,0,0\nCamera1=bad\nListShare=0.4", &mut p);
        assert_eq!(p.layout.list, 0.4);
        assert_eq!(p.layout.kind, LayoutKind::Block, "the mock-up's block by default");
        assert_eq!(p.map, "Hual01b");
        assert_eq!(p.players[1].camera, [1.0, 2.0, 3.0]);
        assert_eq!(p.players[1].camera_rotation, Some([4, 5, 0]));
        assert_eq!(p.players[0].camera_rotation, default_places().players[0].camera_rotation, "a malformed camera keeps the default view");
        assert_eq!((p.players[0].spot, p.players[0].facing), ([10.0, 20.0, 30.0], 16384));
        assert_eq!(p.players[0].camera, default_places().players[0].camera, "malformed line left out");
        read_places("Camera1=7,8,9", &mut p);
        assert_eq!((p.players[0].camera, p.players[0].camera_rotation), ([7.0, 8.0, 9.0], None), "a position alone: the camera looks at the character");
    }

    #[test]
    fn a_recorded_view_is_kept_or_moved_back_on_its_line() {
        let f = forward([0, 16384]);
        assert!(f[0].abs() < 1e-4 && (f[1] - 1.0).abs() < 1e-4 && f[2].abs() < 1e-4, "yaw a quarter turn: +y");
        let down = forward([-8192, 0]);
        assert!(down[2] < -0.7 && down[0] > 0.7, "pitched down: ahead and below");
    }

    #[test]
    fn the_camera_keeps_its_line_to_the_character() {
        let p = along([0.0, 0.0, 0.0], [300.0, 0.0, 0.0], 100.0);
        assert_eq!(p, [100.0, 0.0, 0.0]);
        let spot = [10.0, 20.0, 30.0];
        let q = along(spot, [10.0, 20.0, 230.0], 50.0);
        assert_eq!(q, [10.0, 20.0, 80.0], "from the character towards the file's camera");
    }

    #[test]
    fn the_camera_looks_at_the_character() {
        assert_eq!(look_at([0.0, 0.0, 0.0], [100.0, 0.0, 0.0]), [0, 0]);
        assert_eq!(look_at([0.0, 0.0, 0.0], [0.0, 100.0, 0.0]), [0, 16384]);
        assert_eq!(look_at([0.0, 0.0, 0.0], [100.0, 0.0, -100.0]), [-8192, 0]);
        let p = default_places().players[0].clone();
        let [_, yaw] = look_at(p.camera, p.spot);
        let [_, set_yaw, _] = p.camera_rotation.expect("the default view is a recorded one");
        assert!((signed_angle(yaw - set_yaw)).abs() < 1000, "the default camera looks towards its spot: {yaw} {set_yaw}");
    }

    #[test]
    fn variants_have_names_when_there_are_several() {
        for c in ROSTER {
            for v in c.variants {
                assert_eq!(v.name.is_empty(), c.variants.len() == 1, "{}: {}", c.id, v.mesh);
                if !v.name.is_empty() {
                    for lang in crate::added_text::LANGUAGES {
                        assert!(!crate::added_text::variant_name(lang, v.name).is_empty(), "{} in {lang}", v.name);
                    }
                }
            }
        }
    }

    #[test]
    fn every_variant_has_its_own_slot() {
        let mut seen = std::collections::HashSet::new();
        for (i, c) in ROSTER.iter().enumerate() {
            for v in 0..c.variants.len() {
                assert!(seen.insert(Screen::slot(Choice { character: i, variant: v }).unwrap()));
            }
        }
        assert_eq!(Screen::slot(Choice { character: 2, variant: 1 }), None, "Carrington has one look");
    }
}
