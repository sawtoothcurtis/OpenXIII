//! Improved build only: "Mods" and "Co-op" boxes on the main menu (XIIIMenu) and a "Select
//! level" entry on the Load page (XIIIMenuLoadGameWindow).
//!
//! Main menu: the original page lays out six XIIITextureButton boxes. Options is cut in two: it
//! keeps most of its height and a window of its picture (cropped, not squashed: the textures the
//! box draws are composed into one image exactly as XIIITextureButton.Paint lays them out, then a
//! band of rows is kept), and a new box of the same class takes a thin strip below it. The new box
//! is a real control of the page (created by its own CreateControl, appended to Controls), so
//! hover, frame, focus and zoom behave like the others. Its onomatopoeia and label are drawn
//! after the page's AfterPaint like the original ones; arrows reach it; a click on it is handed to
//! the game window (`take_menu_action`), which opens the mods page. Its picture is a frame of one
//! of the game's videos, decoded from the player's own files in the background (still_frames.rs):
//! the box shows a placeholder until the frame is ready, then its picture is swapped.
//!
//! Load is cut the same way: it keeps the top of its picture and a Co-op box takes the space
//! below. The Co-op box needs only the improved build (not mod support, unlike the Mods box);
//! choosing it opens the Co-op page (coop_page.rs).
//!
//! Load page: an XIIIButton like the save slots, below them, opens the Select level page
//! (select_level_page.rs).

use crate::added_text::Text;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::value::{ObjId, Value};
use crate::{Result, Vm};

/// An RGBA image (8 bits per channel, rows from the top).
#[derive(Clone, Debug, PartialEq)]
pub struct Image {
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<u8>,
}

impl Image {
    pub fn new(w: u32, h: u32) -> Image {
        Image { w, h, rgba: vec![0; (w * h * 4) as usize] }
    }

    fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * self.w + x) * 4) as usize;
        [self.rgba[i], self.rgba[i + 1], self.rgba[i + 2], self.rgba[i + 3]]
    }

    /// Rows `from`..`to` (clamped to the image).
    pub fn rows(&self, from: u32, to: u32) -> Image {
        let (from, to) = (from.min(self.h), to.min(self.h).max(from.min(self.h)));
        let row = (self.w * 4) as usize;
        Image { w: self.w, h: to - from, rgba: self.rgba[from as usize * row..to as usize * row].to_vec() }
    }

    /// The `w` × `h` rectangle at (x, y) (clamped to the image).
    pub fn region(&self, x: u32, y: u32, w: u32, h: u32) -> Image {
        let (x, y) = (x.min(self.w), y.min(self.h));
        let (w, h) = (w.min(self.w - x), h.min(self.h - y));
        let mut rgba = Vec::with_capacity((w * h * 4) as usize);
        for row in y..y + h {
            let at = ((row * self.w + x) * 4) as usize;
            rgba.extend_from_slice(&self.rgba[at..at + (w * 4) as usize]);
        }
        Image { w, h, rgba }
    }
}

/// A crop window of a picture: centre (fractions of the width and height) and height (fraction of
/// the height); the width follows from the aspect ratio of the box it fills.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Crop {
    pub cx: f32,
    pub cy: f32,
    pub h: f32,
}

/// The part of `img` with the `aspect` (width / height) of a box, as `crop` places it: nothing is
/// stretched; the window is shrunk to fit and slid inside the picture when needed.
pub fn crop_to_aspect(img: &Image, aspect: f32, crop: Crop) -> Image {
    let [x, y, w, h] = crop_rect(img, aspect, crop);
    img.region(x, y, w, h)
}

/// The window `crop_to_aspect` keeps: x, y, width, height in pixels of `img`. It never takes the
/// black bars of a letterboxed video frame (`content_rows`).
pub fn crop_rect(img: &Image, aspect: f32, crop: Crop) -> [u32; 4] {
    if img.w == 0 || img.h == 0 || aspect <= 0.0 {
        return [0, 0, img.w, img.h];
    }
    let (top, bottom) = content_rows(img);
    let rows = (bottom - top) as f32;
    let mut h = (crop.h * img.h as f32).round().clamp(1.0, rows);
    let mut w = (h * aspect).round();
    if w > img.w as f32 {
        w = img.w as f32;
        h = (w / aspect).round().clamp(1.0, rows);
    }
    let x = (crop.cx * img.w as f32 - w / 2.0).round().clamp(0.0, img.w as f32 - w);
    let y = (crop.cy * img.h as f32 - h / 2.0).round().clamp(top as f32, bottom as f32 - h);
    [x as u32, y as u32, w as u32, h as u32]
}

/// Rows `top..bottom` of `img` between the black bars of a letterboxed frame (rows where almost
/// every pixel is near black, from the top and from the bottom); the whole image when it has none.
pub fn content_rows(img: &Image) -> (u32, u32) {
    let row = (img.w * 4) as usize;
    let bar = |y: u32| {
        let px = &img.rgba[y as usize * row..(y as usize + 1) * row];
        let dark = px.chunks_exact(4).filter(|p| p[0].max(p[1]).max(p[2]) < 32).count();
        dark * 50 >= img.w as usize * 49
    };
    let top = (0..img.h).find(|&y| !bar(y)).unwrap_or(0);
    let bottom = (top..img.h).rev().find(|&y| !bar(y)).map_or(img.h, |y| y + 1);
    if bottom <= top {
        (0, img.h)
    } else {
        (top, bottom)
    }
}

/// A piece of a box's picture: its image drawn stretched at (x, y), size (w, h), in texels of
/// the composed picture.
pub struct Piece<'a> {
    pub image: &'a Image,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}
/// Size (USize, VSize) of each texture slot of an XIIITextureButton bank: tFirstTex, tSecondTex,
/// tThirdTex, tFourthTex, tAdjustTex.
pub type Slots = [Option<(u32, u32)>; 5];

/// Where XIIITextureButton.Paint draws each texture slot, in texels of the whole picture (before
/// the fXStretch / fYStretch scale), and the size of the whole picture. `sizes` are those of the
/// bank drawn and `sizes1` those of bank 1 (BeforePaint computes the patch and the stretch from
/// bank 1). None for a button drawn from tFirstTex alone (stretched to the box: nothing to compose).
pub fn paint_layout(sizes: &Slots, sizes1: &Slots, xoff: f32, yoff: f32) -> Option<((f32, f32), Vec<(usize, [f32; 4])>)> {
    let size = |s: &Slots, i: usize| s[i].map(|(u, v)| (u as f32, v as f32));
    // BeforePaint: the PS2 patch, from bank 1's tFirstTex.
    let (mut px, mut py) = (1.0, 1.0);
    if let Some((u, v)) = size(sizes1, 0) {
        if u < xoff {
            px = xoff / u;
        }
        if v < yoff {
            py = yoff / v;
        }
    }
    let first = size(sizes, 0)?;
    if sizes1[3].is_some() {
        let (s1, t1) = (size(sizes1, 1)?, size(sizes1, 2)?);
        let whole = (s1.0 + xoff, t1.1 + yoff);
        let (second, third, fourth) = (size(sizes, 1)?, size(sizes, 2)?, size(sizes, 3)?);
        let mut out = vec![
            (0, [0.0, 0.0, first.0 * px, first.1 * py]),
            (1, [xoff, 0.0, second.0, second.1]),
            (2, [0.0, yoff, third.0 * px, third.1 * py]),
            (3, [xoff, yoff, fourth.0 * px, fourth.1]),
        ];
        if let Some(adjust) = size(sizes, 4) {
            out.push((4, [xoff, second.1, adjust.0 * px, adjust.1]));
        }
        Some((whole, out))
    } else if sizes1[1].is_some() {
        let s1 = size(sizes1, 1)?;
        let whole = (s1.0 * px + xoff, s1.1 * py + yoff);
        let second = size(sizes, 1)?;
        Some((whole, vec![(0, [0.0, 0.0, first.0 * px, first.1 * py]), (1, [xoff, yoff, second.0 * px, second.1 * py])]))
    } else {
        None
    }
}

/// The pieces drawn in order on a `w` × `h` picture (nearest texel, each piece replacing what is
/// under it, clipped to the picture as DrawStretchedTexture clips to the box).
pub fn compose(w: u32, h: u32, pieces: &[Piece]) -> Image {
    let mut out = Image::new(w, h);
    for p in pieces {
        if p.w <= 0.0 || p.h <= 0.0 || p.image.w == 0 || p.image.h == 0 {
            continue;
        }
        let (x0, y0) = (p.x.floor().max(0.0) as u32, p.y.floor().max(0.0) as u32);
        let (x1, y1) = (((p.x + p.w).ceil().max(0.0) as u32).min(w), ((p.y + p.h).ceil().max(0.0) as u32).min(h));
        for y in y0..y1 {
            let v = ((y as f32 + 0.5 - p.y) / p.h * p.image.h as f32).floor();
            if v < 0.0 || v >= p.image.h as f32 {
                continue;
            }
            for x in x0..x1 {
                let u = ((x as f32 + 0.5 - p.x) / p.w * p.image.w as f32).floor();
                if u < 0.0 || u >= p.image.w as f32 {
                    continue;
                }
                let i = ((y * w + x) * 4) as usize;
                out.rgba[i..i + 4].copy_from_slice(&p.image.pixel(u as u32, v as u32));
            }
        }
    }
    out
}

