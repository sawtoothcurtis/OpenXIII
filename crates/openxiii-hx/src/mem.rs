//! Memory of sound programs: HXAudio handles raw pointers (object variables,
//! handler stack, parameter block, references passed by address). Each area is
//! a byte region here, and an address is (region << 16) | offset; 0 is the null pointer.
//!
//! String, object or variable slots contain an identifier (≥ 1) into the engine's
//! tables, like the original pointers (0 = null).

#[derive(Debug, Default)]
pub struct Mem {
    regions: Vec<Option<Vec<u8>>>,
    free: Vec<usize>,
    /// String objects (original `char**`: the slot points to an object holding the text).
    strings: Vec<Option<Vec<u8>>>,
    free_strings: Vec<usize>,
}

pub type Addr = u32;

impl Mem {
    pub fn new() -> Self {
        Mem { regions: vec![None], free: Vec::new(), strings: vec![None], free_strings: Vec::new() }
    }

    /// Reserves a zeroed region of `size` bytes; returns its base address.
    pub fn alloc(&mut self, size: usize) -> Addr {
        let data = vec![0u8; size];
        let index = match self.free.pop() {
            Some(i) => {
                self.regions[i] = Some(data);
                i
            }
            None => {
                self.regions.push(Some(data));
                self.regions.len() - 1
            }
        };
        (index as u32) << 16
    }

    pub fn alloc_with(&mut self, bytes: &[u8]) -> Addr {
        let a = self.alloc(bytes.len());
        self.write(a, bytes);
        a
    }

    /// Region placed at a given address (tests: same addresses as the original when pointer bits
    /// are read as a value); the bytes before `addr` in the region are zero.
    #[cfg(test)]
    pub fn alloc_at(&mut self, addr: Addr, bytes: &[u8]) {
        let i = (addr >> 16) as usize;
        if self.regions.len() <= i {
            self.regions.resize(i + 1, None);
        }
        assert!(self.regions[i].is_none(), "region {i:#x} already taken");
        self.regions[i] = Some(vec![0u8; (addr & 0xffff) as usize + bytes.len()]);
        self.write(addr, bytes);
    }

    pub fn release(&mut self, addr: Addr) {
        let i = (addr >> 16) as usize;
        if i != 0 && i < self.regions.len() && self.regions[i].is_some() {
            self.regions[i] = None;
            self.free.push(i);
        }
    }

    pub fn region_len(&self, addr: Addr) -> usize {
        self.regions.get((addr >> 16) as usize).and_then(|r| r.as_ref()).map_or(0, |r| r.len())
    }

    fn slice(&self, addr: Addr, n: usize) -> Option<&[u8]> {
        let r = self.regions.get((addr >> 16) as usize)?.as_ref()?;
        let o = (addr & 0xffff) as usize;
        r.get(o..o + n)
    }

    pub fn read(&self, addr: Addr, out: &mut [u8]) {
        match self.slice(addr, out.len()) {
            Some(s) => out.copy_from_slice(s),
            None => out.fill(0),
        }
    }

    pub fn write(&mut self, addr: Addr, bytes: &[u8]) {
        let o = (addr & 0xffff) as usize;
        if let Some(Some(r)) = self.regions.get_mut((addr >> 16) as usize) {
            if let Some(s) = r.get_mut(o..o + bytes.len()) {
                s.copy_from_slice(bytes);
            }
        }
    }

    pub fn fill(&mut self, addr: Addr, n: usize, value: u8) {
        let o = (addr & 0xffff) as usize;
        if let Some(Some(r)) = self.regions.get_mut((addr >> 16) as usize) {
            let end = (o + n).min(r.len());
            if o < end {
                r[o..end].fill(value);
            }
        }
    }

    pub fn u8(&self, addr: Addr) -> u8 {
        let mut b = [0; 1];
        self.read(addr, &mut b);
        b[0]
    }
    pub fn u16(&self, addr: Addr) -> u16 {
        let mut b = [0; 2];
        self.read(addr, &mut b);
        u16::from_le_bytes(b)
    }
    pub fn u32(&self, addr: Addr) -> u32 {
        let mut b = [0; 4];
        self.read(addr, &mut b);
        u32::from_le_bytes(b)
    }
    pub fn f32(&self, addr: Addr) -> f32 {
        f32::from_bits(self.u32(addr))
    }
    pub fn set_u8(&mut self, addr: Addr, v: u8) {
        self.write(addr, &[v]);
    }
    pub fn set_u16(&mut self, addr: Addr, v: u16) {
        self.write(addr, &v.to_le_bytes());
    }
    pub fn set_u32(&mut self, addr: Addr, v: u32) {
        self.write(addr, &v.to_le_bytes());
    }
    pub fn set_f32(&mut self, addr: Addr, v: f32) {
        self.set_u32(addr, v.to_bits());
    }

    /// New (empty) string object.
    pub fn new_string(&mut self, text: &[u8]) -> u32 {
        let t = text.to_vec();
        match self.free_strings.pop() {
            Some(i) => {
                self.strings[i] = Some(t);
                i as u32
            }
            None => {
                self.strings.push(Some(t));
                (self.strings.len() - 1) as u32
            }
        }
    }

    pub fn delete_string(&mut self, id: u32) {
        let i = id as usize;
        if i != 0 && i < self.strings.len() && self.strings[i].is_some() {
            self.strings[i] = None;
            self.free_strings.push(i);
        }
    }

    /// Text of string object `id` (empty if null).
    pub fn string(&self, id: u32) -> Vec<u8> {
        self.strings.get(id as usize).and_then(|s| s.clone()).unwrap_or_default()
    }

    pub fn string_exists(&self, id: u32) -> bool {
        self.strings.get(id as usize).is_some_and(|s| s.is_some())
    }

    pub fn set_string(&mut self, id: u32, text: &[u8]) {
        if let Some(Some(s)) = self.strings.get_mut(id as usize) {
            *s = text.to_vec();
        }
    }
}

/// Length of a string as HXAudio truncates it (`& 0xFFFF`).
pub fn clamp_len(text: &[u8]) -> &[u8] {
    &text[..text.len() & 0xffff]
}

