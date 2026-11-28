//! Archive values and Recent only; no channel transport operation.
use super::*;
use crate::netbits::BitReader;

pub(crate) enum SpawnArchive<'a>{Write(&'a mut BitWriter),Read(&'a mut BitReader)}
impl Vm {
    /// Shared by the spawn writer and decoder. Position is always archived
    /// and copied first; a missing rotation flag leaves both rotation states intact.
    pub(crate) fn spawn_record(&mut self,mut archive:SpawnArchive<'_>,position:&mut Value,rotation:&mut Value,mut recent:Option<(&mut [Value],Option<usize>,Option<usize>)>,rotate:bool){
        let index=crate::net_map::MapIndex::default();
        match &mut archive {
            SpawnArchive::Write(w)=>{self.net_write_value(w,&index,&HashMap::new(),&NetTy::Vector,position);},
            SpawnArchive::Read(r)=>{*position=self.net_read_value(r,&index,&|_|None,&NetTy::Vector,&Value::Void);},
        }
        if let Some((values,p,_))=recent.as_mut(){if !values.is_empty(){if let Some(p)=*p{values[p]=position.clone()}}}
        if rotate {
            match &mut archive {
                SpawnArchive::Write(w)=>{self.net_write_value(w,&index,&HashMap::new(),&NetTy::Rotator,rotation);},
                SpawnArchive::Read(r)=>{*rotation=self.net_read_value(r,&index,&|_|None,&NetTy::Rotator,&Value::Void);},
            }
            if let Some((values,_,r))=recent.as_mut(){if !values.is_empty(){if let Some(r)=*r{values[r]=rotation.clone()}}}
        }
    }
}

