//! Android: input from the touch controls (Java class org.openxiii.NativeInput), passed to the VM
//! like the Windows keyboard and mouse (same paths as main.rs's KeyboardInput, MouseInput and
//! MouseMotion): the game keeps its DefUser.ini key bindings.
//!
//! The JNI calls arrive on the Android UI thread; they are queued and applied on
//! the game thread at the start of each frame (RedrawRequested).
//!
//! Java conventions: stick(0) movement (towards W/S/A/D), stick(1) look (continuous mouse),
//! x > 0 to the right, y > 0 downwards; touch() for fingers placed outside the controls, in view
//! pixels (full screen, so those of the window): look in game (captured mouse), cursor and left
//! click in menus.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::Mutex;

use openxiii_vm::gui::MenuTouch;

enum Input {
    Key(u8, bool),
    Button(u8, bool),
    Wheel(i32),
    Stick(usize, f32, f32),
    Finger(i32, i32, f32, f32),
    /// Aim assist (iAutoAimMode) turned on or off from the controls panel.
    Aim(bool),
    /// Invincibility (the player's bGodMode, like the God cheat) from the controls panel.
    God(bool),
    /// Player movement speed (×1, ×2, ×3) and infinite ammo (Edit panel).
    Speed(f32),
    Ammo(bool),
    /// Character typed on the phone's keyboard (NativeInput.character): WM_CHAR, as typed on PC.
    Char(u32),
}

static FILE: Mutex<Vec<Input>> = Mutex::new(Vec::new());

fn push_value(e: Input) {
    if let Ok(mut f) = FILE.lock() {
        f.push(e);
    }
}

// JNI signatures: (JNIEnv*, jclass, arguments); jboolean is a byte, jint an i32, jfloat an f32.
#[no_mangle]
pub extern "system" fn Java_org_openxiii_NativeInput_key(_env: *mut c_void, _cls: *mut c_void, vk: i32, down: u8) {
    push_value(Input::Key(vk as u8, down != 0));
}

#[no_mangle]
pub extern "system" fn Java_org_openxiii_NativeInput_mouseButton(_env: *mut c_void, _cls: *mut c_void, button: i32, down: u8) {
    if (0..3).contains(&button) {
        push_value(Input::Button(button as u8, down != 0));
    }
}

#[no_mangle]
pub extern "system" fn Java_org_openxiii_NativeInput_wheel(_env: *mut c_void, _cls: *mut c_void, notches: i32) {
    push_value(Input::Wheel(notches));
}

#[no_mangle]
pub extern "system" fn Java_org_openxiii_NativeInput_stick(_env: *mut c_void, _cls: *mut c_void, which: i32, x: f32, y: f32) {
    if (0..2).contains(&which) {
        push_value(Input::Stick(which as usize, x.clamp(-1.0, 1.0), y.clamp(-1.0, 1.0)));
    }
}

#[no_mangle]
pub extern "system" fn Java_org_openxiii_NativeInput_touch(_env: *mut c_void, _cls: *mut c_void, action: i32, id: i32, x: f32, y: f32) {
    push_value(Input::Finger(action, id, x, y));
}

#[no_mangle]
pub extern "system" fn Java_org_openxiii_NativeInput_setAutoAim(_env: *mut c_void, _cls: *mut c_void, on: u8) {
    // A setting of the Edit panel in both builds, like the consoles' menu (not a cheat).
    push_value(Input::Aim(on != 0));
}

/// Finger look speed (Edit panel setting), bits of an f32; 1.0 by default.
static SPEED_FINGER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0x3F80_0000);

#[no_mangle]
pub extern "system" fn Java_org_openxiii_NativeInput_lookSpeed(_env: *mut c_void, _cls: *mut c_void, speed: f32) {
    if speed.is_finite() && speed > 0.0 {
        SPEED_FINGER.store(speed.to_bits(), Ordering::Relaxed);
    }
}

fn gaze_finger() -> f32 {
    GAZE_FINGER * f32::from_bits(SPEED_FINGER.load(Ordering::Relaxed))
}

#[no_mangle]
pub extern "system" fn Java_org_openxiii_NativeInput_setMoveSpeed(_env: *mut c_void, _cls: *mut c_void, mult: f32) {
    if !super::cheats::ACTIVE {
        return;
    }
    if mult.is_finite() && mult > 0.0 {
        push_value(Input::Speed(mult));
    }
}

#[no_mangle]
pub extern "system" fn Java_org_openxiii_NativeInput_setInfiniteAmmo(_env: *mut c_void, _cls: *mut c_void, on: u8) {
    if !super::cheats::ACTIVE {
        return;
    }
    push_value(Input::Ammo(on != 0));
}

#[no_mangle]
pub extern "system" fn Java_org_openxiii_NativeInput_setGodMode(_env: *mut c_void, _cls: *mut c_void, on: u8) {
    if !super::cheats::ACTIVE {
        return;
    }
    push_value(Input::God(on != 0));
}

