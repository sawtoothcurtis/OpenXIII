//! Natives of Engine.dll and Xiii.dll used in single-player that belong to no other module:
//! Actor, PlayerController, LevelInfo, Pawn, Controller, GameInfo, InteractionMaster, BloodFlow,
//! Texture. The ZoneActors and ActorInIterationCategory iterators are in `actor_iter`.
//!
//! The object registry (.int files, UObject::CacheDrivers) is used by
//! GetNextInt, GetNextIntDesc and GetNextSkin.

use crate::physics::CheckResult;
use crate::value::{ObjId, Value};
use crate::Vm;

type V = [f32; 3];

/// TRACE_World.
const TRACE_WORLD: u32 = 0x86;
/// TRACE_World | TRACE_StopAtFirstHit.
const TRACE_WORLD_STOP: u32 = 0x286;

/// Engine state that these natives write outside the script objects.
#[derive(Debug, Clone, Default)]
pub struct SoloState {
    /// appClipboardCopy / appClipboardPaste (Windows clipboard).
    pub clipboard: String,
    /// UViewport::SetViewport on Client->Viewports(0): X (+0x90), Y (+0x94),
    /// width (+0x88), height (+0x8C).
    pub viewport_rect: Option<[i32; 4]>,
    /// GObjDrivers and the language it was read for (GObjCachedLanguage).
    registry: Option<(String, Vec<RegistryObject>)>,
}

/// FRegistryObjectInfo (0x3C bytes): Object, Class, MetaClass, Description, Autodetect.
#[derive(Debug, Clone, Default)]
struct RegistryObject {
    object: String,
    class: String,
    meta_class: String,
    description: String,
}

/// Parse (Core): value that follows `key` (case-insensitive search); in quotes up to the
/// next quote, otherwise up to a space, CR, LF, tab or comma.
fn parse_value(stream: &str, key: &str) -> Option<String> {
    crate::value::parse_fstring(stream, key)
}

/// File name matching a "prefix*suffix" pattern (case-insensitive, like FindFirstFile).
fn wildcard(name: &str, pattern: &str) -> bool {
    let (n, p) = (name.to_ascii_lowercase(), pattern.to_ascii_lowercase());
    match p.split_once('*') {
        Some((pre, post)) => n.len() >= pre.len() + post.len() && n.starts_with(pre) && n.ends_with(post),
        None => n == p,
    }
}

/// Axes of Core's FCoords::Inverse (the one in execInvert; Engine's Inverse
/// does not have the zero determinant test): determinant
/// (((Y.x·Z.y − Z.x·Y.y)·X.z + (Y.z·Z.x − Z.z·Y.x)·X.y) + (Z.z·Y.y − Y.z·Z.y)·X.x), zero taken as 1,
/// then 1 / Det and each cofactor × 1 / Det on the x87 stack (24-bit mantissa, extended
/// exponent: no overflow before the final fstp). The Z component of each axis goes through the
/// processor stack (fstp [esp+0x48]) before the product:
/// rounded to a float, it can become infinite there.
fn core_inverse_axes(x: [f32; 3], y: [f32; 3], z: [f32; 3]) -> ([f32; 3], [f32; 3], [f32; 3]) {
    use crate::natives::x87_24 as e;
    let p = |a: f32, b: f32| e(a as f64 * b as f64);
    let d = |a: f32, b: f32, c: f32, f: f32| e(p(a, b) - p(c, f));
    let mut det = e(e(e(d(y[0], z[1], z[0], y[1]) * x[2] as f64) + e(d(y[2], z[0], z[2], y[0]) * x[1] as f64)) + e(d(z[2], y[1], y[2], z[1]) * x[0] as f64));
    if det == 0.0 {
        det = 1.0;
    }
    let r = e(1.0 / det);
    let s = |v: f64| e(v * r) as f32;
    (
        [s(d(z[2], y[1], y[2], z[1])), s(d(x[2], z[1], z[2], x[1])), s(d(y[2], x[1], y[1], x[2]) as f32 as f64)],
        [s(d(y[2], z[0], z[2], y[0])), s(d(z[2], x[0], x[2], z[0])), s(d(x[2], y[0], y[2], x[0]) as f32 as f64)],
        [s(d(z[1], y[0], y[1], z[0])), s(d(x[1], z[0], x[0], z[1])), s(d(y[1], x[0], x[1], y[0]) as f32 as f64)],
    )
}

impl Vm {
    /// GFileManager->FindFiles(Pattern, Files = 1, Dirs = 0): names of the files in the pattern's folder
    /// (relative to the System folder), in NTFS order.
    fn find_files(&self, pattern: &str) -> Vec<String> {
        let Some(dir) = self.config.as_ref().map(|c| c.dir().to_path_buf()) else { return Vec::new() };
        let cut = pattern.rfind(['\\', '/']).map_or(0, |i| i + 1);
        let (folder, mask) = pattern.split_at(cut);
        let mut out: Vec<String> = std::fs::read_dir(dir.join(folder))
            .map(|rd| {
                rd.filter_map(|e| e.ok())
                    .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
                    .filter_map(|e| e.file_name().into_string().ok())
                    .filter(|n| wildcard(n, mask))
                    .collect()
            })
            .unwrap_or_default();
        out.sort_by_key(|n| n.to_ascii_lowercase());
        out
    }

    /// Values of [Core.System] Paths of the game's ini (GSys->Paths, +0x64).
    fn sys_paths(&mut self) -> Vec<String> {
        self.config.as_mut().and_then(|c| c.multi_find("Core.System", "Paths", "System")).unwrap_or_default()
    }

    /// UObject::CacheDrivers: once per language, [Public] Object= of the
    /// "*.int" files of each folder of GSys->Paths, read from "Name.<language>", otherwise
    /// "Name.int". The ULanguage objects (Core.*) are only used for languages and are not carried over.
    fn registry_objects(&mut self) -> Vec<RegistryObject> {
        let Some(lang) = self.config.as_ref().map(|c| c.language.clone()) else { return Vec::new() };
        if let Some((l, list)) = &self.solo.registry {
            if l.eq_ignore_ascii_case(&lang) {
                return list.clone();
            }
        }
        let mut list = Vec::new();
        for path in self.sys_paths() {
            let Some(star) = path.find("*.") else { continue };
            let folder = &path[..star];
            for file in self.find_files(&format!("{folder}*.int")) {
                let base = &file[..file.len() - 4];
                let config = self.config.as_mut().unwrap();
                let mut objects = config.multi_find("Public", "Object", &format!("{folder}{base}.{lang}"));
                if objects.is_none() {
                    objects = config.multi_find("Public", "Object", &format!("{folder}{base}.int"));
                }
                for v in objects.unwrap_or_default() {
                    let mut v = v.as_str();
                    if let Some(s) = v.strip_prefix('(') {
                        v = s;
                    }
                    if let Some(s) = v.strip_suffix(')') {
                        v = s;
                    }
                    list.push(RegistryObject {
                        object: parse_value(v, "Name=").unwrap_or_default(),
                        class: parse_value(v, "Class=").unwrap_or_default(),
                        meta_class: parse_value(v, "MetaClass=").unwrap_or_default(),
                        description: parse_value(v, "Description=").unwrap_or_default(),
                    });
                }
            }
        }
        self.solo.registry = Some((lang, list.clone()));
        list
    }