/// Rows kept by a box showing `kept` of the `whole` height of its original box, starting `skip`
/// below the top (same units): the picture keeps its scale (cropped, not squashed).
pub fn window_rows(h: u32, skip: f32, kept: f32, whole: f32) -> (u32, u32) {
    let from = ((h as f32 * skip / whole).round() as u32).min(h.saturating_sub(1));
    let to = (from + (h as f32 * kept / whole).round() as u32).clamp(from + 1, h.max(from + 1));
    (from, to.min(h))
}

/// Grey version of a colour picture, for bank 0 (the box at rest): the original *gris textures
/// are close to the colour ones in luminance, slightly less contrasted and lifted towards white.
/// Per channel: out = GREY_GAIN * Y + GREY_BASE[c], Y the Rec. 601 luminance (alpha kept).
pub fn to_grey(img: &Image) -> Image {
    let mut out = img.clone();
    for px in out.rgba.chunks_exact_mut(4) {
        let y = 0.299 * px[0] as f32 + 0.587 * px[1] as f32 + 0.114 * px[2] as f32;
        for c in 0..3 {
            px[c] = (GREY_GAIN[c] * y + GREY_BASE[c]).round().clamp(0.0, 255.0) as u8;
        }
    }
    out
}

/// Least-squares fit on the original menu pictures (the 15 colour textures against their *gris
/// versions, 573 440 pixels; 13 / 255 of mean difference left): see the test
/// `grey_matches_the_original_grey_pictures`.
const GREY_GAIN: [f32; 3] = [0.912, 0.912, 0.912];
const GREY_BASE: [f32; 3] = [64.5, 64.4, 64.5];

/// The new boxes of the main menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NewBox {
    Mods,
    Coop,
}

/// Where the picture of a new box comes from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BoxPicture {
    /// A frame of one of the game's videos: file name in the Video folder (without .bik), frame
    /// index counted from 0, and the window kept.
    VideoFrame { file: &'static str, frame: u32, crop: Crop },
}

/// Mods box: Cine01, the glove box (gun, papers). 928: the glove box alone; 902: with XIII's hand
/// reaching for the gun.
pub const MODS_VIDEO: &str = "Cine01";
pub const MODS_FRAME: u32 = 928;
/// Window of the Mods frame (640 × 480, letterboxed): the inside of the glove box with its rim.
const MODS_CROP: Crop = Crop { cx: 0.461, cy: 0.496, h: 0.446 };
/// Co-op box (later, under Load): Cine09, frame 131.
pub const COOP_VIDEO: &str = "Cine09";
/// Co-op box (under Load): XIII and Jones in the jungle, Jones lowering her binoculars.
pub const COOP_FRAME: u32 = 1137;
/// Window of the Co-op frame: the whole width, from the leaves at the top of the picture (the
/// black bars are left out by crop_to_aspect).
const COOP_CROP: Crop = Crop { cx: 0.5, cy: 0.49, h: 0.55 };
/// The Co-op box under Load and the multiplayer pages' "Players on this screen" setting: on unless
/// the player turned co-op off on the Mods page (set by the game window at startup).
static COOP_BOX: AtomicBool = AtomicBool::new(true);

/// Co-op in the menus (the Co-op box, the players setting) on or off, from the next main menu.
pub fn set_coop_box(on: bool) {
    COOP_BOX.store(on, Ordering::Relaxed);
}

/// Co-op is in the menus.
pub fn coop_box() -> bool {
    COOP_BOX.load(Ordering::Relaxed)
}

/// The cuts made on the main menu: Options for the Mods box (with mod support), Load for the
/// Co-op box (when co-op is on; otherwise Load keeps its original box, picture and label).
fn cuts(coop: bool) -> impl Iterator<Item = &'static Cut> {
    CUTS.iter().filter(move |c| (c.which != NewBox::Coop || coop) && (c.which != NewBox::Mods || cfg!(feature = "mods")))
}

fn mods_frame() -> u32 {
    MODS_FRAME
}

/// Picture chosen for each new box.
pub fn picture_of(which: NewBox) -> BoxPicture {
    match which {
        NewBox::Mods => BoxPicture::VideoFrame { file: MODS_VIDEO, frame: mods_frame(), crop: MODS_CROP },
        NewBox::Coop => BoxPicture::VideoFrame { file: COOP_VIDEO, frame: COOP_FRAME, crop: COOP_CROP },
    }
}

/// Video file and frame index of a new box's picture.
pub fn box_video(which: NewBox, video_dir: &Path) -> (std::path::PathBuf, u32) {
    let BoxPicture::VideoFrame { file, frame, .. } = picture_of(which);
    (video_dir.join(format!("{file}.bik")), frame)
}

/// Colour picture of a new box with the box's `aspect`, from the player's own video files in
/// `video_dir`, when its frame is ready (None while it is being made, or without the video or
/// ffmpeg). Never blocks.
pub fn menu_box_frame(which: NewBox, video_dir: &Path, aspect: f32) -> Option<Image> {
    let BoxPicture::VideoFrame { crop, .. } = picture_of(which);
    let (video, frame) = box_video(which, video_dir);
    let f = crate::still_frames::get(&video, frame)?;
    Some(crop_to_aspect(&Image { w: f.w, h: f.h, rgba: f.rgba.clone() }, aspect, crop))
}

/// Picture shown until (or instead of) the video frame: `placeholder` (a part of the box above
/// it) cropped to the box's `aspect`.
pub fn menu_box_placeholder(aspect: f32, placeholder: &Image) -> Image {
    crop_to_aspect(placeholder, aspect, Crop { cx: 0.5, cy: 0.5, h: 1.0 })
}

/// Starts making, in the background, every video frame the added menus show (Mods and Co-op
/// boxes, Select level background), so they are ready (or read from the cache) when shown.
pub fn prefetch_pictures(video_dir: &Path) {
    for (video, frame) in video_frames(video_dir) {
        crate::still_frames::request(&video, frame);
    }
}

/// Every video frame the added menus show: video file and frame index.
pub fn video_frames(video_dir: &Path) -> Vec<(std::path::PathBuf, u32)> {
    let mut out: Vec<_> = cuts(coop_box()).map(|c| box_video(c.which, video_dir)).collect();
    out.push((video_dir.join(format!("{}.bik", crate::select_level_art::BACKGROUND_VIDEO)), crate::select_level_art::BACKGROUND_FRAME));
    out
}

/// Arrow keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    Up,
    Down,
    Left,
    Right,
}

/// Indices in the page's Controls: the original six, then the new boxes.
pub const CONTINUE: usize = 0;
pub const MULTI: usize = 1;
pub const LOAD: usize = 2;
pub const OPTIONS: usize = 3;
pub const NEW: usize = 4;
pub const QUIT: usize = 5;
pub const MODS: usize = 6;
pub const COOP: usize = 7;

/// Arrow-key move of the improved menu from box `from`; None where the original page's own
/// table (XIIIMenu.InternalOnKeyEvent) is kept. `coop`: the Co-op box exists.
/// Layout (640 × 480): top row Continue, Quit, Multi (tall); bottom row Load over Co-op,
/// Options over Mods, New. A box with a new neighbour changes: below Load and Options, and the
/// moves that wrap around to the bottom of a column or the start of a row now land on the new
/// box there (up from Continue and Quit, right from New). The new boxes get their own moves
/// (down wraps to the top row, as the original does from the bottom row).
pub fn navigate(from: usize, dir: Dir, coop: bool) -> Option<usize> {
    match (from, dir) {
        (LOAD, Dir::Down) if coop => Some(COOP),
        (CONTINUE, Dir::Up) if coop => Some(COOP),
        (NEW, Dir::Right) if coop => Some(COOP),
        (QUIT, Dir::Up) => Some(MODS),
        (OPTIONS, Dir::Down) => Some(MODS),
        (MODS, Dir::Up) => Some(OPTIONS),
        (MODS, Dir::Down) => Some(QUIT),
        (MODS, Dir::Left) => Some(if coop { COOP } else { LOAD }),
        (MODS, Dir::Right) => Some(NEW),
        (COOP, Dir::Up) => Some(LOAD),
        (COOP, Dir::Down) => Some(CONTINUE),
        (COOP, Dir::Left) => Some(NEW),
        (COOP, Dir::Right) => Some(MODS),
        _ => None,
    }
}

