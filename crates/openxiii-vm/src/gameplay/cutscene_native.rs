//! Cutscene natives (XIDCine.dll): parsing of the commands of the Cine2 scripts
//! (GetFirstWord, CharIsNum), actor lookup by name (FindAnActor), driving pawns
//! (Steering, IsTargetReached), rotation on the spot (OnTheSpotRotation) and placing waypoints
//! on the ground (AutoPosition).

use crate::value::{ObjId, Value};
use crate::Vm;

/// ACineController2::GetFirstWord: skips spaces, cuts off the first word and
/// consumes the space after it. Tabs are part of the word.
fn first_word(s: &str) -> (&str, &str) {
    let trimmed = s.trim_start_matches(' ');
    trimmed.split_once(' ').unwrap_or((trimmed, ""))
}

impl Vm {
    pub(crate) fn identify_find_label_package(&mut self, name: &str, data: &[u8], index: usize) {
        let (slot, digest) = if name.eq_ignore_ascii_case("Core") {
            (0, [0x01, 0x14, 0x0f, 0x33, 0xec, 0xe9, 0x63, 0xf8, 0xff, 0xfa, 0xb6, 0x7c, 0xe3, 0x12, 0xff, 0x7c])
        } else if name.eq_ignore_ascii_case("XIDCine") {
            (1, [0x47, 0x2c, 0xbb, 0x19, 0x76, 0xf2, 0x98, 0x8e, 0x1d, 0xda, 0xbc, 0x1d, 0x02, 0xd5, 0xef, 0xeb])
        } else {
            return;
        };
        self.find_label_packages[slot] = (crate::stat_log::Md5::digest(data) == digest).then_some(index);
    }

    fn cutscene_script(&self, f: usize, package: usize, class: &str, name: &str) -> bool {
        let d = &self.functions[f];
        d.package == package && self.classes[d.owner].name == class && d.name == name
            && d.state.is_none() && d.code.is_some() && d.native == 0
            && d.flags & (openxiii_script::objects::func::SINGULAR | openxiii_script::objects::func::NATIVE) == 0
            && crate::world::probe_bit(name).is_none()
    }

    /// The jump and return numbers below come from the game's bytecode.
    /// A modified package, even with the same class and function names, keeps its script.
    pub(crate) fn find_label_original(&self, f: usize) -> bool {
        let d = &self.functions[f];
        let [Some(core), Some(cutscene)] = self.find_label_packages else { return false };
        if !self.cutscene_script(f, cutscene, "CineController2", "FindLabel") || d.params.len() != 1 || d.params[0].out
            {
            return false;
        }
        let Some(word) = self.function_by_path("CineController2.GetFirstWord") else { return false };
        if self.functions[word].owner != d.owner || self.functions[word].package != d.package {
            return false;
        }
        // These Core operators are called by number in the three functions.
        [122, 124, 132, 150, 154, 165].iter().all(|i| self.native_index.get(i).is_some_and(|id| self.functions[*id].package == core))
    }

    pub(crate) fn find_label_direct(&mut self, controller: ObjId, package: usize, label: &Value) -> Option<i32> {
        if self.ipdrv.net_driver.is_some() || self.depth >= 250 || self.trace_steps.is_some() || self.native_counts.is_some()
           
            || !self.alive(controller) {
            return None;
        }
        let label = label.name_ref();
        let pawn = self.propr(controller, "MyPawn").as_object()?;
        if !self.alive(pawn) || self.native_prop(pawn, crate::NativeField::DeleteMe).as_bool() {
            return None;
        }
        // GetAction and GetTabActionLength are virtual: a state or a child class
        // can change their result or have effects on the game.
        for name in ["GetTabActionLength", "GetAction"] {
            let f = self.find_function(pawn, name, false)?;
            if !self.cutscene_script(f, package, "Cine2", name) {
                return None;
            }
        }
        let Value::Int(index) = self.propr(pawn, "CurrentTabActionIndex") else { return None };
        let table = match *index {
            0 | 1 => Some(("tabActions", 2)),
            2 => Some(("tabActions2", 3)),
            3 => Some(("tabActions3", 4)),
            _ => None,
        };
        let mut result = -1;
        // Condition of the for loop (offset 7), once more than the completed iterations.
        let mut jumps = 1u64;
        if let Some((field, get_action_jumps)) = table {
            let Value::Array(lines) = self.propr(pawn, field) else { return None };
            // The script compares i with Length as 32-bit integers.
            i32::try_from(lines.len()).ok()?;
            if lines.iter().any(|line| !matches!(line, Value::Str(_))) {
                return None;
            }
            for (i, line) in lines.iter().enumerate() {
                // GetAction then test of the first word (57).
                jumps += get_action_jumps + 1;
                let (word, rest) = first_word(line.name_ref());
                if word == "'" {
                    jumps += 1; // test of the label (76)
                    if first_word(rest).0.eq_ignore_ascii_case(label) {
                        result = i as i32;
                        break;
                    }
                }
                jumps += 2; // loop back (110), next condition (7)
            }
        }
        // If the fatal jump is reached, only the interpreter knows the offset,
        // the stack and the exact effects at the moment it must report the error.
        let count = u64::from(self.runaway) + jumps;
        if count > 1_000_000 {
            return None;
        }
        self.runaway = count as u32;
        self.cur_offset = if result == -1 { 113 } else { 97 };
        self.last_outputs.clear();
        Some(result)
    }

