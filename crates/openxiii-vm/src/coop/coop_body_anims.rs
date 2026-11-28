//! Improved build only: a co-op player's body drawn with another character's mesh plays every
//! gesture of the player's body.
//!
//! The player's body (XIIIPersos.XIIIM) uses the animation XIIIPersosG.MigA, which has every
//! sequence the player's scripts ask for (XIIIPawn: waiting, moving, crouching, ladders, swimming,
//! weapons, hits, deaths). The other men of the cast use the very same animation. The women
//! (JonesM, KimM, with XIIIPersosG.JonesMajA) lack seven of those sequences: PlayAnim then fails
//! and the channel keeps its previous gesture. AmosM has no points on his left arm, so a punch
//! of the left hand shows nothing.
//!
//! `body_sequence` gives the sequence to play instead; `Vm::coop_body_animations` puts those
//! replacements in the body's animation under the asked names, so the scripts, the native
//! movement code and the pose all find them. Nothing changes for XIII's mesh, for a sequence the
//! mesh has, or outside the actors this is called for: the faithful game never calls it.

use std::rc::Rc;

use crate::anim::AnimSet;
use crate::value::Value;
use crate::{NativeField as Field, ObjId, Vm};

/// Name of the player's own mesh (without its package): every gesture is in its animation.
const PLAYER_MESH: &str = "XIIIM";

/// Meshes (without their package) whose left arm has no points.
const ONE_ARMED: &[&str] = &["AmosM"];

/// Sequences of the player's body that some animations lack, each with its replacements in order
/// of preference (the first one the animation has is played).
const MISSING: &[(&str, &[&str])] = &[
    // Turning on the spot crouched: the crouched wait (the original turn barely moves the feet).
    ("RotationGAccroupi", &["WaitAccroupi"]),
    ("RotationDAccroupi", &["WaitAccroupi"]),
    // Standing alternate fire: the gesture the scripts play for it crouched
    // (XIIIPawn.PlayFiring), or else the main fire of the same weapon.
    ("FireAltGunRetour", &["HandDRetour"]),
    ("FireAltBazookaRetour", &["HandDRetour"]),
    ("FireAltHarponRetour", &["GunRetourAccroupi", "GunRetour"]),
    ("FireAltM16Retour", &["GunRetour", "RetourKalash"]),
    ("FireAltMagnumRetour", &["PistoletRetour"]),
];

/// One-armed meshes: the left hand's gesture played by the right hand.
const ONE_ARMED_SWAPS: &[(&str, &str)] = &[("HandGRetour", "HandDRetour")];

/// Suffix of the path of an animation that carries these replacements.
const BODY_SUFFIX: &str = "#coopbody";

fn short(mesh: &str) -> &str {
    mesh.rsplit('.').next().unwrap_or(mesh)
}

/// The sequence a co-op player's body with `mesh` (package and name, as in the actor's Mesh)
/// plays when the scripts ask for `sequence`; `exists` tells whether the body's animation has a
/// sequence. `None`: play `sequence` itself (always for XIII's mesh).
pub fn body_sequence(mesh: &str, sequence: &str, exists: impl Fn(&str) -> bool) -> Option<&'static str> {
    let mesh = short(mesh);
    if mesh.eq_ignore_ascii_case(PLAYER_MESH) {
        return None;
    }
    if ONE_ARMED.iter().any(|m| m.eq_ignore_ascii_case(mesh)) {
        if let Some(&(_, to)) = ONE_ARMED_SWAPS.iter().find(|(from, _)| from.eq_ignore_ascii_case(sequence)) {
            if exists(to) {
                return Some(to);
            }
        }
    }
    if exists(sequence) {
        return None;
    }
    let (_, replacements) = MISSING.iter().find(|(name, _)| name.eq_ignore_ascii_case(sequence))?;
    replacements.iter().copied().find(|r| exists(r))
}

/// `base` with the replacements of `body_sequence` for `mesh`, put first under the asked names
/// (a lookup takes the first sequence with a name); `None` when nothing is replaced.
pub(crate) fn body_anim_set(base: &AnimSet, mesh: &str) -> Option<AnimSet> {
    let asked = MISSING.iter().map(|(name, _)| *name).chain(ONE_ARMED_SWAPS.iter().map(|(from, _)| *from));
    let mut seqs = Vec::new();
    let mut moves = Vec::new();
    for name in asked {
        let Some(to) = body_sequence(mesh, name, |n| base.seq(n).is_some()) else { continue };
        let Some(i) = base.seq(to) else { continue };
        let mut seq = base.seqs[i].clone();
        seq.name = name.to_string();
        seqs.push(seq);
        moves.push(base.moves.get(i)?.clone());
    }
    if seqs.is_empty() {
        return None;
    }
    seqs.extend(base.seqs.iter().cloned());
    moves.extend(base.moves.iter().cloned());
    Some(AnimSet { path: format!("{}{BODY_SUFFIX}.{}", base.path, short(mesh)), seqs, bones: base.bones.clone(), moves })
}

