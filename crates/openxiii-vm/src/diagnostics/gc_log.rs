//! Lines of the garbage collector log of UGameEngine::LoadMap, without changing anything in what the VM
//! keeps or forgets (class_gc.rs, load_order.rs):
//! - UObject::CollectGarbage: "Collecting garbage";
//! - UObject::PurgeGarbage: "Purging garbage", then ULinkerLoad::Destroy
//!   of each purged linker: "%1.1fms Unloading: Package X";
//! - the end of PurgeGarbage writes "Garbage: objects: %i->%i; refs: %i" (objects of GObjObjects,
//!   references seen): these counts are not reproduced, since the VM does not keep every UObject
//!   (textures, sounds, meshes…). The line is omitted; the comparison tools ignore it, like
//!   the time in milliseconds (appSeconds × 1000, the machine's clock).
//!
//! LoadMap calls CollectGarbage(RF_Native) twice when the map changes: GC1 (0x1037C0xx)
//! once the old level is released, which purges the linkers of the new map and of its imports
//! still missing (created without an object by the package check); GC2 after the new
//! level is loaded, which purges the packages of the old map that only it used. On the first
//! map and when reloading the same map, only GC2 happens, with no package to unload.
//! The order of the "Unloading" lines of one GC follows GObjObjects in the original and changes from one game to
//! another (reused slots): here, the order in which the imports are discovered.

use crate::Vm;
use std::collections::HashSet;

impl Vm {
    /// Packages of a map and of its imports, step by step (each linker creates those of its
    /// imports), in discovery order. The name is that of the first reference (case of
    /// the FName: the import "SSH1_pack", not the file SSH1_Pack.utx); the map itself keeps
    /// that of its file.
    pub(crate) fn gc_package_closure(&mut self, map: &str) -> Vec<String> {
        let shown = self.library.as_ref().and_then(|l| l.file_of(map)).and_then(|p| p.file_stem()).and_then(|s| s.to_str()).map(str::to_string);
        self.gc_closure_from(vec![shown.unwrap_or_else(|| map.to_string())])
    }

    /// Packages the engine always holds: those of the code packages that have their native DLL
    /// (Core, Engine, XIII…, loaded by UEngine::Init) and everything they import. A code
    /// package without a DLL (Gameplay) goes away with the last map that imported it.
    pub(crate) fn gc_held_closure(&mut self) -> Vec<String> {
        let system = self.library.as_ref().and_then(|l| l.file_of("Core")).and_then(|p| p.parent()).map(|p| p.to_path_buf());
        let boot: Vec<String> = self
            .packages
            .iter()
            .enumerate()
            .filter(|(i, p)| Some(*i) != self.map_package && !p.name.starts_with(char::from(1u8)))
            .map(|(_, p)| p.name.clone())
            .filter(|n| system.as_ref().is_some_and(|d| d.join(format!("{n}.dll")).exists()))
            .collect();
        self.gc_closure_from(boot)
    }

    pub(crate) fn gc_closure_from(&mut self, start: Vec<String>) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        let mut todo: Vec<String> = start.into_iter().rev().collect();
        while let Some(name) = todo.pop() {
            if !seen.insert(name.to_ascii_lowercase()) {
                continue;
            }
            let Some(path) = self.library.as_ref().and_then(|l| l.file_of(&name)).cloned() else { continue };
            out.push(name.clone());
            let key = name.to_ascii_lowercase();
            let imports = match self.gc_imports_cache.get(&key) {
                Some(v) => v.clone(),
                None => {
                    let v = std::fs::read(&path)
                        .ok()
                        .and_then(|data| openxiii_package::Package::parse(&data).ok())
                        .map(|p| {
                            p.imports
                                .iter()
                                .filter(|i| i.package == 0 && p.name(i.class_name).eq_ignore_ascii_case("Package"))
                                .map(|i| p.name(i.object_name).to_string())
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default();
                    self.gc_imports_cache.insert(key, v.clone());
                    v
                }
            };
            for i in imports.into_iter().rev() {
                todo.push(i);
            }
        }
        out
    }

    /// "Collecting garbage", "Purging garbage" and one "Unloading: Package X" per package.
    pub(crate) fn gc_log(&mut self, unloaded: &[String]) {
        self.log.push("Log: Collecting garbage".into());
        self.log.push("Log: Purging garbage".into());
        let ms = self.app_seconds * 1000.0;
        for p in unloaded {
            self.log.push(format!("Log: {ms:.1}ms Unloading: Package {p}"));
        }
    }

    /// Packages of the host driver's MasterMap (UGameEngine::BuildServerMasterMap:
    /// the map, its loaded imports, the ServerPackages, the game's package). Empty outside a host.
    pub(crate) fn gc_master_packages(&self) -> Vec<String> {
        match self.ipdrv.net_driver.as_ref() {
            Some(d) if d.server_conn.is_none() => d.master_map.list.iter().map(|p| p.name.clone()).collect(),
            _ => Vec::new(),
        }
    }

    /// Packages of the map of the pending level's connection (the joining client): its driver
    /// moves to the new level (LoadMap, GLevel->NetDriver = Pending->NetDriver) and stays
    /// alive during both GCs.
    pub(crate) fn gc_net_held_packages(&self) -> Vec<String> {
        self.ipdrv.net_driver.as_ref().and_then(|d| d.server_conn.as_ref()).and_then(|c| c.index.as_ref()).map_or_else(Vec::new, |i| i.packages.iter().map(|p| p.name.clone()).collect())
    }

    /// Travel to another map (`old` → `new`): GC1 right away, GC2 kept for the end of
    /// loading (`load_map`).
    ///
    /// In LAN (operator<<(FArchive&, FPackageInfo&), called by UPackageMap::Serialize during
    /// GC marking: Parent and Linker of each package of the map):
    /// - `master`: packages of the host's MasterMap. LoadMap destroys the old driver before GC1
    ///   those that neither the code nor the old map hold (GamePlay, ServerPackage
    ///   without a DLL, if the old map does not import it) go at GC1. The new driver is only created
    ///   after GC2 (check(!GLevel->NetDriver), Listen afterwards), with no effect on the GCs.
    /// - `held`: packages of the map of the pending connection (client): held at GC1 and at GC2.
    pub(crate) fn gc_log_travel(&mut self, old: &str, new: &str, master: &[String], held: &[String]) {
        let code = self.gc_held_closure();
        let mut old_set = self.gc_package_closure(old);
        let mut new_set = self.gc_package_closure(new);
        let has = |set: &[String], p: &str| set.iter().any(|x| x.eq_ignore_ascii_case(p));
        let held_frames = |p: &String| has(&code, p) || has(held, p);
        old_set.retain(|p| !held_frames(p));
        new_set.retain(|p| !held_frames(p));
        let mut gc1: Vec<String> = new_set.iter().filter(|p| !has(&old_set, p)).cloned().collect();
        let master_only = self.gc_closure_from(master.to_vec());
        for p in master_only {
            if !held_frames(&p) && !has(&old_set, &p) && !has(&gc1, &p) {
                gc1.push(p);
            }
        }
        let gc2: Vec<String> = old_set.iter().filter(|p| !has(&new_set, p)).cloned().collect();
        self.gc_log(&gc1);
        self.gc_pending_unload = Some(gc2);
    }
}

#[cfg(test)]
mod tests {
}