    pub(crate) fn cutscene_native(&mut self, name: &str, this: ObjId, args: &mut [Value], supplied: &[bool]) -> Option<Value> {
        let given = |i: usize| supplied.get(i) == Some(&true);
        Some(match name {
            "GetFirstWord" => {
                // execGetFirstWord: the word is returned, the string keeps the rest.
                let s = args.first().map(|v| v.as_name()).unwrap_or_default();
                let (word, rest) = first_word(&s);
                if let Some(a) = args.get_mut(0) {
                    *a = Value::Str(rest.to_owned());
                }
                Value::Str(word.to_owned())
            }
            "CharIsNum" => {
                // execCharIsNum: first character between '0' and '9'.
                let s = args.first().map(|v| v.as_name()).unwrap_or_default();
                Value::Bool(s.as_bytes().first().is_some_and(|c| c.is_ascii_digit()))
            }
            "FindAnActor" => Value::Object(self.find_an_actor(this, &args.first().map(|v| v.as_name()).unwrap_or_default())),
            "Steering" => {
                let target = args.first().map(|v| v.as_vector()).unwrap_or([0.0; 3]);
                let detection = args.get(2).map(|v| v.as_float()).unwrap_or(0.0);
                let no_avoid = given(3) && args[3].as_bool();
                let final_location = given(4) && args[4].as_bool();
                self.cutscene_steering(this, target, detection, no_avoid, final_location);
                Value::Void
            }
            "OnTheSpotRotation" => {
                self.on_the_spot_rotation(this);
                Value::Void
            }
            "AutoPosition" => {
                // APositionInfo::execAutoPosition: SingleLineCheck from Location to
                // Location − (0, 0, 1000), TRACE_World (0x86), without extent; Location = Hit.Location
                // (zero without a contact, FCheckResult(1.0)), then Z = fAltitude + Z.
                let location = self.propr(this, "Location").as_vector();
                let end = [location[0], location[1], location[2] - 1000.0];
                let mut hit = crate::physics::CheckResult::new(1.0);
                self.single_line_check(&mut hit, Some(this), end, location, crate::physics::TRACE_WORLD, [0.0; 3]);
                let altitude = self.propr(this, "fAltitude").as_float();
                self.set_prop(this, "Location", Value::vector(hit.location[0], hit.location[1], altitude + hit.location[2]));
                Value::Void
            }
            _ => return None,
        })
    }

    /// USkeletalMeshInstance::EnableChannelNotify.
    fn channel_notify(&mut self, actor: ObjId, ch: i32, on: bool) {
        if let Some(inst) = self.mesh_instance(actor) {
            if inst.validate(ch) {
                inst.channels[ch as usize].notify_ignore = !on;
            }
        }
    }

    /// ACineController2::execFindAnActor: "player" (Player, +0x394), "none" or
    /// empty, "self" (Pawn), a number (PositionInfo<n>), otherwise the actor with this name. FString ==
    /// comparisons (case-insensitive); lookup through StaticFindObject(Class, ANY_PACKAGE, Name)
    /// (no flag test) then Cast<AActor>: a destroyed actor
    /// is still found until it is deleted (CleanupDestroyed), hence the search
    /// in all objects and not only in the level's actor list.
    fn find_an_actor(&self, this: ObjId, name: &str) -> Option<ObjId> {
        let (wanted, class) = match search_actor(name) {
            SearchActor::Player => return self.propr(this, "Player").as_object(),
            SearchActor::NoActor => return None,
            SearchActor::Pawn => return self.propr(this, "Pawn").as_object(),
            SearchActor::Find { class_value, name_value } => (name_value, class_value),
        };
        let class = self.class(class)?;
        // GObjHash puts an object at the head of its bucket's chain (HashObject): with equal names,
        // StaticFindObject returns the most recent one, so the last one created here (slots are not reused).
        let found = (0..self.objects.len()).rev().find(|&a| self.objects[a].as_ref().is_some_and(|o| o.name.eq_ignore_ascii_case(&wanted) && self.is_child_of(o.class, class)));
        self.cutscene_cast_actor(found)
    }

