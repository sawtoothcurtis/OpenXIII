//! SetupSectors: dimensions, bitmaps and sector slots.
//!
//! The factory receives the arguments actually passed to Init. It must
//! produce the consumer's objects; this module does not simulate that callee.

/// Rectangle passed to the initialiser of a new sector, in native order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SectorInit {
    pub offset_x: i32,
    pub offset_y: i32,
    pub quads_x: i32,
    pub quads_y: i32,
}

/// Mutable production state matching only the fields written by SetupSectors.
/// `None` keeps the semantics of a sector allocation with no returned object.
#[derive(Debug, Clone)]
pub struct SetupSectorsState<T> {
    pub heightmap: [i32; 2],
    pub sector_counts: [i32; 2],
    pub sectors: Vec<Option<T>>,
    pub visible: Vec<u32>,
    pub turned: Vec<u32>,
    pub quad_materials: Vec<u8>,
}

/// SetupSectors's own effect, with the two explicit callees of its contract.
/// Texture sizes must be representable non-negative dimensions.
/// The factory is called in Y then X order; old objects are kept
/// without reinitialisation when the slot count already matches.
pub fn setup_sectors<T>(
    state: &mut SetupSectorsState<T>,
    texture_size: Option<[i32; 2]>,
    mut create_sector: impl FnMut(SectorInit) -> Option<T>,
    mut log_turned_growth: impl FnMut(),
) {
    if let Some([w, h]) = texture_size {
        assert!(w >= 0 && h >= 0, "texture dimensions must not be negative");
        let first_dimensions = state.heightmap == [0, 0];
        state.heightmap = [w, h];
        state.sector_counts = [w / 8, h / 8];
        if first_dimensions {
            let pixels = w.checked_mul(h).expect("representable terrain size");
            let words = (pixels / 32) as usize;
            if state.visible.len() < words {
                state.visible.resize(words, u32::MAX);
            }
            if state.turned.len() < words {
                log_turned_growth();
                state.turned.resize(words, 0);
            }
            let materials = (pixels / 2) as usize;
            if state.quad_materials.len() < materials {
                state.quad_materials.resize(materials, 0);
            }
        }
    } else {
        state.heightmap = [0, 0];
        state.sector_counts = [0, 0];
    }

    let [sx, sy] = state.sector_counts;
    let count = sx.checked_mul(sy).expect("number of sectors representable") as usize;
    if state.sectors.len() == count {
        return;
    }
    let pixels = state.heightmap[0].checked_mul(state.heightmap[1])
        .expect("representable terrain size");
    let words = (pixels / 32) as usize;
    if state.visible.len() != words {
        state.visible = vec![u32::MAX; words];
    }
    if state.turned.len() != words {
        state.turned = vec![0; words];
    }
    let materials = (pixels / 2) as usize;
    if state.quad_materials.len() != materials {
        state.quad_materials = vec![0; materials];
    }
    state.sectors.clear();
    for y in 0..sy {
        for x in 0..sx {
            let init = SectorInit {
                offset_x: x * 8,
                offset_y: y * 8,
                quads_x: 7 + i32::from(x < sx - 1),
                quads_y: 7 + i32::from(y < sy - 1),
            };
            state.sectors.push(create_sector(init));
        }
    }
}
