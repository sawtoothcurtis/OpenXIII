//! zlib 1.1.3, the "inflate" part (infblock.c, infcodes.c, inftrees.c, inffast.c, infutil.c, inflate.c,
//! uncompress.c) statically linked into Engine.dll (inflate_blocks,
//! inflate_codes, huft_build, inflate_trees_bits, inflate_trees_dynamic,
//! inflate_trees_fixed, inflate_flush, inflate_fast), 1:1 reimplementation: same
//! states, same tables, same error codes as the original game, including for damaged streams (a copy distance
//! larger than the history reads back the 32 KiB zero-filled window; an incomplete length
//! tree returns Z_DATA_ERROR). Valid streams give the same output as any inflate.

pub const Z_OK: i32 = 0;
pub const Z_STREAM_END: i32 = 1;
pub const Z_NEED_DICT: i32 = 2;
pub const Z_STREAM_ERROR: i32 = -2;
pub const Z_DATA_ERROR: i32 = -3;
pub const Z_MEM_ERROR: i32 = -4;
pub const Z_BUF_ERROR: i32 = -5;
const Z_FINISH: i32 = 4;
const Z_DEFLATED: u32 = 8;
const MANY: usize = 1440;
const BMAX: usize = 15;
const PRESET_DICT: u32 = 0x20;
const WBITS: u32 = 15;
const NIL: usize = usize::MAX;

const INFLATE_MASK: [u32; 17] = [0, 1, 3, 7, 15, 31, 63, 127, 255, 511, 1023, 2047, 4095, 8191, 16383, 32767, 65535];
const BORDER: [usize; 19] = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];
const CPLENS: [u32; 31] = [3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258, 0, 0];
const CPLEXT: [u32; 31] = [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0, 112, 112];
const CPDIST: [u32; 30] = [1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577];
const CPDEXT: [u32; 30] = [0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13];

/// inflate_huft: exop (Exop), bits (Bits), base (offset or value).
#[derive(Clone, Copy, Default)]
struct Huft {
    exop: u8,
    bits: u8,
    base: u32,
}

/// adler32 (zlib 1.1.3 adler32.c).
fn adler32(adler: u32, buf: &[u8]) -> u32 {
    const BASE: u32 = 65521;
    const NMAX: usize = 5552;
    let mut s1 = adler & 0xffff;
    let mut s2 = (adler >> 16) & 0xffff;
    for chunk in buf.chunks(NMAX) {
        for &b in chunk {
            s1 += u32::from(b);
            s2 += s1;
        }
        s1 %= BASE;
        s2 %= BASE;
    }
    (s2 << 16) | s1
}

/// huft_build (inftrees.c): `hp` is the index of the table area, `hn` the number of entries already
/// taken; `t` returns the index of the first table (NIL: none), `m` the lookup bits.
#[allow(clippy::too_many_arguments)]
fn huft_build(b: &[u32], n: usize, s: u32, d: Option<&[u32]>, e: Option<&[u32]>, t: &mut usize, m: &mut u32, arena: &mut [Huft], hp: usize, hn: &mut usize, v: &mut [u32]) -> i32 {
    let mut c = [0u32; BMAX + 1];
    for &x in &b[..n] {
        c[x as usize] += 1;
    }
    if c[0] as usize == n {
        *t = NIL;
        *m = 0;
        return Z_OK;
    }
    let mut l = *m as i32;
    let mut j = 1usize;
    while j <= BMAX {
        if c[j] != 0 {
            break;
        }
        j += 1;
    }
    let mut k = j as i32;
    if (l as u32) < j as u32 {
        l = j as i32;
    }
    let mut i = BMAX;
    while i != 0 {
        if c[i] != 0 {
            break;
        }
        i -= 1;
    }
    let g = i as i32;
    if l as u32 > i as u32 {
        l = i as i32;
    }
    *m = l as u32;
    let mut y: i32 = 1 << j;
    while j < i {
        y -= c[j] as i32;
        if y < 0 {
            return Z_DATA_ERROR;
        }
        j += 1;
        y <<= 1;
    }
    y -= c[i] as i32;
    if y < 0 {
        return Z_DATA_ERROR;
    }
    c[i] = c[i].wrapping_add(y as u32);
    let mut x = [0u32; BMAX + 1];
    j = 0;
    x[1] = 0;
    let mut pi = 1usize;
    let mut xp = 2usize;
    let mut ii = i;
    while {
        ii -= 1;
        ii != 0
    } {
        j += c[pi] as usize;
        pi += 1;
        x[xp] = j as u32;
        xp += 1;
    }
    let mut iu = 0u32;
    let mut pb = 0usize;
    loop {
        let jj = b[pb] as usize;
        pb += 1;
        if jj != 0 {
            v[x[jj] as usize] = iu;
            x[jj] += 1;
        }
        iu += 1;
        if iu as usize >= n {
            break;
        }
    }
    let n = x[g as usize] as usize;
    // Huffman codes and table entries.
    let mut i: u32 = 0;
    x[0] = 0;
    let mut p = 0usize;
    let mut h: i32 = -1;
    let mut w: i32 = -l;
    let mut u = [NIL; BMAX];
    let mut q = NIL;
    let mut z: u32 = 0;
    let mut r = Huft::default();
    while k <= g {
        let mut a = c[k as usize];
        while a != 0 {
            a -= 1;
            while k > w + l {
                h += 1;
                w += l;
                z = (g - w) as u32;
                z = if z > l as u32 { l as u32 } else { z };
                let mut jj = (k - w) as u32;
                let mut f: u32 = 1u32 << jj;
                if f > a + 1 {
                    f -= a + 1;
                    let mut xpp = k as usize;
                    if jj < z {
                        loop {
                            jj += 1;
                            if jj >= z {
                                break;
                            }
                            xpp += 1;
                            f <<= 1;
                            if f <= c[xpp] {
                                break;
                            }
                            f -= c[xpp];
                        }
                    }
                }
                z = 1u32 << jj;
                if *hn + z as usize > MANY {
                    return Z_MEM_ERROR;
                }
                q = hp + *hn;
                u[h as usize] = q;
                *hn += z as usize;
                if h != 0 {
                    x[h as usize] = i;
                    r.bits = l as u8;
                    r.exop = jj as u8;
                    let jx = i >> (w - l) as u32;
                    r.base = (q - u[h as usize - 1]) as u32 - jx;
                    arena[u[h as usize - 1] + jx as usize] = r;
                } else {
                    *t = q;
                }
            }
            r.bits = (k - w) as u8;
            if p >= n {
                r.exop = 128 + 64;
            } else if v[p] < s {
                r.exop = if v[p] < 256 { 0 } else { 32 + 64 };
                r.base = v[p];
                p += 1;
            } else {
                let off = (v[p] - s) as usize;
                r.exop = (e.unwrap()[off] + 16 + 64) as u8;
                r.base = d.unwrap()[off];
                p += 1;
            }
            let f: u32 = 1u32 << (k - w) as u32;
            let mut jj = i >> w as u32;
            while jj < z {
                arena[q + jj as usize] = r;
                jj += f;
            }
            let mut jj = 1u32 << (k - 1) as u32;
            while i & jj != 0 {
                i ^= jj;
                jj >>= 1;
            }
            i ^= jj;
            let mut mask = (1u32 << w as u32).wrapping_sub(1);
            while (i & mask) != x[h as usize] {
                h -= 1;
                w -= l;
                mask = (1u32 << w as u32).wrapping_sub(1);
            }
        }
        k += 1;
    }
    if y != 0 && g != 1 {
        Z_BUF_ERROR
    } else {
        Z_OK
    }
}