/// "Cheats" build (`cheats` feature): the Edit panel shows the cheats. Otherwise it hides them
/// and their settings are ignored (normal build, original game).
#[no_mangle]
pub extern "system" fn Java_org_openxiii_NativeInput_cheats(_env: *mut c_void, _cls: *mut c_void) -> u8 {
    u8::from(super::cheats::ACTIVE)
}

/// The player's iAutoAimMode in the last frame: 0 off, 1 on, 2 no player.
static AIM: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(2);

#[no_mangle]
pub extern "system" fn Java_org_openxiii_NativeInput_autoAim(_env: *mut c_void, _cls: *mut c_void) -> i32 {
    i32::from(AIM.load(Ordering::Relaxed))
}

/// Mouse captured by the game in the last frame: in game, the touch controls are shown;
/// in menus, the finger acts as the cursor.
static CAPTURE: AtomicBool = AtomicBool::new(false);

#[no_mangle]
pub extern "system" fn Java_org_openxiii_NativeInput_captured(_env: *mut c_void, _cls: *mut c_void) -> u8 {
    u8::from(CAPTURE.load(Ordering::Relaxed))
}

/// Startup, for the Java loading screen laid over the game (GameActivityXiii.java): -2 while the
/// game loads before its first frame, 0…100 while the menus' pictures are made (install_pictures),
/// -1 once the game draws its own frames.
static LOADING: AtomicI32 = AtomicI32::new(-2);

/// The startup's progress for the Java loading screen: a percent, or -1 (the game shows itself).
pub fn set_loading(percent: i32) {
    LOADING.store(percent, Ordering::Relaxed);
}

/// A frame of the game is about to be drawn: from the second one on (the first is on the
/// screen), the Java loading screen goes.
pub fn game_frame() {
    static DRAWN: AtomicBool = AtomicBool::new(false);
    if DRAWN.swap(true, Ordering::Relaxed) {
        set_loading(-1);
    }
}

#[no_mangle]
pub extern "system" fn Java_org_openxiii_NativeInput_loading(_env: *mut c_void, _cls: *mut c_void) -> i32 {
    LOADING.load(Ordering::Relaxed)
}

/// Menu text field with the focus in the last frame (XIIIEditCtrl, Vm::menu_text_field): its
/// object, its text and whether it is masked; None outside the menus or on another control. The
/// phone's keyboard is open while there is one (TextKeyboard.java).
static TEXT_FIELD: Mutex<Option<(usize, String, bool)>> = Mutex::new(None);

#[no_mangle]
pub extern "system" fn Java_org_openxiii_NativeInput_character(_env: *mut c_void, _cls: *mut c_void, ch: i32) {
    // UEngine::Key takes a byte (the PC path also keeps only the characters below 256).
    if (1..256).contains(&ch) {
        push_value(Input::Char(ch as u32));
    }
}

/// 0: no text field; otherwise (field object + 1) × 2, + 1 if it is masked (password).
#[no_mangle]
pub extern "system" fn Java_org_openxiii_NativeInput_textField(_env: *mut c_void, _cls: *mut c_void) -> i32 {
    match TEXT_FIELD.lock().ok().and_then(|f| f.as_ref().map(|t| (t.0, t.2))) {
        None => 0,
        Some((o, masked)) => ((o as i32 + 1) << 1) | i32::from(masked),
    }
}

/// Text of the focused field (empty without one), as a Java String.
#[no_mangle]
pub extern "system" fn Java_org_openxiii_NativeInput_textFieldText(env: *mut c_void, _cls: *mut c_void) -> *mut c_void {
    let text = TEXT_FIELD.lock().ok().and_then(|f| f.as_ref().map(|t| t.1.clone())).unwrap_or_default();
    // Modified UTF-8 (JNI): the same as UTF-8 for these characters, without NUL.
    let c = std::ffi::CString::new(text.replace('\0', "")).unwrap_or_default();
    // JNIEnv is a pointer to the JNI function table; NewStringUTF is its entry 167.
    type NewStringUtf = unsafe extern "system" fn(*mut c_void, *const std::ffi::c_char) -> *mut c_void;
    unsafe {
        let table = *(env as *const *const *const c_void);
        let new_string: NewStringUtf = std::mem::transmute(*table.add(167));
        new_string(env, c.as_ptr())
    }
}

/// Where the game image is shown in the full-screen touch view (left, top, width, height, in
/// screen pixels): the image may be smaller (GameActivityXiii's setFixedSize) and narrower than
/// the screen (black bars), fingers are mapped to it.
static VIEW: Mutex<(f32, f32, f32, f32)> = Mutex::new((0.0, 0.0, 0.0, 0.0));

#[no_mangle]
pub extern "system" fn Java_org_openxiii_NativeInput_viewRect(_env: *mut c_void, _cls: *mut c_void, left: i32, top: i32, w: i32, h: i32) {
    if let Ok(mut v) = VIEW.lock() {
        *v = (left as f32, top as f32, w as f32, h as f32);
    }
}

/// The phone's screen in landscape (pixels), told by the activity before the window opens.
static SCREEN: Mutex<Option<(u32, u32)>> = Mutex::new(None);

