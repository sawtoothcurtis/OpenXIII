//! Player and bone natives (Engine.dll): view shakes and flashes, walking
//! bob (APlayerController), pawn orientation (APawn::FaceRotation), frame of a bone in the
//! world (AActor::GetBoneCoords). Operations in machine-code order, in 32-bit floats
//! (D3DDrv creates the device without D3DCREATE_FPU_PRESERVE: the game's FPU computes on 24 bits).

#[allow(unused_imports)]
use openxiii_math::SimMath as _;
use crate::ai_natives::x87_d24;
use crate::physics::CheckResult;
use crate::value::{ObjId, Value};
use crate::Vm;
use openxiii_assets::coords::{self, Coords, Matrix};

type V = [f32; 3];

/// (_ftol): float → integer conversion by truncation, low 32 bits.
fn ftol(f: f32) -> i32 {
    (f as i64) as i32
}

/// CheckShake — the shake bounces when it reaches its maximum; on the last
/// bounce (Time ≤ 1), it stops.
fn check_shake(max: &mut f32, offset: &mut f32, rate: &mut f32, time: &mut f32) {
    if offset.abs() < max.abs() {
        return;
    }
    if *time > 1.0 {
        let v = (*max / *rate).abs() * *time;
        *max = if v > 1.0 || v.is_nan() { -*max } else { (1.0 / *time - 1.0) * *max };
        *time -= 1.0;
        *rate = -*rate;
        *offset = -*max;
    } else {
        *max = 0.0;
        *offset = 0.0;
        *rate = 0.0;
    }
}

impl Vm {
    pub(crate) fn pv3(&self, o: ObjId, name: &str) -> V {
        self.propr(o, name).as_vector()
    }

    pub(crate) fn set_v3(&mut self, o: ObjId, name: &str, v: V) {
        self.set_prop(o, name, Value::vector(v[0], v[1], v[2]));
    }

    fn pf32(&self, o: ObjId, name: &str) -> f32 {
        self.propr(o, name).as_float()
    }

    fn set_f32(&mut self, o: ObjId, name: &str, f: f32) {
        self.set_prop(o, name, Value::Float(f));
    }

    pub(crate) fn player_native(&mut self, name: &str, this: ObjId, args: &mut [Value]) -> Option<Value> {
        Some(match name {
            "ViewShake" => {
                self.view_shake(this, args.first()?.as_float());
                Value::Void
            }
            "ViewFlash" => {
                self.view_flash(this, args.first()?.as_float());
                Value::Void
            }
            "CheckBob" => {
                let (dt, speed, y) = (args.first()?.as_float(), args.get(1)?.as_float(), args.get(2)?.as_vector());
                self.check_bob(this, dt, speed, y);
                Value::Void
            }
            "CalcFirstPersonView" => {
                let (l, r) = self.calc_first_person_view(this, args.first()?.as_vector());
                args[0] = Value::vector(l[0], l[1], l[2]);
                args[1] = Value::rotator(r[0], r[1], r[2]);
                Value::Void
            }
            "FaceRotation" => {
                self.face_rotation(this, args.first()?.as_rotator());
                Value::Void
            }
            "GetBoneCoords" => {
                let c = self.get_bone_coords(this, &args.first()?.as_name()).unwrap_or_default();
                let v = |a: V| Value::vector(a[0], a[1], a[2]);
                Value::Struct("object.coords".into(), vec![v(c.origin), v(c.x), v(c.y), v(c.z)].into())
            }
            // AActor::execGetBoneRotation → USkeletalMeshInstance::GetBoneRotation
            // OrthoRotation of Transpose(SpaceBases[bone]) × frame +0x2A8;
            // the Space parameter is ignored; unknown bone: zero rotator.
            "GetBoneRotation" => {
                let bone = args.first()?.as_name();
                let Some(info) = self.mesh_instance(this).map(|i| i.info.clone()) else {
                    return Some(Value::rotator(0, 0, 0));
                };
                let idx = crate::skel::match_ref_bone(&info, &bone);
                if idx < 0 {
                    self.log.push(format!("Log: Warning: Invalid bone index ({idx}) for GetBoneRotation."));
                    return Some(Value::rotator(0, 0, 0));
                }
                let _ = self.get_frame(this);
                let r = self.mesh_instance(this).map_or([0; 3], |i| crate::skel::bone_rotation_world(&i.pose, idx as usize));
                Value::rotator(r[0], r[1], r[2])
            }
            _ => return None,
        })
    }

