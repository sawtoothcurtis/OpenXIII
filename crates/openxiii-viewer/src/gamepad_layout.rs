//! Layout of the gamepad settings page (gamepad_page.rs) in any area: the whole window, half of
//! it in split screen (top / bottom), a quarter with three or four players, wide or narrow.
//!
//! Two arrangements, both with one scale across and down (nothing is flattened):
//! - picture: the console page as it is (640 × 480 units, the controls picture with each
//!   button's action at the end of its line), scaled to the largest 4:3 rectangle of the area,
//!   centred. Used when every box keeps clear of the others and the text stays readable;
//! - list: for small or flat areas. A title row, the controls picture (still 4:3) at the left,
//!   beside it a list of the buttons (button → action) in one to three columns, then the help
//!   line and the bottom row. The text keeps a readable size; the picture shrinks, and goes
//!   away when the list needs its room.
//!
//! Everything is computed here from text sizes (`Measure`), without drawing, so that a test can
//! check every rectangle for every area size and language.

use openxiii_vm::gamepad_config::Button;

/// x0, y0, x1, y1 (pixels of the page's area).
pub type Rect = [f32; 4];

/// Width and height of a text at scale 1: `small` for the console font, otherwise the menu font.
pub type Measure<'a> = &'a dyn Fn(bool, &str) -> (f32, f32);

/// Width / height of the controls picture: the console page's 640 × 480.
pub const PICTURE_ASPECT: f32 = 4.0 / 3.0;
/// Smallest text scale of the picture arrangement (below it: the list).
const PICTURE_MIN_SCALE: f32 = 0.85;
/// Smallest height of the console font's letters, in pixels (below it: the list, then a
/// smaller list).
const MIN_LETTERS: f32 = 10.0;
/// Text scales tried for the list, largest first.
const LIST_SCALES: [f32; 9] = [1.0, 0.95, 0.9, 0.85, 0.8, 0.75, 0.7, 0.65, 0.6];

/// Where each button's label goes on the picture, in 640 × 480 units: the button, the side
/// (left column: the label ends at x; right column: it starts at x) and the height of the end
/// of its line (measured on the picture).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Side {
    Left,
    Right,
}
pub const LABELS: [(Button, Side, f32, f32); 16] = [
    (Button::Up, Side::Left, 136.0, 94.0),
    (Button::Left, Side::Left, 136.0, 108.0),
    (Button::Right, Side::Left, 136.0, 122.0),
    (Button::Down, Side::Left, 136.0, 136.0),
    (Button::LeftThumb, Side::Left, 136.0, 160.0),
    (Button::LeftTrigger, Side::Left, 136.0, 336.0),
    (Button::LeftBumper, Side::Left, 136.0, 370.0),
    (Button::Back, Side::Left, 296.0, 58.0),
    (Button::Start, Side::Right, 342.0, 44.0),
    (Button::Y, Side::Right, 506.0, 96.0),
    (Button::B, Side::Right, 506.0, 110.0),
    (Button::X, Side::Right, 506.0, 124.0),
    (Button::A, Side::Right, 506.0, 138.0),
    (Button::RightThumb, Side::Right, 506.0, 160.0),
    (Button::RightTrigger, Side::Right, 506.0, 336.0),
    (Button::RightBumper, Side::Right, 506.0, 370.0),
];

/// Short name of a button in the list (the d-pad has the console page's arrows instead).
pub fn button_name(b: Button) -> &'static str {
    match b {
        Button::A => "A",
        Button::B => "B",
        Button::X => "X",
        Button::Y => "Y",
        Button::LeftBumper => "LB",
        Button::RightBumper => "RB",
        Button::LeftTrigger => "LT",
        Button::RightTrigger => "RT",
        Button::LeftThumb => "L3",
        Button::RightThumb => "R3",
        Button::Back => "Back",
        Button::Start => "Start",
        Button::Up | Button::Down | Button::Left | Button::Right => "",
    }
}

/// The texts of the page, in the game's language.
#[derive(Clone, Debug, Default)]
pub struct Texts {
    pub title: String,
    pub player: String,
    /// Action of each button, in [`LABELS`] order.
    pub labels: Vec<String>,
    pub advanced: String,
    pub defaults: String,
    pub back: String,
    pub help: String,
    pub advanced_title: String,
    /// Vibration, aim assist, inverted look, look speed.
    pub advanced_rows: Vec<String>,
    /// Yes / no, the widest values of the advanced rows.
    pub values: Vec<String>,
}

/// One button of the page.
#[derive(Clone, Debug)]
pub struct Row {
    /// Index in [`LABELS`].
    pub index: usize,
    /// The box with the button's name (list only).
    pub name: Option<Rect>,
    /// The box with its action.
    pub value: Rect,
    /// The action as shown (cut with "..." to fit).
    pub shown: String,
}