    /// XIDCine's Cast<AActor>: the object if it exists and derives from Actor, otherwise nothing.
    pub(crate) fn cutscene_cast_actor(&self, o: Option<ObjId>) -> Option<ObjId> {
        o.filter(|&o| self.objects.get(o).is_some_and(|x| x.is_some()) && self.is_a(o, "Actor"))
    }

    /// ACineController2::IsTargetReached: within the detection radius, or passing
    /// plane crossed. x87 comparisons: if (dx, dy) is not zero (NaN included), side of the
    /// plane (Py·dy + Px·dx) + Pz·0; reached if it is at most 0 (not for NaN);
    /// otherwise reached only if Detection² > dy² + dx² (false for NaN).
    fn cutscene_target_reached(&self, pawn: ObjId, target: [f32; 3], detection: f32, plane: [f32; 3]) -> bool {
        let loc = self.propr(pawn, "Location").as_vector();
        let (dx, dy) = (target[0] - loc[0], target[1] - loc[1]);
        if dx != 0.0 || dy != 0.0 {
            let side = (plane[1] * dy + plane[0] * dx) + plane[2] * 0.0;
            if side <= 0.0 {
                return true;
            }
        }
        detection * detection > dy * dy + dx * dx
    }

    /// ACineController2::execSteering.
    fn cutscene_steering(&mut self, this: ObjId, target: [f32; 3], detection: f32, no_avoid: bool, final_location: bool) {
        self.cutscene_steering_with(
            this,
            target,
            detection,
            no_avoid,
            final_location,
            |vm, obj, name_value| vm.event_pub(obj, name_value, vec![]),
            |vm, obj, target, accel| {
                // ACineController2::eventAvoidObstacle (XIDCine.dll).
                let f = vm.find_function(obj, "AvoidObstacle", false)?;
                let args = vec![Some(Value::vector(target[0], target[1], target[2])), Some(Value::vector(accel[0], accel[1], accel[2]))];
                match vm.call(f, obj, args) {
                    Ok((_, outs)) => outs.get(1).cloned().flatten().map(|v| v.as_vector()),
                    Err(_) => None,
                }
            },
        );
    }

    /// `event_value`: eventEndOfMove on the controller, APawn::eventChangeAnimation on the pawn; `avoided`: eventAvoidObstacle,
    /// which returns the desired acceleration (None: the script function returned nothing); tests can supply it.
    pub(crate) fn cutscene_steering_with(
        &mut self,
        this: ObjId,
        target: [f32; 3],
        detection: f32,
        no_avoid: bool,
        final_location: bool,
        mut event_value: impl FnMut(&mut Vm, ObjId, &str),
        mut avoided: impl FnMut(&mut Vm, ObjId, [f32; 3], [f32; 3]) -> Option<[f32; 3]>,
    ) {
        let Some(pawn) = self.propr(this, "Pawn").as_object() else { return };
        let ground = self.propr(this, "bConstrainedToGround").as_bool();
        let loc = self.propr(pawn, "Location").as_vector();
        let d0 = [target[0] - loc[0], target[1] - loc[1], if ground { 0.0 } else { target[2] - loc[2] }];
        let plane = self.propr(this, "Plane").as_vector();
        if self.cutscene_target_reached(pawn, target, detection, plane) {
            // ACineController2::eventEndOfMove (XIDCine.dll).
            event_value(self, this, "EndOfMove");
            return;
        }
        let loc = self.propr(pawn, "Location").as_vector();
        let mut dir = [target[0] - loc[0], target[1] - loc[1], if ground { 0.0 } else { target[2] - loc[2] }];
        let mut speed = self.propr(this, "MyPawnGroundSpeed").as_float() * self.propr(this, "wantedspeed").as_float();
        if final_location {
            // Braking when approaching the end point. Distance like FVector::Size: appSqrt in
            // double of (Z² + Y²) + X² (fsqrt, rounded to 24 bits by the game's FPU), stored as a
            // qword at [ebp−0x34] then as a float. The test compares
            // speed² × (1/300) with the LOW 32 bits of this double, read as a float: with a 24-bit
            // mantissa, these are the last 3 bits of the mantissa placed in bits 29 to 31 (sign
            // and top of the exponent), the others at zero. The braking (speed √(300 × dist), at
            // least 100) applies unless speed² / 300 ≤ this value (or NaN): this is the case
            // when these bits are 011 (2^65), about one frame in eight.
            let dist = (((d0[2] * d0[2] + d0[1] * d0[1]) + d0[0] * d0[0]) as f64).sqrt() as f32;
            let low = f32::from_bits((dist as f64).to_bits() as u32);
            if speed * speed * 0.003_333_333_4 > low {
                speed = ((dist * 300.0) as f64).sqrt() as f32;
                if speed <= 100.0 {
                    speed = 100.0;
                }
            }
        }
        self.set_prop(pawn, "GroundSpeed", Value::Float(speed));
        self.set_prop(pawn, "AirSpeed", Value::Float(speed));
        if !self.propr(this, "bCineControlAnims").as_bool() {
            event_value(self, pawn, "ChangeAnimation");
        }
        dir = normalized_cutscene(dir).1;
        let k = self.propr(this, "AccelerationFactor").as_float() * 200.0;
        let mut accel = [dir[0] * k, dir[1] * k, k * dir[2]];
        if !no_avoid {
            if let Some(v) = avoided(self, this, target, accel) {
                accel = v;
            }
        }
        self.set_prop(pawn, "Acceleration", Value::vector(accel[0], accel[1], accel[2]));
    }