/// Height of Options and Load in the original page (640 × 480 units, multiplied by fScaleTo).
const WHOLE_HEIGHT: f32 = 252.0;
/// Boxes (640 × 480 units, heights multiplied by fScaleTo): x, y, width, height. Options keeps
/// most of its height, Mods is a strip below with the gap of the menu's other boxes.
const OPTIONS_KEPT: [f32; 4] = [287.0, 181.0, 156.0, 188.0];
const MODS_BOX: [f32; 4] = [287.0, 379.0, 156.0, 54.0];
/// Options' picture starts this far below its top (units of the original box): the computer
/// screen and the keyboard stay in the box.
const OPTIONS_SKIP: f32 = 23.0;
/// Load keeps the top of its box, Co-op takes the rest below, with the gap of the menu's other
/// boxes (Continue to Load); Load keeps the top of its picture (the room and the woman in it).
const LOAD_KEPT: [f32; 4] = [27.0, 181.0, 244.0, 138.0];
const COOP_BOX_RECT: [f32; 4] = [27.0, 334.0, 244.0, 99.0];
/// Load's label top moved into its cut box (originally 350).
const LOAD_LABEL_Y: f32 = 264.0;

/// Onomatopoeia of a box: texture, top left at full zoom (640 × 480 units), size in pixels (as
/// XIIIMenu.AfterPaint draws them).
type Onomatopoeia = (&'static str, f32, f32, f32, f32);
/// Label of a box: left, top (640 × 480 units), text (in the game's language).
type Label = (f32, f32, Text);

/// Mods' onomatopoeia, over Options' computer screen and Mods' gun, at the size
/// XIIIMenuMultiplayer draws the same texture (128 × 64), zooming from its bottom right corner
/// like the others.
const MODS_ONOMATOPOEIA: Onomatopoeia = ("XIIIMenuStart.Controle_PC.clapclap", 330.0, 336.0, 128.0, 64.0);
const MODS_LABEL: Label = (355.0, 389.0, Text::MenuMods);

/// Co-op's onomatopoeia at its original size (XIIIMenuStart draws it 224 × 64), over the bottom of
/// the box and past its right edge; the label on the left of the box, like Load's.
const COOP_ONOMATOPOEIA: Onomatopoeia = ("XIIIMenuStart.onlineBaomm", 146.0, 369.0, 224.0, 64.0);
const COOP_LABEL: Label = (10.0, 384.0, Text::MenuCoop);

/// Load page: the Select level entry, below the save slots (XIIIMenuLoadGameWindow lays out
/// MaxViewable = 5 slots from y 160, 40 apart, 30 high, all heights multiplied by fScaleTo).
const LEVEL_ENTRY: [f32; 4] = [50.0, 370.0, 320.0, 30.0];
const LEVEL_ENTRY_TEXT: Text = Text::SelectLevel;

/// Name under which `menu_register` registers a picture: `name` and the picture's size.
fn registered_name(name: &str, img: &Image) -> String {
    format!("{name}_{}x{}", img.w, img.h)
}

/// Texture names of the composed pictures (bank 0 grey, bank 1 colour).
fn texture_name(what: &str, bank: usize) -> String {
    format!("Mod.MenuBox.{what}{}", ["Grey", "Colour"][bank])
}

/// The Select level entry exists (the game window has the level list); set by the window.
static SELECT_LEVEL_ENTRY: AtomicBool = AtomicBool::new(false);

/// The game window can open the level list: the Load page gets its Select level entry.
pub fn set_select_level_entry(on: bool) {
    SELECT_LEVEL_ENTRY.store(on, Ordering::Relaxed);
}

/// What a click on a new box asks of the game window (the Co-op box's page is the VM's own).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuAction {
    Mods,
}

/// The new controls of the main menu and Load pages in front.
#[derive(Default)]
pub struct MenuBoxes {
    pub(crate) page: Option<ObjId>,
    mods: Option<ObjId>,
    pub(crate) coop: Option<ObjId>,
    load_page: Option<ObjId>,
    level_entry: Option<ObjId>,
    action: Option<MenuAction>,
    /// New boxes still showing their placeholder: (box, button, aspect), swapped when the video
    /// frame is ready.
    waiting: Vec<(NewBox, ObjId, f32)>,
    /// A full-screen page of the game window (the Mods page) is open over the menu.
    overlay: bool,
    /// The Select level page (select_level_page.rs).
    pub(crate) select: crate::select_level_page::SelectPage,
    /// The Co-op page (coop_page.rs).
    pub(crate) coop_page: crate::coop_page::CoopPage,
    /// The Co-op page was left: its box takes the focus at the main menu's next AfterPaint.
    pub(crate) refocus_coop: bool,
    /// An added page (Mods, Co-op) closing back to the main menu: its hMainPage gives only the
    /// arrival hit (menu_sounds.rs, MAIN_ARRIVAL).
    pub(crate) arrival_only: bool,
    /// The multiplayer page in front and its "Players on this screen" control.
    pub(crate) players_setting: Option<(ObjId, ObjId)>,
    /// The Join LAN page's address box and Join entry (multi_join.rs).
    pub(crate) multi_join: crate::multi_join::MultiJoin,
}

/// A cut of an original box: its button property, texture name, the part it keeps and where its
/// picture starts, the new box below and its kind, and its label moved (property, top).
struct Cut {
    prop: &'static str,
    what: &'static str,
    kept: [f32; 4],
    skip: f32,
    below: [f32; 4],
    which: NewBox,
    label: Option<(&'static str, f32)>,
}

const CUTS: [Cut; 2] = [
    Cut { prop: "OptionsButton", what: "Options", kept: OPTIONS_KEPT, skip: OPTIONS_SKIP, below: MODS_BOX, which: NewBox::Mods, label: None },
    Cut { prop: "LoadButton", what: "Load", kept: LOAD_KEPT, skip: 0.0, below: COOP_BOX_RECT, which: NewBox::Coop, label: Some(("LoadLabel", LOAD_LABEL_Y)) },
];

/// Class and name of the hooked functions: (XIIIMenu, XIIIMenuLoadGameWindow).
fn hooked(class: &str, name: &str) -> Option<bool> {
    if class.eq_ignore_ascii_case("XIIIMenu") && matches!(name, "Created" | "AfterPaint" | "InternalOnClick" | "InternalOnKeyEvent") {
        Some(true)
    } else if class.eq_ignore_ascii_case("XIIIMenuLoadGameWindow") && matches!(name, "Created" | "Paint" | "InternalOnKeyEvent") {
        Some(false)
    } else {
        None
    }
}

impl Vm {
    /// The Load page in front, when it has the Select level entry (the level list may open over
    /// it).
    pub fn load_page_in_front(&self) -> Option<ObjId> {
        self.improved.menu_boxes.level_entry?;
        let page = self.gui_local().and_then(|c| self.propr(c, "ActivePage").as_object())?;
        (Some(page) == self.improved.menu_boxes.load_page && self.objects.get(page).is_some_and(|o| o.is_some())).then_some(page)
    }

    /// Click on a new box or entry since the last call (taken).
    pub fn take_menu_action(&mut self) -> Option<MenuAction> {
        self.improved.menu_boxes.action.take()
    }

    /// The game window has a full-screen page open over the menu (the Mods page): the menu behind
    /// is not hovered (gui.rs, LookUnderCursor).
    pub fn set_menu_overlay(&mut self, open: bool) {
        let closed = self.improved.menu_boxes.overlay && !open;
        self.improved.menu_boxes.overlay = open;
        // Back on the main menu from a full-screen page (Mods): its sound of a page closed over
        // it, as GUIController.CloseMenu's ActivePage.OnReOpen gives it for the game's pages
        // (XIIIMenu: InternalOnOpen, hSoundMenu1 AmbianceMenu__hMainPage), the arrival hit
        // alone, as the main menu's loop went on under the page (menu_sounds.rs).
        if closed && crate::menu_mouse::enabled(self.engine.client.session_graphics) {
            if let Some(menu) = self.menu_main() {
                self.with_main_arrival_only(|vm| {
                    let _ = vm.call_named(menu, "InternalOnOpen", vec![]);
                });
            }
        }
    }

    pub(crate) fn menu_overlay_open(&self) -> bool {
        self.improved.menu_boxes.overlay
    }