#[derive(Clone, Debug)]
pub struct Layout {
    pub list: bool,
    /// Height of the console font's letters at scale 1.
    pub small_letters: f32,
    /// Text scale.
    pub s: f32,
    /// Text scale of the buttons' labels and the help line (console font): on the picture,
    /// small enough for the picture's lines, 14 units apart.
    pub label_s: f32,
    /// Background of the page (the picture arrangement leaves black bars around it).
    pub page: Rect,
    /// The controls picture (4:3); empty when the list has no room for it.
    pub picture: Rect,
    pub title: Rect,
    pub player: Rect,
    pub rows: Vec<Row>,
    pub advanced: Rect,
    pub defaults: Rect,
    pub back: Rect,
    pub help: Rect,
    pub help_shown: String,
    /// Advanced page: its box, title tab, the four rows, label x, value centre x, slider ends.
    pub adv_box: Rect,
    pub adv_title: Rect,
    pub adv_rows: Vec<Rect>,
    pub adv_label_x: f32,
    pub adv_value_x: f32,
    pub slider: [f32; 2],
}

fn width(r: Rect) -> f32 {
    r[2] - r[0]
}

fn height(r: Rect) -> f32 {
    r[3] - r[1]
}

/// The two boxes cross (touching edges do not).
pub fn overlap(a: Rect, b: Rect) -> bool {
    const E: f32 = 0.01;
    a[0] < b[2] - E && b[0] < a[2] - E && a[1] < b[3] - E && b[1] < a[3] - E
}

fn inside(a: Rect, b: Rect) -> bool {
    const E: f32 = 0.01;
    a[0] >= b[0] - E && a[1] >= b[1] - E && a[2] <= b[2] + E && a[3] <= b[3] + E
}

/// `t` cut with "..." to fit in `room` pixels at scale `s`.
pub fn fit(m: Measure, small: bool, s: f32, t: &str, room: f32) -> String {
    let w = |t: &str| m(small, t).0 * s;
    if w(t) <= room {
        return t.to_string();
    }
    let mut cut: String = t.to_string();
    while !cut.is_empty() {
        cut.pop();
        let c = format!("{}...", cut.trim_end());
        if w(&c) <= room {
            return c;
        }
    }
    String::new()
}

impl Layout {
    /// The page in an area `w` × `h` pixels.
    pub fn new(w: f32, h: f32, t: &Texts, m: Measure) -> Layout {
        match Layout::picture(w, h, t, m) {
            Some(l) if l.clean(w, h) => l,
            _ => Layout::list(w, h, t, m),
        }
    }

    /// The boxes that must keep clear of each other, with a name for messages: the page's own
    /// (not the advanced page's, drawn instead of the labels and bottom row).
    pub fn boxes(&self) -> Vec<(String, Rect)> {
        let mut v = vec![("title".to_string(), self.title), ("player".to_string(), self.player)];
        for r in &self.rows {
            if let Some(n) = r.name {
                v.push((format!("name {}", r.index), n));
            }
            v.push((format!("label {}", r.index), r.value));
        }
        v.push(("advanced".into(), self.advanced));
        v.push(("defaults".into(), self.defaults));
        v.push(("back".into(), self.back));
        if !self.help_shown.is_empty() {
            v.push(("help".into(), self.help));
        }
        v
    }

    /// The advanced page's boxes that must keep clear of each other.
    pub fn advanced_boxes(&self) -> Vec<(String, Rect)> {
        let mut v = vec![("title".to_string(), self.title), ("advanced title".to_string(), self.adv_title), ("player".into(), self.player), ("back".into(), self.back)];
        for (i, r) in self.adv_rows.iter().enumerate() {
            v.push((format!("advanced row {i}"), *r));
        }
        v
    }

    /// Problems of the layout in an area `w` × `h`: boxes crossing, outside the area, text
    /// too small. Empty when clean.
    pub fn problems(&self, w: f32, h: f32) -> Vec<String> {
        let area = [0.0, 0.0, w, h];
        let mut out = Vec::new();
        for set in [self.boxes(), self.advanced_boxes()] {
            for (i, (na, a)) in set.iter().enumerate() {
                if !inside(*a, area) {
                    out.push(format!("{na} {a:?} outside {w}x{h}"));
                }
                for (nb, b) in &set[i + 1..] {
                    if overlap(*a, *b) {
                        out.push(format!("{na} {a:?} crosses {nb} {b:?}"));
                    }
                }
            }
        }
        // The title tab of the advanced page sits on its box's top edge, by design.
        for (i, r) in self.adv_rows.iter().enumerate() {
            if !inside(*r, self.adv_box) {
                out.push(format!("advanced row {i} outside its box"));
            }
        }
        if self.s < if self.list { LIST_SCALES[LIST_SCALES.len() - 1] } else { PICTURE_MIN_SCALE } - 0.001 {
            out.push(format!("text scale {} too small", self.s));
        }
        if self.small_letters * self.label_s < MIN_LETTERS - 0.01 {
            out.push(format!("labels {} pixels high", self.small_letters * self.label_s));
        }
        out
    }

