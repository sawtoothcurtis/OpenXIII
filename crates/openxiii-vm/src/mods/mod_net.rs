//! Mods build only: the mods in network games.
//!
//! The engine designates the objects a server and its clients share by their rank in the
//! package map (UPackageMap: each package's exports, the packages in the server's order). The
//! mods' objects (new classes "Mod.<Name>", sounds, textures, models, animations) are in no
//! package of the game: they are given a package of their own, "Mod", added at the end of the
//! map. Its exports are those objects in name order and its GUID identifies the mods (each
//! mod's folder name, version and fingerprint, in load order): a client is given the same ranks
//! only if it has the same mods.
//!
//! - The server announces the package with the others ("USES … PKG=Mod"): a game without mods,
//!   including the original game, cannot find it and does not join.
//! - A client with mods adds its mods to its LOGIN URL (`?OXIIIMods=<GUID>?OXIIIModList=…`).
//!   The server refuses a client whose mods differ, or that has none, with a FAILURE message
//!   that says which mods it needs; the game shows it ("Rejected By Server").
//! - A client with mods that joins a server without mods is let in; its log warns that its mods
//!   change its own game only.

use std::sync::{Arc, OnceLock};

use openxiii_mods::ModId;

use crate::net_map::{assign_names, MapIndex, PackageInfo, PackageMap};
use crate::Vm;

/// Name of the package of the mods' objects in the package map.
pub const PACKAGE: &str = "Mod";
/// LOGIN URL option: the GUID of the client's mods.
pub const OPTION_GUID: &str = "OXIIIMods";
/// LOGIN URL option: the client's mods, for the message of a refusal.
pub const OPTION_LIST: &str = "OXIIIModList";

/// RF_LoadForClient | RF_LoadForServer: the exports both sides create.
const LOAD_FOR_BOTH: u32 = 0x30000;

/// The mods of this game, as the network sees them.
#[derive(Debug, Clone, Default)]
pub struct ModNetwork {
    pub ids: Vec<ModId>,
    pub guid: [u8; 16],
    /// ("Mod.<Name>"…, class) in name order (case ignored).
    pub objects: Vec<(String, String)>,
    /// Names the package brings (the objects' own names), in order.
    pub names: Vec<String>,
}

impl ModNetwork {
    pub fn new(ids: Vec<ModId>, objects: Vec<(String, &str)>) -> ModNetwork {
        let mut objects: Vec<(String, String)> = objects.into_iter().map(|(p, c)| (p, c.to_string())).collect();
        objects.sort_by_key(|(p, _)| p.to_lowercase());
        objects.dedup_by(|a, b| a.0.eq_ignore_ascii_case(&b.0));
        let mut names: Vec<String> = Vec::new();
        for (p, _) in &objects {
            let short = p.rsplit('.').next().unwrap_or(p).to_string();
            if !names.iter().any(|n| n.eq_ignore_ascii_case(&short)) {
                names.push(short);
            }
        }
        let guid = openxiii_mods::mods_guid(&ids);
        ModNetwork { ids, guid, objects, names }
    }
}

static GLOBAL: OnceLock<Arc<ModNetwork>> = OnceLock::new();

/// Sets the network identity of the mods (once, at startup).
pub fn set_mod_network(ids: Vec<ModId>, objects: Vec<(String, &'static str)>) {
    let _ = GLOBAL.set(Arc::new(ModNetwork::new(ids, objects)));
}

/// The mods' list in a URL option: "name:version:fingerprint,…", each part with its
/// characters other than letters, digits, '.', '-' and '_' written %XX.
pub fn encode_list(ids: &[ModId]) -> String {
    let enc = |s: &str| s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"._-".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect::<String>();
    ids.iter().map(|i| format!("{}:{}:{}", enc(&i.name), enc(&i.version), enc(&i.fingerprint))).collect::<Vec<_>>().join(",")
}

/// Reverse of [`encode_list`].
pub fn decode_list(s: &str) -> Vec<ModId> {
    let dec = |s: &str| {
        let b = s.as_bytes();
        let mut out = Vec::new();
        let mut i = 0;
        while i < b.len() {
            if b[i] == b'%' && i + 2 < b.len() {
                if let Some(v) = std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()) {
                    out.push(v);
                    i += 3;
                    continue;
                }
            }
            out.push(b[i]);
            i += 1;
        }
        String::from_utf8_lossy(&out).into_owned()
    };
    s.split(',')
        .filter(|x| !x.is_empty())
        .filter_map(|x| {
            let mut p = x.split(':');
            Some(ModId { name: dec(p.next()?), version: dec(p.next()?), fingerprint: dec(p.next()?) })
        })
        .collect()
}

