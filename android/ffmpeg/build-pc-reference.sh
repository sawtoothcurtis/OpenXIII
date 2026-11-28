#!/bin/bash
# Builds the tiny ffmpeg.exe shipped with OpenXIII on PC: it only reads the game's .bik videos
# (RGB565 frames and PCM audio, as crates/openxiii-vm/src/render/video.rs expects).
# LGPL license (no GPL, nothing "nonfree"): it may be shipped with the game.
# Run it in an MSYS2 UCRT64 terminal. Reference copy of the script used for the shipped ffmpeg.exe.
# Build folder: OXIII_FFMPEG_PC, default: OpenXIII-ffmpeg-pc next to the repository, with the
# sources in src/ffmpeg-7.1.2; objects go to build/obj and the result to dist.
set -e

REPO=$(cd "$(dirname "$0")/../.." && pwd)
ROOT=${OXIII_FFMPEG_PC:-$(dirname "$REPO")/OpenXIII-ffmpeg-pc}
SOURCES=$ROOT/src/ffmpeg-7.1.2
BUILD_DIR=$ROOT/build/obj
OUT=$ROOT/dist
VERSION=7.1.2
URL=https://ffmpeg.org/releases/ffmpeg-7.1.2.tar.xz
SHA256=089bc60fb59d6aecc5d994ff530fd0dcb3ee39aa55867849a2bbc4e555f9c304

# Needs the UCRT64 compiler (package mingw-w64-ucrt-x86_64-gcc) and make.
if ! command -v gcc > /dev/null || ! command -v make > /dev/null; then
    echo "gcc or make not found: run this script from the MSYS2 UCRT64 terminal."
    exit 1
fi
if [ ! -x "$SOURCES/configure" ]; then
    echo "ffmpeg sources not found in $SOURCES"
    exit 1
fi

# Settings: everything off, then only what the .bik files need is turned back on.
# --disable-x86asm: nasm is not installed (decoding is a little slower, nothing more).
# --extra-ldflags=-static: a single .exe, no DLL next to it.
CONFIG="--prefix=$BUILD_DIR/prefix \
 --disable-everything --disable-autodetect --disable-network --disable-doc \
 --disable-ffplay --disable-ffprobe --enable-ffmpeg --enable-small --disable-x86asm \
 --enable-static --disable-shared --extra-ldflags=-static \
 --enable-protocol=file,pipe \
 --enable-demuxer=bink \
 --enable-decoder=bink,binkaudio_dct,binkaudio_rdft \
 --enable-muxer=rawvideo,pcm_s16le \
 --enable-encoder=rawvideo,pcm_s16le \
 --enable-filter=scale,format,aresample,aformat,null,anull,buffer,buffersink,abuffer,abuffersink \
 --enable-swscale --enable-swresample"

# Clean work folder for every attempt.
rm -rf "$BUILD_DIR"
mkdir -p "$BUILD_DIR" "$OUT"
cd "$BUILD_DIR"

echo "1/3 Configure..."
if ! "$SOURCES/configure" $CONFIG > configure.log 2>&1; then
    echo "configure failed: end of the log below (full log: $BUILD_DIR/configure.log)"
    tail -20 configure.log
    exit 1
fi

echo "2/3 Build (a few minutes, 4 cores)..."
if ! make -j 4 > make.log 2>&1; then
    echo "Build failed: first errors below (full log: $BUILD_DIR/make.log)"
    grep -i -m 20 "error" make.log
    exit 1
fi

echo "3/3 Copy to $OUT"
cp ffmpeg.exe "$OUT/ffmpeg.exe"
cp "$SOURCES/COPYING.LGPLv2.1" "$OUT/COPYING.LGPLv2.1"
cat > "$OUT/README.txt" <<END
Minimal ffmpeg.exe for OpenXIII (plays the game's .bik videos)

Version   : FFmpeg $VERSION
Sources   : $URL
SHA-256   : $SHA256
License   : LGPL 2.1 or later (see COPYING.LGPLv2.1)
Built     : $(date '+%Y-%m-%d %H:%M') with $(gcc --version | head -1) (MSYS2 UCRT64)

Configure line:
$(echo $CONFIG)

The exact sources are available at the address above; this file rebuilds them
identically with android/ffmpeg/build-pc-reference.sh from the OpenXIII repository.
END

echo
echo "License reported by ffmpeg -L:"
"$OUT/ffmpeg.exe" -hide_banner -L | head -3
echo
echo "Size: $(stat -c %s "$OUT/ffmpeg.exe") bytes"
echo "Done: $OUT/ffmpeg.exe"