    fn clean(&self, w: f32, h: f32) -> bool {
        self.problems(w, h).is_empty()
    }

    /// The console page scaled to the largest 4:3 rectangle of the area.
    pub fn picture(w: f32, h: f32, t: &Texts, m: Measure) -> Option<Layout> {
        let k = (w / 640.0).min(h / 480.0);
        if k <= 0.0 {
            return None;
        }
        // Whole text steps from 1 up (sharp letters), the page's own scale below.
        let s = if k >= 1.0 { k.floor() } else { k };
        let (pw, ph) = (640.0 * k, 480.0 * k);
        let (ox, oy) = (((w - pw) / 2.0).floor(), ((h - ph) / 2.0).floor());
        let u = |x: f32, y: f32| (ox + x * k, oy + y * k);
        let r = |x0: f32, y0: f32, x1: f32, y1: f32| {
            let (a, b) = u(x0, y0);
            let (c, d) = u(x1, y1);
            [a, b, c, d]
        };
        let page = [ox, oy, ox + pw, oy + ph];
        let (tw, _) = m(false, &t.title);
        let title = [ox + 24.0 * k, oy + 30.0 * k, ox + 24.0 * k + tw * s + 30.0 * k, oy + 62.0 * k];
        let player = r(470.0, 30.0, 616.0, 62.0);
        // The labels' letters fit the picture's lines, 14 units apart (with 2 for the box).
        let (_, sh) = m(true, "Ag");
        let ls = s.min(12.0 * k / sh.max(1.0));
        let mut rows = Vec::new();
        for (i, (_, side, x, y)) in LABELS.iter().enumerate() {
            let label = t.labels.get(i).map_or("", String::as_str);
            let shown = fit(m, true, ls, label, 128.0 * k);
            let (lw, _) = m(true, &shown);
            let (bw, bh) = (lw * ls + 8.0 * k, sh * ls + 2.0 * k);
            let (cx, cy) = u(*x, *y);
            let x0 = match side {
                Side::Left => cx - bw,
                Side::Right => cx,
            };
            rows.push(Row { index: i, name: None, value: [x0, cy - bh / 2.0, x0 + bw, cy + bh / 2.0], shown });
        }
        // The help line ends just above the bottom row.
        let help_h = sh * ls + 4.0 * k;
        let (help_shown, help) = help_line(m, ls, &t.help, ox + 20.0 * k, oy + 437.0 * k - help_h, 600.0 * k, k);
        let mut l = Layout {
            list: false,
            small_letters: sh,
            s,
            label_s: ls,
            page,
            picture: page,
            title,
            player,
            rows,
            advanced: r(150.0, 440.0, 310.0, 468.0),
            defaults: r(320.0, 440.0, 480.0, 468.0),
            back: r(490.0, 440.0, 616.0, 468.0),
            help,
            help_shown,
            adv_box: r(140.0, 110.0, 500.0, 330.0),
            adv_title: [0.0; 4],
            adv_rows: (0..4).map(|i| r(160.0, 135.0 + 40.0 * i as f32, 480.0, 165.0 + 40.0 * i as f32)).collect(),
            adv_label_x: ox + 170.0 * k,
            adv_value_x: ox + 410.0 * k,
            slider: [ox + 350.0 * k, ox + 470.0 * k],
        };
        let (aw, _) = m(false, &t.advanced_title);
        l.adv_title = [ox + 110.0 * k, oy + 90.0 * k, ox + 110.0 * k + aw * s + 30.0 * k, oy + 122.0 * k];
        Some(l)
    }

    /// Title row, picture and button list, help line, bottom row.
    pub fn list(w: f32, h: f32, t: &Texts, m: Measure) -> Layout {
        let mut last = None;
        for &s in &LIST_SCALES {
            for with_picture in [true, false] {
                let l = Layout::list_at(w, h, t, m, s, with_picture);
                match l {
                    Some(l) if l.clean(w, h) => return l,
                    Some(l) => last = Some(l),
                    None => {}
                }
            }
        }
        last.unwrap_or_else(|| Layout::list_at(w, h, t, m, LIST_SCALES[LIST_SCALES.len() - 1], false).expect("list layout"))
    }