#[no_mangle]
pub extern "system" fn Java_org_openxiii_NativeInput_screenSize(_env: *mut c_void, _cls: *mut c_void, w: i32, h: i32) {
    if w > 0 && h > 0 {
        if let Ok(mut s) = SCREEN.lock() {
            *s = Some((w.max(h) as u32, w.min(h) as u32));
        }
    }
}

/// The phone's screen (landscape), once the activity told it.
pub fn screen_size() -> Option<(u32, u32)> {
    SCREEN.lock().ok().and_then(|s| *s)
}

/// Size of the game image the activity is to give its surface (width << 32 | height), 0 when
/// it has not changed since the activity last read it.
static WANTED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// The game image is to be `size` pixels (a resolution chosen in the Video menu, or the ini's at
/// startup): the activity sets its surface to it, the screen scales it up keeping its shape.
pub fn want_surface(size: (u32, u32)) {
    if size.0 > 0 && size.1 > 0 {
        println!("Android: game image {}x{}", size.0, size.1);
        WANTED.store((u64::from(size.0) << 32) | u64::from(size.1), Ordering::Relaxed);
    }
}

#[no_mangle]
pub extern "system" fn Java_org_openxiii_NativeInput_wantedSurface(_env: *mut c_void, _cls: *mut c_void) -> i64 {
    WANTED.swap(0, Ordering::Relaxed) as i64
}

/// Finger drag (screen pixels) → mouse pixels.
const GAZE_FINGER: f32 = 0.6;
/// A look finger that moved more than this fraction of the image height only
/// turns the camera and will not fire.
const SLIDES: f32 = 0.03;
/// Look finger held without moving for at least this time (seconds): held fire (burst) until lifted.
/// Lifted earlier, it is a tap: one shot.
const LONG_PRESS: f32 = 0.3;
/// Tap: left click (Fire) held for this number of frames.
const TAPE_IMAGES: u8 = 3;
/// Menu: frames of hovering under the finger before the click, then frames it stays pressed at least.
const MENU_HOVER: u8 = 6;
const MENU_HELD: u8 = 6;
/// Menu slider worked by clicks (MenuTouch::Clicks): frames each click stays pressed.
const SLIDER_HELD: u8 = 2;
/// Menu list scrolled by the finger (MenuTouch::List): one wheel step per this share of the
/// menu page's height (about a row of the lists).
const LIST_ROWS: f32 = 14.0;
/// touch() of a finger aiming from a button (TouchControls.AIM_ID_OFFSET): look only.
const AIM_BUTTON: i32 = 100;

/// State of the touch input between two frames.
#[derive(Default)]
pub struct TouchControls {
    sticks: [(f32, f32); 2],
    /// Finger looking or pointing.
    finger: Option<Finger>,
    /// Where the game image was on the screen and its size (VIEW, window size) when the finger's
    /// points were last mapped: the image can change size while a finger is down.
    mapping: Option<((f32, f32, f32, f32), (u32, u32))>,
    /// GUIController of the last menu opened (faster lookup).
    menu: Option<usize>,
    /// Frames left before releasing a tap's left click.
    tape: u8,
    /// Help bar key sent by a finger lifted over its side, and the frames left before releasing it.
    key_tap: Option<(u8, u8)>,
    /// The gamepad was used after the last finger: the menus show their cursor.
    pad_last: bool,
    /// The Edit panel's aim assist switch changed: the profile's configuration is to be written
    /// (USaveGameDevice RequestWriteUserConfig) once the save device is free.
    aim_save: bool,
    /// Testing (variables read by android.rs): start time and last time seen.
    test_start: Option<std::time::Instant>,
    test_before: f32,
}

#[derive(Clone, Copy)]
struct Finger {
    id: i32,
    /// Last position, in game image pixels.
    x: f32,
    y: f32,
    mode: Mode,
}

#[derive(Clone, Copy)]
enum Mode {
    /// Menu: cursor brought under the finger, which hovers; the click comes when the finger lifts
    /// over the control it was put on (`start`, None: no control, then only near `start_point`)
    /// and over the same element drawn outside the controls (`start_part`, gui.rs, menu_part_at).
    /// `kind`: how the control put on is worked (gui.rs, MenuTouch). `after`: frames since the
    /// lift. `focus`: 1 while a click that only gives the focus is held, 2 once sent. `last`:
    /// MenuTouch::Clicks, the cursor of the last click. `scroll_from`: MenuTouch::List, the page's
    /// height where the last wheel step was sent; `scrolled`: the finger scrolled the list (no click).
    Menu {
        sunk: bool,
        raised: bool,
        images: u8,
        pose: u8,
        held_frames: u8,
        start: Option<usize>,
        start_part: Option<usize>,
        start_point: (f32, f32),
        kind: MenuTouch,
        after: u8,
        focus: u8,
        last: Option<(i32, i32)>,
        scroll_from: Option<f32>,
        scrolled: bool,
    },
    /// In game (captured mouse): dragging looks; held without moving, a tap fires one shot and a
    /// long press fires continuously. `slides`: the finger moved beyond SLIDES (look only, never fires);
    /// `burst`: left click held by the long press.
    Gaze { start_point: (f32, f32), pose: std::time::Instant, shot: bool, slides: bool, burst: bool },
    /// Windows mouse (not captured): direct cursor and click.
    Window,
    /// "Select level" (choice_level.rs): finger taken by its entry or its open list.
    Choice,
    /// Improved build: finger taken by the open Mods page (mods_page.rs).
    MenuPage(MenuPageTouch),
    /// Menu: finger on the bottom help bar ("Escape Back" on the left, "Select Enter" on the
    /// right), as a phone's button: the side is drawn highlighted while the finger stays on it,
    /// and its key is sent when the finger lifts there; slid off the side and lifted elsewhere,
    /// nothing is sent.
    Key(u8),
}