    /// ACine2::execOnTheSpotRotation: on-the-spot rotation animation (channels 2 and
    /// 3) set to the rotation speed since the previous frame.
    fn on_the_spot_rotation(&mut self, this: ObjId) {
        rotation_on_place(&mut VmRotation { vm: self, this });
    }
}

/// XIDCine's in-place normalization: (X² + Y²) + Z², normalized by appRSqrt if at least 1e-8 (comparison in double); returns whether it took place.
pub(crate) fn normalized_cutscene(mut dir: [f32; 3]) -> (bool, [f32; 3]) {
    let len2 = (dir[0] * dir[0] + dir[1] * dir[1]) + dir[2] * dir[2];
    if len2 as f64 >= 1e-8 {
        let r = 1.0 / len2.sqrt();
        dir = dir.map(|c| r * c);
        return (true, dir);
    }
    (false, dir)
}

/// What ACineController2::execFindAnActor decides from the received name, before any object lookup.
#[derive(Debug, PartialEq)]
pub(crate) enum SearchActor {
    /// "player": Player (+0x394).
    Player,
    /// "none" or empty name.
    NoActor,
    /// "self": Pawn (+0x1F0).
    Pawn,
    /// StaticFindObject(class, ANY_PACKAGE, name) then Cast<AActor>; a leading digit: PositionInfo<name>.
    Find { class_value: &'static str, name_value: String },
}

pub(crate) fn search_actor(name: &str) -> SearchActor {
    if name.eq_ignore_ascii_case("player") {
        return SearchActor::Player;
    }
    if name.eq_ignore_ascii_case("none") || name.is_empty() {
        return SearchActor::NoActor;
    }
    if name.eq_ignore_ascii_case("self") {
        return SearchActor::Pawn;
    }
    if name.as_bytes()[0].is_ascii_digit() {
        SearchActor::Find { class_value: "PositionInfo", name_value: format!("PositionInfo{name}") }
    } else {
        SearchActor::Find { class_value: "Actor", name_value: name.to_string() }
    }
}

/// What ACine2::execOnTheSpotRotation reads and writes of the actor and its skeletal instance (the Vm, or a test).
pub(crate) trait RotationOnPlace {
    /// Cast<USkeletalMeshInstance> of the mesh instance (+0x6C).
    fn instance_skeletal(&mut self) -> bool;
    /// Bit 0x20 of +0x360 (bInitializeAnimation).
    fn initializes(&self) -> bool;
    fn set_initializes(&mut self);
    fn anim_right(&self) -> String;
    fn anim_left(&self) -> String;
    fn sequence(&self, ch: i32) -> String;
    fn image(&self, ch: i32) -> f32;
    fn yaw(&self) -> i32;
    fn old_yaw(&self) -> i32;
    fn set_old_yaw(&mut self, v: i32);
    fn name_pawn(&self) -> String;
    fn play_anim(&mut self, ch: i32, sequence: &str, rate: f32, tween: f32, looping: bool);
    fn channel_notify(&mut self, ch: i32, on: bool);
    fn set_blend_alpha(&mut self, ch: i32, alpha: f32);
    fn zero_movement_alpha(&mut self, first: i32, last: i32, delta: f32);
    /// Log "TURN RIGHT name speed sequence frame" (Logf of GLog).
    fn record_log(&mut self, name_value: &str, speed_value: f32, sequence: &str, image: f32);
}

pub(crate) fn rotation_on_place<R: RotationOnPlace>(r: &mut R) {
    if !r.instance_skeletal() {
        return;
    }
    if !r.initializes() {
        r.set_initializes();
        let (right, left) = (r.anim_right(), r.anim_left());
        r.play_anim(2, &right, 1.0, 0.1, true);
        r.play_anim(3, &left, 1.0, 0.1, true);
        for ch in 2..14 {
            r.channel_notify(ch, false);
            r.set_blend_alpha(ch, 0.0);
        }
        r.channel_notify(4, true);
    }
    let yaw = r.yaw();
    let old = r.old_yaw();
    let diff = ((yaw.wrapping_sub(old).wrapping_sub(0x8000)) & 0xffff) - 0x8000;
    let speed = diff.wrapping_mul(30);
    if speed as f32 <= 1000.0 {
        r.zero_movement_alpha(2, 3, 0.1);
    } else {
        let right = r.anim_right();
        let current = r.sequence(2);
        if !current.eq_ignore_ascii_case(&right) {
            let rate = speed.wrapping_abs();
            let (speed_value, image, name_value) = (rate as f32 * 0.000_488_281_25, r.image(2), r.name_pawn());
            r.record_log(&name_value, speed_value, &right, image);
            r.play_anim(2, &right, speed_value, 0.1, true);
        }
    }
    r.set_old_yaw(yaw);
}

struct VmRotation<'a> {
    vm: &'a mut Vm,
    this: ObjId,
}