    /// Hook of the XIIIMenu and XIIIMenuLoadGameWindow functions (Vm::call): the result when it
    /// is handled here, otherwise the arguments back.
    #[allow(clippy::type_complexity)]
    pub(crate) fn menu_boxes_call(&mut self, f: usize, this: ObjId, mut args: Vec<Option<Value>>) -> std::result::Result<Result<(Value, Vec<Option<Value>>)>, Vec<Option<Value>>> {
        let d = &self.functions[f];
        if d.state.is_some() {
            return Err(args);
        }
        // The Co-op and Select level pages' XIIIWindow functions.
        if matches!(d.name.as_str(), "Created" | "Paint" | "AfterPaint" | "InternalOnKeyEvent") && self.classes[d.owner].name.eq_ignore_ascii_case("XIIIWindow") {
            let name = d.name.clone();
            if self.character_page_of(this).is_some() || self.improved.character_screen.opening.is_some() {
                return self.character_page_call(f, this, &name, args);
            }
            if self.improved.menu_boxes.coop_page.page == Some(this) || self.improved.menu_boxes.coop_page.is_pending() {
                return self.coop_page_call(f, this, &name, args);
            }
            if self.improved.menu_boxes.select.page == Some(this) || self.improved.menu_boxes.select.is_pending() {
                return self.select_page_call(f, this, &name, args);
            }
        }
        // The multiplayer pages' players setting (not there when co-op is off: the pages are the
        // original ones).
        // The Join LAN page's address box and Join entry (multi_join.rs), with the same switch.
        if coop_box() && crate::multi_join::multi_join_hooked(&self.classes[d.owner].name, &d.name) {
            let name = d.name.clone();
            match self.multi_join_call(f, this, &name, args) {
                Ok(r) => return Ok(r),
                Err(a) => args = a,
            }
        }
        let d = &self.functions[f];
        if coop_box() && crate::coop_page::players_setting_hooked(&self.classes[d.owner].name, &d.name) {
            let (class, name) = (self.classes[d.owner].name.clone(), d.name.clone());
            return self.players_setting_call(f, this, &class, &name, args);
        }
        let Some(main) = hooked(&self.classes[d.owner].name, &d.name) else { return Err(args) };
        let name = d.name.clone();
        if !main {
            return self.load_page_call(f, this, &name, args);
        }
        match name.as_str() {
            "Created" => {
                let r = self.call_inner(f, this, args);
                if r.is_ok() {
                    self.menu_boxes_created(this);
                }
                Ok(r)
            }
            "AfterPaint" if self.improved.menu_boxes.page == Some(this) => {
                let canvas = args.first().cloned().flatten().and_then(|v| v.as_object());
                let r = self.call_inner(f, this, args);
                if let Some(c) = canvas {
                    self.menu_boxes_after_paint(this, c);
                }
                Ok(r)
            }
            "InternalOnClick" if self.improved.menu_boxes.page == Some(this) => {
                let sender = args.first().cloned().flatten().and_then(|v| v.as_object());
                if sender.is_some() && sender == self.improved.menu_boxes.coop {
                    self.open_coop_page();
                    return Ok(Ok((Value::Bool(true), Vec::new())));
                }
                let action = match sender {
                    Some(s) if Some(s) == self.improved.menu_boxes.mods => Some(MenuAction::Mods),
                    _ => None,
                };
                if let Some(a) = action {
                    self.improved.menu_boxes.action = Some(a);
                    return Ok(Ok((Value::Bool(true), Vec::new())));
                }
                let r = self.call_inner(f, this, args);
                // New game video started: the new boxes stop taking the focus, as the others.
                if self.propr(this, "VP").as_object().is_some() {
                    for b in [self.improved.menu_boxes.mods, self.improved.menu_boxes.coop].into_iter().flatten() {
                        self.set_prop(b, "bNeverFocus", Value::Bool(true));
                    }
                }
                Ok(r)
            }
            "InternalOnKeyEvent" if self.improved.menu_boxes.page == Some(this) => {
                let byte = |i: usize| args.get(i).cloned().flatten().map_or(0, |v| v.as_int());
                let (key, state) = (byte(0), byte(1));
                let dir = match key {
                    37 => Some(Dir::Left),
                    38 => Some(Dir::Up),
                    39 => Some(Dir::Right),
                    40 => Some(Dir::Down),
                    _ => None,
                };
                let playing = self.propr(this, "VP").as_object().is_some();
                if let (1, Some(dir), false) = (state, dir, playing) {
                    let controls = self.controls(this);
                    // Logical indices (MODS, COOP) whatever their place in Controls.
                    let (mods, coop) = (self.improved.menu_boxes.mods, self.improved.menu_boxes.coop);
                    let focused = self.propr(this, "FocusedControl").as_object();
                    let from = focused.and_then(|o| match Some(o) {
                        x if x == mods => Some(MODS),
                        x if x == coop => Some(COOP),
                        x => controls.iter().take(MODS).position(|c| *c == x),
                    });
                    let target = from.and_then(|i| navigate(i, dir, coop.is_some())).and_then(|t| match t {
                        MODS => mods,
                        COOP => coop,
                        t => controls.get(t).copied().flatten(),
                    });
                    if let Some(target) = target {
                        let _ = self.call_named(target, "SetFocus", vec![Value::Object(None)]);
                        return Ok(Ok((Value::Bool(true), self.key_event_outs(f, &args))));
                    }
                }
                Ok(self.call_inner(f, this, args))
            }
            _ => Err(args),
        }
    }

    /// The page's Controls.
    pub(crate) fn controls(&self, page: ObjId) -> Vec<Option<ObjId>> {
        match self.prop(page, "Controls") {
            Value::Array(c) => c.iter().map(|v| v.as_object()).collect(),
            _ => Vec::new(),
        }
    }

    /// Out parameters of a key event handled here (Key and State unchanged).
    pub(crate) fn key_event_outs(&self, f: usize, args: &[Option<Value>]) -> Vec<Option<Value>> {
        self.functions[f].params.iter().enumerate().map(|(i, p)| if p.out { Some(args.get(i).cloned().flatten().unwrap_or(Value::Byte(0))) } else { None }).collect()
    }

    /// XIIIMenuLoadGameWindow: the Select level entry made after Created, its frame cleared before
    /// Paint (Paint only clears the save slots' before framing the focused control), and Enter or
    /// a click on it opening the level list (the page would read it as save slot 6).
    #[allow(clippy::type_complexity)]
    fn load_page_call(&mut self, f: usize, this: ObjId, name: &str, args: Vec<Option<Value>>) -> std::result::Result<Result<(Value, Vec<Option<Value>>)>, Vec<Option<Value>>> {
        match name {
            "Created" => {
                let r = self.call_inner(f, this, args);
                if r.is_ok() {
                    self.load_page_created(this);
                }
                Ok(r)
            }
            "Paint" if self.improved.menu_boxes.load_page == Some(this) => {
                if let Some(e) = self.improved.menu_boxes.level_entry {
                    self.set_prop(e, "bUseBorder", Value::Bool(false));
                }
                Ok(self.call_inner(f, this, args))
            }
            "InternalOnKeyEvent" if self.improved.menu_boxes.load_page == Some(this) && self.improved.menu_boxes.level_entry.is_some() => {
                let byte = |i: usize| args.get(i).cloned().flatten().map_or(0, |v| v.as_int());
                let (key, state) = (byte(0), byte(1));
                let focused = self.propr(this, "FocusedControl").as_object();
                if state == 1 && matches!(key, 1 | 13) && focused.is_some() && focused == self.improved.menu_boxes.level_entry && !self.propr(this, "bIgnoreKeys").as_bool() {
                    let outs = self.key_event_outs(f, &args);
                    self.open_select_level_page();
                    return Ok(Ok((Value::Bool(true), outs)));
                }
                // Down: the page wraps from its last save slot to the first one, never reaching the
                // entry below them; from the entry, it goes back to the first slot.
                let entry = self.improved.menu_boxes.level_entry;
                if state == 1 && key == 0x28 && focused.is_some() && focused == entry && !self.propr(this, "bIgnoreKeys").as_bool() {
                    if let Some(first) = self.load_page_slots(this).first().map(|&(_, c)| c) {
                        let _ = self.call_named(first, "SetFocus", vec![Value::Object(None)]);
                        return Ok(Ok((Value::Bool(true), self.key_event_outs(f, &args))));
                    }
                }
                let r = self.call_inner(f, this, args);
                if state == 1 && key == 0x28 {
                    let slots = self.load_page_slots(this);
                    let top = |c: Option<ObjId>| slots.iter().find(|&&(_, s)| Some(s) == c).map(|&(t, _)| t);
                    let now = self.propr(this, "FocusedControl").as_object();
                    // Moved up from a slot: the wrap from the last one; the entry comes first.
                    if let (Some(e), Some(was), Some(is)) = (entry, top(focused), top(now)) {
                        if is < was {
                            let _ = self.call_named(e, "SetFocus", vec![Value::Object(None)]);
                        }
                    }
                }
                Ok(r)
            }
            _ => Err(args),
        }
    }