impl super::App {
    // key_windows, button_mouse, mouse_relative and the gamepad: pad_keys.rs (shared with PC).

    /// A finger (window pixels) on the menu's page: the improved build's whole-window menu frame
    /// (gui.rs, window_menu_frame) puts the GUI's MouseX / MouseY on its 4:3 page, centred and
    /// scaled, not on the window (as main.rs does for the mouse-less Mods page).
    fn on_menu_page(&self, x: f32, y: f32) -> (f32, f32) {
        let size = self.window.as_ref().map(|w| w.inner_size()).unwrap_or_default();
        self.game.as_ref().map_or((x, y), |g| g.vm.menu_screen(size.width.max(1), size.height.max(1)).to_page((x, y)))
    }

    /// The menu control under a finger (window pixels), and how it is worked (gui.rs,
    /// menu_control_at).
    fn menu_control_at(&mut self, x: f32, y: f32) -> Option<(usize, MenuTouch)> {
        let (px, py) = self.on_menu_page(x, y);
        let ctrl = self.touch_controls.menu?;
        self.game.as_mut()?.vm.menu_control_at(ctrl, px, py)
    }

    /// The element drawn outside the game's controls under a finger (window pixels; gui.rs,
    /// menu_part_at).
    fn menu_part_at(&self, x: f32, y: f32) -> Option<usize> {
        let (px, py) = self.on_menu_page(x, y);
        self.game.as_ref()?.vm.menu_part_at(px, py)
    }

    fn cursor_to(&mut self, x: f32, y: f32) {
        self.cursor_value = (x.clamp(0.0, 65535.0) as u32, y.clamp(0.0, 65535.0) as u32);
        if let Some(g) = &mut self.game {
            let (x, y) = self.cursor_value;
            g.vm.win_message(0x200, 0, (y << 16) | x);
        }
    }