    /// UObject::GetRegistryObjects: entries whose Class is the name of class
    /// `class` and MetaClass the path of `meta` (empty without MetaClass).
    fn registry_of(&mut self, class: &str, meta: &str) -> Vec<RegistryObject> {
        self.registry_objects().into_iter().filter(|r| r.class.eq_ignore_ascii_case(class) && r.meta_class.eq_ignore_ascii_case(meta)).collect()
    }

    /// StaticFindObjectChecked(UClass, ANY_PACKAGE, Name) then GetPathName; the engine stops
    /// on a class that cannot be found, here the warning is recorded and the list is empty.
    fn registry_meta(&mut self, name: &str) -> Option<Vec<RegistryObject>> {
        match self.class(name) {
            Some(c) => {
                let path = self.class_path(c);
                Some(self.registry_of("Class", &path))
            }
            None => {
                self.warn(format!("Failed to find object 'Class {name}'"));
                None
            }
        }
    }

    /// APawn::SetWalking: bIsWalking, then the ChangeAnimation event if it changes.
    pub(crate) fn set_walking(&mut self, pawn: ObjId, walking: bool) {
        self.set_walking_raw(pawn, i32::from(walking));
    }

    // APawn::SetWalking compares the full UBool DWORD, then stores only bit zero.
    fn set_walking_raw(&mut self, pawn: ObjId, walking: i32) {
        if i32::from(self.propr(pawn, "bIsWalking").as_bool()) != walking {
            self.set_prop(pawn, "bIsWalking", Value::Bool((walking & 1) != 0));
            self.event_pub(pawn, "ChangeAnimation", vec![]);
        }
    }

    /// APlayerController::HandleWalking: the pawn walks if bRun or bDuck, except in
    /// a WarpZoneInfo.
    pub(crate) fn handle_walking(&mut self, pc: ObjId) {
        let Some(pawn) = self.propr(pc, "Pawn").as_object() else { return };
        let zone = match self.prop(pc, "Region") {
            Value::Struct(_, f) => f.first().and_then(|z| z.as_object()),
            _ => None,
        };
        let walking = (self.propr(pc, "bRun").as_int() != 0 || self.propr(pc, "bDuck").as_int() != 0) && !zone.is_some_and(|z| self.is_a(z, "WarpZoneInfo"));
        self.set_walking(pawn, walking);
    }

    fn trace_time(&self, source: ObjId, end: V, start: V, flags: u32, extent: V) -> CheckResult {
        let mut hit = CheckResult::new(1.0);
        self.single_line_check(&mut hit, Some(source), end, start, flags, extent);
        hit
    }

    /// APlayerController::execFindStairRotation: pitch of the view on
    /// stairs, from three traces forward and downward.
    fn find_stair_rotation(&mut self, pc: ObjId, dt: f32) -> i32 {
        let mut rot = self.propr(pc, "Rotation").as_rotator();
        let pawn = self.propr(pc, "Pawn").as_object();
        let Some(pawn) = pawn.filter(|_| (dt as f64) <= 0.33) else { return rot[0] };
        if rot[0] > 0x8000 {
            rot[0] = (rot[0] & 0xffff) - 0x10000;
            self.set_prop(pc, "Rotation", Value::rotator(rot[0], rot[1], rot[2]));
        }
        let pitch = rot[0];
        let dir = crate::ai_natives::axes([0, rot[1], rot[2]])[0];
        let loc = self.propr(pawn, "Location").as_vector();
        let base_eye = self.propr(pawn, "BaseEyeHeight").as_float();
        let eye = [loc[0], loc[1], base_eye + loc[2]];
        let radius = self.propr(pawn, "CollisionRadius").as_float();
        let height = base_eye + self.propr(pawn, "CollisionHeight").as_float();
        let slice = [radius, radius, 1.0];
        let along = |k: f32| [dir[0] * k + eye[0], dir[1] * k + eye[1], dir[2] * k + eye[2]];
        let down_from = |s: V, down: f32| [s[0], s[1], s[2] - down];
        let h2 = height + height;
        let hit = self.trace_time(pc, along(h2), eye, TRACE_WORLD, slice);
        let dist = hit.time * height + hit.time * height;
        let mut stair = 0;
        if dist > crate::ai_move::fmul_d(height, 0.8) {
            let spot = along(dist * 0.5);
            let down = height * 3.0;
            let hit = self.trace_time(pc, down_from(spot, down), spot, TRACE_WORLD, slice);
            if hit.time < 1.0 {
                let first = down * hit.time;
                let seven = height * 0.7f32;
                if seven - 6.0 > first {
                    let spot = along(dist);
                    let hit = self.trace_time(pc, down_from(spot, down), spot, TRACE_WORLD, slice);
                    stair = pitch.max(0);
                    if first - 10.0 > down * hit.time {
                        stair = 3600;
                    }
                } else if first > seven + 6.0 {
                    let k = crate::ai_move::fmul_d(dist, 0.9);
                    let far = [dir[0] * k + loc[0], dir[1] * k + loc[1], dir[2] * k + loc[2]];
                    if self.trace_time(pc, far, loc, TRACE_WORLD_STOP, [0.0; 3]).time == 1.0 {
                        let spot = along(dist);
                        let hit = self.trace_time(pc, down_from(spot, down), spot, TRACE_WORLD, slice);
                        stair = pitch.min(0);
                        if first + 10.0 < down * hit.time {
                            stair = -4000;
                        }
                    }
                }
            }
        }
        let diff = (pitch - stair).abs();
        let now = self.level.map(|l| self.propr(l, "TimeSeconds").as_float()).unwrap_or(0.0);
        let since = now - self.propr(pc, "GroundPitchTime").as_float();
        if diff > 0 && (since as f64) > 0.25 {
            let mut rate = if diff < 1000 { (4000 / diff) as f32 } else { 4.0 };
            rate *= dt;
            // Min(1, RotRate × dt): 1 unless 1 > RotRate × dt.
            if rate >= 1.0 || rate.is_nan() {
                rate = 1.0;
            }
            return ((1.0 - rate) * pitch as f32 + stair as f32 * rate).round_ties_even() as i32;
        }
        if diff < 10 && stair < 10 {
            self.set_prop(pc, "GroundPitchTime", Value::Float(now));
        }
        pitch
    }