    fn list_at(w: f32, h: f32, t: &Texts, m: Measure, s: f32, with_picture: bool) -> Option<Layout> {
        let (_, fh) = m(false, "Ag");
        let (_, sh) = m(true, "Ag");
        let gap = (6.0 * s).round().max(3.0);
        let bar = (fh * s + 8.0 * s).ceil();
        let row_h = (sh * s + 4.0 * s).ceil();
        let pitch = row_h + (2.0 * s).ceil();
        // Title row.
        let (pw, _) = m(false, &t.player);
        let player = [w - gap - (pw * s + 24.0 * s).max(90.0 * s), gap, w - gap, gap + bar];
        let (tw, _) = m(false, &t.title);
        let title = [gap, gap, (gap + tw * s + 24.0 * s).min(player[0] - gap), gap + bar];
        // Bottom row: advanced, defaults, back (placed once the list's columns are known).
        let bottom = [h - gap - bar, h - gap];
        let words = [&t.advanced, &t.defaults, &t.back];
        let mut widths: Vec<f32> = words.iter().map(|x| (m(false, x).0 * s + 20.0 * s).max(80.0 * s)).collect();
        let room = w - 2.0 * gap - 2.0 * gap;
        let total: f32 = widths.iter().sum();
        if total > room {
            widths.iter_mut().for_each(|x| *x *= room / total);
        }
        // Help line above it.
        let help_top = bottom[0] - gap - row_h;
        let (help_shown, help) = help_line(m, s, &t.help, gap, help_top, w - 2.0 * gap, s);
        // The middle: picture and list.
        let (top, bot) = (title[3] + gap, help_top - gap);
        let mid = bot - top;
        if mid < pitch || title[2] <= title[0] {
            return None;
        }
        let names_w = ["Start", "Back", "L3", "LB"].iter().map(|n| m(true, n).0 * s).fold(row_h, f32::max) + 8.0 * s;
        let min_value = 70.0 * s;
        let fits = |x0: f32, cols: usize| {
            let rows = LABELS.len().div_ceil(cols);
            let col_w = (w - gap - x0 - (cols as f32 - 1.0) * 2.0 * gap) / cols as f32;
            (rows as f32 * pitch - (pitch - row_h) <= mid && col_w - names_w - 2.0 * s >= min_value).then_some((rows, col_w))
        };
        let (picture, x0, rows_n, col_w, cols) = if with_picture {
            let ph = mid;
            let pw = (ph * PICTURE_ASPECT).min(w * 0.34);
            let ph = pw / PICTURE_ASPECT;
            let pic = [gap, top + ((mid - ph) / 2.0).floor(), gap + pw, top + ((mid - ph) / 2.0).floor() + ph];
            let x0 = pic[2] + 2.0 * gap;
            let (cols, (rows_n, col_w)) = (1..=3).find_map(|c| fits(x0, c).map(|f| (c, f)))?;
            (pic, x0, rows_n, col_w, cols)
        } else {
            let x0 = gap;
            let (cols, (rows_n, col_w)) = (1..=3).find_map(|c| fits(x0, c).map(|f| (c, f)))?;
            ([0.0; 4], x0, rows_n, col_w, cols)
        };
        let _ = cols;
        // The bottom row spreads under the list, from its left edge to the right edge: the
        // first box under the first column, the last under the last column, the middle one
        // centred, so that Up from each box reaches the column above it (spatial navigation,
        // gamepad_page.rs). Where it has no room for that, across the whole width.
        let spread = |left: f32| -> Option<(Rect, Rect, Rect)> {
            let right = w - gap;
            if right - left < widths.iter().sum::<f32>() + 2.0 * gap {
                return None;
            }
            let advanced = [left, bottom[0], left + widths[0], bottom[1]];
            let back = [right - widths[2], bottom[0], right, bottom[1]];
            let d0 = ((left + right) / 2.0 - widths[1] / 2.0).max(advanced[2] + gap).min(back[0] - gap - widths[1]);
            Some((advanced, [d0, bottom[0], d0 + widths[1], bottom[1]], back))
        };
        let (advanced, defaults, back) = spread(x0).or_else(|| spread(gap)).unwrap_or_else(|| {
            let back = [w - gap - widths[2], bottom[0], w - gap, bottom[1]];
            let defaults = [back[0] - gap - widths[1], bottom[0], back[0] - gap, bottom[1]];
            ([defaults[0] - gap - widths[0], bottom[0], defaults[0] - gap, bottom[1]], defaults, back)
        });
        let block = rows_n as f32 * pitch - (pitch - row_h);
        let y0 = top + ((mid - block) / 2.0).floor().max(0.0);
        let mut rows = Vec::new();
        for (i, _) in LABELS.iter().enumerate() {
            let (c, r) = (i / rows_n, i % rows_n);
            let x = x0 + c as f32 * (col_w + 2.0 * gap);
            let y = y0 + r as f32 * pitch;
            let name = [x, y, x + names_w, y + row_h];
            let value = [name[2] + 2.0 * s, y, x + col_w, y + row_h];
            let label = t.labels.get(i).map_or("", String::as_str);
            let shown = fit(m, true, s, label, width(value) - 8.0 * s);
            rows.push(Row { index: i, name: Some(name), value, shown });
        }
        // Advanced page: its title tab on the top edge of a box over the middle, four rows.
        let (aw, _) = m(false, &t.advanced_title);
        let adv_title = [gap * 2.0, top, (gap * 2.0 + aw * s + 24.0 * s).min(player[0] - gap), top + bar];
        let adv_box = [gap, top + bar / 2.0, w - gap, bot];
        let rows_top = adv_title[3] + gap / 2.0;
        let adv_pitch = ((bot - gap / 2.0 - rows_top) / 4.0).min(bar + 12.0 * s);
        let adv_h = (fh * s + 6.0 * s).min(adv_pitch - 1.0);
        let adv_rows: Vec<Rect> = (0..4).map(|i| [gap * 2.0, rows_top + adv_pitch * i as f32, w - gap * 2.0, rows_top + adv_pitch * i as f32 + adv_h]).collect();
        let longest = t.advanced_rows.iter().map(|x| m(false, x).0 * s).fold(0.0, f32::max);
        let value_w = t.values.iter().map(|x| m(false, x).0 * s).fold(60.0 * s, f32::max) + 60.0 * s;
        let adv_label_x = gap * 3.0;
        let adv_value_x = w - gap * 3.0 - value_w / 2.0;
        let slider = [adv_value_x - value_w / 2.0, adv_value_x + value_w / 2.0 - 20.0 * s];
        if adv_h < fh * s || adv_label_x + longest > slider[0] - gap {
            return None;
        }
        let page = [0.0, 0.0, w, h];
        Some(Layout { list: true, small_letters: sh, s, label_s: s, page, picture, title, player, rows, advanced, defaults, back, help, help_shown, adv_box, adv_title, adv_rows, adv_label_x, adv_value_x, slider })
    }