    /// The Load page's buttons other than the Select level entry (the save slots), with their top
    /// edge, from the top.
    fn load_page_slots(&self, page: ObjId) -> Vec<(f32, ObjId)> {
        let entry = self.improved.menu_boxes.level_entry;
        let class = entry.and_then(|e| self.objects[e].as_ref().map(|o| o.class));
        // As wide as the entry (the page's arrows are buttons too).
        let width = entry.map_or(0.0, |e| self.propr(e, "WinWidth").as_float());
        let mut slots: Vec<(f32, ObjId)> = self
            .controls(page)
            .into_iter()
            .flatten()
            .filter(|&c| Some(c) != entry && self.objects[c].as_ref().map(|o| o.class) == class)
            .filter(|&c| (self.propr(c, "WinWidth").as_float() - width).abs() <= width * 0.2)
            .map(|c| (self.propr(c, "WinTop").as_float(), c))
            .collect();
        slots.sort_by(|a, b| a.0.total_cmp(&b.0));
        slots
    }

    /// After XIIIMenuLoadGameWindow.Created: an XIIIButton like the save slots, below them.
    fn load_page_created(&mut self, page: ObjId) {
        self.improved.menu_boxes.load_page = Some(page);
        self.improved.menu_boxes.level_entry = None;
        if !SELECT_LEVEL_ENTRY.load(Ordering::Relaxed) {
            return;
        }
        let scale = self.page_scale(page);
        let Some(class) = self.class("XIIIButton") else { return };
        let [x, y, w, h] = LEVEL_ENTRY;
        let args = vec![Value::Class(Some(class)), Value::Float(x), Value::Float(160.0 + (y - 160.0) * scale), Value::Float(w), Value::Float(h * scale)];
        let Some(entry) = self.call_named(page, "CreateControl", args).ok().and_then(|v| v.as_object()) else { return };
        self.set_prop(entry, "Text", Value::Str(self.added_text(LEVEL_ENTRY_TEXT).to_string()));
        if let Value::Array(mut controls) = self.prop(page, "Controls") {
            controls.push(Value::Object(Some(entry)));
            self.set_prop(page, "Controls", Value::Array(controls));
        }
        self.improved.menu_boxes.level_entry = Some(entry);
    }

    pub(crate) fn page_scale(&self, page: ObjId) -> f32 {
        let scale = self.propr(page, "fScaleTo").as_float();
        if scale > 0.0 {
            scale
        } else {
            1.0
        }
    }

    /// Element `i` of a static array property set to `v`.
    fn set_prop_elem(&mut self, obj: ObjId, name: &str, i: usize, v: Value) {
        if let Value::Array(mut a) = self.prop(obj, name) {
            if let Some(slot) = a.get_mut(i) {
                *slot = v;
                self.set_prop(obj, name, Value::Array(a));
            }
        }
    }

    fn prop_elem(&self, obj: ObjId, name: &str, i: usize) -> Value {
        match self.propr(obj, name) {
            Value::Array(a) => a.get(i).cloned().unwrap_or(Value::Void),
            _ => Value::Void,
        }
    }

    /// RGBA of the first level of game texture `path` (as the renderer uploads it).
    pub(crate) fn menu_texture_image(&mut self, path: &str) -> Option<Image> {
        let library = self.library.as_mut()?;
        let (loaded, index) = library.resolve(path)?;
        let t = openxiii_assets::texture::Texture::read(&loaded.package, &loaded.data, index).ok()?;
        let (w, h, rgba) = t.d3d_mips().into_iter().next()?;
        Some(Image { w, h, rgba })
    }

    /// The whole picture of a button's bank, composed as Paint draws it (None if the button
    /// draws a single texture or a texture cannot be read).
    fn button_picture(&mut self, button: ObjId, bank: usize) -> Option<Image> {
        const SLOTS: [&str; 5] = ["tFirstTex", "tSecondTex", "tThirdTex", "tFourthTex", "tAdjustTex"];
        let mut images: [Option<Image>; 5] = Default::default();
        let mut sizes1: Slots = [None; 5];
        for (i, slot) in SLOTS.iter().enumerate() {
            if let Value::Asset(p) = self.prop_elem(button, slot, 1) {
                sizes1[i] = self.menu_texture_image(&p).map(|m| (m.w, m.h));
            }
            if let Value::Asset(p) = self.prop_elem(button, slot, bank) {
                images[i] = Some(self.menu_texture_image(&p)?);
            }
        }
        let sizes: Slots = std::array::from_fn(|i| images[i].as_ref().map(|m| (m.w, m.h)));
        let xoff = self.propr(button, "xoff").as_float();
        let yoff = self.propr(button, "yoff").as_float();
        let ((w, h), layout) = paint_layout(&sizes, &sizes1, xoff, yoff)?;
        let pieces: Vec<Piece> = layout.iter().filter_map(|&(i, [x, y, pw, ph])| Some(Piece { image: images[i].as_ref()?, x, y, w: pw, h: ph })).collect();
        Some(compose(w.round() as u32, h.round() as u32, &pieces))
    }

    /// Registers `img` as a texture named after `name` and its size, and returns its value. The
    /// size is in the name because a texture's size is read once per name (the props of a
    /// runtime texture, openxiii_assets::props): a picture of another size registered under the
    /// same name (the main menu rebuilt with a box's real picture after its placeholder) was
    /// drawn with the first size, tiled.
    pub(crate) fn menu_register(name: &str, img: Image) -> Value {
        Self::menu_register_with(name, img, false)
    }

    /// `menu_register`, `fine`: a master picture read with the bicubic filter at any screen size
    /// (mod_textures::set_runtime_fine).
    pub(crate) fn menu_register_with(name: &str, img: Image, fine: bool) -> Value {
        let name = registered_name(name, &img);
        let mips = openxiii_assets::mod_textures::mips(img.w, img.h, img.rgba);
        if fine {
            openxiii_assets::mod_textures::set_runtime_fine(&name, mips);
        } else {
            openxiii_assets::mod_textures::set_runtime(&name, mips);
        }
        Value::Asset(name)
    }

    /// `button` draws `pictures` (bank 0, bank 1) stretched to its box `rect`.
    fn menu_set_box(&mut self, button: ObjId, pictures: [Value; 2], rect: [f32; 4], scale: f32) {
        for slot in ["tSecondTex", "tThirdTex", "tFourthTex", "tAdjustTex"] {
            for bank in 0..2 {
                self.set_prop_elem(button, slot, bank, Value::Object(None));
            }
        }
        for (bank, p) in pictures.into_iter().enumerate() {
            self.set_prop_elem(button, "tFirstTex", bank, p);
        }
        self.set_prop(button, "xoff", Value::Float(0.0));
        self.set_prop(button, "yoff", Value::Float(0.0));
        self.set_prop(button, "WinLeft", Value::Float(rect[0] / 640.0));
        self.set_prop(button, "WinTop", Value::Float(rect[1] * scale / 480.0));
        self.set_prop(button, "WinWidth", Value::Float(rect[2] / 640.0));
        self.set_prop(button, "WinHeight", Value::Float(rect[3] * scale / 480.0));
    }

    /// After XIIIMenu.Created: Options (and Load, with the Co-op box) cut, the new boxes created
    /// and appended to Controls, the labels moved.
    fn menu_boxes_created(&mut self, page: ObjId) {
        self.improved.menu_boxes = MenuBoxes {
            page: Some(page),
            load_page: self.improved.menu_boxes.load_page,
            level_entry: self.improved.menu_boxes.level_entry,
            overlay: self.improved.menu_boxes.overlay,
            ..Default::default()
        };
        let scale = self.page_scale(page);
        let Some(class) = self.class("XIIITextureButton") else { return };
        let video_dir = self.engine.video.dir.clone();
        self.select_level_prefetch_jones();
        for cut in cuts(coop_box()) {
            let Some(button) = self.propr(page, cut.prop).as_object() else { continue };
            let (Some(grey), Some(colour)) = (self.button_picture(button, 0), self.button_picture(button, 1)) else {
                self.warn(format!("main menu: pictures of {} unreadable, no {:?} box", cut.prop, cut.which));
                continue;
            };
            let (from, to) = window_rows(colour.h, cut.skip, cut.kept[3], WHOLE_HEIGHT);
            let kept = [Self::menu_register(&texture_name(cut.what, 0), grey.rows(from, to)), Self::menu_register(&texture_name(cut.what, 1), colour.rows(from, to))];
            self.menu_set_box(button, kept, cut.kept, scale);
            // The new box: its picture (the rows the cut leaves out if the video is missing), and
            // the grey made from it.
            let leftover = if to < colour.h { colour.rows(to, colour.h) } else { colour.rows(0, from.max(1)) };
            let aspect = cut.below[2] / (cut.below[3] * scale);
            let frame = menu_box_frame(cut.which, &video_dir, aspect);
            let ready = frame.is_some();
            let picture = frame.unwrap_or_else(|| menu_box_placeholder(aspect, &leftover));
            let name = format!("{:?}", cut.which);
            let pictures = [Self::menu_register(&texture_name(&name, 0), to_grey(&picture)), Self::menu_register(&texture_name(&name, 1), picture)];
            let below = cut.below;
            let args = vec![Value::Class(Some(class)), Value::Float(below[0]), Value::Float(below[1] * scale), Value::Float(below[2]), Value::Float(below[3] * scale)];
            let Some(new) = self.call_named(page, "CreateControl", args).ok().and_then(|v| v.as_object()) else { continue };
            self.menu_set_box(new, pictures, below, scale);
            let (video, index) = box_video(cut.which, &video_dir);
            if !ready && !crate::still_frames::is_done(&video, index) {
                self.improved.menu_boxes.waiting.push((cut.which, new, aspect));
            }
            if let Value::Array(mut controls) = self.prop(page, "Controls") {
                controls.push(Value::Object(Some(new)));
                self.set_prop(page, "Controls", Value::Array(controls));
            }
            match cut.which {
                NewBox::Mods => self.improved.menu_boxes.mods = Some(new),
                NewBox::Coop => self.improved.menu_boxes.coop = Some(new),
            }
            if let Some((label, label_y)) = cut.label {
                if let Value::Struct(n, mut fields) = self.prop(page, label) {
                    if let Some(y) = fields.get_mut(1) {
                        *y = Value::Float(label_y * scale);
                    }
                    self.set_prop(page, label, Value::Struct(n, fields));
                }
            }
        }
    }