    /// Applies the touch input received since the previous frame, then the look stick
    /// for `dt` seconds.
    pub(crate) fn apply_touch_controls(&mut self, _dt: f32) {
        // Menu open (GUIController with a page): the finger points the menu cursor.
        let menu = self.game.as_ref().and_then(|g| g.vm.cursor_menu(self.touch_controls.menu));
        self.touch_controls.menu = menu.map(|m| m.0);
        let capture = self.game.as_ref().is_some_and(|g| g.vm.win_input.capture != 0);
        CAPTURE.store(capture && menu.is_none(), Ordering::Relaxed);
        let field = menu.and_then(|m| self.game.as_ref().and_then(|g| g.vm.menu_text_field(m.0)));
        if let Ok(mut f) = TEXT_FIELD.lock() {
            *f = field;
        }
        self.apply_cheats();
        let inputs = FILE.lock().map(|mut f| std::mem::take(&mut *f)).unwrap_or_default();
        // Aim assist of the device in use (the VM's aim_device.rs), as on PC: a finger or a touch
        // control (not the Edit panel's settings) gives the touch value, the gamepad its own.
        let touched = inputs.iter().any(|e| matches!(e, Input::Key(..) | Input::Button(..) | Input::Wheel(_) | Input::Stick(..) | Input::Finger(..)));
        // The first player's gamepads: in split screen the other players' gamepads drive only
        // their own player (the VM's split_versus.rs, as on PC); otherwise every gamepad. The
        // first player's gamepad settings page has them: the game sees them at rest.
        let pad = self.first_player_pads();
        super::pad::set_first_player_gamepad(pad.is_some());
        let pad = if self.first_page_has_pad { Some(([0.0; 6], 0)) } else { pad };
        // No menu cursor while the finger drives the menus; a gamepad used last shows it again.
        if inputs.iter().any(|e| matches!(e, Input::Finger(..))) {
            self.touch_controls.pad_last = false;
        } else if pad.is_some_and(|(a, b)| b != 0 || a.iter().any(|v| v.abs() > 0.3)) {
            self.touch_controls.pad_last = true;
        }
        if let Some(g) = &mut self.game {
            g.vm.touch_hides_cursor = !self.touch_controls.pad_last;
        }
        self.aim_device_frame(touched, pad);
        // Touch controls in use: the console versions' view turning towards the held target
        // (bRotationAssist) follows the touch aim assist (iAutoAimMode, the Edit panel's switch),
        // as both follow the gamepad's setting (the VM's aim_device.rs): the switch shows the whole
        // help, and turning it off leaves none.
        if !self.aim_on_pad {
            if let Some(g) = &mut self.game {
                g.vm.rotation_assist_follows_aim(g.controller);
            }
        }
        // (Lockstep co-op: this machine's player's controller, not the session's first player's.)
        let aim = self.game.as_ref().and_then(|g| g.vm.lockstep_local_controller().or(g.controller).map(|pc| u8::from(g.vm.propr(pc, "iAutoAimMode").as_int() != 0)));
        AIM.store(aim.unwrap_or(2), Ordering::Relaxed);
        if self.touch_controls.aim_save {
            if let Some(d) = self.game.as_mut().and_then(|g| g.vm.save_device()) {
                if d.is_memory_card_ready() && d.has_profile() {
                    d.request_write_user_config();
                    self.touch_controls.aim_save = false;
                }
            } else {
                self.touch_controls.aim_save = false;
            }
        }
        self.follow_image_resize();
        for e in inputs {
            match e {
                // Android Back over an open "Select level": back in the menu, not in the game below.
                // Android Back over the open Mods page (improved build): closes it.
                Input::Key(0x1B, down) if self.mods_touch_back(down) => {}
                // Android Back over the open gamepad settings page: back in the page.
                Input::Key(0x1B, down) if self.gamepad_page.open => {
                    if down {
                        self.gamepad_page_key(super::gamepad_page::Key::Back);
                    }
                }
                Input::Key(0x1B, down) if self.choice.unlocked => {
                    if down {
                        self.choice.key_col(super::choice_level::Key::Return);
                    }
                }
                Input::Key(vk, down) => self.key_windows(vk, down),
                Input::Char(ch) => {
                    if let Some(g) = &mut self.game {
                        g.vm.win_message(0x102, ch, 1);
                    }
                }
                Input::Speed(m) => {
                    self.cheats.speed_value = Some(m);
                    println!("touch: movement speed ×{m}");
                }
                Input::Ammo(on) => {
                    self.cheats.ammo = on;
                    println!("touch: infinite ammo {}", if on { "on" } else { "off" });
                }
                Input::God(on) => {
                    self.cheats.god = on;
                    println!("touch: invincibility {}", if on { "on" } else { "off" });
                }
                Input::Button(b, down) => self.button_mouse(b, down),
                Input::Wheel(n) => {
                    if let Some(g) = &mut self.game {
                        g.vm.win_input.active = true;
                        g.vm.win_input.push_mouse(8, (n * 120) as u32);
                    }
                }
                Input::Stick(w, x, y) => self.touch_controls.sticks[w] = (x, y),
                Input::Aim(on) => {
                    self.set_auto_aim(on);
                    // The profile keeps its own copy of User.ini (read again when it is loaded):
                    // written there too, as the game's options do.
                    self.touch_controls.aim_save = true;
                    println!("touch: aim assist {}", if on { "on" } else { "off" });
                }
                Input::Finger(action, id, x, y) => {
                    // Screen pixels → game image pixels, with the mapping the held finger's points
                    // are in (the image can change size between two events).
                    self.follow_image_resize();
                    let (x, y) = match self.touch_controls.mapping {
                        Some((rect, size)) => openxiii_vm::video_settings::android_touch_to_image(rect, size, (x, y)),
                        None => (x, y),
                    };
                    self.finger(action, id, x, y, menu.is_some(), capture);
                }
            }
        }
        if let Some((_, mx, my, sens)) = menu {
            self.cursor_to_finger(mx, my, sens);
        } else if let Some(Finger { mode: Mode::Menu { sunk, .. }, .. }) = self.touch_controls.finger {
            // The click closed the menu (Resume, loading a save…): this finger no longer has
            // a cursor to drive. Kept, it would block every following finger (no more finger look).
            self.touch_controls.finger = None;
            if sunk {
                self.button_mouse(0, false);
            }
        }
        // Look finger left in place without moving: burst.
        if let Some(Finger { mode: Mode::Gaze { pose, shot: true, slides: false, burst, .. }, .. }) = self.touch_controls.finger.as_mut() {
            if !*burst && pose.elapsed().as_secs_f32() >= LONG_PRESS {
                *burst = true;
                println!("touch: long press, burst");
                if self.touch_controls.tape > 0 {
                    self.touch_controls.tape = 0;
                    self.button_mouse(0, false);
                }
                self.button_mouse(0, true);
            }
        }
        // End of a help bar tap's key.
        if let Some((vk, n)) = self.touch_controls.key_tap {
            if n <= 1 {
                self.touch_controls.key_tap = None;
                self.key_windows(vk, false);
            } else {
                self.touch_controls.key_tap = Some((vk, n - 1));
            }
        }
        // End of a tap's shot.
        if self.touch_controls.tape > 0 {
            self.touch_controls.tape -= 1;
            if self.touch_controls.tape == 0 {
                self.button_mouse(0, false);
            }
        }
        // Lockstep co-op: the gamepad reaches the game in this player's commands (the VM's
        // lockstep.rs, coop_pad_input on every machine), not also as keys; the touch controls do.
        let pad = if self.game.as_ref().is_some_and(|g| g.vm.lockstep_on()) { None } else { pad };
        self.gamepad(pad, menu.is_some() || !capture);
        // Movement stick (or the gamepad's left stick in game) → W, S, A, D.
        self.pad_wsad(pad, menu.is_some() || !capture, self.touch_controls.sticks[0]);
        // Look stick (and the gamepad's right stick): the game's joystick path, as on the consoles.
        self.look_stick_frame(true, menu.is_some() || !capture, self.touch_controls.sticks[1]);
    }

