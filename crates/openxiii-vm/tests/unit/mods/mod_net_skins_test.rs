//! Mods build only: a mod's skin reaches a network guest (mod_net_skins.rs), on the real
//! PRock03 (OXIII_GAME = game folder; skipped when it is not set). The host is a VM that made
//! the mod's change; the guest is a VM of the same map whose guard got no change (a guest never
//! makes the mods' changes itself): the guest must end up with the host's skins.

use std::path::{Path, PathBuf};

use openxiii_mods::placement::{Change, Edit, MapEdit, Source, DEFAULT_RADIUS};

use crate::mod_net_skins::parse_skins_message;
use crate::value::{ObjId, Value};
use crate::Vm;

fn game() -> Option<PathBuf> {
    let g = PathBuf::from(openxiii_package::env::var_os("OXIII_GAME")?);
    g.join("System").exists().then_some(g)
}

fn change(look: Option<&str>, skin: Option<&str>) -> MapEdit {
    let c = Change { who: "gardien_lostpass".into(), near: None, radius: DEFAULT_RADIUS, look: look.map(str::to_string), skin: skin.map(str::to_string), health: None, properties: Vec::new(), character: None };
    MapEdit { map: "PRock03".into(), edit: Edit::Change(Box::new(c)), source: Source { mod_name: "test".into(), file: PathBuf::from("test/mod.toml"), line: 1 }, checkpoint: Default::default() }
}

/// PRock03 with `edits`, its level started (the mods' changes made).
fn prock03(game: &Path, edits: Vec<MapEdit>) -> Vm {
    let mut vm = Vm::new();
    vm.ipdrv.offline = true;
    vm.set_mod_edits_here(edits);
    vm.load_dir(&game.join("System")).unwrap();
    let url = crate::travel::start_url(&game.join("System"), "PRock03");
    vm.url = url.clone();
    vm.load_map(&game.join("Maps").join("PRock03.unr")).unwrap();
    let game_class = vm.game_class_for(&url);
    vm.start_game(&game_class, &url.options_string()).unwrap();
    vm.begin_play();
    vm
}

fn skins(vm: &Vm, a: ObjId) -> Vec<String> {
    match vm.prop(a, "Skins") {
        Value::Array(items) => items.iter().map(|v| format!("{v}").to_lowercase()).collect(),
        v => vec![format!("{v}").to_lowercase()],
    }
}

#[test]
fn a_mod_skin_reaches_a_network_guest() {
    let Some(game) = game() else {
        eprintln!("OXIII_GAME not set: nothing to check");
        return;
    };
    let mut host = prock03(&game, vec![change(Some("Spads"), Some("XIIIPersos.Skins.SpadsNeigeTex"))]);
    let guard = host.find_placed("gardien_lostpass", None, 0.0)[0];
    let host_skins = skins(&host, guard);
    assert!(host_skins.iter().any(|s| s.contains("spadsneigetex")), "{host_skins:?}");
    // The host queues the message when the guard's channel opens for guest 0, and only then.
    host.mod_net_channel_actor(Some(0), 9, guard);
    assert_eq!(host.mods.net_skin_outbox, vec![(0, 9, guard)]);
    let text = host.mod_net_skins_message(guard, 9).expect("a skins message");
    let (ch, sent) = parse_skins_message(&text).expect("a readable message");
    assert_eq!(ch, 9);
    assert!(sent.skins.iter().any(|s| s.to_lowercase().contains("spadsneigetex")), "{text}");

    // The guest: same map, no change made (a network client never makes them).
    let mut guest = prock03(&game, Vec::new());
    let guest_guard = guest.find_placed("gardien_lostpass", None, 0.0)[0];
    assert_ne!(skins(&guest, guest_guard), host_skins, "negative control: the guest starts without the mod's skin");
    // A message for another class is ignored (a channel given to another actor since).
    let mut other = sent.clone();
    other.class = "Engine.Pickup".into();
    guest.mod_net_apply_skins(guest_guard, &other);
    assert_ne!(skins(&guest, guest_guard), host_skins);
    // The message arrives before the actor: kept, then written when its channel gets the actor.
    guest.mods.net_skin_pending.insert(ch, sent.clone());
    guest.mod_net_channel_actor(None, ch, guest_guard);
    assert_eq!(skins(&guest, guest_guard), host_skins);
    assert!(guest.mods.net_skin_pending.is_empty());

    // Negative controls on the host: a new look without a skin sends nothing (the class's own
    // skins, the same on both sides), and an actor without a mod's skin sends nothing.
    let mut plain = prock03(&game, vec![change(Some("Jones"), None)]);
    let g = plain.find_placed("gardien_lostpass", None, 0.0)[0];
    assert!(plain.mod_net_skins_message(g, 1).is_none());
    plain.mod_net_channel_actor(Some(0), 1, g);
    assert!(plain.mods.net_skin_outbox.is_empty());
}