    /// AController::execPickTarget: the pawn most in the firing axis among the
    /// controllers (except the team in a team game), seen from the pawn's eyes; beyond 4000 units
    /// only if the player zooms (FovAngle ≠ DefaultFOV). Otherwise, the first visible in
    /// the horizontal angle, without changing bestAim.
    fn pick_target(&mut self, c: ObjId, args: &mut [Value]) -> Value {
        let (mut best_aim, mut best_dist) = (args[0].as_float(), args[1].as_float());
        let (fire, start) = (args[2].as_vector(), args[3].as_vector());
        let trace = args.get(4).map_or(0, |v| v.as_int()) as u32 | TRACE_WORLD_STOP;
        let vertical = best_aim * 3.0 - 2.0;
        let mut pick = None;
        let is_pc = self.is_a(c, "PlayerController");
        let my_pri = self.propr(c, "PlayerReplicationInfo").as_object();
        let team_game = self.propr(c, "Level").as_object().and_then(|l| self.propr(l, "Game").as_object()).is_some_and(|g| self.propr(g, "bTeamGame").as_bool());
        let mut next = self.level.and_then(|l| self.propr(l, "ControllerList").as_object());
        let mut guard = 0;
        while let Some(n) = next {
            guard += 1;
            if guard > 100_000 {
                break;
            }
            next = self.native_prop(n, crate::NativeField::NextController).as_object();
            let Some(p) = self.propr(n, "Pawn").as_object().filter(|_| n != c) else { continue };
            if self.propr(p, "Health").as_int() <= 0 || !self.propr(p, "bProjTarget").as_bool() {
                continue;
            }
            let other_pri = self.propr(n, "PlayerReplicationInfo").as_object();
            if let (Some(a), Some(b)) = (my_pri, other_pri) {
                if team_game && self.propr(a, "Team").as_object() == self.propr(b, "Team").as_object() {
                    continue;
                }
            }
            let l = self.propr(p, "Location").as_vector();
            let d = [l[0] - start[0], l[1] - start[1], l[2] - start[2]];
            let aim = (fire[2] * d[2] + fire[1] * d[1]) + d[0] * fire[0];
            let mut flat = [fire[0], fire[1], 0.0];
            let sq = (flat[0] * flat[0] + flat[1] * flat[1]) + flat[2] * flat[2];
            if (sq as f64) >= 1e-8 {
                let k = crate::natives::app_rsqrt(sq);
                flat = flat.map(|x| k * x);
            }
            let horiz = (flat[1] * d[1] + flat[0] * d[0]) + d[2] * flat[2];
            let dist_sq = (d[2] * d[2] + d[1] * d[1]) + d[0] * d[0];
            let zoomed = is_pc && self.propr(c, "FovAngle").as_float() != self.propr(c, "DefaultFOV").as_float();
            let candidate = aim > 0.0 && (dist_sq < 1.6e7 || zoomed);
            if !candidate {
                continue;
            }
            let dist = (dist_sq as f64).sqrt() as f32;
            let inv = 1.0 / dist;
            let aim = inv * aim;
            let mine = self.propr(c, "Pawn").as_object();
            let eye = |vm: &Vm, a: Option<ObjId>| {
                a.map_or([0.0; 3], |a| {
                    let l = vm.propr(a, "Location").as_vector();
                    [l[0], l[1], vm.propr(a, "EyeHeight").as_float() + l[2]]
                })
            };
            if aim > best_aim {
                if self.trace_time(c, eye(self, Some(p)), eye(self, mine), trace, [0.0; 3]).actor.is_none() {
                    best_aim = aim;
                    pick = Some(p);
                    best_dist = dist;
                }
            } else if pick.is_none()
                && inv * horiz > best_aim
                && aim > vertical
                && self.trace_time(c, eye(self, Some(p)), eye(self, mine), trace, [0.0; 3]).actor.is_none()
            {
                pick = Some(p);
                best_dist = dist;
            }
        }
        args[0] = Value::Float(best_aim);
        args[1] = Value::Float(best_dist);
        Value::Object(pick)
    }

    /// AController::execPickAnyTarget: non-pawn bProjTarget actor of XLevel->Actors
    /// the most in the axis within 2000 units, in sight (LineOfSightTo).
    fn pick_any_target(&mut self, c: ObjId, args: &mut [Value]) -> Value {
        self.pick_any_target_with(c, args, |vm, a| vm.line_of_sight_to(c, Some(a), false))
    }

    /// `pick_any_target` with the LineOfSightTo answer given by `los`.
    fn pick_any_target_with(&mut self, c: ObjId, args: &mut [Value], mut los: impl FnMut(&mut Vm, ObjId) -> bool) -> Value {
        let _ = c;
        let (mut best_aim, mut best_dist) = (args[0].as_float(), args[1].as_float());
        let (fire, start) = (args[2].as_vector(), args[3].as_vector());
        let mut pick = None;
        let mut i = 0;
        while let Some(&a) = self.actors.get(i) {
            i += 1;
            if self.objects.get(a).is_none_or(|o| o.is_none()) || !self.propr(a, "bProjTarget").as_bool() || self.is_a(a, "Pawn") {
                continue;
            }
            let l = self.propr(a, "Location").as_vector();
            let d = [l[0] - start[0], l[1] - start[1], l[2] - start[2]];
            let aim = (fire[2] * d[2] + fire[1] * d[1]) + d[0] * fire[0];
            let dist_sq = (d[2] * d[2] + d[1] * d[1]) + d[0] * d[0];
            let candidate = aim > 0.0 && dist_sq < 4e6;
            if !candidate {
                continue;
            }
            let dist = (dist_sq as f64).sqrt() as f32;
            let aim = aim / dist;
            if aim > best_aim && los(self, a) {
                best_aim = aim;
                pick = Some(a);
                best_dist = dist;
            }
        }
        args[0] = Value::Float(best_aim);
        args[1] = Value::Float(best_dist);
        Value::Object(pick)
    }

    /// AActor::execGetMapName: maps "NameEnding*.unr" of the folders of
    /// GSys->Paths that contain "*.unr" (without duplicates); the one after MapName in Dir
    /// (wrapping around), the first one if MapName is not there.
    fn get_map_name(&mut self, ending: &str, map: &str, dir: i32) -> String {
        let ext = self.config.as_mut().and_then(|c| c.get_string("URL", "MapExt", "System")).unwrap_or_else(|| "unr".into());
        let mask = format!("*.{ext}");
        let mut maps: Vec<String> = Vec::new();
        for path in self.sys_paths() {
            let Some(at) = path.find(&mask) else { continue };
            for f in self.find_files(&format!("{}{ending}{mask}", &path[..at])) {
                if !maps.iter().any(|m| m.eq_ignore_ascii_case(&f)) {
                    maps.push(f);
                }
            }
        }
        for m in maps.iter_mut() {
            while let Some(p) = m.find('\\') {
                *m = m[p + 1..].to_string();
            }
        }
        match maps.iter().position(|m| m == map) {
            Some(i) => {
                let mut i = i as i32 + dir;
                if i < 0 {
                    i = maps.len() as i32 - 1;
                } else if i >= maps.len() as i32 {
                    i = 0;
                }
                maps[i as usize].clone()
            }
            None => maps.first().cloned().unwrap_or_default(),
        }
    }

