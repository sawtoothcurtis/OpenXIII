//! Local co-op campaign: enemy generators that hide their spawns from the player.
//!
//! A gennmi with bGenerationCamouflee (XIDPawn gennmi.genese.Timer) only makes a soldier where
//! "the player" does not see it: its variable XIII, MapInfo.XIIIPawn taken once when the
//! generation starts, so the first player only. In co-op a wave then waited on the first
//! player alone (Toits01's last fight, Kellownee's chalet): standing still with the spot in
//! view, they held the whole wave, whatever the second player did, and a first player down
//! or respawned left the generator watching a body. Co-op hides the spawns from every living
//! player: before each Timer the generator watches the living player who sees one of its
//! spawn spots (the first living player when nobody does), so a soldier only appears where no
//! player looks, and the wave goes on as soon as nobody does.

#[allow(unused_imports)]
use openxiii_math::SimMath as _;
use crate::types::FuncId;
use crate::value::{ObjId, Value};
use crate::Vm;

impl Vm {
    /// Before gennmi.Timer in co-op: the player the generator hides from (its variable XIII)
    /// chosen among the living players; the previous value, given back after the call.
    pub(crate) fn coop_generator_begin(&mut self, f: FuncId, this: ObjId) -> Option<Value> {
        if self.coop.generator_busy || self.coop.players.len() < 2 || !self.functions[f].name.eq_ignore_ascii_case("Timer") || !self.is_a(this, "gennmi") {
            return None;
        }
        if !self.prop(this, "bGenerationCamouflee").as_bool() {
            return None;
        }
        let pawns: Vec<ObjId> = (0..self.coop.players.len()).filter_map(|i| self.coop_living_pawn(i)).collect();
        let first = *pawns.first()?;
        let spots = self.coop_generator_spots(this);
        let watcher = pawns.iter().copied().find(|&p| spots.iter().any(|&s| self.coop_generator_seen(this, p, s))).unwrap_or(first);
        let saved = self.prop(this, "XIII");
        self.set_prop(this, "XIII", Value::Object(Some(watcher)));
        self.coop.generator_busy = true;
        Some(saved)
    }

    /// After the call: the generator's own variable as it was.
    pub(crate) fn coop_generator_end(&mut self, this: ObjId, saved: Value) {
        self.coop.generator_busy = false;
        if self.coop_object_alive(this) {
            self.set_prop(this, "XIII", saved);
        }
    }

    /// The spots gennmi.genese.Timer tries: PointSpawn, or Location + 78 up + i × 90 × (−1)^i
    /// along (1, 1, 0) for each of its NbNMIGen soldiers.
    fn coop_generator_spots(&self, g: ObjId) -> Vec<[f32; 3]> {
        let point = self.prop(g, "PointSpawn").as_vector();
        if point != [0.0, 0.0, 0.0] {
            return vec![point];
        }
        let at = self.prop(g, "Location").as_vector();
        let n = self.prop(g, "NbNMIGen").as_int().clamp(1, 16);
        (0..n)
            .map(|i| {
                let k = i as f32 * 90.0 * if i % 2 == 0 { 1.0 } else { -1.0 };
                [at[0] + k, at[1] + k, at[2] + 78.0]
            })
            .collect()
    }

    /// The test of gennmi.genese.Timer for one player: the spot is in sight (FastTrace from the
    /// eyes) and in front of them ((Location − spot) · vector(Rotation) ≤ 0).
    fn coop_generator_seen(&mut self, g: ObjId, pawn: ObjId, spot: [f32; 3]) -> bool {
        let at = self.prop(pawn, "Location").as_vector();
        let eye = self.call_named(pawn, "EyePosition", vec![]).map(|v| v.as_vector()).unwrap_or([0.0; 3]);
        let from = [at[0] + eye[0], at[1] + eye[1], at[2] + eye[2]];
        let clear = self.call_named(g, "FastTrace", vec![Value::vector(spot[0], spot[1], spot[2]), Value::vector(from[0], from[1], from[2])]).is_ok_and(|v| v.as_bool());
        let r = self.prop(pawn, "Rotation").as_rotator();
        let (pitch, yaw) = (r[0] as f32 * std::f32::consts::TAU / 65536.0, r[1] as f32 * std::f32::consts::TAU / 65536.0);
        let dir = [pitch.dcos() * yaw.dcos(), pitch.dcos() * yaw.dsin(), pitch.dsin()];
        let behind = (at[0] - spot[0]) * dir[0] + (at[1] - spot[1]) * dir[1] + (at[2] - spot[2]) * dir[2] > 0.0;
        clear && !behind
    }
}
