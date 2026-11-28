use super::*;
use crate::types::{ClassDef, FunctionDef, Type};

#[test]
fn added_and_renamed_functions_dispatch_by_their_current_name() {
    let mut vm = Vm::new();
    vm.classes.push(ClassDef {
        name: "MapInfo".into(), package: 0, export: 0, super_name: None, super_class: None,
        props: vec![], prop_index: HashMap::default(), functions: HashMap::default(),
        states: HashMap::default(), cdo: usize::MAX, linked: true, flags: 0,
        config_name: String::new(), probe_mask: 0,
    });
    vm.functions.push(FunctionDef {
        name: "FindPlayerController".into(), owner: 0, state: None, package: 0, export: 0,
        flags: openxiii_script::objects::func::SINGULAR, native: 0, params: vec![], locals: vec![],
        local_index: HashMap::default(), ret: Some(Type::Int), code: None,
    });
    let receiver = 0;
    let player = 1;
    vm.coop.on = true;
    vm.coop.players.push(player);
    // An already active singular function falls through to its typed zero if no hook answers.
    vm.in_singular.insert(receiver);
    assert_eq!(vm.call(0, receiver, vec![]).unwrap().0, Value::Object(Some(player)));
    vm.functions[0].name.replace_range(19..20, "x");
    assert_eq!(vm.call(0, receiver, vec![]).unwrap().0, Value::Int(0));
    vm.functions[0].name.replace_range(19..20, "r");
    assert_eq!(vm.call(0, receiver, vec![]).unwrap().0, Value::Object(Some(player)));
    // The candidate search also accepts other case spellings. MapInfo's answer is exact.
    vm.functions[0].name.make_ascii_lowercase();
    assert_eq!(vm.call(0, receiver, vec![]).unwrap().0, Value::Int(0));
}
