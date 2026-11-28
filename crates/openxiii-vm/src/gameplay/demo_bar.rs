//! The original has no map for it: the layout is not the original's code. The room is placed at the
//! centre of the largest flat, clear area of the map, and the player is brought there facing the
//! counter. Nothing is called without the variable: the game stays unchanged.

#[allow(unused_imports)]
use openxiii_math::SimMath as _;
use crate::physics::V;
use crate::value::{ObjId, Value};
use crate::Vm;

/// Scenery object: mesh, position (u towards the counter, v to the right, height above the
/// ground of its base), yaw in degrees relative to the u axis, and bottom of the mesh below its origin.
struct Prop {
    mesh: &'static str,
    u: f32,
    v: f32,
    up: f32,
    yaw: f32,
    bottom: f32,
}

const fn p(mesh: &'static str, u: f32, v: f32, up: f32, yaw: f32, bottom: f32) -> Prop {
    Prop { mesh, u, v, up, yaw, bottom }
}

/// Bottom of each mesh: -min of its box height (OBJ export of StatiBarfight.usx).
const PROPS: &[Prop] = &[
    // Counter at the back, neon lights at both ends.
    p("bar_comptoir", 380.0, 0.0, 0.0, 90.0, -153.0),
    p("barneon", 380.0, -560.0, 0.0, 0.0, -5.0),
    p("barneon", 380.0, 560.0, 0.0, 0.0, -5.0),
    // Pool corner on the right: table, balls, triangle, cue.
    p("barbillard", -40.0, 300.0, 0.0, 90.0, -42.0),
    p("billardboulerouge", -70.0, 290.0, 84.0, 0.0, -3.0),
    p("billardboulejaune", -20.0, 320.0, 84.0, 0.0, -3.0),
    p("billardbouleblanc", -100.0, 330.0, 84.0, 0.0, -3.0),
    p("billardtriangle", 10.0, 270.0, 84.0, 0.0, -2.0),
    p("canne1", -40.0, 420.0, 0.0, 0.0, -60.0),
    // Music and darts corner on the left.
    p("BAR_jukebox", 120.0, -500.0, 0.0, 0.0, 0.0),
    p("BAR_dartgame", -120.0, -520.0, 0.0, 0.0, 0.0),
    p("tV", 250.0, 520.0, 0.0, -90.0, -24.0),
    // Cleaning.
    p("poubelles", -330.0, -480.0, 0.0, 0.0, -34.0),
    p("balai", -380.0, -430.0, 0.0, 0.0, -71.0),
    // Mirror ball on the ceiling.
    p("dancingboule", 80.0, 0.0, 230.0, 0.0, -59.0),
    // Wall of the team's graffiti and posters behind the counter.
    p("TAGdave", 560.0, -480.0, 290.0, -90.0, -18.0),
    p("TAGflav", 560.0, -400.0, 290.0, -90.0, -18.0),
    p("TAGfuck", 560.0, -320.0, 290.0, -90.0, -18.0),
    p("TAGgreg", 560.0, -240.0, 290.0, -90.0, -18.0),
    p("TAGivan", 560.0, -160.0, 290.0, -90.0, -18.0),
    p("TAGmeyer", 560.0, -80.0, 290.0, -90.0, -18.0),
    p("TAGnath", 560.0, 0.0, 290.0, -90.0, -18.0),
    p("TAGsmile", 560.0, 80.0, 290.0, -90.0, -18.0),
    p("TAGtoumou", 560.0, 160.0, 290.0, -90.0, -12.0),
    p("afficheplaymate", 560.0, 260.0, 290.0, -90.0, -18.0),
    p("affichecoyottegirls", 560.0, 340.0, 290.0, -90.0, -18.0),
    p("afficheDreamcar", 560.0, 420.0, 290.0, -90.0, -18.0),
    p("affichepapiers1", 560.0, 500.0, 290.0, -90.0, -14.0),
];

/// Bar weapons, as pickup objects: class, u, v, CollisionHeight of the class (placed
/// directly at that height above the ground: an object moved afterwards would no longer be
/// touched by the player).
const PICKUPS: &[(&str, f32, f32, f32)] = &[
    ("BarSitDecoPick", 200.0, -300.0, 37.0),
    ("BarSitDecoPick", 200.0, -100.0, 37.0),
    ("BarSitDecoPick", 200.0, 100.0, 37.0),
    ("BarSitDecoPick", 200.0, 300.0, 37.0),
    ("DartDecoPick", -60.0, -380.0, 16.0),
    ("DartDecoPick", -10.0, -380.0, 16.0),
    ("DartDecoPick", 40.0, -380.0, 16.0),
    ("PhoneDecoPick", -380.0, 0.0, 20.0),
];

/// Half-side of the room (clearance searched around the centre).
const ROOM: f32 = 600.0;
/// The player is brought to this distance from the centre, facing the counter.
const START: f32 = 520.0;