    /// Height of a box of the advanced page's rows, for the drawing.
    pub fn adv_row_height(&self) -> f32 {
        self.adv_rows.first().map_or(0.0, |r| height(*r))
    }
}

/// A direction of the menu keys (d-pad, left stick, arrow keys).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dir {
    Up,
    Down,
    Left,
    Right,
}

#[cfg(test)]
pub const DIRS: [Dir; 4] = [Dir::Up, Dir::Down, Dir::Left, Dir::Right];

fn centre(r: Rect) -> (f32, f32) {
    ((r[0] + r[2]) / 2.0, (r[1] + r[3]) / 2.0)
}

/// Distance between the spans [a0, a1] and [b0, b1] (0 when they overlap).
fn span_gap(a0: f32, a1: f32, b0: f32, b1: f32) -> f32 {
    (b0 - a1).max(a0 - b1).max(0.0)
}

/// Offset across of two spans: the difference of their centres; for spans that overlap, the
/// smallest of the differences of their starts, centres and ends (boxes aligned on one edge,
/// as the picture's labels, are in line).
fn span_off(a0: f32, a1: f32, b0: f32, b1: f32) -> f32 {
    let centres = ((b0 + b1) - (a0 + a1)).abs() / 2.0;
    if span_gap(a0, a1, b0, b1) > 0.0 {
        return centres;
    }
    (b0 - a0).abs().min((b1 - a1).abs()).min(centres)
}

/// How far `c` lies from `a` in direction `dir`, as seen on the screen: `None` when its centre
/// is not on that side; otherwise the distance along `dir`, the gap across it (0 when the two
/// boxes face each other) and their offset across it (`span_off`).
pub fn towards(a: Rect, c: Rect, dir: Dir) -> Option<(f32, f32, f32)> {
    let ((ax, ay), (cx, cy)) = (centre(a), centre(c));
    let (along, gap, off) = match dir {
        Dir::Up => (ay - cy, span_gap(a[0], a[2], c[0], c[2]), span_off(a[0], a[2], c[0], c[2])),
        Dir::Down => (cy - ay, span_gap(a[0], a[2], c[0], c[2]), span_off(a[0], a[2], c[0], c[2])),
        Dir::Left => (ax - cx, span_gap(a[1], a[3], c[1], c[3]), span_off(a[1], a[3], c[1], c[3])),
        Dir::Right => (cx - ax, span_gap(a[1], a[3], c[1], c[3]), span_off(a[1], a[3], c[1], c[3])),
    };
    (along >= 0.5).then_some((along, gap, off))
}