    /// Onomatopoeia `tex` zoomed like XIIIMenu.AfterPaint's, then the label.
    fn menu_draw_hover(&mut self, page: ObjId, c: ObjId, button: ObjId, tex: Value, (x, y, w, h): (f32, f32, f32, f32), label: Value) {
        let zoom = self.propr(button, "zoom").as_float();
        let (rx, ry) = (self.propr(page, "fRatioX").as_float(), self.propr(page, "fRatioY").as_float());
        self.set_prop(c, "Style", Value::Byte(5));
        let args = vec![Value::Object(Some(c)), Value::Float(x * rx + w - w * zoom), Value::Float(y * ry + h - h * zoom), Value::Float(w * zoom), Value::Float(h * zoom), tex];
        let _ = self.call_named(page, "DrawStretchedTexture", args);
        let _ = self.call_named(page, "DrawLabel", vec![Value::Object(Some(c)), label]);
        self.set_prop(c, "Style", Value::Byte(1));
    }

    /// A label struct of the page (XIIILabel) at (x, y) with text `t`.
    fn menu_label(&self, page: ObjId, (x, y, t): Label, scale: f32) -> Value {
        match self.prop(page, "OptionsLabel") {
            Value::Struct(n, mut fields) if fields.len() >= 5 => {
                fields[0] = Value::Float(x);
                fields[1] = Value::Float(y * scale);
                fields[2] = Value::Float(128.0);
                fields[3] = Value::Float(32.0 * scale);
                fields[4] = Value::Str(self.added_text(t).to_string());
                Value::Struct(n, fields)
            }
            other => other,
        }
    }

    /// After XIIIMenu.AfterPaint: the onomatopoeia and label of each new box hovered.
    /// Boxes still showing their placeholder get their video frame once it is ready (under a
    /// new texture name: the renderer keeps what it uploaded for a name).
    fn menu_boxes_swap_ready(&mut self) {
        if self.improved.menu_boxes.waiting.is_empty() {
            return;
        }
        let video_dir = self.engine.video.dir.clone();
        for (which, button, aspect) in std::mem::take(&mut self.improved.menu_boxes.waiting) {
            let (video, index) = box_video(which, &video_dir);
            match menu_box_frame(which, &video_dir, aspect) {
                Some(picture) => {
                    let name = format!("{which:?}Video");
                    let grey = Self::menu_register(&texture_name(&name, 0), to_grey(&picture));
                    let colour = Self::menu_register(&texture_name(&name, 1), picture);
                    self.set_prop_elem(button, "tFirstTex", 0, grey);
                    self.set_prop_elem(button, "tFirstTex", 1, colour);
                }
                None if crate::still_frames::is_done(&video, index) => {}
                None => self.improved.menu_boxes.waiting.push((which, button, aspect)),
            }
        }
    }