impl Vm {
    /// Clearance around `c` (on the ground): smallest free distance over 16 directions, a
    /// box swept at waist height; 0 if the ground is not flat within `ROOM` / 2 around.
    fn bar_clearance(&self, c: V) -> f32 {
        let waist = [c[0], c[1], c[2] + 60.0];
        let mut min = ROOM;
        for k in 0..16 {
            let a = k as f32 * std::f32::consts::TAU / 16.0;
            let to = [waist[0] + ROOM * a.dcos(), waist[1] + ROOM * a.dsin(), waist[2]];
            min = min.min(ROOM * self.demo_sweep(waist, to, [20.0, 20.0, 30.0]));
            let half = [c[0] + ROOM / 2.0 * a.dcos(), c[1] + ROOM / 2.0 * a.dsin(), c[2] + 60.0];
            match self.demo_floor(half, 160.0) {
                Some(z) if (z - c[2]).abs() <= 30.0 => {}
                _ => return 0.0,
            }
        }
        min
    }

    fn bar_spawn(&mut self, class: &str, at: V, yaw: f32) -> Option<ObjId> {
        let class = self.class(class)?;
        let y = (yaw * 32768.0 / std::f32::consts::PI) as i32 & 0xFFFF;
        self.spawn_actor(class, Value::Object(None), None, Value::vector(at[0], at[1], at[2]), Value::rotator(0, y, 0), Value::Object(None), true).ok()?
    }

    /// Bar room at the centre of the largest clear area; the player is brought there. Returns the
    /// number of objects placed and the clearance found.
    pub fn demo_bar(&mut self, pawn: ObjId) -> (usize, f32) {
        let here = self.native_prop(pawn, crate::NativeField::Location).as_vector();
        let mut best: Option<(V, f32)> = None;
        let spots: Vec<V> = self
            .actors
            .iter()
            .copied()
            .filter(|&a| self.objects[a].is_some() && (self.is_a(a, "PathNode") || self.is_a(a, "PlayerStart")))
            .map(|a| self.native_prop(a, crate::NativeField::Location).as_vector())
            .chain(std::iter::once(here))
            .collect();
        for s in spots {
            let Some(z) = self.demo_floor(s, 300.0) else { continue };
            let c = [s[0], s[1], z];
            let score = self.bar_clearance(c);
            if best.is_none_or(|b| score > b.1) {
                best = Some((c, score));
            }
        }
        let Some((c, clearance)) = best else { return (0, 0.0) };
        // u axis: towards the counter, chosen in the clearest direction seen from the centre.
        let waist = [c[0], c[1], c[2] + 60.0];
        let facing = (0..8)
            .map(|k| k as f32 * std::f32::consts::TAU / 8.0)
            .max_by(|a, b| {
                let f = |a: f32| self.demo_sweep(waist, [waist[0] + 2000.0 * a.dcos(), waist[1] + 2000.0 * a.dsin(), waist[2]], [20.0, 20.0, 30.0]);
                f(*a).total_cmp(&f(*b))
            })
            .unwrap_or(0.0)
            + std::f32::consts::PI;
        let (cu, su) = (facing.dcos(), facing.dsin());
        // Room shrunk if the area is smaller than it (by at most half).
        let k = (clearance / ROOM).clamp(0.5, 1.0);
        let at = |u: f32, v: f32| [c[0] + k * (u * cu - v * su), c[1] + k * (u * su + v * cu), c[2]];
        let ground = |vm: &Vm, q: V| vm.demo_floor([q[0], q[1], c[2] + 100.0], 300.0).unwrap_or(c[2]);
        let mut placed = 0;
        for prop in PROPS {
            let q = at(prop.u, prop.v);
            let z = ground(self, q) + prop.up - prop.bottom;
            let Some(a) = self.bar_spawn("Effects", [q[0], q[1], z], facing + prop.yaw.to_radians()) else { continue };
            self.set_prop(a, "DrawType", Value::Byte(8));
            self.set_prop(a, "StaticMesh", Value::Asset(format!("StatiBarfight.{}", prop.mesh)));
            self.set_prop(a, "bUnlit", Value::Bool(false));
            placed += 1;
        }
        for &(class, u, v, height) in PICKUPS {
            let q = at(u, v);
            let floor = ground(self, q);
            match self.bar_spawn(class, [q[0], q[1], floor + height + 2.0], facing) {
                Some(_) => placed += 1,
                None => println!("bar: {class} refused"),
            }
        }
        // XIIIPlayerInteraction only offers pickups with Level.bLonePlayer, and the bar's
        // objects (XIIIDecoPickup) can only be picked up that way: in the arena, the game is set to single-player.
        if let Some(level) = self.level {
            self.set_prop(level, "bLonePlayer", Value::Bool(true));
        }
        // Player brought facing the counter, at their usual height above the ground.
        let lift = self.demo_floor(here, 400.0).map_or(80.0, |z| here[2] - z);
        let start = at(-START, 0.0);
        let start = [start[0], start[1], ground(self, start) + lift];
        if let Some(f) = self.find_function(pawn, "SetLocation", false) {
            let _ = self.call(f, pawn, vec![Some(Value::vector(start[0], start[1], start[2]))]);
        }
        let yaw = (facing * 32768.0 / std::f32::consts::PI) as i32 & 0xFFFF;
        self.set_prop(pawn, "Rotation", Value::rotator(0, yaw, 0));
        if let Some(pc) = self.propr(pawn, "Controller").as_object() {
            self.set_prop(pc, "Rotation", Value::rotator(0, yaw, 0));
        }
        self.demo_message(pawn, "Bar brawl: the counter, the pool table, the jukebox, the graffiti. Pick up the stools, the darts and the phone.");
        (placed, clearance)
    }
}
