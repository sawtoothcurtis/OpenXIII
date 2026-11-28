//! Sound program (`usnd::CProgramResData`): `CProgramHdr` header, code, named event
//! handlers ("hXIIIFootCar", "OnTimer"…), functions and table of tracked objects.
//!
//! Reading: `CProgramResData` vtable+8, symmetric write; standard handler table:.

use openxiii_assets::hx::Cuuid;

/// Named handler: 16-byte entry in the code (start, length, stack size, name).
#[derive(Debug, Clone)]
pub struct Handler {
    pub start: u32,
    pub len: u32,
    pub stack: u32,
    /// Offset of the name in the string table.
    pub name_offset: u32,
    pub name: String,
}

/// Function called by `call`: 12-byte entry (start, length, stack size).
#[derive(Debug, Clone, Copy)]
pub struct Function {
    pub start: u32,
    pub len: u32,
    pub stack: u32,
}

/// Standard messages (identifiers 0..0x20) and the name of their handler.
pub const STD_HANDLERS: [(usize, &str); 8] =
    [(0, "OnPlay"), (1, "OnStop"), (2, "OnPause"), (3, "OnResume"), (4, "OnIsPlaying"), (6, "OnSetVol"), (0xc, "OnSetPitch"), (0x16, "OnSet2DParams")];

#[derive(Debug, Clone)]
pub struct Program {
    pub cuuid: Cuuid,
    pub version: u32,
    /// Field read before the header then ignored (0x58 in XIII's banks).
    pub skipped: u32,
    /// `CProgramHdr` (object + 0x28): 24 words, see the accessors.
    pub hdr: [u32; 24],
    /// Object + 0x144: "is playing" mode (bits 0..2).
    pub play_mode: u32,
    /// Object + 0x18 and + 0x1C (versions ≥ 10 and ≥ 11).
    pub x18: u32,
    pub x1c: bool,
    pub code: Vec<u8>,
    /// Object + 0x24: offsets, in the object's variables, of the sub-objects to query.
    pub watched: Vec<u16>,
    pub links: Vec<Cuuid>,
    pub handlers: Vec<Handler>,
    pub functions: Vec<Function>,
    /// Header + 0x84: for each message 0..0x20, index of the standard handler or `None`.
    pub std_handler: [Option<u32>; 0x21],
}

impl Program {
    pub fn stack_init(&self) -> u32 {
        self.hdr[2]
    }
    pub fn stack_attach(&self) -> u32 {
        self.hdr[3]
    }
    pub fn stack_term(&self) -> u32 {
        self.hdr[4]
    }
    pub fn locals_size(&self) -> u32 {
        self.hdr[7]
    }
    pub fn strings_offset(&self) -> u32 {
        self.hdr[8]
    }
    pub fn strings_size(&self) -> u32 {
        self.hdr[9]
    }
    /// Block run on destruction (message 0x3EE): header + 0x28/0x2C.
    pub fn term_block(&self) -> (u32, u32) {
        (self.hdr[10], self.hdr[11])
    }
    /// Block run when attached to an actor: header + 0x30/0x34.
    pub fn attach_block(&self) -> (u32, u32) {
        (self.hdr[12], self.hdr[13])
    }
    /// Block run on message 0x20 after attachment: header + 0x38/0x3C.
    pub fn init_block(&self) -> (u32, u32) {
        (self.hdr[14], self.hdr[15])
    }
    pub fn table_offset(&self) -> u32 {
        self.hdr[16]
    }

    /// C string from the program's string table, at offset `offset`.
    pub fn string(&self, offset: u32) -> &[u8] {
        let start = self.strings_offset().wrapping_add(offset) as usize;
        let rest = self.code.get(start..).unwrap_or(&[]);
        let end = rest.iter().position(|&b| b == 0).unwrap_or(rest.len());
        &rest[..end]
    }

    /// Index of the named handler `name` (`CHdlrMgr` vtable+0).
    pub fn handler_index(&self, name: &[u8]) -> Option<usize> {
        self.handlers.iter().position(|h| self.string(h.name_offset) == name)
    }

    pub fn parse(data: &[u8], offset: usize) -> Result<Program, String> {
        let mut c = Reader { data, pos: offset };
        let class_len = c.u32()? as usize;
        c.pos += class_len;
        let cuuid = Cuuid(c.u32()?, c.u32()?);
        let version = c.u32()?;
        let mut p = Program {
            cuuid,
            version,
            skipped: 0,
            // CProgramHdr::Init; Read keeps this state before version 6.
            hdr: [
                0x18, 0x8, 0, 0, 0, 0, 0, 0,
                u32::MAX, 0, u32::MAX, 0, u32::MAX, 0, u32::MAX, 0,
                u32::MAX, 0, u32::MAX, 0, u32::MAX, 0, u32::MAX, 0,
            ],
            play_mode: 0,
            x18: 0,
            x1c: false,
            code: Vec::new(),
            watched: Vec::new(),
            links: Vec::new(),
            handlers: Vec::new(),
            functions: Vec::new(),
            std_handler: [None; 0x21],
        };
        if version <= 5 {
            return Ok(p);
        }
        p.skipped = c.u32()?;
        for h in p.hdr.iter_mut() {
            *h = c.u32()?;
        }
        p.play_mode = if version < 8 { 0 } else { c.u32()? };
        let watched_count = if version > 8 { c.u32()? } else { 0 };
        p.x18 = if version < 10 { 0 } else { c.u32()? };
        p.x1c = if version < 11 { false } else { c.u32()? != 0 };
        // Code size: header + 0x40 + header + 0x44, zero if there is no table.
        let size = if p.hdr[16] == u32::MAX { 0 } else { p.hdr[16].wrapping_add(p.hdr[17]) as usize };
        if size == 0 {
            return Ok(p);
        }
        p.code = c.bytes(size)?.to_vec();
        if version > 8 {
            for _ in 0..watched_count {
                p.watched.push(c.u16()?);
            }
        }
        let links = c.u32()?;
        for _ in 0..links {
            p.links.push(Cuuid(c.u32()?, c.u32()?));
        }
        p.build_tables();
        Ok(p)
    }

    fn build_tables(&mut self) {
        let table = self.hdr[16] as usize;
        let rd = |code: &[u8], at: usize| -> u32 { code.get(at..at + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap())).unwrap_or(0) };
        for i in 0..self.hdr[5] as usize {
            let at = table + i * 16;
            let name_offset = rd(&self.code, at + 12);
            let name = String::from_utf8_lossy(self.string(name_offset)).into_owned();
            self.handlers.push(Handler { start: rd(&self.code, at), len: rd(&self.code, at + 4), stack: rd(&self.code, at + 8), name_offset, name });
        }
        let functions = table + self.hdr[5] as usize * 16;
        for i in 0..self.hdr[6] as usize {
            let at = functions + i * 12;
            self.functions.push(Function { start: rd(&self.code, at), len: rd(&self.code, at + 4), stack: rd(&self.code, at + 8) });
        }
        for (id, name) in STD_HANDLERS {
            self.std_handler[id] = self.handler_index(name.as_bytes()).map(|i| i as u32);
        }
    }
}

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn bytes(&mut self, n: usize) -> Result<&'a [u8], String> {
        let b = self.data.get(self.pos..self.pos + n).ok_or_else(|| format!("program truncated at 0x{:X}", self.pos))?;
        self.pos += n;
        Ok(b)
    }
    fn u16(&mut self) -> Result<u16, String> {
        Ok(u16::from_le_bytes(self.bytes(2)?.try_into().unwrap()))
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.bytes(4)?.try_into().unwrap()))
    }
}