/// Spatial navigation: the box of `boxes` that `dir` leads to from box `from`, among those
/// `allowed` keeps. The nearest one whose centre lies on that side of the screen, where a box
/// off to the side costs four times its gap across plus the offset of the centres across: Down
/// at the foot of a short column of the list goes to the bottom row below it, not to the next
/// line of the column beside it.
pub fn neighbour(boxes: &[Rect], from: usize, dir: Dir, allowed: &dyn Fn(usize) -> bool) -> Option<usize> {
    let a = boxes[from];
    let mut best: Option<(f32, usize)> = None;
    for (i, &c) in boxes.iter().enumerate() {
        if i == from || !allowed(i) {
            continue;
        }
        let Some((along, gap, off)) = towards(a, c, dir) else { continue };
        let score = along + 4.0 * gap + off;
        if best.is_none_or(|(s, _)| score < s) {
            best = Some((score, i));
        }
    }
    best.map(|b| b.1)
}

/// Box `c` lies entirely on side `dir` of box `a` (Left / Right): no part of it above or below
/// `a`.
pub fn on_side(a: Rect, c: Rect, dir: Dir) -> bool {
    const E: f32 = 1.0;
    match dir {
        Dir::Right => c[0] >= a[2] - E,
        Dir::Left => c[2] <= a[0] + E,
        Dir::Up | Dir::Down => false,
    }
}

/// Box `c` lies entirely on side `dir` of box `a` (Left / Right), facing it: in the same row on
/// the screen (their heights overlap).
pub fn beside(a: Rect, c: Rect, dir: Dir) -> bool {
    on_side(a, c, dir) && span_gap(a[1], a[3], c[1], c[3]) <= 0.0
}

/// Left / Right as the eye reads them, among the boxes entirely on that side (a box of the same
/// column that sticks out a little is not "to the right"): the nearest one of the same row (best
/// aligned first: distance plus twice the offset of the centres); with none, the nearest by
/// [`neighbour`]'s score. None at the edge.
pub fn neighbour_across(boxes: &[Rect], from: usize, dir: Dir, allowed: &dyn Fn(usize) -> bool) -> Option<usize> {
    let a = boxes[from];
    let row = boxes
        .iter()
        .enumerate()
        .filter(|&(i, &c)| i != from && allowed(i) && beside(a, c, dir))
        .filter_map(|(i, &c)| towards(a, c, dir).map(|(along, _, off)| (along + 2.0 * off, i)))
        .min_by(|x, y| x.0.total_cmp(&y.0))
        .map(|b| b.1);
    row.or_else(|| neighbour(boxes, from, dir, &|i| allowed(i) && on_side(a, boxes[i], dir)))
}

/// The direction opposite `dir`.
pub fn opposite(dir: Dir) -> Dir {
    match dir {
        Dir::Up => Dir::Down,
        Dir::Down => Dir::Up,
        Dir::Left => Dir::Right,
        Dir::Right => Dir::Left,
    }
}

/// Wrapping at an edge, as the weapon menu does: when nothing lies in direction `dir` of box
/// `from`, the box at the far other end of its row (Left / Right) or column (Up / Down). Among
/// the boxes on the other side, those best in line with `from` (the ones facing it, with no gap
/// across, if any; gap and offset across as in [`neighbour`], within half the box's own size
/// across): the farthest one.
pub fn wrap(boxes: &[Rect], from: usize, dir: Dir, allowed: &dyn Fn(usize) -> bool) -> Option<usize> {
    let a = boxes[from];
    let back = opposite(dir);
    let across = matches!(dir, Dir::Left | Dir::Right);
    let found: Vec<(usize, f32, f32, f32)> = boxes
        .iter()
        .enumerate()
        .filter(|&(i, &c)| i != from && allowed(i) && (!across || on_side(a, c, back)))
        .filter_map(|(i, &c)| towards(a, c, back).map(|(along, gap, off)| (i, along, gap, 4.0 * gap + off)))
        .collect();
    let facing = found.iter().any(|f| f.2 <= 0.0);
    let kept: Vec<&(usize, f32, f32, f32)> = found.iter().filter(|f| !facing || f.2 <= 0.0).collect();
    let best = kept.iter().map(|f| f.3).fold(f32::INFINITY, f32::min);
    let tolerance = match dir {
        Dir::Left | Dir::Right => height(a),
        Dir::Up | Dir::Down => width(a),
    } / 2.0;
    kept.into_iter().filter(|f| f.3 <= best + tolerance).max_by(|x, y| x.1.total_cmp(&y.1)).map(|f| f.0)
}

