# Third-party code and assets

OpenXIII is a reimplementation made by studying the original game; it contains none of the
game's files (you need your own copy of XIII). OpenXIII is licensed under the GNU General Public
License v3.0 or later (`LICENSE`). The parts below come from, or are based on, other projects and
keep their own licences. Rust crates downloaded by Cargo (listed in `Cargo.lock`) are not part of
this repository; each keeps its own licence.

## vgmstream (Ubisoft ADPCM decoder)

`crates/openxiii-assets/src/ubi_adpcm.rs` is a Rust port of vgmstream's Ubisoft ADPCM decoder
(`src/coding/ubi_adpcm_decoder.c`, https://github.com/vgmstream/vgmstream). vgmstream's licence:

```
Copyright (c) 2008-2025 Adam Gashlin, Fastelbja, Ronny Elfert, bnnm,
                        Christopher Snowhill, NicknineTheEagle, bxaimc,
                        Thealexbarney, CyberBotX, EdnessP, et al

Portions Copyright (c) 2004-2008, Marko Kreen
Portions Copyright 2001-2007  jagarl / Kazunori Ueno <jagarl@creator.club.ne.jp>
Portions Copyright (c) 1998, Justin Frankel/Nullsoft Inc.
Portions Copyright (C) 2006 Nullsoft, Inc.
Portions Copyright (c) 2005-2007 Paul Hsieh
Portions Copyright (C) 2000-2004 Leshade Entis, Entis-soft.
Portions Public Domain originating with Sun Microsystems

Permission to use, copy, modify, and distribute this software for any
purpose with or without fee is hereby granted, provided that the above
copyright notice and this permission notice appear in all copies.

THE SOFTWARE IS PROVIDED "AS IS" AND THE AUTHOR DISCLAIMS ALL WARRANTIES
WITH REGARD TO THIS SOFTWARE INCLUDING ALL IMPLIED WARRANTIES OF
MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE FOR
ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES
WHATSOEVER RESULTING FROM LOSS OF USE, DATA OR PROFITS, WHETHER IN AN
ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING OUT OF
OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.
```

## zlib 1.1.3 (inflate)

`crates/openxiii-vm/src/storage/zlib11.rs` reimplements in Rust the "inflate" part of zlib 1.1.3 (the version the
game uses), so that damaged streams behave as in the game. zlib's licence:

```
(C) 1995-1998 Jean-loup Gailly and Mark Adler

This software is provided 'as-is', without any express or implied
warranty.  In no event will the authors be held liable for any damages
arising from the use of this software.

Permission is granted to anyone to use this software for any purpose,
including commercial applications, and to alter it and redistribute it
freely, subject to the following restrictions:

1. The origin of this software must not be misrepresented; you must not
   claim that you wrote the original software. If you use this software
   in a product, an acknowledgment in the product documentation would be
   appreciated but is not required.
2. Altered source versions must be plainly marked as such, and must not be
   misrepresented as being the original software.
3. This notice may not be removed or altered from any source distribution.

Jean-loup Gailly        Mark Adler
jloup@gzip.org          madler@alumni.caltech.edu
```

This file is an altered (rewritten) version of zlib's inflate.

## FxHasher (rustc-hash)

The `FxHasher` in `crates/openxiii-vm/src/script/types.rs` is a small Rust port of the hasher of the Rust compiler,
published as the `rustc-hash` crate (https://github.com/rust-lang/rustc-hash, the Rust Project Developers),
dual-licensed under MIT or Apache-2.0 (https://github.com/rust-lang/rustc-hash/blob/master/LICENSE-MIT,
https://github.com/rust-lang/rustc-hash/blob/master/LICENSE-APACHE). MIT licence text:

```
Permission is hereby granted, free of charge, to any person obtaining a copy of this software and
associated documentation files (the "Software"), to deal in the Software without restriction,
including without limitation the rights to use, copy, modify, merge, publish, distribute,
sublicense, and/or sell copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all copies or
substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT
NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND
NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES
OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN
CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
```

## wgpu-hal and wgpu-core

`vendor/wgpu-hal` and `vendor/wgpu-core` are patched copies of wgpu-hal and wgpu-core 30.0.1
(https://github.com/gfx-rs/wgpu, the gfx-rs developers), dual-licensed under MIT or Apache-2.0: see the
`LICENSE.MIT` and `LICENSE.APACHE` files in each folder. What OpenXIII changed is described in each folder's
`README-OpenXIII.md`.

## Outfit font

`crates/openxiii-viewer/icon/font/Outfit-Variable.ttf` (used to draw OpenXIII's icon) is the Outfit font,
licensed under the SIL Open Font License 1.1: see `crates/openxiii-viewer/icon/font/OFL.txt`.

## FFmpeg (built separately)

FFmpeg is not part of this repository. OpenXIII plays the game's cutscenes by running a small FFmpeg build
(LGPL 2.1 or later, built without `--enable-gpl` and without `--enable-nonfree`); the scripts in
`android/ffmpeg/` rebuild it from the official FFmpeg 7.1.2 sources. When an APK bundles it, the APK also
contains its licence text and a source notice (`assets/licenses/ffmpeg/`).

## Lua (mods build only)

The mods build (cargo feature `mods`, turned on by `improved`) runs the mods' Lua scripts
(docs/modding). The faithful build includes none of this.

| Component | Version | Licence | Part of the executable |
|---|---|---|---|
| Lua (sources built with the game) | 5.4.8 | MIT, Lua.org, PUC-Rio | yes |
| mlua (Rust bindings) | 0.11.6 | MIT | yes |
| mlua-sys | 0.10.0 | MIT | yes |
| lua-src (Lua's sources, built by cargo) | 550.0.0 | MIT | its build of Lua |
| luajit-src | 210.6.6 | MIT | no (build time only, not used with Lua 5.4) |
| bstr, either, num-traits, parking_lot, parking_lot_core, lock_api, scopeguard, smallvec, rustc-hash, libc, cfg-if, windows-link, rustversion | see Cargo.lock | MIT or Apache-2.0 | yes |
| memchr | see Cargo.lock | Unlicense or MIT | yes |
| cc, pkg-config, which | see Cargo.lock | MIT or Apache-2.0 (which: MIT) | no (build time only) |

### Lua

```
Copyright (C) 1994-2025 Lua.org, PUC-Rio.

Permission is hereby granted, free of charge, to any person obtaining
a copy of this software and associated documentation files (the
"Software"), to deal in the Software without restriction, including
without limitation the rights to use, copy, modify, merge, publish,
distribute, sublicense, and/or sell copies of the Software, and to
permit persons to whom the Software is furnished to do so, subject to
the following conditions:

The above copyright notice and this permission notice shall be
included in all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND,
EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF
MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT.
IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY
CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT,
TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE
SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
```

### mlua and mlua-sys

```
MIT License

Copyright (c) 2019-2021 A. Orlenko
Copyright (c) 2017 rlua

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## ScaleFX and SMAA (improved build only)

The improved build (cargo feature `improved`) redraws the 15 onomatopoeias of the game's menus
with smooth outlines when Graphics is set to Enhanced (crates/openxiii-viewer/src/onomatopoeias).
Both algorithms are ported to Rust from their shader sources. The faithful build includes none
of this.

| Component | Source | Licence | Part of the executable |
|---|---|---|---|
| ScaleFX (ported to Rust) | libretro shaders, ScaleFX by Sp00kyFox | MIT | yes |
| SMAA 1x and its area and search textures (ported to Rust) | github.com/iryoku/smaa | MIT | yes |

### ScaleFX

```
ScaleFX: Copyright (c) 2016 Sp00kyFox - ScaleFX@web.de

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in
all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
THE SOFTWARE.
```

### SMAA

```
Copyright (C) 2013 Jorge Jimenez (jorge@iryoku.com)
Copyright (C) 2013 Jose I. Echevarria (joseignacioechevarria@gmail.com)
Copyright (C) 2013 Belen Masia (bmasia@unizar.es)
Copyright (C) 2013 Fernando Navarro (fernandn@microsoft.com)
Copyright (C) 2013 Diego Gutierrez (diegog@unizar.es)

Permission is hereby granted, free of charge, to any person obtaining a copy
this software and associated documentation files (the "Software"), to deal in
the Software without restriction, including without limitation the rights to
use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies
of the Software, and to permit persons to whom the Software is furnished to
do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in
all copies or substantial portions of the Software. As clarification, there
is no requirement that the copyright notice and permission be included in
binary distributions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## Android libraries and Rust crates

The Android app uses AndroidX libraries (`androidx.games:games-activity`, `androidx.appcompat:appcompat`,
`androidx.core:core`) and the Kotlin standard library, all under the Apache License 2.0; Gradle downloads them
when the APK is built, they are not part of this repository. The Rust crates the code depends on are listed,
with their versions, in `Cargo.lock` and `crates/openxiii-android/Cargo.lock`; all of them are under
permissive licences (MIT, Apache-2.0, BSD, Zlib, ISC or Unicode-3.0).