    /// AActor::execGetNextSkin: registry textures (Class=Texture) whose name
    /// starts with Prefix; the one after (Dir = 1) or before (-1) CurrentSkin, or itself
    /// (0); wrapping around the matches, the last one if CurrentSkin is not there.
    fn get_next_skin(&mut self, prefix: &str, current: &str, dir: i32) -> (String, String) {
        let list = self.registry_of("Texture", "");
        let prefix = prefix.to_ascii_lowercase();
        let (mut first, mut last, mut found) = (-1i32, -1i32, false);
        let mut pick = -1i32;
        for (i, r) in list.iter().enumerate() {
            let i = i as i32;
            if !r.object.to_ascii_lowercase().starts_with(&prefix) {
                continue;
            }
            if found {
                pick = i;
                break;
            }
            if first == -1 {
                first = i;
            }
            if r.object.eq_ignore_ascii_case(current) {
                if dir == -1 {
                    pick = last;
                    break;
                }
                if dir == 0 {
                    pick = i;
                    break;
                }
                found = true;
            }
            last = i;
        }
        if pick == -1 && found {
            pick = first;
        }
        if pick == -1 {
            pick = last;
        }
        match usize::try_from(pick).ok().and_then(|p| list.get(p)) {
            Some(r) => (r.object.clone(), r.description.clone()),
            None => (String::new(), String::new()),
        }
    }

    /// Path of an object referenced by a package (like mesh_info for the default animation).
    fn package_ref_path(loaded: &openxiii_package::library::LoadedPackage, r: i32) -> Option<String> {
        use openxiii_package::ObjectRef;
        match ObjectRef::from(r) {
            ObjectRef::None => None,
            ObjectRef::Export(_) => Some(format!("{}.{}", loaded.name, loaded.package.object_path(r.into()))),
            _ => Some(loaded.package.object_path(r.into())),
        }
    }

    /// AActor::GetSkin: Skins(Index), nothing outside the array; an empty slot also counts as nothing (the caller,
    /// USkeletalMeshInstance::GetTexture, then moves on to the mesh's materials).
    pub(crate) fn actor_skin(skins: &[Value], index: i32) -> Option<Value> {
        let none = |v: &Value| matches!(v, Value::Object(None) | Value::Void);
        usize::try_from(index).ok().and_then(|i| skins.get(i)).filter(|s| !none(s)).cloned()
    }

    /// Choice of USkeletalMeshInstance::GetTexture (`GetTexture(index, actor)`): with an actor, its skin
    /// `index` (AActor::GetSkin) if it is set; otherwise the mesh's material `index` if it exists (even empty);
    /// otherwise, with an actor that has skins, its skin 0 (even empty); otherwise the mesh's material 0. A negative
    /// index is not modelled (the original then reads before the array).
    pub fn texture_source(actor: bool, n_skins: usize, skin_set: impl Fn(usize) -> bool, n_materials: usize, index: i32) -> TextureSource {
        let i = usize::try_from(index).ok();
        if actor {
            if let Some(i) = i.filter(|&i| i < n_skins && skin_set(i)) {
                return TextureSource::Skin(i);
            }
        }
        if let Some(i) = i.filter(|&i| i < n_materials) {
            return TextureSource::Material(i);
        }
        if actor && n_skins > 0 {
            return TextureSource::Skin(0);
        }
        if n_materials > 0 {
            return TextureSource::Material(0);
        }
        TextureSource::Nothing
    }

    /// AActor::execGetMaterial → USkeletalMeshInstance::GetTexture:
    /// Skins(Index) of the actor (AActor::GetSkin) if it is set, otherwise Materials(Index)
    /// of the mesh (ULodMesh +0x74), otherwise Skins(0) if there are Skins, otherwise Materials(0).
    fn get_material(&mut self, a: ObjId, index: i32) -> Value {
        if !self.instance_skeletal_presents(a) {
            return Value::Object(None);
        }
        let Value::Asset(mesh) = self.prop(a, "Mesh") else { return Value::Object(None) };
        let skins = match self.prop(a, "Skins") {
            Value::Array(items) => items,
            _ => Vec::new(),
        };
        // Mods build only: the textures of a mod's animated model.
        #[cfg(feature = "mods")]
        let own = openxiii_assets::mod_skeletal::get(&mesh).map(|m| m.textures.iter().map(|t| (!t.is_empty()).then(|| t.clone())).collect());
        #[cfg(not(feature = "mods"))]
        let own: Option<Vec<Option<String>>> = None;
        let materials: Vec<Option<String>> = own.or_else(|| self
            .library
            .as_mut()
            .and_then(|lib| {
                let (loaded, i) = lib.resolve(&mesh)?;
                let m = openxiii_assets::skeletal::SkeletalMesh::read(&loaded.package, &loaded.data, i).ok()?;
                Some(m.textures.iter().map(|&t| Self::package_ref_path(loaded, t)).collect())
            }))
            .unwrap_or_default();
        let as_value = |m: &Option<String>| m.clone().map_or(Value::Object(None), Value::Asset);
        match Self::texture_source(true, skins.len(), |i| Self::actor_skin(&skins, i as i32).is_some(), materials.len(), index) {
            TextureSource::Skin(i) => skins[i].clone(),
            TextureSource::Material(i) => as_value(&materials[i]),
            TextureSource::Nothing => Value::Object(None),
        }
    }

    /// The actor has a mesh whose instance (+0x6C) exists (MeshGetInstance).
    pub(crate) fn instance_skeletal_presents(&mut self, a: ObjId) -> bool {
        self.mesh_instance(a).is_some()
    }

    /// AActor::execGetRenderBoundingSphere: sphere of GetPrimitive (the mesh,
    /// otherwise the StaticMesh: BoundingSphere +0x4C; otherwise the cylinder, UPrimitive::
    /// GetRenderBoundingSphere: zero centre, radius √(R² + H²)) passed through MeshToWorld
    /// (DT_Mesh) or LocalToWorld, FSphere::TransformBy.
    pub(crate) fn render_bounding_sphere(&mut self, a: ObjId) -> [f32; 4] {
        let matrix = self.dyn_init_matrix(a);
        self.render_sphere_with_matrix(a, &matrix)
    }