/// The help line from (`x`, `y`), at most `room` wide: its text as shown and its box.
fn help_line(m: Measure, s: f32, help: &str, x: f32, y: f32, room: f32, pad: f32) -> (String, Rect) {
    if help.is_empty() {
        return (String::new(), [x, y, x, y]);
    }
    let shown = fit(m, true, s, help, room - 12.0 * pad);
    let (lw, lh) = m(true, &shown);
    (shown, [x, y, x + lw * s + 12.0 * pad, y + lh * s + 4.0 * pad])
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Area sizes: whole windows, split screen halves (top / bottom at 1024 × 576, 800 × 450…),
    /// quarters for three or four players.
    pub(crate) const SIZES: [(f32, f32); 20] = [
        (800.0, 300.0),
        (1024.0, 288.0),
        (800.0, 225.0),
        (1024.0, 576.0),
        (640.0, 240.0),
        (1280.0, 360.0),
        (1920.0, 540.0),
        (512.0, 288.0),
        (400.0, 225.0),
        (640.0, 360.0),
        (960.0, 540.0),
        (320.0, 240.0),
        (400.0, 300.0),
        (800.0, 450.0),
        (800.0, 600.0),
        (640.0, 480.0),
        (1280.0, 720.0),
        (1920.0, 1080.0),
        (1280.0, 960.0),
        (2560.0, 720.0),
    ];

    /// Character sizes of the game's two fonts at scale 1, from the game's files when OXIII_GAME
    /// is set, otherwise wide estimates (every letter as wide as the widest one).
    pub(crate) struct Fonts {
        small: Option<(std::collections::HashMap<char, (f32, f32)>, f32)>,
        menu: Option<(std::collections::HashMap<char, (f32, f32)>, f32)>,
    }

    impl Fonts {
        pub(crate) fn load() -> Fonts {
            let read = |name: &str| -> Option<(std::collections::HashMap<char, (f32, f32)>, f32)> {
                let game = openxiii_package::env::var("OXIII_GAME").ok()?;
                let path = std::path::Path::new(&game).join("TexturesPC").join("XIIIFonts.utx");
                let data = std::fs::read(path).ok()?;
                let pkg = openxiii_package::Package::parse(&data).ok()?;
                let i = (0..pkg.exports.len()).find(|&i| pkg.export_class(i) == "Font" && pkg.object_path(openxiii_package::ObjectRef::from(i as i32 + 1)).eq_ignore_ascii_case(name))?;
                let f = openxiii_assets::font::Font::read("XIIIFonts", &pkg, &data, i, "int").ok()?;
                let mut map = std::collections::HashMap::new();
                let mut widest = 0.0f32;
                for code in 32u32..256 {
                    let Some(c) = char::from_u32(code) else { continue };
                    if let Some((_, g)) = f.glyph(c) {
                        map.insert(c, (g.width as f32, g.height as f32));
                        widest = widest.max(g.width as f32);
                    }
                }
                Some((map, widest))
            };
            Fonts { small: read("XIIIConsoleFont"), menu: read("PoliceF16") }
        }

        pub(crate) fn real(&self) -> bool {
            self.small.is_some() && self.menu.is_some()
        }

        pub(crate) fn measure(&self, small: bool, t: &str) -> (f32, f32) {
            let (font, guess) = if small { (&self.small, (9.0, 13.0)) } else { (&self.menu, (13.0, 18.0)) };
            match font {
                Some((map, widest)) => t.chars().fold((0.0, 0.0), |(w, h), c| {
                    let (cw, ch) = map.get(&c).copied().unwrap_or((*widest, 0.0));
                    (w + cw, f32::max(h, ch))
                }),
                None => (t.chars().count() as f32 * guess.0, if t.is_empty() { 0.0 } else { guess.1 }),
            }
        }
    }

    /// The page's texts in language `lang` (int, frt, det, itt, est): the game's own from its
    /// files when OXIII_GAME is set, otherwise long English stand-ins.
    pub(crate) fn texts(lang: &str) -> Texts {
        use openxiii_vm::gamepad_config::PlayerPad;
        let mut config = openxiii_package::env::var("OXIII_GAME").ok().map(|g| {
            let mut c = openxiii_vm::config::ConfigCache::with_user_dir(std::path::Path::new(&g).join("System"), None);
            c.language = lang.to_string();
            c
        });
        let mut game = |section: &str, key: &str, fallback: &str| -> String {
            config.as_mut().and_then(|c| c.localize2(section, key, "XIDInterf")).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).unwrap_or_else(|| fallback.to_string())
        };
        let pad = PlayerPad::default();
        let labels = LABELS
            .iter()
            .map(|(b, ..)| {
                let a = pad.action(*b);
                match a.game_label() {
                    Some((section, key)) => game(section, key, &format!("{} (long)", a.english())),
                    None => crate::gamepad_page::txt_for(lang, a),
                }
            })
            .collect();
        Texts {
            title: game("XIIIMenuControlsWindow", "TitleText", "Controls"),
            player: format!("{} 4", game("XIIIMenuPlayersSetupClient", "PlayerText", "Player")),
            labels,
            advanced: game("XIIIMenuControlsWindow", "AdvancedText", "Advanced"),
            defaults: crate::gamepad_page::defaults_text(lang).to_string(),
            back: format!("{} (B)", game("XIIIWindow", "BackText", "Back").to_uppercase()),
            // The longest help line the page shows (while a button is edited).
            help: crate::gamepad_page::edit_help_text(lang, true).to_string(),
            advanced_title: game("XIIIMenuAdvancedControlsWindow", "TitleText", "Advanced"),
            advanced_rows: ["RumbleText", "AutoAimText", "InvPadText"].iter().map(|k| game("XIIIMenuAdvancedControlsWindow", k, k)).chain(std::iter::once(crate::gamepad_page::sensitivity_text(lang).to_string())).collect(),
            values: vec![game("XIIICheckBoxControl", "sYes", "Yes"), game("XIIICheckBoxControl", "sNo", "No")],
        }
    }

    pub(crate) const LANGS: [&str; 5] = ["int", "frt", "det", "itt", "est"];

    #[test]
    fn no_box_crosses_another_in_any_area_or_language() {
        let fonts = Fonts::load();
        let m = |small: bool, t: &str| fonts.measure(small, t);
        let mut checked = 0;
        let mut lists = 0;
        for lang in LANGS {
            let t = texts(lang);
            for (w, h) in SIZES {
                let l = Layout::new(w, h, &t, &m);
                let p = l.problems(w, h);
                assert!(p.is_empty(), "{lang} {w}x{h} (list {}): {p:#?}", l.list);
                // Each label's text fits its box.
                for r in &l.rows {
                    assert!(m(true, &r.shown).0 * l.label_s <= width(r.value) + 0.5, "{lang} {w}x{h} label {} too wide", r.index);
                }
                lists += usize::from(l.list);
                checked += 1;
            }
        }
        eprintln!("gamepad page layout: {checked} areas checked ({lists} as a list), game fonts {}", fonts.real());
        assert_eq!(checked, LANGS.len() * SIZES.len());
        // The large areas keep the console page, the flat halves get the list.
        let t = texts("frt");
        assert!(!Layout::new(1024.0, 576.0, &t, &m).list);
        assert!(!Layout::new(1920.0, 1080.0, &t, &m).list);
        assert!(Layout::new(1024.0, 288.0, &t, &m).list);
        assert!(Layout::new(800.0, 225.0, &t, &m).list);
    }

    #[test]
    fn crossings_are_seen() {
        // Negative control: the console page forced into a flat half has crossing labels or
        // letters too small, which the check sees.
        let fonts = Fonts::load();
        let m = |small: bool, t: &str| fonts.measure(small, t);
        let t = texts("det");
        for (w, h) in [(1024.0, 288.0), (800.0, 225.0), (640.0, 240.0), (400.0, 225.0)] {
            let l = Layout::picture(w, h, &t, &m).unwrap();
            assert!(!l.problems(w, h).is_empty(), "{w}x{h}");
        }
        // And two boxes moved onto each other are seen.
        let mut l = Layout::new(1024.0, 576.0, &t, &m);
        l.rows[1].value = l.rows[0].value;
        assert!(!l.problems(1024.0, 576.0).is_empty());
    }

    #[test]
    fn picture_keeps_its_shape() {
        let fonts = Fonts::load();
        let m = |small: bool, t: &str| fonts.measure(small, t);
        let mut seen = 0;
        for lang in LANGS {
            let t = texts(lang);
            for (w, h) in SIZES {
                let l = Layout::new(w, h, &t, &m);
                let p = l.picture;
                if width(p) <= 0.0 {
                    continue;
                }
                // Same scale across and down: 4:3 within a pixel.
                assert!((width(p) - height(p) * PICTURE_ASPECT).abs() <= 1.0, "{lang} {w}x{h}: {p:?}");
                assert!(inside(p, [0.0, 0.0, w, h]));
                seen += 1;
            }
        }
        assert!(seen >= LANGS.len() * SIZES.len() * 3 / 4, "{seen}");
        // Negative control: the former page (640 × 480 units stretched to 16:9 at most) is not
        // 4:3 in a split screen half.
        let (w, h) = (1024.0f32, 288.0f32);
        let former_w = w.min((h * 16.0 / 9.0).floor());
        assert!((former_w - h * PICTURE_ASPECT).abs() > 1.0);
    }
}