/// Value of option `key` in a URL ("Map?A=1?B=2"), case ignored.
fn url_option<'a>(url: &'a str, key: &str) -> Option<&'a str> {
    url.split('?').skip(1).find_map(|o| {
        let (k, v) = o.split_once('=')?;
        k.eq_ignore_ascii_case(key).then_some(v)
    })
}

impl Vm {
    /// Network identity of the mods of this Vm only (tests).
    pub fn set_mod_network_here(&mut self, network: ModNetwork) {
        self.mods.network = Some(Arc::new(network));
    }

    /// The mods of this game on the network (None without mods).
    pub(crate) fn mod_network(&self) -> Option<Arc<ModNetwork>> {
        self.mods.network.clone().or_else(|| GLOBAL.get().cloned()).filter(|n| !n.ids.is_empty())
    }

    fn mod_package_info(n: &ModNetwork) -> PackageInfo {
        PackageInfo {
            name: PACKAGE.into(),
            url: String::new(),
            guid: n.guid,
            file_size: 0,
            object_base: -1,
            object_count: -1,
            name_index: -1,
            name_count: -1,
            local_generation: 0,
            remote_generation: 0,
            package_flags: 0,
            generations: vec![(n.objects.len() as i32, n.names.len() as i32)],
            export_count: n.objects.len() as i32,
            names_total: n.names.len() as i32,
        }
    }

    /// Server, end of BuildServerMasterMap: the mods' package after the game's ones.
    pub(crate) fn mod_net_add_package(&self, map: &mut PackageMap) {
        let Some(n) = self.mod_network() else { return };
        if map.list.iter().any(|p| p.name.eq_ignore_ascii_case(PACKAGE)) {
            return;
        }
        map.list.push(Self::mod_package_info(&n));
        map.compute();
    }

    /// Client, VerifyNetPackageMap at JOIN: the local counts of the announced "Mod" package.
    pub(crate) fn mod_net_local_info(&self, info: &mut PackageInfo) {
        if !info.name.eq_ignore_ascii_case(PACKAGE) {
            return;
        }
        if let Some(n) = self.mod_network() {
            let local = Self::mod_package_info(&n);
            info.generations = local.generations;
            info.export_count = local.export_count;
            info.names_total = local.names_total;
        }
    }

    /// Client, WELCOME: whether an announced package is the mods' one with the same mods
    /// (None: not the mods' package).
    pub(crate) fn mod_net_package_here(&self, name: &str, guid: &[u8; 16]) -> Option<bool> {
        name.eq_ignore_ascii_case(PACKAGE).then(|| self.mod_network().is_some_and(|n| &n.guid == guid))
    }

    /// Client, WELCOME: a client with mods on a server without mods is warned in the log.
    pub(crate) fn mod_net_welcome_check(&mut self, announced: &[PackageInfo]) {
        if self.mod_network().is_some() && !announced.iter().any(|p| p.name.eq_ignore_ascii_case(PACKAGE)) {
            self.mod_report("warning: the server has no mods: your mods change your own game only (values, sounds and models the server does not know are not shared)".into());
        }
    }

    /// MapIndex of a package map: the exports, names and classes of the mods' package.
    pub(crate) fn mod_net_index(&self, index: &mut MapIndex) {
        let Some(n) = self.mod_network() else { return };
        let Some(k) = index.packages.iter().position(|p| p.name.eq_ignore_ascii_case(PACKAGE)) else { return };
        let count = index.packages[k].object_count as usize;
        let base = index.packages[k].object_base;
        let name_base = index.packages[k].name_base;
        let name_count = index.packages[k].name_count;
        let mut exports = Vec::new();
        for (i, (path, class)) in n.objects.iter().take(count).enumerate() {
            index.by_path.entry(path.to_lowercase()).or_insert(base + i as u32);
            if class == "Class" {
                let short = path.rsplit('.').next().unwrap_or(path).to_lowercase();
                index.class_package.entry(short.clone()).or_insert_with(|| PACKAGE.to_string());
                if let Some(parent) = self.class(path).and_then(|c| self.mod_class_parent(c)) {
                    index.class_super.entry(short).or_insert_with(|| self.classes[parent].name.clone());
                }
            }
            exports.push((path.clone(), class.clone()));
        }
        assign_names(&mut index.names, name_base, name_count, n.names.iter().map(String::as_str));
        let p = &mut index.packages[k];
        p.export_flags = vec![LOAD_FOR_BOTH; exports.len()];
        p.exports = exports;
        p.names = n.names.iter().take(name_count as usize).cloned().collect();
    }

    /// Client, LOGIN: the mods' options added to the URL sent to the server.
    pub(crate) fn mod_login_url(&self, url: String) -> String {
        match self.mod_network() {
            Some(n) => format!("{url}?{OPTION_GUID}={}?{OPTION_LIST}={}", crate::net_map::guid_string(&n.guid), encode_list(&n.ids)),
            None => url,
        }
    }