    fn asset_render_sphere(&mut self, a: ObjId) -> Option<[f32; 4]> {
        let revision = self.library.as_ref().map(|library| library.revision());
        if self.sphere_library_revision != revision {
            self.primitive_spheres.clear();
            self.sphere_library_revision = revision;
        }
        let path = match self.native_prop(a, crate::NativeField::Mesh) {
            Value::Asset(p) => p,
            _ => match self.native_prop(a, crate::NativeField::StaticMesh) {
                Value::Asset(p) => p,
                _ => return None,
            },
        };
        if let Some(&s) = self.primitive_spheres.get(path) { return Some(s); }
        let path = path.clone();
        let sphere = self.library.as_mut().and_then(|library| {
            let (loaded, i) = library.resolve(&path)?;
            openxiii_assets::skeletal_collision::read_primitive_sphere(&loaded.package, &loaded.data, i).ok()
        }).unwrap_or([0.; 4]);
        self.primitive_spheres.insert(path, sphere);
        Some(sphere)
    }

    pub(crate) fn render_sphere_with_matrix(&mut self, a: ObjId, m: &openxiii_assets::coords::Matrix) -> [f32; 4] {
        let local = self.asset_render_sphere(a).unwrap_or_else(|| {
            let r = self.native_prop(a, crate::NativeField::CollisionRadius).as_float();
            let h = self.native_prop(a, crate::NativeField::CollisionHeight).as_float();
            [0.0, 0.0, 0.0, ((r * r + h * h) as f64).sqrt() as f32]
        });
        let c = openxiii_assets::coords::transform_fvector(m, [local[0], local[1], local[2]]);
        // Row norms (Z² + Y²) + X² (FSphere::TransformBy).
        let row = |i: usize| m[i + 2] * m[i + 2] + m[i + 1] * m[i + 1] + m[i] * m[i];
        let (r0, r1, r2) = (row(0), row(4), row(8));
        let mut s = if r1 < r2 { r2 } else { r1 };
        if s <= r0 {
            s = r0;
        }
        [c[0], c[1], c[2], ((s as f64).sqrt() as f32) * local[3]]
    }

    /// Camera of the UInteraction natives: Viewport->Actor of the ViewportOwner (otherwise of the first
    /// viewport), CameraLocation and CameraRotation (the player's Location and Rotation by default)
    /// passed to PlayerCalcView, then FCameraSceneNode with its FovAngle.
    fn interaction_scene(&mut self, this: ObjId, args: &[Value], supplied: &[bool]) -> Option<(Option<ObjId>, V, [i32; 3], crate::visibility::SceneNode)> {
        let pc =
            self.propr(this, "ViewportOwner").as_object().and_then(|p| self.propr(p, "Actor").as_object()).or_else(|| self.input_sys.as_ref().map(|s| s.pc))?;
        // Split screen (co-op, split-screen multiplayer): the players share the first viewport's
        // console; a HUD drawn in another view projects with that view's player (the grapple's
        // hook points, the co-op hint frame HudCartoonFocus.DrawWnd, a speaker's arrow in the
        // second player's view).
        let pc = self.split_view_controller().unwrap_or(pc);
        let given = |i: usize| supplied.get(i) == Some(&true) && !matches!(args.get(i), None | Some(Value::Void));
        let mut cam = if given(1) { args[1].as_vector() } else { self.propr(pc, "Location").as_vector() };
        let mut rot = if given(2) { args[2].as_rotator() } else { self.propr(pc, "Rotation").as_rotator() };
        let mut view = None;
        let forced: Option<(Option<ObjId>, V, [i32; 3])> = None;
        if let Some((v, c, r)) = forced {
            view = v;
            cam = c;
            rot = r;
        } else if let Some(f) = self.find_function(pc, "PlayerCalcView", false) {
            if let Ok((_, outs)) =
                self.call(f, pc, vec![Some(Value::Object(None)), Some(Value::vector(cam[0], cam[1], cam[2])), Some(Value::rotator(rot[0], rot[1], rot[2]))])
            {
                view = outs.first().cloned().flatten().and_then(|v| v.as_object());
                if let Some(Some(v)) = outs.get(1) {
                    cam = v.as_vector();
                }
                if let Some(Some(r)) = outs.get(2) {
                    rot = r.as_rotator();
                }
            }
        }
        let fov = self.propr(pc, "FovAngle").as_float();
        let size = (self.viewport.0.max(1) as u32, self.viewport.1.max(1) as u32);
        // Local split screen: the angle the player's picture is drawn with (client.rs).
        let fov = {
            let [_, _, w, h] = self.viewport_rect_of(pc, self.client_size);
            let (w, h) = if w > 0 && h > 0 { (w as u32, h as u32) } else { size };
            self.split_view_angle(Some(pc), fov, w, h)
        };
        let node = self.camera_scene_view(Some(pc), view, cam, rot, fov, size);
        Some((view, cam, rot, node))
    }

    /// FCanvasUtil::vInit: CanvasToScreen = translation (−SizeX × 0.5 − 0.5,
    /// −SizeY × 0.5 − 0.5) × scale (2 / SizeX, −2 / SizeY, 1); ScreenToCanvas, its inverse.
    fn canvas_to_screen(&self) -> openxiii_assets::coords::Matrix {
        let (sx, sy) = self.viewport;
        let mut t = [0.0f32; 16];
        t[0] = 1.0;
        t[5] = 1.0;
        t[10] = 1.0;
        t[15] = 1.0;
        t[12] = (-sx) as f32 * 0.5 - 0.5;
        t[13] = (-sy) as f32 * 0.5 - 0.5;
        let mut s = [0.0f32; 16];
        s[0] = 2.0 / sx as f32;
        s[5] = -2.0 / sy as f32;
        s[10] = 1.0;
        s[15] = 1.0;
        openxiii_assets::coords::matrix_mul(&t, &s)
    }

    /// UInteraction::execWorldToScreen: (0, 0, 0) without a ViewActor; otherwise
    /// Project, then ScreenToCanvas (viewport pixels, Z of the projection).
    fn world_to_screen(&mut self, this: ObjId, args: &[Value], supplied: &[bool]) -> V {
        // Lockstep co-op: the primitives drawn so far belong to the previous projected point.
        self.hud_anchor_close();
        let Some((Some(_), cam, rot, node)) = self.interaction_scene(this, args, supplied) else { return [0.0; 3] };
        let p = node.project(args[0].as_vector());
        let m = crate::visibility::SceneNode::matrix_inverse(&self.canvas_to_screen());
        let screen = openxiii_assets::coords::transform_fvector(&m, [p[0], p[1], p[2]]);
        self.hud_anchor_open(args[0].as_vector(), (cam, rot), &node, m, screen);
        screen
    }