    /// APlayerController::execViewShake: view offset (ShakeOffset) and roll
    /// of the shake, applied by SetRotation.
    fn view_shake(&mut self, pc: ObjId, dt: f32) {
        let mut rate = self.pv3(pc, "ShakeOffsetRate");
        if rate[0] != 0.0 || rate[1] != 0.0 || rate[2] != 0.0 {
            let (mut offset, mut max, mut time) = (self.pv3(pc, "ShakeOffset"), self.pv3(pc, "MaxShakeOffset"), self.pv3(pc, "ShakeOffsetTime"));
            for k in 0..3 {
                offset[k] += dt * rate[k];
                check_shake(&mut max[k], &mut offset[k], &mut rate[k], &mut time[k]);
            }
            self.set_v3(pc, "ShakeOffset", offset);
            self.set_v3(pc, "MaxShakeOffset", max);
            self.set_v3(pc, "ShakeOffsetRate", rate);
            self.set_v3(pc, "ShakeOffsetTime", time);
        }
        let [pitch, yaw, mut roll] = self.propr(pc, "Rotation").as_rotator();
        let mut roll_rate = self.pf32(pc, "ShakeRollRate");
        if roll_rate != 0.0 {
            let mut r = ftol(dt * roll_rate + (roll & 0xffff) as f32) & 0xffff;
            if r > 0x8000 {
                r -= 0x10000;
            }
            let mut froll = r as f32;
            let (mut max, mut time) = (self.pf32(pc, "MaxShakeRoll"), self.pf32(pc, "ShakeRollTime"));
            check_shake(&mut max, &mut froll, &mut roll_rate, &mut time);
            self.set_f32(pc, "MaxShakeRoll", max);
            self.set_f32(pc, "ShakeRollRate", roll_rate);
            self.set_f32(pc, "ShakeRollTime", time);
            roll = ftol(froll);
        } else if self.propr(pc, "bZeroRoll").as_bool() {
            roll = 0;
        }
        let mut hit = CheckResult::new(1.0);
        self.move_actor(pc, [0.0; 3], [pitch, yaw, roll], &mut hit, false, false, false, false);
    }

    /// APlayerController::execViewFlash: screen flash and fog, which
    /// tend towards their target (desired, constant, head volume).
    fn view_flash(&mut self, pc: ObjId, dt: f32) {
        let delta = if 0.1 > dt || dt.is_nan() { dt } else { 0.1 };
        let mut goal = self.pf32(pc, "ConstantGlowScale") + self.pf32(pc, "DesiredFlashScale") + 1.0;
        let (dff, cgf) = (self.pv3(pc, "DesiredFlashFog"), self.pv3(pc, "ConstantGlowFog"));
        let mut fog = [dff[0] + cgf[0], dff[1] + cgf[1], dff[2] + cgf[2]];
        if let Some(head) = self.propr(pc, "Pawn").as_object().and_then(|p| self.propr(p, "HeadVolume").as_object()) {
            goal += self.pv3(head, "ViewFlash")[0];
            let vf = self.pv3(head, "ViewFog");
            fog = [fog[0] + vf[0], fog[1] + vf[1], fog[2] + vf[2]];
        }
        let dfs = self.pf32(pc, "DesiredFlashScale");
        let d = delta * dfs;
        self.set_f32(pc, "DesiredFlashScale", dfs - (d + d));
        self.set_v3(pc, "DesiredFlashFog", [dff[0] - (dff[0] + dff[0]) * delta, dff[1] - (dff[1] + dff[1]) * delta, dff[2] - (dff[2] + dff[2]) * delta]);
        let fs_x = self.pv3(pc, "FlashScale")[0];
        let mut fs = (((goal - fs_x) + self.pf32(pc, "InstantFlash")) * delta) * 10.0 + fs_x;
        let (ff, inst) = (self.pv3(pc, "FlashFog"), self.pv3(pc, "InstantFog"));
        let mut ff = [0, 1, 2].map(|k| (((fog[k] - ff[k]) + inst[k]) * 10.0) * delta + ff[k]);
        self.set_f32(pc, "InstantFlash", 0.0);
        self.set_v3(pc, "InstantFog", [0.0; 3]);
        if fs > 0.981 {
            fs = 1.0;
        }
        self.set_v3(pc, "FlashScale", [fs; 3]);
        for f in &mut ff {
            if *f < 0.019 {
                *f = 0.0;
            }
        }
        self.set_v3(pc, "FlashFog", ff);
    }

