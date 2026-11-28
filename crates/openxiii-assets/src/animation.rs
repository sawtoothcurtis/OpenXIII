//! XIII's skeletal animations (MeshAnimation class), a compressed format specific to the game.
//!
//! After the properties: i32 (0), RefBones (compact n × [compact name, u32 flags, i32 parent]),
//! Moves (compact n), each: RootSpeed3D (3 × f32), TrackTime f32, StartBone i32, Flags u32,
//! BoneIndices (compact n × i32), tracks, one per bone of BoneIndices (compact n × [first
//! position u16, first quaternion u16,
//! quaternion count u16, position count u16]), quaternions (compact n × 4 × i16 / 32767),
//! positions (compact n × 3 × f32), rotation key times (compact n × f32);
//! then AnimSeqs (compact n × [name, groups (compact n × name), StartFrame i32, NumFrames i32,
//! notifies (compact n × [Time f32, function (name)]; no object in XIII), Rate f32]).

use openxiii_package::{Package, Reader, Result};

#[derive(Debug, Clone)]
pub struct Track {
    /// First position of the track (tracks follow the bone order of BoneIndices).
    pub pos_start: u16,
    pub quat_start: u16,
    pub quat_count: u16,
    pub pos_count: u16,
}

#[derive(Debug, Clone)]
pub struct Move {
    pub root_speed: [f32; 3],
    pub track_time: f32,
    pub start_bone: i32,
    pub flags: u32,
    pub bone_indices: Vec<i32>,
    pub tracks: Vec<Track>,
    pub quats: Vec<[f32; 4]>,
    /// Quaternions as stored (4 × i16), which the engine converts on every read.
    pub raw_quats: Vec<[i16; 4]>,
    pub positions: Vec<[f32; 3]>,
    pub times: Vec<f32>,
}

#[derive(Debug, Clone)]
pub struct Sequence {
    pub name: i32,
    pub groups: Vec<i32>,
    pub start_frame: i32,
    pub num_frames: i32,
    pub rate: f32,
    /// (time, script function called).
    pub notifies: Vec<(f32, i32)>,
}

#[derive(Debug, Clone)]
pub struct MeshAnimation {
    /// (name, flags, parent) of the animated bones.
    pub bones: Vec<(i32, u32, i32)>,
    pub moves: Vec<Move>,
    pub sequences: Vec<Sequence>,
}

fn f32r(r: &mut Reader) -> Result<f32> {
    Ok(f32::from_bits(r.u32()?))
}

impl MeshAnimation {
    pub fn read(package: &Package, data: &[u8], index: usize) -> Result<Self> {
        let object = package.object_data(data, index)?;
        let export = &package.exports[index];
        Self::read_native(&data[object.end..export.serial_offset as usize + export.serial_size as usize])
    }

    /// Native part of the object (after the properties), up to the end of the export.
    pub fn read_native(data: &[u8]) -> Result<Self> {
        let end = data.len();
        let mut r = Reader::new(data, 0);
        r.i32()?;
        let bones = (0..r.compact()?).map(|_| Ok((r.compact()?, r.u32()?, r.i32()?))).collect::<Result<Vec<_>>>()?;
        let moves = (0..r.compact()?)
            .map(|_| {
                let root_speed = [f32r(&mut r)?, f32r(&mut r)?, f32r(&mut r)?];
                let track_time = f32r(&mut r)?;
                let start_bone = r.i32()?;
                let flags = r.u32()?;
                let bone_indices = (0..r.compact()?).map(|_| r.i32()).collect::<Result<Vec<_>>>()?;
                let tracks = (0..r.compact()?)
                    .map(|_| Ok(Track { pos_start: r.u16()?, quat_start: r.u16()?, quat_count: r.u16()?, pos_count: r.u16()? }))
                    .collect::<Result<Vec<_>>>()?;
                let raw_quats =
                    (0..r.compact()?).map(|_| Ok([r.u16()? as i16, r.u16()? as i16, r.u16()? as i16, r.u16()? as i16])).collect::<Result<Vec<_>>>()?;
                let quats = raw_quats.iter().map(|q| q.map(|v| v as f32 / 32767.0)).collect();
                let positions = (0..r.compact()?).map(|_| Ok([f32r(&mut r)?, f32r(&mut r)?, f32r(&mut r)?])).collect::<Result<Vec<_>>>()?;
                let times = (0..r.compact()?).map(|_| f32r(&mut r)).collect::<Result<Vec<_>>>()?;
                Ok(Move { root_speed, track_time, start_bone, flags, bone_indices, tracks, quats, raw_quats, positions, times })
            })
            .collect::<Result<Vec<_>>>()?;
        let sequences = (0..r.compact()?)
            .map(|_| {
                let name = r.compact()?;
                let groups = (0..r.compact()?).map(|_| r.compact()).collect::<Result<Vec<_>>>()?;
                let start_frame = r.i32()?;
                let num_frames = r.i32()?;
                let notifies = (0..r.compact()?).map(|_| Ok((f32r(&mut r)?, r.compact()?))).collect::<Result<Vec<_>>>()?;
                let rate = f32r(&mut r)?;
                Ok(Sequence { name, groups, start_frame, num_frames, rate, notifies })
            })
            .collect::<Result<Vec<_>>>()?;
        if r.pos != end {
            return Err(openxiii_package::Error::Eof { offset: r.pos, wanted: end - r.pos });
        }
        let anim = MeshAnimation { bones, moves, sequences };
        anim.validate()?;
        Ok(anim)
    }

    fn validate(&self) -> Result<()> {
        for m in &self.moves {
            let quats: usize = m.tracks.iter().map(|t| t.quat_count as usize).sum();
            let positions: usize = m.tracks.iter().map(|t| t.pos_count as usize).sum();
            if quats != m.quats.len() || positions != m.positions.len() || m.times.len() != quats {
                return Err(openxiii_package::Error::BadIndex(quats as i32));
            }
        }
        Ok(())
    }
}
