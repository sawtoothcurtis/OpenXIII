//! Natives of GUI.dll (XIII's menus): GUIController (fonts, styles, inactivity delay,
//! resolution), GUIFont.

use crate::value::{ObjId, Value};
use crate::Vm;

impl Vm {
    /// Entry of `stack` (FontStack, StyleStack) whose KeyName is `key` (FString::operator==,
    /// case-insensitive), otherwise None.
    fn gui_find_key(&self, this: ObjId, stack: &str, key: &str) -> Value {
        if let Value::Array(items) = self.prop(this, stack) {
            for item in items {
                if let Some(o) = item.as_object() {
                    if self.objects.get(o).is_some_and(|x| x.is_some()) && self.propr(o, "KeyName").as_name().eq_ignore_ascii_case(key) {
                        return Value::Object(Some(o));
                    }
                }
            }
        }
        Value::Object(None)
    }

    /// Size of the player's viewport (UViewport::SizeX/SizeY).
    pub fn viewport_size(&self) -> (i32, i32) {
        self.viewport
    }

    /// Viewport->SizeX, SizeY: zero as long as the window is not open (GETCURRENTRES returns
    /// "0x0" in the first menu, opened before the window by the first map).
    pub(crate) fn viewport_size_x_y(&self) -> (i32, i32) {
        if self.engine.client.window_open {
            self.viewport
        } else {
            (0, 0)
        }
    }