impl Vm {
    /// Gives a co-op player's body (`pawn`, with its character's Mesh already set) the
    /// replacements of `body_sequence`. Cheap and idempotent: it can be called every frame, and
    /// must be called again after the Mesh changes (a new mesh instance starts from the mesh's
    /// own animation). Does nothing for XIII's mesh or a mesh whose animation has everything.
    pub fn coop_body_animations(&mut self, pawn: ObjId) {
        let mesh = match self.native_prop(pawn, Field::Mesh) {
            Value::Asset(m) => m.to_string(),
            _ => return,
        };
        if short(&mesh).eq_ignore_ascii_case(PLAYER_MESH) {
            return;
        }
        let Some(inst) = self.mesh_instance(pawn) else { return };
        let (first, default) = (inst.anims.first().cloned(), inst.info.default_anim.clone());
        if first.as_ref().is_some_and(|a| a.path.contains(BODY_SUFFIX)) {
            return;
        }
        let base = match first {
            Some(a) => a,
            None => {
                let Some(path) = default else { return };
                let Some(set) = self.anim_set(&path) else { return };
                set
            }
        };
        let key = format!("{}{BODY_SUFFIX}.{}", base.path, short(&mesh)).to_lowercase();
        let set = match self.anim_sets.get(&key) {
            Some(s) => s.clone(),
            None => {
                let s = body_anim_set(&base, &mesh).map(Rc::new);
                self.anim_sets.insert(key, s.clone());
                s
            }
        };
        let Some(set) = set else { return };
        if let Some(inst) = self.mesh_insts.get_mut(&pawn) {
            match inst.anims.first_mut() {
                Some(first) => *first = set,
                None => inst.anims.push(set),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Every sequence the player's body can be asked for (XIIIPawn.uc, XIIIPlayerController.uc).
    const PLAYER_BODY: &[&str] = &[
        "WaitNeutre",
        "WaitAlerteNeutre",
        "WaitPistolet",
        "WaitPistoletAlerte2",
        "WaitKalash",
        "WaitKalashAlerte",
        "WaitSniperAlerte",
        "SniperWait",
        "WaitBazooka",
        "WaitGatling",
        "WaitCouteauAlerte",
        "WaitNeutreAlerte",
        "Walk",
        "StrafeG",
        "StrafeD",
        "MarcheArrStandar",
        "WalkAlerte",
        "RotationG",
        "RotationD",
        "RunBazook",
        "WalkGatling",
        "Run",
        "StrafeGSpeed",
        "StrafeDSpeed",
        "BackPaddle",
        "RunPistolet",
        "RunPistoletAlerte",
        "WalkBazook",
        "RunGatling",
        "WaitAccroupi",
        "WalkAccroupi",
        "StrafeGAccroupi",
        "StrafeDAccroupi",
        "MarcheArrAccroupi",
        "RotationGAccroupi",
        "RotationDAccroupi",
        "WaitNeutreAccroupi",
        "WaitPistoletAccroupi",
        "WaitFusilAccroupi",
        "WalkPistoletAccroupi",
        "RunGunAccroupi",
        "JumpUp",
        "Jump",
        "Swim",
        "WaitEchelle",
        "ClimbUp",
        "ClimbDown",
        "SearchPistol",
        "SearchGun",
        "SearchGatling",
        "ReloadPistol",
        "ReloadPistolAccroupi",
        "ReloadGun",
        "ReloadGunAccroupi",
        "ReloadBazooka",
        "ReloadGatling",
        "PistoletRetour",
        "GunRetour",
        "GunRetourAccroupi",
        "RetourKalash",
        "SniperC",
        "BazookaRetour",
        "GatlingRetour",
        "FireAltGunRetour",
        "FireAltHarponRetour",
        "FireAltM16Retour",
        "FireAltMagnumRetour",
        "FireAltBazookaRetour",
        "ThrowRetour",
        "ThrowRetourAccroupi",
        "HandDRetour",
        "HandGRetour",
        "DecoRetour",
        "PafNeutre",
        "PafPistolet",
        "PafGun",
        "PafSniper",
        "PafBazooka",
        "PafM60",
        "PafNeutreAccroupi",
        "PafPistoletAccroupi",
        "PafGunAccroupi",
        "PafSniperAccroupi",
        "PafBazookaAccroupi",
        "PafM60Accroupi",
        "DeathTete",
        "DeathTete2",
        "DeathAccroupi",
        "DeathOtage",
        "DeathAssomme",
        "DeathEpauleGauche",
        "DeathChevilleGauche",
        "DeathChevilleDroite",
        "DeathBuste0",
        "DeathBuste2",
        "DeathBuste3",
        "DeathDos",
        "DeathVentre",
        "OpenDoor",
        "SearchGround",
    ];

    /// The co-op cast's meshes (the characters whose first-person arms exist).
    const CAST: &[&str] = &[
        "XIIIM",
        "JonesM",
        "KimM",
        "AmosM",
        "CarringtonM",
        "MangousteM",
        "McCallM",
        "SheridanM",
        "PachaM",
        "StandwellM",
        "StandKM",
        "KKKM",
        "FbiM",
        "FBI2M",
        "GiM",
        "SpadsM",
        "Tueur1M",
        "Tueur2M",
        "Tueur3M",
        "Tueur5M",
        "GroflicM",
        "GardienM",
    ];

    /// The sequences the women's animation lacks.
    const WOMEN_LACK: &[&str] = &[
        "RotationGAccroupi",
        "RotationDAccroupi",
        "FireAltGunRetour",
        "FireAltHarponRetour",
        "FireAltM16Retour",
        "FireAltMagnumRetour",
        "FireAltBazookaRetour",
    ];

    #[test]
    fn the_players_mesh_plays_what_it_is_asked() {
        for s in PLAYER_BODY {
            assert_eq!(body_sequence("XIIIPersos.XIIIM", s, |_| false), None, "{s}");
        }
    }

    #[test]
    fn a_sequence_the_mesh_has_is_played_as_it_is() {
        for s in PLAYER_BODY.iter().filter(|s| **s != "HandGRetour") {
            assert_eq!(body_sequence("XIIIPersos.AmosM", s, |_| true), None, "{s}");
            assert_eq!(body_sequence("XIIIPersos.JonesM", s, |_| true), None, "{s}");
        }
    }

    #[test]
    fn missing_sequences_take_the_first_replacement_there() {
        let jones = |n: &str| !WOMEN_LACK.iter().any(|w| w.eq_ignore_ascii_case(n));
        assert_eq!(body_sequence("XIIIPersos.JonesM", "RotationGAccroupi", jones), Some("WaitAccroupi"));
        assert_eq!(body_sequence("XIIIPersos.KimM", "FireAltGunRetour", jones), Some("HandDRetour"));
        assert_eq!(body_sequence("XIIIPersos.KimM", "FireAltHarponRetour", jones), Some("GunRetourAccroupi"));
        assert_eq!(body_sequence("XIIIPersos.KimM", "FireAltMagnumRetour", jones), Some("PistoletRetour"));
        // Second choice when the first one is missing too; nothing when none is there.
        let neither = |n: &str| n != "FireAltHarponRetour" && n != "GunRetourAccroupi";
        assert_eq!(body_sequence("Mod.Other", "FireAltHarponRetour", neither), Some("GunRetour"));
        assert_eq!(body_sequence("Mod.Other", "FireAltHarponRetour", |_| false), None);
        // A sequence outside the table stays as it is (PlayAnim fails as in the game).
        assert_eq!(body_sequence("XIIIPersos.JonesM", "Patrouille1Gun", |_| false), None);
    }

    #[test]
    fn amos_punches_with_his_right_hand() {
        assert_eq!(body_sequence("XIIIPersos.AmosM", "HandGRetour", |_| true), Some("HandDRetour"));
        assert_eq!(body_sequence("XIIIPersos.AmosM", "handgretour", |_| true), Some("HandDRetour"));
        assert_eq!(body_sequence("XIIIPersos.CarringtonM", "HandGRetour", |_| true), None);
        assert_eq!(body_sequence("XIIIPersos.XIIIM", "HandGRetour", |_| true), None);
    }

    fn game() -> Option<PathBuf> {
        openxiii_package::env::var_os("OXIII_GAME").map(PathBuf::from).filter(|g| g.join("System").exists())
    }

    fn vm_with_library(game: &std::path::Path) -> Vm {
        let mut vm = Vm::new();
        vm.library = Some(openxiii_package::library::Library::new(game).expect("library"));
        vm
    }

    /// Each mesh's own animation, and that animation with the replacements.
    fn animations(vm: &mut Vm, mesh: &str) -> (Rc<AnimSet>, Option<AnimSet>) {
        let path = format!("XIIIPersos.{mesh}");
        let info = vm.mesh_info(&path).unwrap_or_else(|| panic!("{path}"));
        let base = vm.anim_set(info.default_anim.as_deref().expect("default animation")).expect("animation");
        let body = body_anim_set(&base, &path);
        (base, body)
    }

    /// The whole cast plays every gesture of the player's body once the replacements are in;
    /// without them, the women lack exactly the seven sequences of the table (so removing the
    /// table makes this test fail).
    #[test]
    fn the_whole_cast_has_every_gesture_of_the_players_body() {
        let Some(game) = game() else {
            eprintln!("OXIII_GAME not set: skipped");
            return;
        };
        let mut vm = vm_with_library(&game);
        for mesh in CAST {
            let (base, body) = animations(&mut vm, mesh);
            let set = body.as_ref().unwrap_or(&base);
            let missing: Vec<_> = PLAYER_BODY.iter().filter(|s| set.seq(s).is_none()).collect();
            assert!(missing.is_empty(), "{mesh}: {missing:?}");
            let lacking: Vec<&str> = PLAYER_BODY.iter().copied().filter(|s| base.seq(s).is_none()).collect();
            let women = *mesh == "JonesM" || *mesh == "KimM";
            assert_eq!(lacking, if women { WOMEN_LACK.to_vec() } else { Vec::new() }, "{mesh}");
            // Only the women and Amos get another animation; the replaced sequences come first.
            assert_eq!(body.is_some(), women || *mesh == "AmosM", "{mesh}");
            if let Some(body) = &body {
                assert_eq!(body.seqs.len(), body.moves.len());
                assert_eq!(body.bones, base.bones);
                for s in PLAYER_BODY {
                    let to = body_sequence(&format!("XIIIPersos.{mesh}"), s, |n| base.seq(n).is_some()).unwrap_or(s);
                    let (a, b) = (&body.seqs[body.seq(s).unwrap()], &base.seqs[base.seq(to).unwrap()]);
                    assert_eq!((a.num_frames, a.rate, &a.notifies), (b.num_frames, b.rate, &b.notifies), "{mesh} {s} → {to}");
                }
            }
        }
    }

    /// On a real pawn (Plage01's player): PlayAnim of a missing gesture fails on Jones's mesh as in
    /// the game, then works once the body's animations are given; XIII's mesh keeps its own.
    #[test]
    fn a_coop_body_plays_the_missing_gestures() {
        let Some(game) = game() else {
            eprintln!("OXIII_GAME not set: skipped");
            return;
        };
        let mut vm = Vm::new();
        vm.ipdrv.offline = true;
        vm.load_dir(&game.join("System")).unwrap();
        let url = crate::travel::start_url(&game.join("System"), "Plage01");
        vm.url = url.clone();
        vm.load_map(&game.join("Maps").join("Plage01.unr")).unwrap();
        let game_class = vm.game_class_for(&url);
        let gi = vm.start_game(&game_class, &url.options_string()).unwrap();
        vm.begin_play();
        let pc = vm.spawn_play_actors(gi, &url.portal, &url.options_string(), None).unwrap()[0];
        let pawn = vm.native_prop(pc, Field::Pawn).as_object().unwrap();
        // XIII's mesh: nothing changes.
        let before = vm.mesh_instance(pawn).and_then(|i| i.anims.first().map(|a| a.path.clone()));
        vm.coop_body_animations(pawn);
        assert!(vm.play_anim(pawn, 0, "WaitNeutre", 1.0, 0.0, false));
        let after = vm.mesh_instance(pawn).and_then(|i| i.anims.first().map(|a| a.path.clone()));
        assert!(after.is_some_and(|p| !p.contains(BODY_SUFFIX)), "{before:?}");
        // Jones's mesh, as the game plays it: the alternate fire is missing.
        vm.set_prop(pawn, "Mesh", Value::Asset("XIIIPersos.JonesM".into()));
        assert!(!vm.play_anim(pawn, 14, "FireAltGunRetour", 1.0, 0.0, false));
        // With the body's animations: played under its own name, with the crouched gesture's frames.
        vm.coop_body_animations(pawn);
        vm.coop_body_animations(pawn);
        assert!(vm.play_anim(pawn, 14, "FireAltGunRetour", 1.0, 0.0, false));
        assert!(vm.play_anim(pawn, 3, "RotationGAccroupi", 1.0, 0.0, true));
        let channels = vm.anim_channels(pawn);
        assert!(channels.iter().any(|c| c.0 == "FireAltGunRetour"), "{channels:?}");
        vm.update_animation(pawn, 0.1);
        assert!(vm.get_frame(pawn).is_some());
        // A new mesh instance starts again from the mesh's own animation.
        vm.set_prop(pawn, "Mesh", Value::Asset("XIIIPersos.KimM".into()));
        assert!(!vm.play_anim(pawn, 14, "FireAltM16Retour", 1.0, 0.0, false));
        vm.coop_body_animations(pawn);
        assert!(vm.play_anim(pawn, 14, "FireAltM16Retour", 1.0, 0.0, false));
    }
}