    /// UInteraction::execScreenToWorld: viewport point through CanvasToScreen,
    /// Deproject with W = 5, point − camera. The engine calls FVector::SafeNormal (result through a hidden pointer) but
    /// returns the non-normalized vector (Result = [ebp − 0x1C…], not SafeNormal's result).
    fn screen_to_world(&mut self, this: ObjId, args: &[Value], supplied: &[bool]) -> V {
        let Some((_, cam, _, node)) = self.interaction_scene(this, args, supplied) else { return [0.0; 3] };
        let s = openxiii_assets::coords::transform_fvector(&self.canvas_to_screen(), args[0].as_vector());
        let w = node.deproject_plane([s[0], s[1], s[2], 5.0]);
        [w[0] - cam[0], w[1] - cam[1], w[2] - cam[2]]
    }

    /// AActor::execGetCacheEntry: Num-th pair with a non-empty value in the
    /// [Cache] section of "CachePath\cache.ini".
    fn get_cache_entry(&mut self, num: i32) -> Option<(String, String)> {
        let config = self.config.as_mut()?;
        let cache = config.get_string("Core.System", "CachePath", "System").unwrap_or_default();
        let pairs = config.section_pairs("Cache", &format!("{cache}\\cache.ini"));
        pairs.into_iter().filter(|(_, v)| !v.is_empty()).nth(usize::try_from(num).ok()?)
    }

