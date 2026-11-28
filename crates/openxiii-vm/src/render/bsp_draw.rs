//! BSP surfaces drawn every frame: end (Engine.dll), called by the
//! visibility traversal (`visibility.rs`) for each coplanar node that has
//! passed the facing test and the sphere test in the volumes of its zone.
//!
//! Such a node is put into the draw record of its surface (record per surface and
//! per zone, opaque ones in a list, materials to sort separately), except:
//! - PF_FakeBackdrop: the zone's sky scene replaces it if ShowFlags has 4 and SHOW_Backdrop
//!   (0x800) and the zone has a SkyZone; otherwise it is drawn (SceneNode+0x1DC, which is 2 for
//!   an FLightMapSceneNode, is 0 or 1 for a camera);
//! - PF_Mirrored: the mirror scene (no XIII map has one);
//! - PF_Portal portal (non-zero view zone) marked PF_Invisible, unless it has no Actor (drawn);
//! - PF_AntiPortal anti-portal marked PF_Invisible, or the player's RendMap (+0x39C) equal to 1;
//! - other PF_Invisible surface if ShowFlags has SHOW_Backdrop (always in game);
//! - record zone equal to InvisibleZone (+0x1D4).
//! Only the nodes of the records are drawn. Each scene has its list (`bsp_views`, one boolean per
//! node): those of the last root scene (render_scene: a player's view, DrawPortal, one
//! view per window in split screen) and of each sky scene it opens. The drawing
//! reads them right after the render_scene of its view.

use crate::value::ObjId;
use crate::Vm;

const PF_INVISIBLE: u32 = 0x1;
const PF_ANTIPORTAL: u32 = 0x10;
const PF_FAKE_BACKDROP: u32 = 0x80;
const PF_PORTAL: u32 = 0x400_0000;
const PF_MIRRORED: u32 = 0x800_0000;
const SHOW_BACKDROP: u32 = 0x800;

/// List of the BSP nodes drawn by a scene.
#[derive(Debug, Clone, Default)]
pub struct BspView {
    /// None: the root scene (a player's view, DrawPortal); Some(zone): a sky scene
    /// it opened (FSkySceneNode of that zone).
    pub sky_zone: Option<i32>,
    pub visible: Vec<bool>,
    /// Actors drawn by this scene (FDynamicActor::Render), in order.
    pub actors: Vec<ObjId>,
    /// Terrains rendered by this scene (ATerrainInfo::Render, loop).
    pub terrains: Vec<ObjId>,
}

impl Vm {
    /// Start of render_scene: new root scene, with no node drawn, and its sky scenes.
    pub(crate) fn bsp_draw_begin(&mut self) {
        let n = self.level_bsp.as_ref().map_or(0, |m| m.nodes.len());
        self.bsp_views = vec![BspView { sky_zone: None, visible: vec![false; n], actors: Vec::new(), terrains: Vec::new() }];
    }

    /// BSP nodes drawn by the last root scene (render_scene), or by its sky scene
    /// `sky_zone`.
    pub fn bsp_visible(&self, sky_zone: Option<i32>) -> Option<&[bool]> {
        self.bsp_views.iter().find(|v| v.sky_zone == sky_zone).map(|v| v.visible.as_slice())
    }