    /// Server, LOGIN: the refusal of a client whose mods differ from the server's (None: let in).
    pub(crate) fn mod_login_refusal(&self, url: &str) -> Option<String> {
        let n = self.mod_network()?;
        let theirs = url_option(url, OPTION_GUID).and_then(crate::net_map::parse_guid);
        if theirs == Some(n.guid) {
            return None;
        }
        let ours: Vec<String> = n.ids.iter().map(|i| format!("{} {}", i.name, i.version)).collect();
        let message = match theirs {
            None => format!("This server uses mods ({}): join it with OpenXIII (improved build) and the same mods", ours.join(", ")),
            Some(_) => {
                let client = url_option(url, OPTION_LIST).map(decode_list).unwrap_or_default();
                let diffs = openxiii_mods::mods_differences(&n.ids, &client);
                format!("Your mods differ from the server's ({}): {}", ours.join(", "), diffs.join("; "))
            }
        };
        Some(message)
    }

    /// A mod's new class as the network names it: "Mod.<Name>" (None for the game's classes).
    pub(crate) fn mod_class_net_path(&self, c: crate::value::ClassId) -> Option<String> {
        self.mod_class_package(c).map(|p| format!("{p}.{}", self.classes[c].name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: &str, v: &str) -> ModId {
        ModId { name: n.into(), version: v.into(), fingerprint: "00AA".into() }
    }

    #[test]
    fn list_in_url_round_trip() {
        let ids = vec![id("my mod?x", "1.0"), id("b", "2,3:4%")];
        let s = encode_list(&ids);
        assert!(!s.contains('?') && !s.contains(' '), "{s}");
        assert_eq!(decode_list(&s), ids);
        assert_eq!(url_option(&format!("DM_Amos?Name=X?{OPTION_LIST}={s}"), OPTION_LIST), Some(s.as_str()));
    }

    /// List sent by a client: a '%' followed by a non-ASCII character is kept as text, without a panic.
    #[test]
    fn list_from_bad_text() {
        assert_eq!(decode_list("a%\u{e9}:1:00AA"), vec![id("a%\u{e9}", "1")]);
        assert_eq!(decode_list("%4\u{e9}:%\u{e9}4:%"), vec![ModId { name: "%4\u{e9}".into(), version: "%\u{e9}4".into(), fingerprint: "%".into() }]);
    }

    #[test]
    fn package_order_names_and_refusals() {
        let n = ModNetwork::new(vec![id("a", "1")], vec![("Mod.Zeta".into(), "Sound"), ("Mod.alpha".into(), "Class"), ("Mod.Beta".into(), "StaticMesh")]);
        let paths: Vec<&str> = n.objects.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(paths, ["Mod.alpha", "Mod.Beta", "Mod.Zeta"]);
        assert_eq!(n.names, ["alpha", "Beta", "Zeta"]);
        let mut vm = Vm::new();
        vm.set_mod_network_here(n.clone());
        // Server: the mods' package at the end of the map.
        let mut map = PackageMap::default();
        vm.mod_net_add_package(&mut map);
        assert_eq!((map.list[0].name.as_str(), map.max_object_index, map.list[0].uses_line().contains("PKG=Mod")), (PACKAGE, 3, true));
        // Refusals.
        let ok = vm.mod_login_url("DM_Amos?Name=X".into());
        assert_eq!(vm.mod_login_refusal(&ok), None);
        let none = vm.mod_login_refusal("DM_Amos?Name=X").unwrap();
        assert!(none.contains("This server uses mods (a 1)"), "{none}");
        let other = ModNetwork::new(vec![id("a", "2")], Vec::new());
        let url = format!("DM_Amos?{OPTION_GUID}={}?{OPTION_LIST}={}", crate::net_map::guid_string(&other.guid), encode_list(&other.ids));
        let differ = vm.mod_login_refusal(&url).unwrap();
        assert!(differ.contains("mod a: version 2 instead of 1"), "{differ}");
        let malformed = format!("DM_Amos?{OPTION_GUID}={}0", crate::net_map::guid_string(&n.guid));
        assert!(vm.mod_login_refusal(&malformed).is_some());
        // Client: the package is ours only with the same GUID.
        assert_eq!(vm.mod_net_package_here("Mod", &n.guid), Some(true));
        assert_eq!(vm.mod_net_package_here("Mod", &other.guid), Some(false));
        assert_eq!(vm.mod_net_package_here("Engine", &n.guid), None);
        // A server without mods lets everyone in.
        assert_eq!(Vm::new().mod_login_refusal("DM_Amos?Name=X"), None);
    }
}