impl RotationOnPlace for VmRotation<'_> {
    fn instance_skeletal(&mut self) -> bool {
        self.vm.mesh_instance(self.this).is_some()
    }
    fn initializes(&self) -> bool {
        self.vm.propr(self.this, "bInitializeAnimation").as_bool()
    }
    fn set_initializes(&mut self) {
        self.vm.set_prop(self.this, "bInitializeAnimation", Value::Bool(true));
    }
    fn anim_right(&self) -> String {
        self.vm.propr(self.this, "TurnRightAnim").as_name()
    }
    fn anim_left(&self) -> String {
        self.vm.propr(self.this, "TurnLeftAnim").as_name()
    }
    fn sequence(&self, ch: i32) -> String {
        self.vm.mesh_insts.get(&self.this).and_then(|i| i.channels.get(ch as usize)).map(|c| c.sequence.clone()).unwrap_or_default()
    }
    fn image(&self, ch: i32) -> f32 {
        self.vm.mesh_insts.get(&self.this).and_then(|i| i.channels.get(ch as usize)).map(|c| c.frame).unwrap_or(0.0)
    }
    fn yaw(&self) -> i32 {
        self.vm.propr(self.this, "Rotation").as_rotator()[1]
    }
    fn old_yaw(&self) -> i32 {
        self.vm.propr(self.this, "MyOldRotYaw").as_int()
    }
    fn set_old_yaw(&mut self, v: i32) {
        self.vm.set_prop(self.this, "MyOldRotYaw", Value::Int(v));
    }
    fn name_pawn(&self) -> String {
        self.vm.propr(self.this, "PawnName").as_name()
    }
    fn play_anim(&mut self, ch: i32, sequence: &str, rate: f32, tween: f32, looping: bool) {
        self.vm.play_anim(self.this, ch, sequence, rate, tween, looping);
    }
    fn channel_notify(&mut self, ch: i32, on: bool) {
        self.vm.channel_notify(self.this, ch, on);
    }
    fn set_blend_alpha(&mut self, ch: i32, alpha: f32) {
        self.vm.set_blend_alpha(self.this, ch, alpha);
    }
    fn zero_movement_alpha(&mut self, first: i32, last: i32, delta: f32) {
        self.vm.zero_movement_alpha(self.this, first, last, delta);
    }
    fn record_log(&mut self, name_value: &str, speed_value: f32, sequence: &str, image: f32) {
        let line_value = format!("TURN RIGHT {name_value} {} {sequence} {}", crate::value::printf_f(speed_value), crate::value::printf_f(image));
        self.vm.log.push(line_value);
    }
}

#[cfg(test)]
mod tests {

}

