#!/bin/bash
# Builds libffmpeg.so (arm64-v8a) for the OpenXIII APK: a tiny ffmpeg that only reads the game's
# .bik videos (RGB565 frames, PCM audio), LGPL like the PC ffmpeg.exe (build-pc-reference.sh).
# Uses the NDK already installed in the Android
# tools folder and the verified sources of the work folder (ffmpeg-7.1.2, SHA-256
# 089bc60fb59d6aecc5d994ff530fd0dcb3ee39aa55867849a2bbc4e555f9c304).
# Android tools folder: OXIII_TOOLS, default: OpenXIII-android-tools next to the repository
# (same default as android/build-apk.ps1).
# Work folder (sources tarball, extracted sources, build objects): OXIII_FFMPEG_WORK, default:
# OpenXIII-build/ffmpeg-android next to the repository.
# Output: <tools>/ffmpeg/arm64-v8a/libffmpeg.so, which android/build-apk.ps1 copies into the
# APK on its own (jniLibs\arm64-v8a).
# --configure-only: only checks the configure step (about one minute).
set -euo pipefail

REPO=$(cd "$(dirname "$0")/../.." && pwd)
WORK=${OXIII_FFMPEG_WORK:-$(dirname "$REPO")/OpenXIII-build/ffmpeg-android}
SRC=$WORK/ffmpeg-7.1.2
TOOLS=${OXIII_TOOLS:-$(dirname "$REPO")/OpenXIII-android-tools}
API=26
TRIPLE=aarch64-linux-android
NDK=$(ls -d "$TOOLS"/sdk/ndk/* | sort | tail -1)
BIN=$NDK/toolchains/llvm/prebuilt/windows-x86_64/bin
# MSYS2 make (the NDK's make.exe is a Windows program that does not understand configure's
# /d/... paths). The script runs in MSYS2 UCRT64 (see the .cmd).
MAKE=/usr/bin/make
[ -x "$MAKE" ] || { echo "make not found: run this script through build-ffmpeg-android.cmd (MSYS2)."; exit 1; }
BUILD_DIR=$WORK/obj-arm64
OUT=$TOOLS/ffmpeg/arm64-v8a
# Host compiler (small tools run on the PC during the build): MSYS2 UCRT64 gcc, already used
# for the PC ffmpeg.exe; Git Bash has none.
HOST=/ucrt64/bin
[ -x "$HOST/gcc.exe" ] || { echo "MSYS2 gcc not found: $HOST/gcc.exe"; exit 1; }
export PATH="$PATH:$HOST"

if [ ! -x "$SRC/configure" ]; then
    echo "Extracting the sources..."
    (cd "$WORK" && tar -xJf ffmpeg-7.1.2.tar.xz)
fi
rm -rf "$BUILD_DIR"; mkdir -p "$BUILD_DIR"; cd "$BUILD_DIR"

echo "1/3 Configure..."
if ! "$SRC/configure" \
    --target-os=android --arch=aarch64 --enable-cross-compile --host-cc="$HOST/gcc.exe" \
    --cc="$BIN/clang.exe" --cxx="$BIN/clang++.exe" --ld="$BIN/clang.exe" \
    --ar="$BIN/llvm-ar.exe" --nm="$BIN/llvm-nm.exe" --ranlib="$BIN/llvm-ranlib.exe" --strip="$BIN/llvm-strip.exe" \
    --extra-cflags="--target=$TRIPLE$API -fPIE" --extra-ldflags="--target=$TRIPLE$API -pie" \
    --enable-static --disable-shared --enable-small --disable-debug --disable-doc \
    --disable-autodetect --disable-network --disable-avdevice --disable-postproc \
    --disable-ffplay --disable-ffprobe --enable-ffmpeg \
    --disable-everything \
    --enable-protocol=file,pipe \
    --enable-demuxer=bink \
    --enable-decoder=bink,binkaudio_rdft,binkaudio_dct \
    --enable-muxer=rawvideo,pcm_s16le \
    --enable-encoder=rawvideo,pcm_s16le \
    --enable-filter=scale,format,aformat,aresample,null,anull,copy,acopy,buffer,buffersink,abuffer,abuffersink \
    --enable-swscale --enable-swresample > configure.log 2>&1; then
    echo "configure failed (log: $BUILD_DIR/configure.log)"; tail -20 configure.log; exit 1
fi
if ! grep -q "^License: LGPL" configure.log; then
    echo "Unexpected license:"; grep "^License" configure.log; exit 1
fi
echo "   $(grep '^License' configure.log)"
if [ "${1:-}" = "--configure-only" ]; then
    echo "Configure succeeded (nothing compiled)."; exit 0
fi

echo "2/3 Build (a few minutes)..."
if ! "$MAKE" -j4 ffmpeg > make.log 2>&1; then
    echo "Build failed (log: $BUILD_DIR/make.log)"; grep -i -m 20 "error" make.log; exit 1
fi

echo "3/3 Copy to $OUT"
mkdir -p "$OUT"
"$BIN/llvm-strip.exe" -o "$OUT/libffmpeg.so" ffmpeg
cp "$SRC/COPYING.LGPLv2.1" "$OUT/"
echo "Done: $OUT/libffmpeg.so ($(stat -c %s "$OUT/libffmpeg.so") bytes)"
echo "Next: rebuild the APK (android\\build-apk.ps1), it will pick up this file."