    /// Where the game image is on the screen (VIEW) and its size (the window's), with which the
    /// fingers' screen pixels are mapped to image pixels.
    fn image_mapping(&self) -> Option<((f32, f32, f32, f32), (u32, u32))> {
        let size = self.window.as_ref()?.inner_size();
        Some((VIEW.lock().map(|v| *v).unwrap_or_default(), (size.width, size.height)))
    }

    /// The game image changed size or place (the activity applying the ini's resolution as the
    /// first menu appears, or the Video menu's) while a finger is down: its points, kept in the
    /// old image's pixels, are mapped again onto the new image. Otherwise a menu tap made just
    /// then brought the cursor elsewhere and clicked nothing.
    fn follow_image_resize(&mut self) {
        let now = self.image_mapping();
        let before = std::mem::replace(&mut self.touch_controls.mapping, now);
        let (Some(old), Some(new), Some(d)) = (before, now, self.touch_controls.finger.as_mut()) else { return };
        if old == new {
            return;
        }
        let remap = |p: (f32, f32)| openxiii_vm::video_settings::android_touch_remap(old, new, p);
        (d.x, d.y) = remap((d.x, d.y));
        match &mut d.mode {
            Mode::Menu { start_point, .. } | Mode::Gaze { start_point, .. } => *start_point = remap(*start_point),
            _ => {}
        }
    }

    /// A finger outside the controls: in a menu, it brings the cursor then clicks; in game
    /// (captured mouse), it looks; otherwise (Windows mouse), direct cursor and click.
    fn finger(&mut self, action: i32, id: i32, x: f32, y: f32, menu: bool, capture: bool) {
        match action {
            0 => {
                if self.touch_controls.finger.is_some() {
                    return;
                }
                if self.begin_mods_touch(id, x, y) { return; }
                if self.choice.active_value() || self.choice.unlocked {
                    let _ = self.choice.finger(id as u64, super::choice_level::Finger::Pose, (x, y));
                    if self.choice.follows(id as u64) || self.choice.unlocked {
                        self.touch_controls.finger = Some(Finger { id, x, y, mode: Mode::Choice });
                        return;
                    }
                }
                let mode = if let Some(vk) = self.help_bar_key(x, y).filter(|_| menu) {
                    // Highlighted only: the key waits for the lift (help_bar_key_moves).
                    Mode::Key(vk)
                } else if menu {
                    let (start, kind) = self.menu_control_at(x, y).map_or((None, MenuTouch::Tap), |(c, k)| (Some(c), k));
                    let start_part = self.menu_part_at(x, y);
                    Mode::Menu { sunk: false, raised: false, images: 0, pose: 0, held_frames: 0, start, start_part, start_point: (x, y), kind, after: 0, focus: 0, last: None, scroll_from: None, scrolled: false }
                } else if capture {
                    let shot = id < AIM_BUTTON;
                    Mode::Gaze { start_point: (x, y), pose: std::time::Instant::now(), shot, slides: !shot, burst: false }
                } else {
                    self.cursor_to(x, y);
                    self.button_mouse(0, true);
                    Mode::Window
                };
                self.touch_controls.finger = Some(Finger { id, x, y, mode });
            }
            1 => {
                let Some(d) = self.touch_controls.finger.as_mut().filter(|d| d.id == id) else { return };
                let (px, py) = (d.x, d.y);
                d.x = x;
                d.y = y;
                let mode = d.mode;
                match mode {
                    Mode::Gaze { slides: true, .. } | Mode::Gaze { burst: true, .. } => {
                        self.mouse_relative((x - px) * gaze_finger(), (y - py) * gaze_finger())
                    }
                    Mode::Gaze { start_point, pose, shot, .. } => {
                        let (dx, dy) = (x - start_point.0, y - start_point.1);
                        if (dx * dx + dy * dy).sqrt() > self.threshold_slides() {
                            // The finger moves off: look only, the camera catches up with the whole drag since the press.
                            if let Some(d) = self.touch_controls.finger.as_mut() {
                                d.mode = Mode::Gaze { start_point, pose, shot, slides: true, burst: false };
                            }
                            self.mouse_relative(dx * gaze_finger(), dy * gaze_finger());
                        }
                    }
                    Mode::Window => self.cursor_to(x, y),
                    Mode::Choice => {
                        let _ = self.choice.finger(id as u64, super::choice_level::Finger::Moves, (x, y));
                    }
                    Mode::MenuPage(_) => self.move_mods_touch(id, x, y, super::choice_level::Finger::Moves),
                    Mode::Key(vk) => {
                        self.help_bar_key_moves(vk, x, y, menu);
                    }
                    Mode::Menu { .. } => {}
                }
            }
            _ => {
                let Some(d) = self.touch_controls.finger.as_mut().filter(|d| d.id == id) else { return };
                d.x = x;
                d.y = y;
                match &mut d.mode {
                    Mode::Menu { raised, .. } => *raised = true,
                    Mode::Window => {
                        self.touch_controls.finger = None;
                        self.cursor_to(x, y);
                        self.button_mouse(0, false);
                    }
                    Mode::Choice => {
                        self.touch_controls.finger = None;
                        if let Some(url) = self.choice.finger(id as u64, super::choice_level::Finger::Raised, (x, y)) {
                            self.leave_to(&url);
                        }
                    }
                    Mode::MenuPage(_) => {
                        self.touch_controls.finger = None;
                        self.move_mods_touch(id, x, y, super::choice_level::Finger::Raised);
                    }
                    Mode::Key(vk) => {
                        let vk = *vk;
                        self.touch_controls.finger = None;
                        let over = self.help_bar_key_moves(vk, x, y, menu);
                        if let Some(g) = &mut self.game {
                            g.vm.help_bar_lift();
                        }
                        if over {
                            // Pressed now, released a few frames later (the game reads the key's state
                            // once a frame: pressed and released at once, it would not see it).
                            if let Some((before, _)) = self.touch_controls.key_tap.take() {
                                self.key_windows(before, false);
                            }
                            self.key_windows(vk, true);
                            self.touch_controls.key_tap = Some((vk, TAPE_IMAGES));
                        } else {
                            println!("touch: help bar tap dropped (lifted off its side)");
                        }
                    }
                    Mode::Gaze { pose, slides, burst, .. } => {
                        let (pose, slides, burst) = (*pose, *slides, *burst);
                        self.touch_controls.finger = None;
                        if burst {
                            println!("touch: end of the burst");
                            self.button_mouse(0, false);
                        } else if !slides && pose.elapsed().as_secs_f32() < LONG_PRESS {
                            if self.touch_controls.tape > 0 {
                                self.button_mouse(0, false);
                            }
                            println!("touch: tap, one shot");
                            self.button_mouse(0, true);
                            self.touch_controls.tape = TAPE_IMAGES;
                        }
                    }
                }
            }
        }
    }