    /// APlayerController::execCheckBob: view bob while walking (WalkBob,
    /// bobtime) and drop after a landing (LandBob, AppliedBob).
    fn check_bob(&mut self, pc: ObjId, dt: f32, speed: f32, y: V) {
        let pawn = self.propr(pc, "Pawn").as_object();
        // The engine's jump on "Speed2D < 10": NaN goes to the fast side.
        let fast = speed >= 10.0 || speed.is_nan();
        let rate = match pawn {
            Some(p) if fast => (speed / self.pf32(p, "GroundSpeed")) * 0.7 + 0.3,
            _ => 0.2,
        };
        let bobtime = rate * dt + self.pf32(pc, "bobtime");
        self.set_f32(pc, "bobtime", bobtime);
        let bob = self.pf32(pc, "Bob");
        // fsin leaves its result at full precision on the stack: the following product is rounded
        // only once to 24 bits.
        let s = x87_d24(((speed * bob) * 0.4) as f64, ((bobtime * 5.0) as f64).dsin(), false);
        let v = coords::safe_normal(pawn.map(|p| self.pv3(p, "Velocity")).unwrap_or_default());
        let old = self.pv3(pc, "WalkBob");
        let mut wb = [y[0] * s, y[1] * s, old[2]];
        let d = y[2] * v[2] + v[1] * y[1] + v[0] * y[0];
        wb = [d * wb[0], d * wb[1], d * wb[2]];
        let k = |c: f32| c * 0.4 * bob * speed * 1.5;
        wb = [k(v[0]) + wb[0], k(v[1]) + wb[1], k(v[2]) + wb[2]];
        let m16 = dt * 16.0;
        let m = if 1.0 > m16 || m16.is_nan() { m16 } else { 1.0 };
        let applied = (1.0 - m) * self.pf32(pc, "AppliedBob");
        self.set_f32(pc, "AppliedBob", applied);
        wb[2] = if fast { x87_d24(((speed * bob) * 0.3) as f64, ((bobtime * 16.0) as f64).dsin(), false) + applied } else { applied };
        self.set_v3(pc, "WalkBob", wb);
        let land = self.pf32(pc, "LandBob");
        if land > 0.01 {
            self.set_f32(pc, "AppliedBob", m * land + applied);
            self.set_f32(pc, "LandBob", (1.0 - dt * 8.0) * land);
        }
    }

    /// APlayerController::execCalcFirstPersonView: controller rotation;
    /// position + bob (WalkBob) + shake (ShakeOffset rotated by UnitCoords * Rotation)
    /// + the pawn's eye height.
    fn calc_first_person_view(&self, pc: ObjId, location: V) -> (V, [i32; 3]) {
        let rotation = self.propr(pc, "Rotation").as_rotator();
        let c = coords::mul_rotator(&coords::UNIT, rotation);
        let shake = coords::transform_vector(self.pv3(pc, "ShakeOffset"), &c);
        let wb = self.pv3(pc, "WalkBob");
        let mut l = [(wb[0] + location[0]) + shake[0], (shake[1] + wb[1]) + location[1], (shake[2] + wb[2]) + location[2]];
        if let Some(p) = self.propr(pc, "Pawn").as_object() {
            l[2] += self.pf32(p, "EyeHeight");
        }
        (l, rotation)
    }