    /// Natives of GUI.dll; `None` if `name` is not one of them.
    pub(crate) fn gui_native(&mut self, name: &str, this: ObjId, args: &mut [Value], supplied: &[bool]) -> Option<Value> {
        Some(match name {
            // UGUIController::execGetMenuFont / execGetStyle.
            "GetMenuFont" => self.gui_find_key(this, "FontStack", &args[0].as_name()),
            "GetStyle" => self.gui_find_key(this, "StyleStack", &args[0].as_name()),
            // UGUIController::execSetInputTimeOut: TimeOutArmed, delay (by default
            // the current one), time of the last input = appSeconds().
            "SetInputTimeOut" => {
                let arm = args[0].as_bool();
                if supplied.get(1) == Some(&true) {
                    self.set_prop(this, "DelayOfInactivityBeforeCallingTimeOut", Value::Float(args[1].as_float()));
                }
                self.set_prop(this, "TimeOutArmed", Value::Bool(arm));
                if arm {
                    self.set_prop(this, "TimeOfLastInput", Value::Float(self.app_seconds as f32));
                }
                Value::Void
            }
            // UGUIController::execInitStateFrame: Object->InitExecution; the VM's objects
            // have an implicit frame, but a second call must also clear it.
            "InitStateFrame" => {
                if let Some(target) = args.first().and_then(Value::as_object) {
                    if let Some(Some(object)) = self.objects.get_mut(target) {
                        object.state = None;
                        object.state_class = None;
                        self.threads.remove(&target);
                        self.probe_disabled.remove(&target);
                        self.init_execution(target);
                    }
                }
                Value::Void
            },
            // UGUIController::execGetCurrentRes: "WxH" of the player's viewport.
            "GetCurrentRes" => {
                let player = self.propr(this, "ViewportOwner").as_object();
                if player.is_some() {
                    let (w, h) = self.viewport_size_x_y();
                    Value::Str(format!("{w}x{h}"))
                } else {
                    Value::Str(String::new())
                }
            }
            // UGUIFont::execGetFont: font for resolution XRes, loaded once
            // (FontArrayFonts) from FontArrayNames.
            "GetFont" if self.is_a(this, "GUIFont") => {
                let x = args[0].as_int();
                let mut i = if x < 800 || self.propr(this, "bFixedSize").as_bool() {
                    0
                } else if x < 1024 {
                    1
                } else if x < 1280 {
                    2
                } else {
                    3 + (x > 1599) as i32
                };
                let names = match self.prop(this, "FontArrayNames") {
                    Value::Array(n) => n,
                    _ => Vec::new(),
                };
                i = i.min(names.len() as i32 - 1);
                if i < 0 {
                    return Some(Value::Object(None));
                }
                let i = i as usize;
                let mut fonts = match self.prop(this, "FontArrayFonts") {
                    Value::Array(f) => f,
                    _ => Vec::new(),
                };
                if fonts.len() <= i {
                    fonts.resize(i + 1, Value::Object(None));
                }
                if !matches!(fonts[i], Value::Asset(_)) {
                    let path = names[i].as_name();
                    fonts[i] = if self.font(&path).is_some() {
                        Value::Asset(path)
                    } else {
                        self.log
                            .push(format!("Warning: {} Couldn't load font {path}", self.objects[this].as_ref().map(|o| o.name.clone()).unwrap_or_default()));
                        Value::Object(None)
                    };
                    self.set_prop(this, "FontArrayFonts", Value::Array(fonts.clone()));
                }
                fonts[i].clone()
            }
            // UGUIComponent::execActualWidth, execActualHeight, execActualLeft,
            // execActualTop: virtual methods.
            "ActualWidth" if self.is_a(this, "GUIComponent") => Value::Float(self.gui_actual_size(this, false)),
            "ActualHeight" if self.is_a(this, "GUIComponent") => Value::Float(self.gui_actual_size(this, true)),
            "ActualLeft" if self.is_a(this, "GUIComponent") => Value::Float(self.gui_actual_pos(this, false)),
            "ActualTop" if self.is_a(this, "GUIComponent") => Value::Float(self.gui_actual_pos(this, true)),
            // UGUIComponent::execSetTimer: timer held by the component's page.
            "SetTimer" if self.is_a(this, "GUIComponent") => {
                let interval = args[0].as_float();
                let repeat = supplied.get(1) == Some(&true) && args[1].as_bool();
                let mut page = this;
                let mut guard = 0;
                while let (Some(owner), true) = (self.propr(page, "MenuOwner").as_object(), guard < 256) {
                    page = owner;
                    guard += 1;
                }
                if !self.is_a(page, "GUIPage") {
                    self.warn(format!("SetTimer: {} is not in a GUIPage", self.objects[page].as_ref().map(|o| o.name.clone()).unwrap_or_default()));
                } else if self.propr(this, "TimerIndex").as_int() < 0 && interval > 0.0 {
                    let mut timers = match self.prop(page, "Timers") {
                        Value::Array(t) => t,
                        _ => Vec::new(),
                    };
                    timers.push(Value::Object(Some(this)));
                    let index = timers.len() as i32 - 1;
                    self.set_prop(page, "Timers", Value::Array(timers));
                    self.set_prop(this, "TimerIndex", Value::Int(index));
                }
                self.set_prop(this, "TimerCountdown", Value::Float(interval));
                self.set_prop(this, "bTimerRepeat", Value::Bool(false));
                self.set_prop(this, "TimerInterval", Value::Float(interval));
                if interval > 0.0 {
                    self.set_prop(this, "bTimerRepeat", Value::Bool(repeat));
                }
                Value::Void
            }
            // UGUIComponent::execKillTimer: UpdateTimers removes the component.
            "KillTimer" if self.is_a(this, "GUIComponent") => {
                self.set_prop(this, "TimerInterval", Value::Float(0.0));
                self.set_prop(this, "TimerCountdown", Value::Float(0.0));
                self.set_prop(this, "bTimerRepeat", Value::Bool(false));
                Value::Void
            }
            // UGUIStyles::execDraw → Draw; execDrawText →
            // DrawText.
            "Draw" if self.is_a(this, "GUIStyles") => {
                if let Some(c) = args[0].as_object() {
                    let (state, l, t, w, h) = (args[1].as_byte(), args[2].as_float(), args[3].as_float(), args[4].as_float(), args[5].as_float());
                    self.gui_style_draw(this, c, state, l, t, w, h);
                }
                Value::Void
            }
            "DrawText" if self.is_a(this, "GUIStyles") => {
                if let Some(c) = args[0].as_object() {
                    let (state, l, t, w, h) = (args[1].as_byte(), args[2].as_float(), args[3].as_float(), args[4].as_float(), args[5].as_float());
                    let (align, text) = (args[6].as_byte(), args[7].as_name());
                    self.gui_style_draw_text(this, c, state, l, t, w, h, align, &text);
                }
                Value::Void
            }
            // UGUIList::execSortList: Sort<GUIListElem> of UnTemplate.h, CompareItem
            // if it is assigned, otherwise appStricmp of the Items.
            "SortList" if self.is_a(this, "GUIList") => {
                let mut elems = match self.prop(this, "Elements") {
                    Value::Array(e) => e,
                    _ => Vec::new(),
                };
                let delegate = self.gui_delegate_bound(this, "CompareItem");
                let mut cmp = |vm: &mut Vm, a: &Value, b: &Value| -> i32 {
                    if delegate {
                        vm.gui_delegate(this, "CompareItem", vec![a.clone(), b.clone()]).0.as_int()
                    } else {
                        stricmp(&item_of(a), &item_of(b))
                    }
                };
                crate::sorting::sort_like_engine_template(&mut elems, |a, b| cmp(self, a, b));
                self.set_prop(this, "Elements", Value::Array(elems));
                Value::Void
            }
            // ---- USaveGameDevice (static natives of GUIController): GUI.dll's UGUIController::exec…
            // (IsSavingMediaAvailable IsWriteUserConfigFinished,
            // IsGetProfileListFinished), which call USaveGameDevice::GetInstance ----
            "IsSavingMediaAvailable" => Value::Bool(self.engine.save.is_some()),
            // execGetMaxNumberOfSavingSlots, execRequestCreateProfile,
            // execRequestUseProfile.
            "GetMaxNumberOfSavingSlots" => Value::Int(if self.engine.save.is_some() { 10 } else { 0 }),
            "RequestCreateProfile" => {
                // USaveGameDevice::RequestCreateProfile: after the operation and the profile
                // name, GConfig->SetString("DefaultPlayer", "Name", name, "User") right away.
                let name = args[0].as_name();
                let ok = self.save_device().is_some_and(|d| d.request_create_profile(&name));
                if ok {
                    if let Some(c) = self.config.as_mut() {
                        c.set_string("DefaultPlayer", "Name", &name, "User");
                    }
                }
                Value::Bool(ok)
            }
            "RequestUseProfile" => Value::Bool(self.save_device().is_some_and(|d| d.request_use_profile(&args[0].as_name()))),
            // UGUIController::execRequestGetProfileList: vtable +0x90.
            "RequestGetProfileList" => Value::Bool(self.save_device().is_some_and(|d| d.request_get_profile_list())),
            // UGUIController::execRequestIsSlotEmpty: vtable +0x9C.
            "RequestIsSlotEmpty" => Value::Bool(self.save_device().is_some_and(|d| d.request_is_slot_empty(args[0].as_int()))),
            // UGUIController::execRequestGetSlotContentDescription: vtable +0xA4.
            "RequestGetSlotContentDescription" => Value::Bool(self.save_device().is_some_and(|d| d.request_get_description(args[0].as_int()))),
            // UGUIController::execRequestGetSlotContentDateAndTime: vtable +0xAC.
            "RequestGetSlotContentDateAndTime" => Value::Bool(self.save_device().is_some_and(|d| d.request_get_date(args[0].as_int()))),
            // execRequestEraseSlot, execRequestReadSlot, execRequestWriteSlot
            "RequestEraseSlot" => Value::Bool(self.save_device().is_some_and(|d| d.request_erase_slot(args[0].as_int()))),
            "RequestReadSlot" => Value::Bool(self.save_device().is_some_and(|d| d.request_read_slot(args[0].as_int()))),
            "RequestWriteSlot" => {
                let desc = if supplied.get(1) == Some(&true) { args[1].as_name() } else { String::new() };
                let last = if supplied.get(2) == Some(&true) { args[2].as_bool() } else { true };
                Value::Bool(self.save_device().is_some_and(|d| d.request_write_slot(args[0].as_int(), &desc, last)))
            }
            // UGUIController::execRequestReadUserConfig: vtable +0xD8.
            "RequestReadUserConfig" => Value::Bool(self.save_device().is_some_and(|d| d.request_read_user_config())),
            // UGUIController::execRequestWriteUserConfig: vtable +0xE0.
            "RequestWriteUserConfig" => Value::Bool(self.save_device().is_some_and(|d| d.request_write_user_config())),
            // execIsCreateProfileFinished, execIsUseProfileFinished,
            // execIsEraseSlotFinished, execIsReadSlotFinished,
            // execIsWriteSlotFinished, execIsReadUserConfigFinished.
            "IsCreateProfileFinished"
            | "IsUseProfileFinished"
            | "IsEraseSlotFinished"
            | "IsReadSlotFinished"
            | "IsWriteSlotFinished"
            | "IsReadUserConfigFinished"
            | "IsWriteUserConfigFinished"
            // execDoesMemoryCardIncludeThisProfile: vtable +0x88.
            | "DoesMemoryCardIncludeThisProfile" => {
                let mut code = args[0].as_int();
                let r = match self.save_device() {
                    Some(d) => match name {
                        "IsCreateProfileFinished" => d.is_create_profile_finished(&mut code),
                        "IsUseProfileFinished" => d.is_use_profile_finished(&mut code),
                        "IsEraseSlotFinished" => d.is_erase_slot_finished(&mut code),
                        "IsReadSlotFinished" => d.is_read_slot_finished(&mut code),
                        "IsWriteSlotFinished" => d.is_write_slot_finished(&mut code),
                        "IsReadUserConfigFinished" => d.is_read_user_config_finished(&mut code),
                        "IsWriteUserConfigFinished" => d.is_write_user_config_finished(&mut code),
                        _ => d.does_memory_card_include_this_profile(&mut code),
                    },
                    None => false,
                };
                args[0] = Value::Int(code);
                Value::Bool(r)
            }
            // execStartNewCheckOnMemoryCardForThisProfile.
            "StartNewCheckOnMemoryCardForThisProfile" => {
                let mut code = args[0].as_int();
                let user = args[1].as_name();
                let r = self.save_device().is_some_and(|d| d.start_new_check(&mut code, &user));
                args[0] = Value::Int(code);
                Value::Bool(r)
            }
            "IsGetProfileListFinished" => {
                let mut code = args[0].as_int();
                let mut list = Vec::new();
                let r = self.save_device().is_some_and(|d| d.is_get_profile_list_finished(&mut code, &mut list));
                args[0] = Value::Int(code);
                if r {
                    args[1] = Value::Array(list.into_iter().map(Value::Str).collect());
                }
                Value::Bool(r)
            }
            // UGUIController::execIsSlotEmptyFinished: vtable +0xA0, outputs passed by address.
            "IsSlotEmptyFinished" => {
                let (mut code, mut empty) = (args[0].as_int(), args[1].as_int());
                let r = self.save_device().is_some_and(|d| d.is_slot_empty_finished(&mut code, &mut empty));
                args[0] = Value::Int(code);
                args[1] = Value::Int(empty);
                Value::Bool(r)
            }
            // UGUIController::execIsGetSlotContentDescriptionFinished: vtable +0xA8.
            "IsGetSlotContentDescriptionFinished" => {
                let mut code = args[0].as_int();
                let mut desc = args[1].as_name();
                let r = self.save_device().is_some_and(|d| d.is_get_description_finished(&mut code, &mut desc));
                args[0] = Value::Int(code);
                args[1] = Value::Str(desc);
                Value::Bool(r)
            }
            // UGUIController::execIsGetSlotContentDateAndTimeFinished: vtable +0xB0.
            "IsGetSlotContentDateAndTimeFinished" => {
                let mut code = args[0].as_int();
                let mut date = (args[1].as_int(), [args[2].as_byte(), args[3].as_byte(), args[4].as_byte(), args[5].as_byte()]);
                let r = self.save_device().is_some_and(|d| d.is_get_date_finished(&mut code, &mut date));
                args[0] = Value::Int(code);
                args[1] = Value::Int(date.0);
                for k in 0..4 {
                    args[2 + k] = Value::Byte(date.1[k]);
                }
                Value::Bool(r)
            }
            // execIsMemoryCardReady: vtable +0xCC, 0 without a device.
            "IsMemoryCardReady" => Value::Bool(self.engine.save.as_ref().is_some_and(|d| d.is_memory_card_ready())),
            // execHasMemoryCardBeenChanged (vtable +0xD0) / execSetMemoryCardActivation
            // (+0xD4): 0 on PC.
            "HasMemoryCardBeenChanged" | "SetMemoryCardActivation" => Value::Bool(false),
            // UGUIController::execIsThereACheckpoint / execLoadAtCheckpoint.
            "IsThereACheckpoint" => Value::Bool(self.engine.save.as_ref().is_some_and(|d| d.is_there_a_checkpoint())),
            "LoadAtCheckpoint" => {
                let from_start = if supplied.first() == Some(&true) { args[0].as_int() } else { 0 };
                let ok = match self.save_device() {
                    Some(d) if d.is_there_a_checkpoint() => d.launch_from_checkpoint(from_start, 3),
                    _ => false,
                };
                Value::Bool(ok)
            }
            // execGetDefaultDescription: USaveGameDevice::GetDefaultDescription
            // (+0x54), description of the map's starting point.
            "GetDefaultDescription" => {
                if let Some(d) = self.engine.save.as_ref() {
                    args[0] = Value::Str(d.start_desc.clone());
                }
                Value::Void
            }
            // execEndOfPressStartPage: Client->Engine+0xF4 (waiting for START) to 0.
            "EndOfPressStartPage" => Value::Bool(true),
            // execGetConnectionStatus: Master->Client->Engine, fields +D8/+DC.
            // ClientState/EngineState are global in the VM. Without a Master or if the status
            // is zero, the out parameter keeps its initial value.
            "GetConnectionStatus" => {
                let status = if self.gof_pub(this, "Master").is_some() { self.ipdrv.connection_status } else { 0 };
                if status != 0 {
                    if let Some(error) = args.first_mut() {
                        *error = Value::Str(self.ipdrv.connection_error.clone());
                    }
                }
                Value::Int(status)
            },
            // execGetXIIIEngineVersion: Printf(" v%d", 145).
            "GetXIIIEngineVersion" => Value::Str(" v145".into()),
            // execGetMainMenuClass: nothing is written into the result.
            "GetMainMenuClass" => Value::Str(String::new()),
            // execRebootToDashboardFromScripts: parameters read, nothing on PC.
            "RebootToDashboardFromScripts" => Value::Void,
            // execResetKeyboard: UObject::ResetConfig of the class of the viewport's Input
            // (Engine.Input section taken from DefUser.ini), bindings read again.
            "ResetKeyboard" => {
                // The backend represents UViewport by the objects of the real producer
                // viewport_object. A remote UPlayer does not pass IsA(UViewport).
                let viewport = self.gof_pub(this, "ViewportOwner");
                let is_viewport = viewport.and_then(|v| {
                    let object = self.objects.get(v)?.as_ref()?;
                    let slot = object.name.strip_prefix("WindowsViewport")?.parse::<u8>().ok()?;
                    (self.find_viewport_object(slot) == Some(v)).then_some(v)
                }).is_some();
                if is_viewport {
                    if let Some(c) = self.config.as_mut() {
                        c.reset_section("Engine.Input", "User", "DefUser.ini");
                    }
                    if let Some(pc) = self.input_sys.as_ref().map(|s| s.pc) {
                        let user = self.config.as_mut().map(|c| c.file_text("User")).unwrap_or_default();
                        let bindings = crate::input::Bindings::parse(&user);
                        self.input_init(pc, &bindings);
                    }
                }
                Value::Void
            }
            _ => return None,
        })
    }