    /// The help bar's key under a finger (window pixels): the side of the bar as the active page
    /// drew it on the last frame (its two texts with a margin, the VM's help_bar_click.rs), or,
    /// when the VM does not note it (faithful build), the fixed zones of `key_help_menu`.
    fn help_bar_key(&mut self, x: f32, y: f32) -> Option<u8> {
        let (px, py) = self.on_menu_page(x, y);
        let drawn = match (self.touch_controls.menu, self.game.as_mut()) {
            (Some(ctrl), Some(g)) => g.vm.help_bar_touch(ctrl, px, py),
            _ => None,
        };
        drawn.unwrap_or_else(|| self.key_help_menu(x, y))
    }

    /// A finger put on the help bar's side of `vk` is at (x, y) (window pixels): whether it is
    /// still on that side (a menu shown), which then stays highlighted; off it, no side is.
    fn help_bar_key_moves(&mut self, vk: u8, x: f32, y: f32, menu: bool) -> bool {
        let over = menu && self.help_bar_key(x, y) == Some(vk);
        if !over {
            if let Some(g) = &mut self.game {
                g.vm.help_bar_lift();
            }
        }
        over
    }

    /// Help bar at the bottom of the menus (below 93 % of the height; the main menu centres it in 4:3,
    /// the pause menu spans the full width): Escape on the left third, Enter on the
    /// right third.
    fn key_help_menu(&self, x: f32, y: f32) -> Option<u8> {
        let t = self.window.as_ref()?.inner_size();
        let u = x / t.width as f32;
        if y < t.height as f32 * 0.93 {
            return None;
        }
        if u < 0.36 {
            Some(0x1B)
        } else if u > 0.64 {
            Some(0x0D)
        } else {
            None
        }
    }

    /// SLIDES in game image pixels.
    fn threshold_slides(&self) -> f32 {
        self.window.as_ref().map_or(600.0, |w| w.inner_size().height as f32) * SLIDES
    }