    /// APawn::execFaceRotation: on a ladder, the wall's direction; when walking or
    /// falling, without pitch; applied by SetRotation.
    fn face_rotation(&mut self, pawn: ObjId, rotation: [i32; 3]) {
        let mut r = rotation;
        match self.propr(pawn, "Physics").as_byte() {
            11 => match self.propr(pawn, "OnLadder").as_object() {
                Some(l) => r = self.propr(l, "WallDir").as_rotator(),
                None => return,
            },
            1 | 2 => r[0] = 0,
            _ => {}
        }
        let mut hit = CheckResult::new(1.0);
        self.move_actor(pawn, [0.0; 3], r, &mut hit, false, false, false, false);
    }

    /// ULodMesh::MeshToWorld: the mesh's scale and RotOrigin, the actor's rotation and position,
    /// mesh origin subtracted.
    pub(crate) fn mesh_to_world(&self, actor: ObjId, info: &crate::anim::MeshInfo) -> Matrix {
        let (pp, l) = (self.pv3(actor, "PrePivot"), self.pv3(actor, "Location"));
        let rotation = self.propr(actor, "Rotation").as_rotator();
        matrix_mesh(l, pp, rotation, self.pf32(actor, "DrawScale"), self.pv3(actor, "DrawScale3D"), info)
    }

    /// AActor::execGetBoneCoords + USkeletalMeshInstance::GetBoneCoords:
    /// frame of the bone in the world (normalized axes). None without a skeletal mesh (result
    /// left null); unknown bone: GMath.UnitCoords.
    pub fn get_bone_coords(&mut self, actor: ObjId, bone: &str) -> Option<Coords> {
        let info = self.mesh_instance(actor)?.info.clone();
        let idx = crate::skel::match_ref_bone(&info, bone);
        if idx < 0 {
            self.log.push(format!("Log: Warning: Invalid bone index ({idx}) for GetBoneRotation."));
            return Some(coords::UNIT);
        }
        let bases = self.get_frame(actor).unwrap_or_default();
        let Some(b) = bases.get(idx as usize) else {
            return Some(coords::UNIT);
        };
        Some(frame_os(b, &self.mesh_to_world(actor, &info)))
    }
}

/// ULodMesh::MeshToWorld on the actor's fields.
pub(crate) fn matrix_mesh(location: V, pre_pivot: V, rotation: [i32; 3], ds: f32, ds3: V, info: &crate::anim::MeshInfo) -> Matrix {
    let scale = [ds3[0] * info.scale[0] * ds, ds3[1] * info.scale[1] * ds, ds3[2] * info.scale[2] * ds];
    let a = coords::scale_rotation_matrix(info.rot_origin, scale);
    let b = coords::rotation_translation_matrix(rotation, [pre_pivot[0] + location[0], pre_pivot[1] + location[1], pre_pivot[2] + location[2]]);
    let mut m = coords::matrix_mul(&a, &b);
    let o = info.origin;
    for c in 0..3 {
        m[12 + c] -= m[8 + c] * o[2] + m[4 + c] * o[1] + m[c] * o[0];
    }
    m
}

/// End of USkeletalMeshInstance::GetBoneCoords: the bone's SpaceBase placed in the
/// world by MeshToWorld, normalized axes.
pub(crate) fn frame_os(b: &Coords, m2w: &Matrix) -> Coords {
    let r = coords::matrix_mul(&b.matrix(), m2w);
    let axis = |v: V| {
        let mut a = coords::transform_normal(&r, v);
        coords::normalize(&mut a);
        a
    };
    Coords { origin: coords::transform_fvector(m2w, b.origin), x: axis([1.0, 0.0, 0.0]), y: axis([0.0, 1.0, 0.0]), z: axis([0.0, 0.0, 1.0]) }
}