    /// Node `i_node` (side `u` of the viewpoint) kept by in the root scene or
    /// its sky scene (`child`): put into the record of its surface if it must be drawn.
    /// `view_zone`, `invisible_zone`, `show_flags` and `viewer` are those of the scene.
    #[allow(clippy::too_many_arguments)]
    /// Returns None if the node is not drawn, Some(true) if its surface has a material to sort
    /// (it goes into the translucent list, not into the record), Some(false) if it is put into the
    /// record. In both cases it is marked as drawn in its scene's list.
    pub(crate) fn bsp_draw_node(&mut self, child: bool, i_node: i32, u: usize, view_zone: i32, invisible_zone: i32, show_flags: u32, viewer: Option<ObjId>, portal_without_actor: bool) -> Option<bool> {
        let (zone_u, flags, surf) = self.level_bsp.as_ref().and_then(|m| {
            let n = m.nodes.get(i_node as usize)?;
            let surf = m.surfs.get(n.surf as usize)?;
            Some((n.zone[u] as i32, surf.poly_flags, n.surf as usize))
        })?;
        let backdrop = show_flags & SHOW_BACKDROP != 0;
        let draw = if flags & PF_FAKE_BACKDROP != 0 {
            // FSkySceneNode of the zone: the surface is not drawn. Outside the editor, the zone is that of the
            // node's side even if the view is in zone 0 (zone 0 is only taken in the GIsEditor branch).
            let sky = show_flags & 4 != 0 && backdrop && {
                let za = usize::try_from(zone_u).ok().and_then(|z| self.zone_actors.get(z).copied().flatten()).or(self.level);
                za.is_some_and(|a| self.propr(a, "SkyZone").as_object().is_some())
            };
            !sky
        } else if flags & PF_MIRRORED != 0 {
            false
        } else if view_zone != 0 && flags & PF_PORTAL != 0 {
            flags & PF_INVISIBLE == 0 || portal_without_actor
        } else if flags & PF_ANTIPORTAL != 0 {
            flags & PF_INVISIBLE == 0 && viewer.is_none_or(|v| self.propr(v, "RendMap").as_int() != 1)
        } else {
            flags & PF_INVISIBLE == 0 || !backdrop
        };
        if !draw {
            return None;
        }
        let zone = if view_zone == 0 { 0 } else { zone_u };
        if zone == invisible_zone {
            return None;
        }
        // Kept node (record or translucent list): the drawing keeps it, the scenery cut
        // (fog end, zones, portals) only draws those.
        let sorted = self.surf_requires_sorting(surf);
        let at = self.bsp_view_at(child.then_some(view_zone));
        if let Some(v) = self.bsp_views[at].visible.get_mut(i_node as usize) {
            *v = true;
        }
        Some(sorted)
    }

    /// List of scene `sky_zone` (None: the root), created when needed.
    pub(crate) fn bsp_view_at(&mut self, sky_zone: Option<i32>) -> usize {
        match self.bsp_views.iter().position(|v| v.sky_zone == sky_zone) {
            Some(at) => at,
            None => {
                let n = self.level_bsp.as_ref().map_or(0, |m| m.nodes.len());
                self.bsp_views.push(BspView { sky_zone, visible: vec![false; n], actors: Vec::new(), terrains: Vec::new() });
                self.bsp_views.len() - 1
            }
        }
    }

    /// UMaterial::RequiresSorting of the material of surface `surf` (computed once per map;
    /// the script's texture replacements are not followed).
    fn surf_requires_sorting(&mut self, surf: usize) -> bool {
        if self.surf_sorting.is_none() {
            let mut out = Vec::new();
            if let (Some(bsp), Some(pi), Some(mut library)) = (self.level_bsp.as_ref(), self.map_package, self.library.take()) {
                let lp = &self.packages[pi];
                let mut materials = openxiii_assets::material::Materials::new();
                let mut cache: std::collections::HashMap<i32, bool> = std::collections::HashMap::new();
                for s in &bsp.surfs {
                    let v = *cache.entry(s.material).or_insert_with(|| {
                        let name = match openxiii_package::ObjectRef::from(s.material) {
                            openxiii_package::ObjectRef::None => return false,
                            openxiii_package::ObjectRef::Export(_) => format!("{}.{}", lp.name, lp.package.object_path(s.material.into())),
                            _ => lp.package.object_path(s.material.into()),
                        };
                        materials.requires_sorting(&mut library, &Some(name))
                    });
                    out.push(v);
                }
                self.library = Some(library);
            }
            self.surf_sorting = Some(out);
        }
        self.surf_sorting.as_ref().and_then(|v| v.get(surf).copied()).unwrap_or(false)
    }
}