    /// Menu: mouse movements (DIMOFS_X / Y) that bring the GUIController's MouseX / MouseY under
    /// the finger (UGUIController::NativeKeyEvent: MouseX += dx × MenuMouseSens, MouseY −= −dy ×
    /// MenuMouseSens): the cursor hovers under the finger and follows it. The left click comes
    /// when the finger lifts, over the control it was put on (a finger slid off a button clicks
    /// nothing); a dragged control (slider, scroll bar) is pressed as soon as the cursor arrives,
    /// held while the finger moves and released when it lifts.
    fn cursor_to_finger(&mut self, mx: i32, my: i32, sens: f32) {
        let Some(d) = self.touch_controls.finger else { return };
        let Mode::Menu { mut sunk, raised, mut images, mut pose, mut held_frames, start, start_part, start_point, kind, mut after, mut focus, mut last, mut scroll_from, mut scrolled } = d.mode else { return };
        let (fx, fy) = self.on_menu_page(d.x, d.y);
        let (rx, ry) = (fx - mx as f32, fy - my as f32);
        let arrive = rx.abs() <= 2.0 && ry.abs() <= 2.0;
        // The click waits a few frames after arrival (the menu has seen the hover and given focus to
        // the control), or 30 frames at most; it stays pressed a few frames even for a short tap:
        // a tap lifted immediately often did not click.
        pose = if arrive { pose.saturating_add(1) } else { 0 };
        if sunk {
            held_frames = held_frames.saturating_add(1);
        }
        images = images.saturating_add(1);
        if raised {
            after = after.saturating_add(1);
        }
        if !arrive {
            let s = if sens > 0.0 { sens } else { 1.0 };
            self.mouse_relative(rx / s, ry / s);
        }
        // A list: each row height the finger moves up or down is a wheel step (the list follows the
        // finger: down shows the rows above), and the lift then clicks nothing.
        if kind == MenuTouch::List && !raised {
            let size = self.window.as_ref().map(|w| w.inner_size()).unwrap_or_default();
            let page_h = self.game.as_ref().map_or(600, |g| g.vm.menu_screen(size.width.max(1), size.height.max(1)).size.1);
            let step = (page_h as f32 / LIST_ROWS).max(1.0);
            let from = *scroll_from.get_or_insert(fy);
            let n = ((fy - from) / step).trunc() as i32;
            if n != 0 {
                if let Some(g) = &mut self.game {
                    g.vm.win_input.active = true;
                    g.vm.win_input.push_mouse(8, (n * 120) as u32);
                }
                scroll_from = Some(from + n as f32 * step);
                scrolled = true;
            }
        }
        if scrolled && raised {
            self.touch_controls.finger = None;
            return;
        }
        // A click that would only give the focus (back on a page with the cursor already on the
        // button: gui.rs, menu_click_only_focuses) is sent first, released, then the real click.
        let ctrl = self.touch_controls.menu;
        let only_focuses = |app: &Self| focus == 0 && ctrl.zip(app.game.as_ref()).is_some_and(|(c, g)| g.vm.menu_click_only_focuses(c));
        let focus_done = sunk && focus == 1 && held_frames >= MENU_HELD;
        let done = if focus_done {
            self.button_mouse(0, false);
            (sunk, held_frames, focus, pose, images, after, last) = (false, 0, 2, 0, 0, 0, None);
            false
        } else if kind == MenuTouch::Clicks {
            // A short click each time the cursor is at a new place (the first once it has hovered
            // under the finger), while the finger stays on the slider it was put on; done once
            // lifted and the place it was lifted at clicked.
            if sunk {
                if held_frames >= SLIDER_HELD {
                    self.button_mouse(0, false);
                    (sunk, held_frames) = (false, 0);
                }
                false
            } else if (pose >= MENU_HOVER || last.is_some()) && last != Some((mx, my)) {
                if self.menu_control_at(d.x, d.y).map(|c| c.0) == start {
                    if only_focuses(self) {
                        focus = 1;
                    }
                    self.button_mouse(0, true);
                    sunk = true;
                }
                last = Some((mx, my));
                false
            } else if raised && ((pose >= 2 && last == Some((mx, my))) || after >= 30) {
                self.touch_controls.finger = None;
                return;
            } else {
                false
            }
        } else if kind == MenuTouch::Hold {
            let ready = pose >= MENU_HOVER || images >= 30;
            if ready && !sunk {
                if only_focuses(self) {
                    focus = 1;
                }
                self.button_mouse(0, true);
                sunk = true;
                false
            } else {
                ready && raised && sunk && held_frames >= MENU_HELD
            }
        } else if !raised {
            false
        } else if !sunk {
            if pose >= MENU_HOVER || after >= 30 {
                // Same control under the finger as when it was put down; put down on none (a tap
                // on the page itself, or on a page not laid out yet): only if the finger stayed
                // near where it was put down (gui.rs, menu_tap_clicks).
                let now = self.menu_control_at(d.x, d.y).map(|c| c.0);
                let near = (d.x - start_point.0).hypot(d.y - start_point.1) <= self.threshold_slides();
                if openxiii_vm::gui::menu_tap_clicks(start, start_part, now, self.menu_part_at(d.x, d.y), near) {
                    if only_focuses(self) {
                        focus = 1;
                    }
                    self.button_mouse(0, true);
                    sunk = true;
                    false
                } else {
                    println!("touch: menu tap dropped (lifted over {now:?}, put down on {start:?})");
                    self.touch_controls.finger = None;
                    return;
                }
            } else {
                false
            }
        } else {
            held_frames >= MENU_HELD
        };
        if done {
            self.button_mouse(0, false);
            self.touch_controls.finger = None;
            return;
        }
        if let Some(d) = self.touch_controls.finger.as_mut() {
            d.mode = Mode::Menu { sunk, raised, images, pose, held_frames, start, start_part, start_point, kind, after, focus, last, scroll_from, scrolled };
        }
    }
}

register_viewer_layer!("touch_menus");