    pub(crate) fn solo_native(&mut self, name: &str, this: ObjId, args: &mut [Value], supplied: &[bool]) -> crate::Result<Option<Value>> {
        let given = |i: usize| supplied.get(i) == Some(&true);
        Ok(Some(match name {
            // AActor::execError: message in the log (FFrame::Log), then
            // XLevel->DestroyActor(this).
            "Error" if self.is_a(this, "Actor") => {
                let msg = args.first().map(|v| v.to_string()).unwrap_or_default();
                let at = self.current_function();
                self.warn(format!("{} ({at}) {msg}", self.obj_label(this)));
                self.destroy_actor(this)?;
                Value::Void
            }
            // AActor::execCheckWasVisible: LastRenderTime - Level.TimeSeconds >= -Time.
            "CheckWasVisible" => {
                let level = self.propr(this, "Level").as_object();
                let now = level.map_or(0.0, |l| self.propr(l, "TimeSeconds").as_float());
                Value::Bool(self.propr(this, "LastRenderTime").as_float() - now >= -args[0].as_float())
            }
            // AActor::execSetSoundMode: Engine->Audio->SetSoundMode, empty in
            // HXAudio.dll (UHXAUDIOSubsystem::SetSoundMode).
            "SetSoundMode" => Value::Void,
            // UForceFeedbackController::execStartEffect: passed to
            // UForceFeedbackManager::Instance (vtable +0x74) if it exists; XIII.ini has no
            // ForceFeedbackManager, so nothing on the original PC version (see force_feedback.rs).
            "StartEffect" if self.is_a(this, "ForceFeedbackController") => {
                let fl = |i: usize| args.get(i).map_or(0.0, |v| v.as_float());
                let effect_value = args.first().map_or(0, |v| v.as_int());
                let client = self.coop_feedback_client(this);
                if let Some(m) = self.force_feedback.as_mut() {
                    m.start_effect(client, effect_value, fl(1), fl(2), [fl(3), fl(4), fl(5), fl(6), fl(7)]);
                }
                Value::Void
            }
            // UObject::execInvert: axes of FCoords(X, Y, Z).Inverse(),
            // returned in X, Y and Z.
            "Invert" if args.len() >= 3 => {
                let (x, y, z) = core_inverse_axes(args[0].as_vector(), args[1].as_vector(), args[2].as_vector());
                args[0] = Value::vector(x[0], x[1], x[2]);
                args[1] = Value::vector(y[0], y[1], y[2]);
                args[2] = Value::vector(z[0], z[1], z[2]);
                Value::Void
            }
            // AWarpZoneInfo::execWarp and execUnWarp: Loc through
            // TransformPointBy, Vel through TransformVectorBy and
            // R = OrthoRotation(GMath.UnitCoords / R * C),
            // C being WarpCoords.Transpose() for Warp, WarpCoords for UnWarp.
            "Warp" | "UnWarp" if args.len() >= 3 && self.is_a(this, "WarpZoneInfo") => {
                let f = match self.prop(this, "WarpCoords") {
                    Value::Struct(_, f) => f,
                    _ => Default::default(),
                };
                let axis =|i: usize| f.get(i).map_or([0.0; 3], |v| v.as_vector());
                let warp = openxiii_assets::coords::Coords { origin: axis(0), x: axis(1), y: axis(2), z: axis(3) };
                let (loc, vel, rot) = crate::volumes::warp_core(&warp, name == "UnWarp", args[0].as_vector(), args[1].as_vector(), args[2].as_rotator());
                args[0] = Value::vector(loc[0], loc[1], loc[2]);
                args[1] = Value::vector(vel[0], vel[1], vel[2]);
                args[2] = Value::rotator(rot[0], rot[1], rot[2]);
                Value::Void
            }
            // AActor::execPSX2BootstrapDemoGetGamePlayTimeout: 240.
            "PSX2BootstrapDemoGetGamePlayTimeout" => Value::Float(240.0),
            // AActor::execResetInputs: UInput::ResetInput of each viewport.
            "ResetInputs" => {
                self.reset_all_inputs();
                Value::Void
            }
            // AActor::execGetNextInt: Num-th registry object with MetaClass ClassName.
            "GetNextInt" => {
                let list = self.registry_meta(&args[0].to_string()).unwrap_or_default();
                Value::Str(usize::try_from(args[1].as_int()).ok().and_then(|i| list.get(i)).map(|r| r.object.clone()).unwrap_or_default())
            }
            // AActor::execGetNextIntDesc: object and description of the Num-th entry.
            "GetNextIntDesc" => {
                let list = self.registry_meta(&args[0].to_string()).unwrap_or_default();
                let r = usize::try_from(args[1].as_int()).ok().and_then(|i| list.get(i)).cloned().unwrap_or_default();
                args[2] = Value::Str(r.object);
                args[3] = Value::Str(r.description);
                Value::Void
            }
            // AActor::execGetNextSkin.
            "GetNextSkin" => {
                let (skin, desc) = self.get_next_skin(&args[0].to_string(), &args[1].to_string(), args[2].as_int());
                args[3] = Value::Str(skin);
                args[4] = Value::Str(desc);
                Value::Void
            }
            // AActor::execGetMapName.
            "GetMapName" => Value::Str(self.get_map_name(&args[0].to_string(), &args[1].to_string(), args[2].as_int())),
            // AActor::execGetCacheEntry: false without an entry, Guid and Filename empty.
            "GetCacheEntry" => {
                let entry = self.get_cache_entry(args[0].as_int());
                let found = entry.is_some();
                let (guid, file) = entry.unwrap_or_default();
                args[1] = Value::Str(guid);
                args[2] = Value::Str(file);
                Value::Bool(found)
            }
            // AActor::execGetMaterial: None without a mesh instance.
            "GetMaterial" if self.is_a(this, "Actor") => self.get_material(this, args[0].as_int()),
            // AActor::execGetRenderBoundingSphere.
            "GetRenderBoundingSphere" => {
                let s = self.render_bounding_sphere(this);
                let mut v = self.zero(&crate::types::Type::Struct("Object.Plane".into()), 1);
                if let Value::Struct(_, fields) = &mut v {
                    for (f, x) in fields.iter_mut().zip(s) {
                        *f = Value::Float(x);
                    }
                }
                v
            }
            // AActor::execGetCartoonLightDir: USkeletalMeshInstance+0x23C, which nothing
            // writes in the game's DLLs except copies of the instance (zero); (0, 0, 1) without a
            // skeletal instance.
            "GetCartoonLightDir" => {
                if self.instance_skeletal_presents(this) {
                    Value::vector(0.0, 0.0, 0.0)
                } else {
                    Value::vector(0.0, 0.0, 1.0)
                }
            }
            // UInteraction::execWorldToScreen.
            "WorldToScreen" if self.is_a(this, "Interaction") => {
                let v = self.world_to_screen(this, args, supplied);
                Value::vector(v[0], v[1], v[2])
            }
            // UInteraction::execScreenToWorld.
            "ScreenToWorld" if self.is_a(this, "Interaction") => {
                let v = self.screen_to_world(this, args, supplied);
                Value::vector(v[0], v[1], v[2])
            }
            // AActor::execGetRootLocation, execGetRootRotation,
            // execGetRootLocationDelta, execGetRootRotationDelta:
            // methods of USkeletalMeshInstance (skel.rs); zero without a skeletal mesh.
            "GetRootLocation" | "GetRootLocationDelta" => {
                if self.mesh_instance(this).is_none() {
                    return Ok(Some(Value::vector(0.0, 0.0, 0.0)));
                }
                let v = if name == "GetRootLocation" { self.root_location(this) } else { self.root_location_delta(this) };
                Value::vector(v[0], v[1], v[2])
            }
            "GetRootRotation" | "GetRootRotationDelta" => {
                if self.mesh_instance(this).is_none() {
                    return Ok(Some(Value::rotator(0, 0, 0)));
                }
                if name == "GetRootRotationDelta" {
                    self.root_rotation_delta(this);
                    return Ok(Some(Value::rotator(0, 0, 0)));
                }
                let r = self.root_rotation(this);
                Value::rotator(r[0], r[1], r[2])
            }
            // APlayerController::execAttribPadToViewport → UWindowsClient::
            // AttribPadToViewport: DirectInput gamepads given to the
            // viewports, in order (pad.rs).
            "AttribPadToViewport" => {
                self.attrib_pad_to_viewport();
                Value::Void
            }
            // APlayerController::execGetPadNumber → UWindowsClient::GetGetPadNumber
            // NbJoy, number of gamepads found at startup.
            "GetPadNumber" => {
                Value::Int(self.pads.count)
            }
            // APlayerController::execOptimizeInputBindings → UViewport::
            // OptimizeInputBindings → UInput::OptimizeInputBindings:
            // bindings recompiled from the configuration, then ResetInput.
            "OptimizeInputBindings" => {
                // Without a Player (+0x330) nothing; the Player (UViewport) only calls UInput::OptimizeInputBindings if it has an Input (+0x6C).
                if let Some(pc) = self.input_sys.as_ref().map(|s| s.pc).filter(|_| self.propr(this, "Player").as_object().is_some()) {
                    let user = self.config.as_mut().map(|c| c.file_text("User")).unwrap_or_default();
                    let bindings = crate::input::Bindings::parse(&user);
                    self.input_init(pc, &bindings);
                    self.reset_input();
                }
                Value::Void
            }
            // APlayerController::execGetPlayerNetworkAddress: empty if Player is not
            // a UNetConnection (the local player is a viewport); otherwise
            // UNetConnection::LowLevelGetRemoteAddress (vtable +0x74):
            // IpString(address, port) of the other end, "a.b.c.d:port" (AccessControl.KickBan uses it).
            "GetPlayerNetworkAddress" => {
                let player = self.propr(this, "Player").as_object();
                let address = player.and_then(|p| {
                    let d = self.ipdrv.net_driver.as_ref()?;
                    d.conns.iter().find(|c| c.player == Some(p)).map(|c| crate::ipdrv::ip_string(c.conn.remote.ip().octets(), c.conn.remote.port() as i32))
                });
                Value::Str(address.unwrap_or_default())
            }
            // APlayerController::execCopyToClipboard: appClipboardCopy.
            "CopyToClipboard" => {
                self.solo.clipboard = args[0].to_string();
                Value::Void
            }
            // APlayerController::execPasteFromClipboard: appClipboardPaste.
            "PasteFromClipboard" => Value::Str(self.solo.clipboard.clone()),
            // APlayerController::execFindStairRotation.
            "FindStairRotation" => Value::Int(self.find_stair_rotation(this, args[0].as_float())),
            // APlayerController::execHandleWalking → HandleWalking.
            "HandleWalking" => {
                self.handle_walking(this);
                Value::Void
            }
            // ALevelInfo::execDecalScreen: parameters read, nothing else.
            "DecalScreen" => Value::Void,
            // ALevelInfo::execSetPoisonEffect: method 0xA0 of
            // Client->Viewports(0)+0x154 (MaxIntensity 1 and Hue (R 0x28, G 0x58, B 0x28) by default).
            "SetPoisonEffect" => {
                let max = if given(2) { args[2].as_float() } else { 1.0 };
                let hue = match args.get(3) {
                    Some(Value::Struct(_, f)) if given(3) && f.len() == 4 => [f[0].as_byte(), f[1].as_byte(), f[2].as_byte(), f[3].as_byte()],
                    _ => [0x28, 0x58, 0x28, 0xff],
                };
                let (on, delay) = (args[0].as_bool(), args[1].as_float());
                self.screen_fx_all(|fx| fx.set_poison(on, delay, max, hue));
                Value::Void
            }
            // ALevelInfo::execSetBlurEffect / execSetSharpEffect:
            // method 0xA4 of Viewport+0x154 with (state, 0.5, 1) or (state, 0.5, 0).
            "SetBlurEffect" | "SetSharpEffect" => {
                let (on, blur) = (args[0].as_bool(), name == "SetBlurEffect");
                self.screen_fx_all(|fx| fx.set_blur(on, 0.5, blur));
                Value::Void
            }
            // ALevelInfo::execSetInjuredEffect: method 0xA8 of Viewport+0x154.
            "SetInjuredEffect" => {
                let (on, delay) = (args[0].as_bool(), args[1].as_float());
                self.screen_fx_all(|fx| fx.set_injured(on, delay));
                Value::Void
            }
            // ALevelInfo::execSetViewport → UViewport::SetViewport.
            "SetViewport" if self.is_a(this, "LevelInfo") => {
                self.solo.viewport_rect = Some([args[0].as_int(), args[1].as_int(), args[2].as_int(), args[3].as_int()]);
                Value::Void
            }
            // APawn::execHeadYawControl: bHeadYaw, MaxHeadYaw, HeadYawRotationSpeed.
            "HeadYawControl" => {
                self.set_prop(this, "bHeadYaw", Value::Bool(args[0].as_bool()));
                self.set_prop(this, "MaxHeadYaw", Value::Int(args[1].as_int()));
                self.set_prop(this, "HeadYawRotationSpeed", Value::Float(args[2].as_float()));
                Value::Void
            }
            // APawn::execSetWalking → SetWalking.
            "SetWalking" if self.is_a(this, "Pawn") => {
                self.set_walking_raw(this, args[0].as_int());
                Value::Void
            }
            // AController::execPickTarget.
            "PickTarget" => self.pick_target(this, args),
            // AController::execPickAnyTarget.
            "PickAnyTarget" => self.pick_any_target(this, args),
            // AGameInfo::execGetNetworkNumber: empty without XLevel->NetDriver (single-player),
            // otherwise NetDriver->LowLevelGetNetworkNumber (IpString(LocalAddr)).
            "GetNetworkNumber" => Value::Str(self.ipdrv.net_driver.as_ref().map_or(String::new(), |d| crate::ipdrv::ip_string(d.local_addr, 0))),
            // AGameInfo::execParseKillMessage: %k replaced by KillerName, then %o by
            // VictimName in the result; without %k the result (and so everything) stays empty.
            "ParseKillMessage" => {
                let replace = |text: &str, key: &str, by: &str| -> String {
                    match text.find(key) {
                        Some(i) => format!("{}{by}{}", &text[..i], &text[i + key.len()..]),
                        None => String::new(),
                    }
                };
                let killer = args[0].to_string();
                let victim = args[1].to_string();
                let first = replace(&args[2].to_string(), "%k", &killer);
                let message = match first.find("%o") {
                    Some(_) => replace(&first, "%o", &victim),
                    None => first,
                };
                Value::Str(message)
            }
            // UInteractionMaster::execTravel: TravelURL of Client->Viewports(0),
            // TravelType TRAVEL_Absolute, bTravelItems zero (travel handled by UGameEngine::Tick);
            // nothing without a window (null Client or empty Viewports).
            "Travel" if self.is_a(this, "InteractionMaster") => {
                if self.local_viewport().is_some() {
                    self.client_travel = Some((args[0].to_string(), 0, false));
                }
                Value::Void
            }
            // ABloodFlow::execGrowBloodFlow (Xiii.dll): Detach(1), DrawScale += dt ×
            // 0.05, Attach(); true when DrawScale exceeds 0.35.
            "GrowBloodFlow" => {
                self.projector_detach(this, true);
                let (ds, finished_at) = growth_task(self.propr(this, "DrawScale").as_float(), args[0].as_float());
                self.set_prop(this, "DrawScale", Value::Float(ds));
                self.projector_attach(this);
                Value::Bool(finished_at)
            }
            // UTexture::execGetAnimLength: one duration per texture of the AnimNext chain,
            // 1 / MaxFrameRate of the first one (0.033 if zero) for each.
            "GetAnimLength" if self.is_a(this, "Texture") => {
                let rate = self.propr(this, "MaxFrameRate").as_float();
                let mut total = 0.0f32;
                let mut t = Some(this);
                let mut guard = 0;
                while let Some(x) = t {
                    total += if rate == 0.0 { 0.033 } else { 1.0 / rate };
                    t = self.propr(x, "AnimNext").as_object().filter(|&n| n != this);
                    guard += 1;
                    if guard > 4096 {
                        break;
                    }
                }
                Value::Float(total)
            }
            _ => return Ok(None),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn world_to_screen_projection() {
        let mut vm = Vm::new();
        vm.viewport = (800, 600);
        let node = vm.camera_scene(None, None, [0.0; 3], [0; 3], 90.0, (800, 600));
        let to_canvas = crate::visibility::SceneNode::matrix_inverse(&vm.canvas_to_screen());
        let screen = |p: V| {
            let q = node.project(p);
            openxiii_assets::coords::transform_fvector(&to_canvas, [q[0], q[1], q[2]])
        };
        // Straight ahead: centre of the viewport (FCanvasUtil's half pixel); at 45° to the right (FOV 90):
        // right edge; above: towards the top of the screen.
        let c = screen([100.0, 0.0, 0.0]);
        assert!((c[0] - 400.5).abs() < 1e-3 && (c[1] - 300.5).abs() < 1e-3, "{c:?}");
        let r = screen([100.0, 100.0, 0.0]);
        assert!((r[0] - 800.5).abs() < 0.05, "{r:?}");
        assert!(screen([100.0, 0.0, 50.0])[1] < 300.0);
        // ScreenToWorld: the direction that goes back through the same point.
        let s = openxiii_assets::coords::transform_fvector(&vm.canvas_to_screen(), r);
        let w = node.deproject_plane([s[0], s[1], s[2], 5.0]);
        let k = 1.0 / (w[0] * w[0] + w[1] * w[1] + w[2] * w[2]).sqrt();
        assert!((w[0] * k - 0.70710677).abs() < 1e-3 && (w[1] * k - 0.70710677).abs() < 1e-3, "{w:?}");
    }

    #[test]
    fn parse_like_core() {
        let line = "Name=XIIIMP.GIPlayer,Class=Class,MetaClass=XIIIMP.XIIIMPPlayerPawn,Description=\"Spads, here\"";
        assert_eq!(parse_value(line, "Name=").as_deref(), Some("XIIIMP.GIPlayer"));
        assert_eq!(parse_value(line, "Class=").as_deref(), Some("Class"));
        assert_eq!(parse_value(line, "MetaClass=").as_deref(), Some("XIIIMP.XIIIMPPlayerPawn"));
        assert_eq!(parse_value(line, "Description=").as_deref(), Some("Spads, here"));
        assert_eq!(parse_value(line, "Autodetect="), None);
    }

    #[test]
    fn wildcard_like_findfiles() {
        assert!(wildcard("Engine.INT", "*.int"));
        assert!(wildcard("DM_USA01.unr", "DM_*.unr"));
        assert!(!wildcard("USA01.unr", "DM_*.unr"));
    }
}

/// ABloodFlow::execGrowBloodFlow (Xiii.dll): DrawScale + DeltaTime × 0.05f (24-bit
/// FPU), and true when it exceeds 0.35 (compared in double).
pub(crate) fn growth_task(scale_value: f32, dt: f32) -> (f32, bool) {
    let ds = dt * 0.05 + scale_value;
    (ds, ds as f64 > 0.35)
}

/// Where the value returned by `Vm::texture_source` comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextureSource {
    Skin(usize),
    Material(usize),
    Nothing,
}