/// Fixed tables (inflate_trees_fixed returns bl = 9, bd = 5 and the static tables): the 544 entries of fixed_mem, bl, bd, and the indices
/// of tl and td within them.
fn fixed_tables() -> &'static (Vec<Huft>, u32, u32, usize, usize) {
    static FIXED: std::sync::OnceLock<(Vec<Huft>, u32, u32, usize, usize)> = std::sync::OnceLock::new();
    FIXED.get_or_init(|| {
        let mut mem = vec![Huft::default(); 544];
        let mut c = [0u32; 288];
        let mut v = [0u32; 288];
        for (k, x) in c.iter_mut().enumerate() {
            *x = match k {
                0..=143 => 8,
                144..=255 => 9,
                256..=279 => 7,
                _ => 8,
            };
        }
        let mut f = 0usize;
        let (mut tl, mut td) = (NIL, NIL);
        let mut bl = 9u32;
        huft_build(&c, 288, 257, Some(&CPLENS), Some(&CPLEXT), &mut tl, &mut bl, &mut mem, 0, &mut f, &mut v);
        for x in c.iter_mut().take(30) {
            *x = 5;
        }
        let mut bd = 5u32;
        huft_build(&c, 30, 0, Some(&CPDIST), Some(&CPDEXT), &mut td, &mut bd, &mut mem, 0, &mut f, &mut v);
        (mem, bl, bd, tl, td)
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Method,
    Flag,
    Dict4,
    Dict3,
    Dict2,
    Dict1,
    Dict0,
    Blocks,
    Check4,
    Check3,
    Check2,
    Check1,
    Done,
    Bad,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BMode {
    Type,
    Lens,
    Stored,
    Table,
    Btree,
    Dtree,
    Codes,
    Dry,
    Done,
    Bad,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CMode {
    Start,
    Len,
    LenExt,
    Dist,
    DistExt,
    Copy,
    Lit,
    Wash,
    End,
    BadCode,
}

/// z_stream, inflate_state, inflate_blocks_statef and inflate_codes_statef combined.
struct Inflate<'a> {
    input: &'a [u8],
    next_in: usize,
    avail_in: usize,
    total_in: u32,
    out: &'a mut [u8],
    next_out: usize,
    avail_out: usize,
    total_out: u32,
    adler: u32,
    msg: Option<&'static str>,
    mode: Mode,
    method: u32,
    need: u32,
    was: u32,
    marker: u32,
    nowrap: bool,
    wbits: u32,
    // blocks
    bmode: BMode,
    left: u32,
    table: u32,
    index: u32,
    blens: Vec<u32>,
    bb: u32,
    tb: usize,
    last: u32,
    bitk: u32,
    bitb: u32,
    hufts: Vec<Huft>,
    window: Vec<u8>,
    end: usize,
    read: usize,
    write: usize,
    check: u32,
    use_check: bool,
    // codes
    cmode: CMode,
    len: u32,
    ctree: usize,
    need_bits: u32,
    lit: u32,
    get: u32,
    dist: u32,
    lbits: u32,
    dbits: u32,
    ltree: usize,
    dtree: usize,
}

impl<'a> Inflate<'a> {
    fn new(input: &'a [u8], out: &'a mut [u8], nowrap: bool) -> Self {
        let w = 1usize << WBITS;
        let mut hufts = vec![Huft::default(); MANY + 544];
        hufts[MANY..].copy_from_slice(&fixed_tables().0);
        let avail_out = out.len();
        let mut s = Inflate {
            input,
            next_in: 0,
            avail_in: input.len(),
            total_in: 0,
            out,
            next_out: 0,
            avail_out,
            total_out: 0,
            adler: 0,
            msg: None,
            mode: Mode::Method,
            method: 0,
            need: 0,
            was: 0,
            marker: 0,
            nowrap,
            wbits: WBITS,
            bmode: BMode::Type,
            left: 0,
            table: 0,
            index: 0,
            blens: Vec::new(),
            bb: 0,
            tb: NIL,
            last: 0,
            bitk: 0,
            bitb: 0,
            hufts,
            window: vec![0u8; w],
            end: w,
            read: 0,
            write: 0,
            check: 0,
            use_check: !nowrap,
            cmode: CMode::Start,
            len: 0,
            ctree: NIL,
            need_bits: 0,
            lit: 0,
            get: 0,
            dist: 0,
            lbits: 0,
            dbits: 0,
            ltree: NIL,
            dtree: NIL,
        };
        // inflateInit_: inflateInit2_ with 15 window bits; zcalloc returns the window filled with zeros;
        // inflateReset: BLOCKS mode without a zlib header, METHOD otherwise, then inflate_blocks_reset.
        s.mode = if nowrap { Mode::Blocks } else { Mode::Method };
        s.blocks_reset();
        s
    }

    /// inflate_blocks_reset: also returns the current adler.
    fn blocks_reset(&mut self) -> u32 {
        let c = self.check;
        self.bmode = BMode::Type;
        self.bitk = 0;
        self.bitb = 0;
        self.read = 0;
        self.write = 0;
        if self.use_check {
            self.check = 1; // adler32(0, NULL, 0)
            self.adler = self.check;
        }
        c
    }

    /// inflate_flush (infutil.c).
    fn flush(&mut self, mut r: i32) -> i32 {
        let mut n = (if self.read <= self.write { self.write } else { self.end }) - self.read;
        if n > self.avail_out {
            n = self.avail_out;
        }
        if n != 0 && r == Z_BUF_ERROR {
            r = Z_OK;
        }
        self.avail_out -= n;
        self.total_out = self.total_out.wrapping_add(n as u32);
        let mut p = self.next_out;
        let mut q = self.read;
        if self.use_check {
            self.check = adler32(self.check, &self.window[q..q + n]);
            self.adler = self.check;
        }
        self.out[p..p + n].copy_from_slice(&self.window[q..q + n]);
        p += n;
        q += n;
        if q == self.end {
            q = 0;
            if self.write == self.end {
                self.write = 0;
            }
            let mut n = self.write - q;
            if n > self.avail_out {
                n = self.avail_out;
            }
            if n != 0 && r == Z_BUF_ERROR {
                r = Z_OK;
            }
            self.avail_out -= n;
            self.total_out = self.total_out.wrapping_add(n as u32);
            if self.use_check {
                self.check = adler32(self.check, &self.window[q..q + n]);
                self.adler = self.check;
            }
            self.out[p..p + n].copy_from_slice(&self.window[q..q + n]);
            p += n;
            q += n;
        }
        self.next_out = p;
        self.read = q;
        r
    }

    /// inflate_blocks.
    fn blocks(&mut self, mut r: i32) -> i32 {
        // LOAD
        let mut p = self.next_in;
        let mut n = self.avail_in;
        let mut b = self.bitb;
        let mut k = self.bitk;
        let mut q = self.write;
        macro_rules! wavail {
            () => {
                (if q < self.read { self.read - q - 1 } else { self.end - q })
            };
        }
        let mut m = wavail!();
        macro_rules! update {
            () => {
                self.bitb = b;
                self.bitk = k;
                self.avail_in = n;
                self.total_in = self.total_in.wrapping_add((p - self.next_in) as u32);
                self.next_in = p;
                self.write = q;
            };
        }
        macro_rules! leave {
            () => {{
                update!();
                return self.flush(r);
            }};
        }
        macro_rules! needbyte {
            () => {
                if n != 0 {
                    r = Z_OK;
                } else {
                    leave!();
                }
            };
        }
        macro_rules! needbits {
            ($j:expr) => {
                while k < ($j) {
                    needbyte!();
                    b |= u32::from(self.input[p]) << k;
                    n -= 1;
                    p += 1;
                    k += 8;
                }
            };
        }
        macro_rules! dumpbits {
            ($j:expr) => {
                b >>= $j;
                k -= $j;
            };
        }
        macro_rules! wrap {
            () => {
                if q == self.end && self.read != 0 {
                    q = 0;
                    m = wavail!();
                }
            };
        }
        macro_rules! flush {
            () => {
                self.write = q;
                r = self.flush(r);
                q = self.write;
                m = wavail!();
            };
        }
        macro_rules! needout {
            () => {
                if m == 0 {
                    wrap!();
                    if m == 0 {
                        flush!();
                        wrap!();
                        if m == 0 {
                            leave!();
                        }
                    }
                }
                r = Z_OK;
            };
        }
        loop {
            match self.bmode {
                BMode::Type => {
                    needbits!(3);
                    let t = b & 7;
                    self.last = t & 1;
                    match t >> 1 {
                        0 => {
                            dumpbits!(3);
                            let t = k & 7;
                            dumpbits!(t);
                            self.bmode = BMode::Lens;
                        }
                        1 => {
                            let f = fixed_tables();
                            self.lbits = f.1;
                            self.dbits = f.2;
                            self.ltree = MANY + f.3;
                            self.dtree = MANY + f.4;
                            self.cmode = CMode::Start;
                            dumpbits!(3);
                            self.bmode = BMode::Codes;
                        }
                        2 => {
                            dumpbits!(3);
                            self.bmode = BMode::Table;
                        }
                        _ => {
                            dumpbits!(3);
                            self.bmode = BMode::Bad;
                            self.msg = Some("invalid block type");
                            r = Z_DATA_ERROR;
                            leave!();
                        }
                    }
                }
                BMode::Lens => {
                    needbits!(32);
                    if (((!b) >> 16) & 0xffff) != (b & 0xffff) {
                        self.bmode = BMode::Bad;
                        self.msg = Some("invalid stored block lengths");
                        r = Z_DATA_ERROR;
                        leave!();
                    }
                    self.left = b & 0xffff;
                    b = 0;
                    k = 0;
                    self.bmode = if self.left != 0 {
                        BMode::Stored
                    } else if self.last != 0 {
                        BMode::Dry
                    } else {
                        BMode::Type
                    };
                }
                BMode::Stored => {
                    if n == 0 {
                        leave!();
                    }
                    needout!();
                    let mut t = self.left as usize;
                    if t > n {
                        t = n;
                    }
                    if t > m {
                        t = m;
                    }
                    self.window[q..q + t].copy_from_slice(&self.input[p..p + t]);
                    p += t;
                    n -= t;
                    q += t;
                    m -= t;
                    self.left -= t as u32;
                    if self.left != 0 {
                        continue;
                    }
                    self.bmode = if self.last != 0 { BMode::Dry } else { BMode::Type };
                }
                BMode::Table => {
                    needbits!(14);
                    let t = b & 0x3fff;
                    self.table = t;
                    if (t & 0x1f) > 29 || ((t >> 5) & 0x1f) > 29 {
                        self.bmode = BMode::Bad;
                        self.msg = Some("too many length or distance symbols");
                        r = Z_DATA_ERROR;
                        leave!();
                    }
                    let t = 258 + (t & 0x1f) + ((t >> 5) & 0x1f);
                    self.blens = vec![0u32; t as usize];
                    dumpbits!(14);
                    self.index = 0;
                    self.bmode = BMode::Btree;
                }
                BMode::Btree => {
                    while self.index < 4 + (self.table >> 10) {
                        needbits!(3);
                        self.blens[BORDER[self.index as usize]] = b & 7;
                        self.index += 1;
                        dumpbits!(3);
                    }
                    while self.index < 19 {
                        self.blens[BORDER[self.index as usize]] = 0;
                        self.index += 1;
                    }
                    self.bb = 7;
                    let t = self.trees_bits();
                    if t != Z_OK {
                        r = t;
                        if r == Z_DATA_ERROR {
                            self.bmode = BMode::Bad;
                        }
                        leave!();
                    }
                    self.index = 0;
                    self.bmode = BMode::Dtree;
                }
                BMode::Dtree => {
                    loop {
                        let t = self.table;
                        if self.index >= 258 + (t & 0x1f) + ((t >> 5) & 0x1f) {
                            break;
                        }
                        let t = self.bb;
                        needbits!(t);
                        let h = self.hufts[self.tb + (b & INFLATE_MASK[t as usize]) as usize];
                        let t = u32::from(h.bits);
                        let c = h.base;
                        if c < 16 {
                            dumpbits!(t);
                            self.blens[self.index as usize] = c;
                            self.index += 1;
                        } else {
                            let mut i = if c == 18 { 7 } else { c - 14 };
                            let mut j = if c == 18 { 11 } else { 3 };
                            needbits!(t + i);
                            dumpbits!(t);
                            j += b & INFLATE_MASK[i as usize];
                            dumpbits!(i);
                            i = self.index;
                            let t = self.table;
                            if i + j > 258 + (t & 0x1f) + ((t >> 5) & 0x1f) || (c == 16 && i < 1) {
                                self.bmode = BMode::Bad;
                                self.msg = Some("invalid bit length repeat");
                                r = Z_DATA_ERROR;
                                leave!();
                            }
                            let c = if c == 16 { self.blens[i as usize - 1] } else { 0 };
                            loop {
                                self.blens[i as usize] = c;
                                i += 1;
                                j -= 1;
                                if j == 0 {
                                    break;
                                }
                            }
                            self.index = i;
                        }
                    }
                    self.tb = NIL;
                    let t = self.table;
                    let t = self.trees_dynamic(257 + (t & 0x1f), 1 + ((t >> 5) & 0x1f));
                    if t != Z_OK {
                        if t == Z_DATA_ERROR {
                            self.bmode = BMode::Bad;
                        }
                        r = t;
                        leave!();
                    }
                    self.cmode = CMode::Start;
                    self.bmode = BMode::Codes;
                }
                BMode::Codes => {
                    update!();
                    r = self.codes(r);
                    if r != Z_STREAM_END {
                        return self.flush(r);
                    }
                    r = Z_OK;
                    // LOAD
                    p = self.next_in;
                    n = self.avail_in;
                    b = self.bitb;
                    k = self.bitk;
                    q = self.write;
                    m = wavail!();
                    if self.last == 0 {
                        self.bmode = BMode::Type;
                        continue;
                    }
                    self.bmode = BMode::Dry;
                }
                BMode::Dry => {
                    flush!();
                    if self.read != self.write {
                        leave!();
                    }
                    self.bmode = BMode::Done;
                }
                BMode::Done => {
                    r = Z_STREAM_END;
                    leave!();
                }
                BMode::Bad => {
                    r = Z_DATA_ERROR;
                    leave!();
                }
            }
        }
    }

    /// inflate_trees_bits on blens, result in bb and tb.
    fn trees_bits(&mut self) -> i32 {
        let mut hn = 0usize;
        let mut v = [0u32; 19];
        let blens: Vec<u32> = self.blens.clone();
        let mut bb = self.bb;
        let mut tb = NIL;
        let r = huft_build(&blens, 19, 19, None, None, &mut tb, &mut bb, &mut self.hufts, 0, &mut hn, &mut v);
        self.bb = bb;
        self.tb = tb;
        let mut r = r;
        if r == Z_DATA_ERROR {
            self.msg = Some("oversubscribed dynamic bit lengths tree");
        } else if r == Z_BUF_ERROR || bb == 0 {
            self.msg = Some("incomplete dynamic bit lengths tree");
            r = Z_DATA_ERROR;
        }
        r
    }

    /// inflate_trees_dynamic then inflate_codes_new.
    fn trees_dynamic(&mut self, nl: u32, nd: u32) -> i32 {
        let mut hn = 0usize;
        let mut v = [0u32; 288];
        let blens: Vec<u32> = self.blens.clone();
        let mut bl = 9u32;
        let mut bd = 6u32;
        let (mut tl, mut td) = (NIL, NIL);
        let mut r = huft_build(&blens, nl as usize, 257, Some(&CPLENS), Some(&CPLEXT), &mut tl, &mut bl, &mut self.hufts, 0, &mut hn, &mut v);
        if r != Z_OK || bl == 0 {
            if r == Z_DATA_ERROR {
                self.msg = Some("oversubscribed literal/length tree");
            } else if r != Z_MEM_ERROR {
                self.msg = Some("incomplete literal/length tree");
                r = Z_DATA_ERROR;
            }
            return r;
        }
        r = huft_build(&blens[nl as usize..], nd as usize, 0, Some(&CPDIST), Some(&CPDEXT), &mut td, &mut bd, &mut self.hufts, 0, &mut hn, &mut v);
        if r != Z_OK || (bd == 0 && nl > 257) {
            if r == Z_DATA_ERROR {
                self.msg = Some("oversubscribed distance tree");
            } else if r == Z_BUF_ERROR {
                self.msg = Some("incomplete distance tree");
                r = Z_DATA_ERROR;
            } else if r != Z_MEM_ERROR {
                self.msg = Some("empty distance tree with lengths");
                r = Z_DATA_ERROR;
            }
            return r;
        }
        // inflate_codes_new: lbits, dbits, tl, td, START mode.
        self.lbits = bl;
        self.dbits = bd;
        self.ltree = tl;
        self.dtree = td;
        Z_OK
    }

    /// inflate_codes.
    fn codes(&mut self, mut r: i32) -> i32 {
        let mut p = self.next_in;
        let mut n = self.avail_in;
        let mut b = self.bitb;
        let mut k = self.bitk;
        let mut q = self.write;
        macro_rules! wavail {
            () => {
                (if q < self.read { self.read - q - 1 } else { self.end - q })
            };
        }
        let mut m = wavail!();
        macro_rules! update {
            () => {
                self.bitb = b;
                self.bitk = k;
                self.avail_in = n;
                self.total_in = self.total_in.wrapping_add((p - self.next_in) as u32);
                self.next_in = p;
                self.write = q;
            };
        }
        macro_rules! leave {
            () => {{
                update!();
                return self.flush(r);
            }};
        }
        macro_rules! needbits {
            ($j:expr) => {
                while k < ($j) {
                    if n != 0 {
                        r = Z_OK;
                    } else {
                        leave!();
                    }
                    b |= u32::from(self.input[p]) << k;
                    n -= 1;
                    p += 1;
                    k += 8;
                }
            };
        }
        macro_rules! dumpbits {
            ($j:expr) => {
                b >>= $j;
                k -= $j;
            };
        }
        macro_rules! wrap {
            () => {
                if q == self.end && self.read != 0 {
                    q = 0;
                    m = wavail!();
                }
            };
        }
        macro_rules! flush {
            () => {
                self.write = q;
                r = self.flush(r);
                q = self.write;
                m = wavail!();
            };
        }
        macro_rules! needout {
            () => {
                if m == 0 {
                    wrap!();
                    if m == 0 {
                        flush!();
                        wrap!();
                        if m == 0 {
                            leave!();
                        }
                    }
                }
                r = Z_OK;
            };
        }
        loop {
            match self.cmode {
                CMode::Start => {
                    if m >= 258 && n >= 10 {
                        update!();
                        r = self.fast();
                        // LOAD
                        p = self.next_in;
                        n = self.avail_in;
                        b = self.bitb;
                        k = self.bitk;
                        q = self.write;
                        m = wavail!();
                        if r != Z_OK {
                            self.cmode = if r == Z_STREAM_END { CMode::Wash } else { CMode::BadCode };
                            continue;
                        }
                    }
                    self.need_bits = self.lbits;
                    self.ctree = self.ltree;
                    self.cmode = CMode::Len;
                }
                CMode::Len => {
                    let j = self.need_bits;
                    needbits!(j);
                    let ti = self.ctree + (b & INFLATE_MASK[j as usize]) as usize;
                    let t = self.hufts[ti];
                    dumpbits!(u32::from(t.bits));
                    let e = u32::from(t.exop);
                    if e == 0 {
                        self.lit = t.base;
                        self.cmode = CMode::Lit;
                        continue;
                    }
                    if e & 16 != 0 {
                        self.get = e & 15;
                        self.len = t.base;
                        self.cmode = CMode::LenExt;
                        continue;
                    }
                    if e & 64 == 0 {
                        self.need_bits = e;
                        self.ctree = ti + t.base as usize;
                        continue;
                    }
                    if e & 32 != 0 {
                        self.cmode = CMode::Wash;
                        continue;
                    }
                    self.cmode = CMode::BadCode;
                    self.msg = Some("invalid literal/length code");
                    r = Z_DATA_ERROR;
                    leave!();
                }
                CMode::LenExt => {
                    let j = self.get;
                    needbits!(j);
                    self.len += b & INFLATE_MASK[j as usize];
                    dumpbits!(j);
                    self.need_bits = self.dbits;
                    self.ctree = self.dtree;
                    self.cmode = CMode::Dist;
                }
                CMode::Dist => {
                    let j = self.need_bits;
                    needbits!(j);
                    let ti = self.ctree + (b & INFLATE_MASK[j as usize]) as usize;
                    let t = self.hufts[ti];
                    dumpbits!(u32::from(t.bits));
                    let e = u32::from(t.exop);
                    if e & 16 != 0 {
                        self.get = e & 15;
                        self.dist = t.base;
                        self.cmode = CMode::DistExt;
                        continue;
                    }
                    if e & 64 == 0 {
                        self.need_bits = e;
                        self.ctree = ti + t.base as usize;
                        continue;
                    }
                    self.cmode = CMode::BadCode;
                    self.msg = Some("invalid distance code");
                    r = Z_DATA_ERROR;
                    leave!();
                }
                CMode::DistExt => {
                    let j = self.get;
                    needbits!(j);
                    self.dist += b & INFLATE_MASK[j as usize];
                    dumpbits!(j);
                    self.cmode = CMode::Copy;
                }
                CMode::Copy => {
                    let mut f = if (q as u32) < self.dist { self.end - (self.dist as usize - q) } else { q - self.dist as usize };
                    while self.len != 0 {
                        needout!();
                        self.window[q] = self.window[f];
                        q += 1;
                        f += 1;
                        m -= 1;
                        if f == self.end {
                            f = 0;
                        }
                        self.len -= 1;
                    }
                    self.cmode = CMode::Start;
                }
                CMode::Lit => {
                    needout!();
                    self.window[q] = self.lit as u8;
                    q += 1;
                    m -= 1;
                    self.cmode = CMode::Start;
                }
                CMode::Wash => {
                    if k > 7 {
                        k -= 8;
                        n += 1;
                        p -= 1;
                    }
                    flush!();
                    if self.read != self.write {
                        leave!();
                    }
                    self.cmode = CMode::End;
                }
                CMode::End => {
                    r = Z_STREAM_END;
                    leave!();
                }
                CMode::BadCode => {
                    r = Z_DATA_ERROR;
                    leave!();
                }
            }
        }
    }

    /// inflate_fast: called with m >= 258 and n >= 10.
    fn fast(&mut self) -> i32 {
        let mut p = self.next_in;
        let mut n = self.avail_in;
        let mut b = self.bitb;
        let mut k = self.bitk;
        let mut q = self.write;
        let mut m = if q < self.read { self.read - q - 1 } else { self.end - q };
        let ml = INFLATE_MASK[self.lbits as usize];
        let md = INFLATE_MASK[self.dbits as usize];
        let (tl, td) = (self.ltree, self.dtree);
        macro_rules! grabbits {
            ($j:expr) => {
                while k < ($j) {
                    b |= u32::from(self.input[p]) << k;
                    n -= 1;
                    p += 1;
                    k += 8;
                }
            };
        }
        macro_rules! dumpbits {
            ($j:expr) => {
                b >>= $j;
                k -= $j;
            };
        }
        macro_rules! ungrab_update {
            () => {
                let mut c = self.avail_in - n;
                c = if ((k >> 3) as usize) < c { (k >> 3) as usize } else { c };
                n += c;
                p -= c;
                k -= (c as u32) << 3;
                self.bitb = b;
                self.bitk = k;
                self.avail_in = n;
                self.total_in = self.total_in.wrapping_add((p - self.next_in) as u32);
                self.next_in = p;
                self.write = q;
            };
        }
        loop {
            grabbits!(20);
            let mut t = tl + (b & ml) as usize;
            let mut e = u32::from(self.hufts[t].exop);
            if e == 0 {
                dumpbits!(u32::from(self.hufts[t].bits));
                self.window[q] = self.hufts[t].base as u8;
                q += 1;
                m -= 1;
                if m >= 258 && n >= 10 {
                    continue;
                }
                break;
            }
            // do { ... } while (1): exits through break/return
            loop {
                dumpbits!(u32::from(self.hufts[t].bits));
                if e & 16 != 0 {
                    e &= 15;
                    let mut c = self.hufts[t].base + (b & INFLATE_MASK[e as usize]);
                    dumpbits!(e);
                    grabbits!(15);
                    t = td + (b & md) as usize;
                    e = u32::from(self.hufts[t].exop);
                    loop {
                        dumpbits!(u32::from(self.hufts[t].bits));
                        if e & 16 != 0 {
                            e &= 15;
                            grabbits!(e);
                            let d = self.hufts[t].base + (b & INFLATE_MASK[e as usize]);
                            dumpbits!(e);
                            m -= c as usize;
                            let mut r;
                            if q >= d as usize {
                                r = q - d as usize;
                                self.window[q] = self.window[r];
                                q += 1;
                                r += 1;
                                c -= 1;
                                self.window[q] = self.window[r];
                                q += 1;
                                r += 1;
                                c -= 1;
                            } else {
                                let mut e2 = d as usize - q;
                                r = self.end - e2;
                                if c as usize > e2 {
                                    c -= e2 as u32;
                                    loop {
                                        self.window[q] = self.window[r];
                                        q += 1;
                                        r += 1;
                                        e2 -= 1;
                                        if e2 == 0 {
                                            break;
                                        }
                                    }
                                    r = 0;
                                }
                            }
                            loop {
                                self.window[q] = self.window[r];
                                q += 1;
                                r += 1;
                                c -= 1;
                                if c == 0 {
                                    break;
                                }
                            }
                            break;
                        } else if e & 64 == 0 {
                            t = t + self.hufts[t].base as usize;
                            t += (b & INFLATE_MASK[e as usize]) as usize;
                            e = u32::from(self.hufts[t].exop);
                        } else {
                            self.msg = Some("invalid distance code");
                            ungrab_update!();
                            return Z_DATA_ERROR;
                        }
                    }
                    break;
                }
                if e & 64 == 0 {
                    t = t + self.hufts[t].base as usize;
                    t += (b & INFLATE_MASK[e as usize]) as usize;
                    e = u32::from(self.hufts[t].exop);
                    if e == 0 {
                        dumpbits!(u32::from(self.hufts[t].bits));
                        self.window[q] = self.hufts[t].base as u8;
                        q += 1;
                        m -= 1;
                        break;
                    }
                } else if e & 32 != 0 {
                    ungrab_update!();
                    return Z_STREAM_END;
                } else {
                    self.msg = Some("invalid literal/length code");
                    ungrab_update!();
                    return Z_DATA_ERROR;
                }
            }
            if m >= 258 && n >= 10 {
                continue;
            }
            break;
        }
        ungrab_update!();
        Z_OK
    }

    /// inflate (inflate.c) with the flag f = Z_FINISH or not.
    fn inflate(&mut self, flush: i32) -> i32 {
        let f = if flush == Z_FINISH { Z_BUF_ERROR } else { Z_OK };
        let mut r = Z_BUF_ERROR;
        macro_rules! needbyte {
            () => {
                if self.avail_in == 0 {
                    return r;
                }
                r = f;
            };
        }
        macro_rules! nextbyte {
            () => {{
                self.avail_in -= 1;
                self.total_in = self.total_in.wrapping_add(1);
                let x = self.input[self.next_in];
                self.next_in += 1;
                x
            }};
        }
        loop {
            match self.mode {
                Mode::Method => {
                    needbyte!();
                    self.method = u32::from(nextbyte!());
                    if (self.method & 0xf) != Z_DEFLATED {
                        self.mode = Mode::Bad;
                        self.msg = Some("unknown compression method");
                        self.marker = 5;
                        continue;
                    }
                    if (self.method >> 4) + 8 > self.wbits {
                        self.mode = Mode::Bad;
                        self.msg = Some("invalid window size");
                        self.marker = 5;
                        continue;
                    }
                    self.mode = Mode::Flag;
                }
                Mode::Flag => {
                    needbyte!();
                    let b = u32::from(nextbyte!());
                    if ((self.method << 8) + b) % 31 != 0 {
                        self.mode = Mode::Bad;
                        self.msg = Some("incorrect header check");
                        self.marker = 5;
                        continue;
                    }
                    if b & PRESET_DICT == 0 {
                        self.mode = Mode::Blocks;
                        continue;
                    }
                    self.mode = Mode::Dict4;
                }
                Mode::Dict4 => {
                    needbyte!();
                    self.need = u32::from(nextbyte!()) << 24;
                    self.mode = Mode::Dict3;
                }
                Mode::Dict3 => {
                    needbyte!();
                    self.need = self.need.wrapping_add(u32::from(nextbyte!()) << 16);
                    self.mode = Mode::Dict2;
                }
                Mode::Dict2 => {
                    needbyte!();
                    self.need = self.need.wrapping_add(u32::from(nextbyte!()) << 8);
                    self.mode = Mode::Dict1;
                }
                Mode::Dict1 => {
                    needbyte!();
                    self.need = self.need.wrapping_add(u32::from(nextbyte!()));
                    self.adler = self.need;
                    self.mode = Mode::Dict0;
                    return Z_NEED_DICT;
                }
                Mode::Dict0 => {
                    self.mode = Mode::Bad;
                    self.msg = Some("need dictionary");
                    self.marker = 0;
                    return Z_STREAM_ERROR;
                }
                Mode::Blocks => {
                    r = self.blocks(r);
                    if r == Z_DATA_ERROR {
                        self.mode = Mode::Bad;
                        self.marker = 0;
                        continue;
                    }
                    if r == Z_OK {
                        r = f;
                    }
                    if r != Z_STREAM_END {
                        return r;
                    }
                    r = f;
                    self.was = self.blocks_reset();
                    if self.nowrap {
                        self.mode = Mode::Done;
                        continue;
                    }
                    self.mode = Mode::Check4;
                }
                Mode::Check4 => {
                    needbyte!();
                    self.need = u32::from(nextbyte!()) << 24;
                    self.mode = Mode::Check3;
                }
                Mode::Check3 => {
                    needbyte!();
                    self.need = self.need.wrapping_add(u32::from(nextbyte!()) << 16);
                    self.mode = Mode::Check2;
                }
                Mode::Check2 => {
                    needbyte!();
                    self.need = self.need.wrapping_add(u32::from(nextbyte!()) << 8);
                    self.mode = Mode::Check1;
                }
                Mode::Check1 => {
                    needbyte!();
                    self.need = self.need.wrapping_add(u32::from(nextbyte!()));
                    if self.was != self.need {
                        self.mode = Mode::Bad;
                        self.msg = Some("incorrect data check");
                        self.marker = 5;
                        continue;
                    }
                    self.mode = Mode::Done;
                }
                Mode::Done => return Z_STREAM_END,
                Mode::Bad => return Z_DATA_ERROR,
            }
        }
    }
}

/// uncompress (uncompress.c): `dest` is the destination buffer (its length is destLen
/// on input), returns the code (Z_OK, Z_BUF_ERROR, Z_DATA_ERROR, Z_NEED_DICT…) and destLen on output.
pub fn uncompress(dest: &mut [u8], source: &[u8]) -> (i32, usize) {
    let mut z = Inflate::new(source, dest, false);
    let err = z.inflate(Z_FINISH);
    if err != Z_STREAM_END {
        return (if err == Z_OK { Z_BUF_ERROR } else { err }, z.out.len());
    }
    // inflateEnd returns Z_OK; inflate_blocks_free and inflate_codes_free only free
    // the memory (here when z goes out of scope).
    (Z_OK, z.total_out as usize)
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::write::ZlibEncoder;
    use flate2::Compression;
    use std::io::Write;

    fn compressed(raw: &[u8], level_value: u32) -> Vec<u8> {
        let mut e = ZlibEncoder::new(Vec::new(), Compression::new(level_value));
        e.write_all(raw).unwrap();
        e.finish().unwrap()
    }

    fn patterns(n: usize, seed_value: u32) -> Vec<u8> {
        let mut state_value = seed_value;
        (0..n)
            .map(|i| {
                state_value = state_value.wrapping_mul(1664525).wrapping_add(1013904223);
                match (seed_value >> 3) % 4 {
                    0 => (state_value >> 24) as u8,
                    1 => b"abcabcabd "[i % 10],
                    2 => ((state_value >> 28) as u8) * 3,
                    _ => (i / 7) as u8,
                }
            })
            .collect()
    }

    /// Valid flate2 streams (stored, fixed, dynamic) give back their data.
    #[test]
    fn valid_stream() {
        for seed_value in 0..40u32 {
            for &n in &[0usize, 1, 5, 100, 1000, 5000, 70000] {
                let raw = patterns(n, seed_value);
                for level_value in [0u32, 1, 6, 9] {
                    let z = compressed(&raw, level_value);
                    let mut out = vec![0u8; raw.len()];
                    let (code, len) = uncompress(&mut out, &z);
                    assert_eq!((code, len), (Z_OK, raw.len()), "seed {seed_value} n {n} level {level_value}");
                    assert_eq!(out, raw);
                }
            }
        }
    }

    /// Insufficient capacity: Z_BUF_ERROR; truncated stream: Z_BUF_ERROR; wrong header: Z_DATA_ERROR.
    #[test]
    fn simple_errors() {
        let raw = patterns(2000, 5);
        let z = compressed(&raw, 6);
        let mut small = vec![0u8; 1999];
        assert_eq!(uncompress(&mut small, &z).0, Z_BUF_ERROR);
        let mut out = vec![0u8; 2000];
        assert_eq!(uncompress(&mut out, &z[..z.len() - 3]).0, Z_BUF_ERROR);
        let mut bad = z.clone();
        bad[0] = 0x79;
        assert_eq!(uncompress(&mut out, &bad).0, Z_DATA_ERROR);
        let mut sum_value = z.clone();
        let l = sum_value.len();
        sum_value[l - 1] ^= 1;
        assert_eq!(uncompress(&mut out, &sum_value).0, Z_DATA_ERROR);
    }

    /// Copy distance larger than the history: the zero window is read back (differs from flate2,
    /// minimal distance case).
    #[test]
    fn distance_before_history() {
        let input_value = [0x78, 0x01, 0x03, 0x02, 0x00, 0x00, 0x03, 0x00, 0x01];
        let mut out = [0xa5u8; 3];
        assert_eq!(uncompress(&mut out, &input_value), (Z_OK, 3));
        assert_eq!(out, [0, 0, 0]);
    }

    fn hex(s: &str) -> Vec<u8> {
        if s == "-" {
            return Vec::new();
        }
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    fn fnv(data: &[u8]) -> u64 {
        data.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3))
    }

}