    /// UGUIStyles::Draw: image of the state (normal, stretched or scaled).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn gui_style_draw(&mut self, style: ObjId, c: ObjId, state: u8, l: f32, t: f32, w: f32, h: f32) {
        if state > 5 {
            return;
        }
        let args = vec![Value::Object(Some(c)), Value::Byte(state), Value::Float(l), Value::Float(t), Value::Float(w), Value::Float(h)];
        if self.gui_delegate(style, "OnDraw", args).0.as_bool() {
            return;
        }
        let s = state as usize;
        let img = match self.prop(style, "Images") {
            Value::Array(i) => i.get(s).cloned().unwrap_or(Value::Object(None)),
            _ => Value::Object(None),
        };
        if !img.as_bool() {
            return;
        }
        let grey = if state == 4 { 0x80 } else { 0xff };
        self.set_prop(c, "DrawColor", Value::Struct("object.color".into(), vec![Value::Byte(grey), Value::Byte(grey), Value::Byte(grey), Value::Byte(0xff)].into()));
        let (u, v) = self.material_size(&img);
        let (u, v) = (u as f32, v as f32);
        let byte_at = |vm: &Vm, name: &str| match vm.prop(style, name) {
            Value::Array(a) => a.get(s).map_or(0, |x| x.as_byte()),
            _ => 0,
        };
        let rstyle = byte_at(self, "RStyles");
        self.set_prop(c, "Style", Value::Byte(rstyle));
        if let Value::Array(colors) = self.prop(style, "ImgColors") {
            if let Some(col) = colors.get(s) {
                self.set_prop(c, "DrawColor", col.clone());
            }
        }
        match byte_at(self, "ImgStyle") {
            0 => self.canvas_draw_tile(c, &img, l, t, u, v, 0.0, 0.0, u, v, 0.0, [1.0; 4], [0.0; 4]),
            1 => self.canvas_draw_tile_stretched(c, &img, l, t, w, h),
            2 => self.canvas_draw_tile_scale_bound(c, &img, l, t, w, h),
            _ => {}
        }
    }

    /// UGUIStyles::DrawText: text aligned in the rectangle rounded to the pixel, with
    /// the state's style, colour and font; the Canvas state is restored afterwards.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn gui_style_draw_text(&mut self, style: ObjId, c: ObjId, state: u8, l: f32, t: f32, w: f32, h: f32, align: u8, text: &str) {
        let (old_color, old_style, old_font) = (self.prop(c, "DrawColor"), self.prop(c, "Style"), self.prop(c, "Font"));
        let round = crate::gui::fistp_dword;
        let (l, t, w, h) = (round(l), round(t), round(w), round(h));
        if state > 5 {
            return;
        }
        let s = state as usize;
        let font = match self.prop(style, "Fonts") {
            Value::Array(f) => f.get(s).and_then(|v| v.as_object()).filter(|&f| self.alive(f)),
            _ => None,
        };
        let Some(font) = font else { return };
        let args = vec![
            Value::Object(Some(c)),
            Value::Byte(state),
            Value::Float(l),
            Value::Float(t),
            Value::Float(w),
            Value::Float(h),
            Value::Byte(align),
            Value::Str(text.to_string()),
        ];
        if self.gui_delegate(style, "OnDrawText", args).0.as_bool() {
            return;
        }
        let size_x = self.propr(c, "SizeX").as_int();
        if let Value::Array(r) = self.prop(style, "RStyles") {
            self.set_prop(c, "Style", Value::Byte(r.get(s).map_or(0, |x| x.as_byte())));
        }
        if let Value::Array(colors) = self.prop(style, "FontColors") {
            if let Some(col) = colors.get(s) {
                self.set_prop(c, "DrawColor", col.clone());
            }
        }
        let f = self.gui_event_pub(font, "GetFont", vec![Value::Int(size_x)]);
        self.set_prop(c, "Font", f.clone());
        if f.as_bool() {
            self.canvas_draw_text_justified(c, align, l, t, l + w, t + h, text);
        }
        self.set_prop(c, "Style", old_style);
        self.set_prop(c, "DrawColor", old_color);
        self.set_prop(c, "Font", old_font);
    }
}

/// Item of a GUIListElem.
fn item_of(v: &Value) -> String {
    match v {
        Value::Struct(_, f) => f.first().map(|x| x.as_name()).unwrap_or_default(),
        _ => String::new(),
    }
}

/// appStricmp (MSVCR70's _stricmp, ANSI code page): −1, 0 or 1, sign of the case-insensitive comparison.
pub(crate) fn stricmp(a: &str, b: &str) -> i32 {
    let low = |c: char| {
        let b = if (c as u32) < 256 { c as u32 as u8 } else { b'?' };
        b.to_ascii_lowercase() as i32
    };
    let (mut x, mut y) = (a.chars(), b.chars());
    loop {
        match (x.next(), y.next()) {
            (None, None) => return 0,
            (None, Some(_)) => return -1,
            (Some(_), None) => return 1,
            (Some(p), Some(q)) => {
                let d = low(p) - low(q);
                if d != 0 {
                    return d.signum();
                }
            }
        }
    }
}