    fn menu_boxes_after_paint(&mut self, page: ObjId, c: ObjId) {
        self.menu_boxes_swap_ready();
        // Back from the co-op character screen: the Co-op page again (character_page.rs).
        self.character_screen_reopen_coop();
        // Back from the Co-op page: the Co-op box gets the focus once the main menu is drawn
        // again (given at once, it was lost when the menu was shown again).
        if std::mem::take(&mut self.improved.menu_boxes.refocus_coop) {
            if let Some(b) = self.improved.menu_boxes.coop {
                // Improved menus (menu_mouse.rs): the menu shown again already had its focus tick
                // (CloseMenu's FocusFirst), as when leaving a game page: the move to the Co-op box
                // is silent; with the mouse last used the focus is the cursor's.
                let improved = crate::menu_mouse::enabled(self.engine.client.session_graphics);
                if !(improved && self.improved.menu_mouse.last == crate::menu_mouse::Device::Mouse) {
                    self.improved.menu_mouse.silent = improved;
                    let _ = self.call_named(b, "SetFocus", vec![Value::Object(None)]);
                    self.improved.menu_mouse.silent = false;
                }
            }
        }
        let scale = self.page_scale(page);
        for (button, (tex, x, y, w, h), label) in [(self.improved.menu_boxes.mods, MODS_ONOMATOPOEIA, MODS_LABEL),(self.improved.menu_boxes.coop, COOP_ONOMATOPOEIA, COOP_LABEL)] {
            let Some(b) = button.filter(|&b| self.propr(b, "bDisplayTex").as_bool()) else { continue };
            let texture = self.call_named(page, "DynamicLoadObject", vec![Value::Str(tex.to_string()), Value::Class(self.class("Texture"))]).unwrap_or(Value::Object(None));
            let label = self.menu_label(page, label, scale);
            self.menu_draw_hover(page, c, b, texture, (x, y, w, h), label);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: u32, h: u32, c: [u8; 4]) -> Image {
        Image { w, h, rgba: c.iter().copied().cycle().take((w * h * 4) as usize).collect() }
    }

    #[test]
    fn a_rebuilt_box_picture_of_another_size_gets_its_own_texture() {
        // The main menu built with a box's placeholder, then rebuilt once its video picture is
        // ready: same box name, another size. The first texture (its size read by the props of
        // that name) must stay as it was, the second one get its own name.
        let base = "Mod.MenuBox.TestRebuiltColour";
        let first = Vm::menu_register(base, solid(4, 2, [1, 2, 3, 255]));
        let second = Vm::menu_register(base, solid(8, 6, [9, 9, 9, 255]));
        let (Value::Asset(a), Value::Asset(b)) = (&first, &second) else { panic!("{first:?} {second:?}") };
        assert_ne!(a, b, "one name per size");
        let size = |n: &str| openxiii_assets::mod_textures::get(n).and_then(|m| m.first().map(|l| (l.0, l.1)));
        assert_eq!(size(a), Some((4, 2)), "the placeholder's texture is not replaced by a picture of another size");
        assert_eq!(size(b), Some((8, 6)));
        // The same size again: the same name (a picture redrawn in place).
        assert_eq!(Vm::menu_register(base, solid(8, 6, [7, 7, 7, 255])), second);
    }

    /// Image whose pixel (x, y) has red = x and green = y.
    fn coords(w: u32, h: u32) -> Image {
        Image { w, h, rgba: (0..h).flat_map(|y| (0..w).flat_map(move |x| [x as u8, y as u8, 0, 255])).collect() }
    }

    /// Load's five textures (xoff = yoff = 256) and Options' two (yoff = 256) are laid out as
    /// Paint draws them; a single texture has nothing to compose.
    #[test]
    fn layout_as_paint() {
        let load: Slots = [Some((256, 256)), Some((256, 128)), Some((256, 256)), Some((256, 256)), Some((256, 128))];
        let (whole, l) = paint_layout(&load, &load, 256.0, 256.0).unwrap();
        assert_eq!(whole, (512.0, 512.0));
        assert_eq!(l, vec![(0, [0.0, 0.0, 256.0, 256.0]), (1, [256.0, 0.0, 256.0, 128.0]), (2, [0.0, 256.0, 256.0, 256.0]), (3, [256.0, 256.0, 256.0, 256.0]), (4, [256.0, 128.0, 256.0, 128.0])]);
        let options: Slots = [Some((256, 256)), Some((256, 256)), None, None, None];
        let (whole, l) = paint_layout(&options, &options, 0.0, 256.0).unwrap();
        assert_eq!(whole, (256.0, 512.0));
        assert_eq!(l, vec![(0, [0.0, 0.0, 256.0, 256.0]), (1, [0.0, 256.0, 256.0, 256.0])]);
        // PS2 patch: a first texture smaller than the offset is stretched to it.
        let half: Slots = [Some((128, 128)), Some((128, 128)), None, None, None];
        let (whole, l) = paint_layout(&half, &half, 0.0, 256.0).unwrap();
        assert_eq!(whole, (128.0, 512.0));
        assert_eq!(l[0].1, [0.0, 0.0, 128.0, 256.0]);
        assert!(paint_layout(&[Some((256, 128)), None, None, None, None], &[Some((256, 128)), None, None, None, None], 0.0, 0.0).is_none());
    }

    /// Pieces land at their place, stretched by nearest texel and clipped to the picture.
    #[test]
    fn compose_places_and_clips() {
        let (red, blue) = (solid(2, 2, [255, 0, 0, 255]), solid(1, 1, [0, 0, 255, 255]));
        let out = compose(4, 4, &[Piece { image: &red, x: 0.0, y: 0.0, w: 4.0, h: 2.0 }, Piece { image: &blue, x: 2.0, y: 2.0, w: 4.0, h: 4.0 }]);
        assert_eq!(out.pixel(3, 1), [255, 0, 0, 255]);
        assert_eq!(out.pixel(0, 3), [0, 0, 0, 0]);
        assert_eq!(out.pixel(3, 3), [0, 0, 255, 255]);
        // A gradient keeps its order when stretched twice.
        let ramp = Image { w: 2, h: 1, rgba: vec![10, 0, 0, 255, 20, 0, 0, 255] };
        let out = compose(4, 1, &[Piece { image: &ramp, x: 0.0, y: 0.0, w: 4.0, h: 1.0 }]);
        assert_eq!(out.rgba.chunks(4).map(|p| p[0]).collect::<Vec<_>>(), [10, 10, 20, 20]);
    }

    /// The kept window has the box's share of the height, starting at its share of the skip.
    #[test]
    fn window_keeps_the_scale() {
        assert_eq!(window_rows(512, 0.0, 118.0, 252.0), (0, 240));
        // Options: 188 of 252 units, 23 units down: rows 47..429 of 512.
        assert_eq!(window_rows(512, OPTIONS_SKIP, OPTIONS_KEPT[3], WHOLE_HEIGHT), (47, 429));
        // Never past the picture, never empty.
        assert_eq!(window_rows(512, 200.0, 118.0, 252.0), (406, 512));
        assert_eq!(window_rows(4, 300.0, 1.0, 252.0), (3, 4));
        let img = Image { w: 1, h: 4, rgba: (0..16).collect() };
        assert_eq!(img.rows(1, 3).rgba, (4..12).collect::<Vec<u8>>());
        assert_eq!(img.rows(3, 9).h, 1);
    }

    /// Options and Mods fill the space of the original Options box, Mods with a gap like the
    /// menu's other boxes, and Options keeps most of the height.
    #[test]
    fn options_and_mods_share_the_box() {
        assert_eq!(OPTIONS_KEPT[1], 181.0);
        assert_eq!(MODS_BOX[1] + MODS_BOX[3], OPTIONS_KEPT[1] + WHOLE_HEIGHT);
        let gap = MODS_BOX[1] - (OPTIONS_KEPT[1] + OPTIONS_KEPT[3]);
        assert!((8.0..=14.0).contains(&gap), "{gap}");
        assert!(OPTIONS_KEPT[3] > 3.0 * MODS_BOX[3]);
        assert_eq!((OPTIONS_KEPT[0], OPTIONS_KEPT[2]), (MODS_BOX[0], MODS_BOX[2]));
    }

    /// A crop keeps the box's aspect, takes pixels unchanged (no stretch), follows the centre
    /// and stays inside the picture.
    #[test]
    fn crop_keeps_aspect_and_pixels() {
        let img = coords(200, 100);
        let c = crop_to_aspect(&img, 3.0, Crop { cx: 0.5, cy: 0.5, h: 0.4 });
        assert_eq!((c.w, c.h), (120, 40));
        assert_eq!(c.pixel(0, 0), [40, 30, 0, 255]);
        assert_eq!(c.pixel(119, 39), [159, 69, 0, 255]);
        // Slid back inside at the edges.
        let c = crop_to_aspect(&img, 3.0, Crop { cx: 0.0, cy: 1.0, h: 0.4 });
        assert_eq!((c.pixel(0, 0), c.pixel(119, 39)), ([0, 60, 0, 255], [119, 99, 0, 255]));
        // Too wide for the picture: the whole width, the height follows.
        let c = crop_to_aspect(&img, 4.0, Crop { cx: 0.5, cy: 0.5, h: 1.0 });
        assert_eq!((c.w, c.h), (200, 50));
        // Mods: the strip's aspect in a 640 × 480 frame stays inside it.
        let frame = coords(640, 480);
        let aspect = MODS_BOX[2] / MODS_BOX[3];
        let c = crop_to_aspect(&frame, aspect, MODS_CROP);
        assert!(((c.w as f32 / c.h as f32) - aspect).abs() < 0.02, "{} x {}", c.w, c.h);
        assert!(c.w <= 640 && c.h <= 480 && c.h > 150);
    }

    /// Mods' picture is a frame of Cine01 (928 unless switched); Co-op's is noted for later.
    #[test]
    fn box_pictures() {
        let BoxPicture::VideoFrame { file, frame, .. } = picture_of(NewBox::Mods);
        assert_eq!(file, "Cine01");
        assert!(frame == 928 || frame == 902);
        assert_eq!(MODS_FRAME, 928);
        assert_eq!(picture_of(NewBox::Coop), BoxPicture::VideoFrame { file: "Cine09", frame: 1137, crop: COOP_CROP });
        assert_eq!(crate::added_text::text("int", COOP_LABEL.2), "Co-op");
    }

    /// Co-op on: Load is cut and the Co-op box takes the space below. Co-op off: Load is not
    /// touched (original box, picture and label) and only Options is cut for the Mods box.
    #[test]
    fn coop_on_and_off_layout() {
        let on: Vec<NewBox> = cuts(true).map(|c| c.which).collect();
        let off: Vec<NewBox> = cuts(false).map(|c| c.which).collect();
        let mods: &[NewBox] = if cfg!(feature = "mods") { &[NewBox::Mods] } else { &[] };
        assert_eq!(off, mods);
        assert_eq!(on, [mods, &[NewBox::Coop]].concat());
        assert!(cuts(false).all(|c| c.prop != "LoadButton" && c.label.is_none()));
        let load = cuts(true).find(|c| c.which == NewBox::Coop).unwrap();
        assert_eq!((load.prop, load.kept, load.below), ("LoadButton", LOAD_KEPT, COOP_BOX_RECT));
        // On by default (the game ships with it; no test of this crate turns it off).
        assert!(coop_box());
    }

    /// Without the video (no file, or no ffmpeg), the placeholder is used, cropped to the box.
    #[test]
    fn missing_video_falls_back_to_the_placeholder() {
        let folder = std::env::temp_dir().join(format!("oxiii-menu-boxes-{}", std::process::id()));
        let placeholder = coords(256, 83);
        let (video, frame) = box_video(NewBox::Mods, &folder);
        assert!(crate::still_frames::wait(&video, frame, std::time::Duration::from_secs(5)).is_none());
        assert!(menu_box_frame(NewBox::Mods, &folder, 3.0).is_none());
        let img = menu_box_placeholder(3.0, &placeholder);
        assert_eq!((img.w, img.h), (249, 83));
        assert_eq!(img.pixel(0, 0), [4, 0, 0, 255]);
    }

    /// The frame decoded from the game's own video, when the game folder (OXIII_GAME) and ffmpeg
    /// are there: 640 × 480, and frames 902 and 928 differ (XIII's hand).
    #[test]
    fn mods_frame_from_the_game_video() {
        let Some(game) = openxiii_package::env::var_os("OXIII_GAME").map(std::path::PathBuf::from) else {
            eprintln!("OXIII_GAME not set: nothing to check");
            return;
        };
        let video = game.join("Video").join("Cine01.bik");
        if !video.is_file() {
            eprintln!("no Cine01.bik: nothing to check");
            return;
        }
        let wait = |n| crate::still_frames::wait(&video, n, std::time::Duration::from_secs(60));
        let (Some(a), Some(b)) = (wait(902), wait(928)) else {
            eprintln!("ffmpeg missing: nothing to check");
            return;
        };
        assert_eq!((a.w, a.h, b.w, b.h), (640, 480, 640, 480));
        assert_ne!(a.rgba, b.rgba);
        // Past the end: none.
        assert!(wait(1_000_000).is_none());
        let img = menu_box_frame(NewBox::Mods, &game.join("Video"), MODS_BOX[2] / MODS_BOX[3]).unwrap();
        assert!(img.w > 400 && img.h > 150);
    }

    /// Grey: no colour left, alpha kept, brighter stays brighter.
    #[test]
    fn grey_is_grey() {
        let img = Image { w: 3, h: 1, rgba: vec![255, 0, 0, 128, 0, 255, 0, 255, 255, 255, 255, 0] };
        let g = to_grey(&img);
        for p in g.rgba.chunks(4) {
            assert!(p[0].abs_diff(p[1]) <= 8 && p[1].abs_diff(p[2]) <= 8, "{p:?}");
        }
        assert_eq!([g.rgba[3], g.rgba[7], g.rgba[11]], [128, 255, 0]);
        assert!(g.rgba[8] > g.rgba[4] && g.rgba[4] > g.rgba[0]);
    }

    /// The grey made from each original colour picture is close to the original *gris picture
    /// (OXIII_GAME = original game folder; skipped when it is not set). Prints the per-channel
    /// least-squares fit gris = a * Y + b the constants come from.
    #[test]
    fn grey_matches_the_original_grey_pictures() {
        let Some(game) = openxiii_package::env::var_os("OXIII_GAME").map(std::path::PathBuf::from).filter(|g| openxiii_package::system_dir(g).exists()) else {
            eprintln!("OXIII_GAME not set: nothing to check");
            return;
        };
        let mut library = openxiii_package::library::Library::new(&game).unwrap();
        let mut image = |path: &str| {
            let (loaded, index) = library.resolve(path).unwrap_or_else(|| panic!("{path}"));
            let t = openxiii_assets::texture::Texture::read(&loaded.package, &loaded.data, index).unwrap();
            let (w, h, rgba) = t.d3d_mips().into_iter().next().unwrap();
            Image { w, h, rgba }
        };
        let names = ["continue01", "continue02", "continue03", "continue04", "multiplayer01", "multiplayer02", "loadgame01", "loadgame02b", "loadgame03", "loadgame04", "loadgame02a", "options01", "options02", "newgame01", "sortie"];
        // Sums for the fit of each channel: n, ΣY, ΣY², Σg, ΣYg.
        let mut s = [[0f64; 5]; 3];
        let mut pairs = Vec::new();
        for n in names {
            let (colour, grey) = (image(&format!("XIIIMenuStart.{n}")), image(&format!("XIIIMenuStart.{n}gris")));
            if (colour.w, colour.h) != (grey.w, grey.h) {
                eprintln!("{n}: sizes differ, left out");
                continue;
            }
            for (c, g) in colour.rgba.chunks(4).zip(grey.rgba.chunks(4)) {
                let y = 0.299 * c[0] as f64 + 0.587 * c[1] as f64 + 0.114 * c[2] as f64;
                for k in 0..3 {
                    let v = g[k] as f64;
                    s[k] = [s[k][0] + 1.0, s[k][1] + y, s[k][2] + y * y, s[k][3] + v, s[k][4] + y * v];
                }
            }
            pairs.push((colour, grey));
        }
        for (k, t) in s.iter().enumerate() {
            let a = (t[0] * t[4] - t[1] * t[3]) / (t[0] * t[2] - t[1] * t[1]);
            let b = (t[3] - a * t[1]) / t[0];
            eprintln!("channel {k}: a = {a:.3}, b = {b:.1}");
        }
        let (mut err, mut count) = (0f64, 0f64);
        for (colour, grey) in &pairs {
            for (m, g) in to_grey(colour).rgba.chunks(4).zip(grey.rgba.chunks(4)) {
                err += (0..3).map(|k| (m[k] as f64 - g[k] as f64).abs()).sum::<f64>() / 3.0;
                count += 1.0;
            }
        }
        eprintln!("mean difference {:.2} / 255 over {count} pixels", err / count);
        // About 13 / 255 is left whatever the formula (a free fit on R, G and B, or on 4 x 4 block
        // averages to leave out the texture compression, is no better): the original grey
        // pictures were adjusted one by one. Plain luminance is much further away.
        let plain: f64 = pairs
            .iter()
            .flat_map(|(colour, grey)| colour.rgba.chunks(4).zip(grey.rgba.chunks(4)))
            .map(|(c, g)| ((0.299 * c[0] as f64 + 0.587 * c[1] as f64 + 0.114 * c[2] as f64) - g[0] as f64).abs())
            .sum::<f64>()
            / count;
        eprintln!("plain luminance: mean difference {plain:.2} / 255");
        assert!(err / count < 15.0, "grey too far from the original: {:.2}", err / count);
        assert!(plain > err / count + 10.0);
    }

    /// Arrows: the original table is kept except below Options, up from Quit and (with the
    /// Co-op box) below Load, up from Continue and right from New; every move of the new boxes
    /// lands on a box; without Co-op, nothing points to it.
    #[test]
    fn navigation_table() {
        let changed = |from, dir, coop| match (from, dir) {
            (QUIT, Dir::Up) => true,
            (CONTINUE, Dir::Up) | (NEW, Dir::Right) => coop,
            _ => false,
        };
        for coop in [true, false] {
            for from in 0..6 {
                for dir in [Dir::Up, Dir::Left, Dir::Right] {
                    if !changed(from, dir, coop) {
                        assert_eq!(navigate(from, dir, coop), None, "{from} {dir:?} {coop}");
                    }
                }
            }
        }
        assert_eq!(navigate(QUIT, Dir::Up, true), Some(MODS));
        assert_eq!(navigate(QUIT, Dir::Up, false), Some(MODS));
        assert_eq!(navigate(NEW, Dir::Right, true), Some(COOP));
        assert_eq!(navigate(CONTINUE, Dir::Up, true), Some(COOP));
        // The reverse moves: Mods down to Quit, Co-op left to New and down to Continue.
        assert_eq!(navigate(MODS, Dir::Down, true), Some(QUIT));
        assert_eq!(navigate(COOP, Dir::Left, true), Some(NEW));
        assert_eq!(navigate(COOP, Dir::Down, true), Some(CONTINUE));
        assert_eq!(navigate(LOAD, Dir::Down, true), Some(COOP));
        assert_eq!(navigate(LOAD, Dir::Down, false), None);
        assert_eq!(navigate(OPTIONS, Dir::Down, false), Some(MODS));
        for from in [CONTINUE, MULTI, NEW, QUIT] {
            assert_eq!(navigate(from, Dir::Down, true), None);
        }
        assert_eq!(navigate(COOP, Dir::Right, true), Some(MODS));
        assert_eq!(navigate(MODS, Dir::Left, true), Some(COOP));
        assert_eq!(navigate(MODS, Dir::Left, false), Some(LOAD));
        assert_eq!(navigate(MODS, Dir::Right, false), Some(NEW));
        assert_eq!(navigate(MODS, Dir::Up, false), Some(OPTIONS));
        assert_eq!(navigate(COOP, Dir::Up, true), Some(LOAD));
        for coop in [true, false] {
            for from in [MODS, COOP] {
                for dir in [Dir::Up, Dir::Down, Dir::Left, Dir::Right] {
                    let t = navigate(from, dir, coop).unwrap();
                    assert!(t <= COOP && (coop || from == COOP || t != COOP));
                }
            }
        }
    }

    /// Hooked functions: the main menu's four, the Load page's three, nothing else.
    #[test]
    fn hooked_functions() {
        assert_eq!(hooked("XIIIMenu", "AfterPaint"), Some(true));
        assert_eq!(hooked("xiiimenu", "Created"), Some(true));
        assert_eq!(hooked("XIIIMenuLoadGameWindow", "Paint"), Some(false));
        assert_eq!(hooked("XIIIMenuLoadGameWindow", "InternalOnKeyEvent"), Some(false));
        assert_eq!(hooked("XIIIMenuLoadGameWindow", "AfterPaint"), None);
        assert_eq!(hooked("XIIIMenu", "Paint"), None);
        assert_eq!(hooked("XIIIMenuSave", "Created"), None);
        // Mods' onomatopoeia is drawn at its original size, on the menu, with its label.
        let (tex, x, y, w, h) = MODS_ONOMATOPOEIA;
        assert_eq!((tex, w, h), ("XIIIMenuStart.Controle_PC.clapclap", 128.0, 64.0));
        assert!(x >= 0.0 && x + w <= 640.0 && y >= 0.0 && y + h <= 480.0);
        assert_eq!(crate::added_text::text("int", MODS_LABEL.2), "Mods");
    }
}

impl Vm {
}

impl Vm {
    /// A page of the improved menus built on the game's own XIIIWindowMainMenu class (the Co-op
    /// and Select level pages) is kept out of the menu stack that XIIIRootWindow.CloseAll saves
    /// when a map is started from the main menu: RestoreMenuStack reopens the saved pages by class
    /// name when the menu map loads again, and this one would come back as an empty
    /// XIIIWindowMainMenu over the main menu (a blank page that takes the keys). The original's
    /// Load page is kept out the same way (bDoStoreInSaveMenuStack=False in its defaults).
    pub(crate) fn keep_out_of_saved_menu_stack(&mut self, page: ObjId) {
        self.set_prop(page, "bDoStoreInSaveMenuStack", Value::Bool(false));
    }
}
