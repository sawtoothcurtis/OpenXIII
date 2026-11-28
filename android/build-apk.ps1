# Builds the OpenXIII APK for Android (64-bit phones, arm64-v8a).
# Usage, from the repository root: powershell -ExecutionPolicy Bypass -File android\build-apk.ps1
#   OXIII_ABI=x86_64   test APK for the PC emulator (OpenXIII-0.1-x86_64-test.apk).
#   OXIII_CHEATS=1     test version with the cheats (feature cheats; off by default).
#   OXIII_IMPROVED=1   improved build with mods (as the PC's improved build).
# Tools (JDK 17, Gradle, Android SDK and NDK): OXIII_TOOLS, default: the OpenXIII-android-tools
# folder next to the repository.
# Release signing (see android/app/build.gradle): OXIII_KEYSTORE, OXIII_KEYSTORE_PASSWORD,
# OXIII_KEY_ALIAS, OXIII_KEY_PASSWORD, or Gradle properties. Values missing from the environment are
# read from <tools>\build-apk.local.properties when that file exists (key=value lines: keystore,
# keystore.password, key.alias, key.password). That file stays on your
# machine; never commit it.
# FFmpeg for the cutscenes: <tools>\ffmpeg\<abi>\libffmpeg.so (android/ffmpeg/build-ffmpeg-android.cmd);
# when it is bundled, its LGPL license and source notice go into the APK (assets/licenses/ffmpeg).
# Output: android\output\OpenXIII-0.1.apk
$ErrorActionPreference = 'Stop'
function EnvOr([string]$new, [string]$old) {
    $v = [Environment]::GetEnvironmentVariable($new)
    if (-not $v -and $old) { $v = [Environment]::GetEnvironmentVariable($old) }
    return $v
}
$repo = Split-Path $PSScriptRoot -Parent
$tools = EnvOr 'OXIII_TOOLS'
if (-not $tools) { $tools = Join-Path (Split-Path $repo -Parent) 'OpenXIII-android-tools' }
if (-not (Test-Path $tools)) { throw "Android tools folder not found: $tools (set OXIII_TOOLS)" }

# Local settings (signing), never in the repository.
$local = Join-Path $tools 'build-apk.local.properties'
$settings = @{}
if (Test-Path $local) {
    foreach ($line in Get-Content $local) {
        if ($line -match '^\s*([A-Za-z.]+)\s*=\s*(.*?)\s*$') { $settings[$Matches[1]] = $Matches[2] }
    }
}
foreach ($pair in @(@('OXIII_KEYSTORE', 'keystore'), @('OXIII_KEYSTORE_PASSWORD', 'keystore.password'),
                    @('OXIII_KEY_ALIAS', 'key.alias'), @('OXIII_KEY_PASSWORD', 'key.password'))) {
    if (-not [Environment]::GetEnvironmentVariable($pair[0]) -and $settings[$pair[1]]) {
        Set-Item "env:$($pair[0])" $settings[$pair[1]]
    }
}

$api = 26
$abi = if ($env:OXIII_ABI) { $env:OXIII_ABI } else { 'arm64-v8a' }
$triple, $name = if ($abi -eq 'x86_64') { 'x86_64-linux-android', 'OpenXIII-0.1-x86_64-test.apk' } else { 'aarch64-linux-android', 'OpenXIII-0.1.apk' }
$env:OXIII_ABI = $abi
$env:JAVA_HOME = (Get-ChildItem (Join-Path $tools 'jdk') -Directory | Select-Object -First 1).FullName
$env:ANDROID_HOME = Join-Path $tools 'sdk'
$ndk = (Get-ChildItem (Join-Path $env:ANDROID_HOME 'ndk') -Directory | Sort-Object Name | Select-Object -Last 1).FullName
$env:ANDROID_NDK_HOME = $ndk
$llvm = Join-Path $ndk 'toolchains\llvm\prebuilt\windows-x86_64\bin'
$t = $triple -replace '-', '_'
Set-Item "env:CC_$t" (Join-Path $llvm "$triple$api-clang.cmd")
Set-Item "env:CXX_$t" (Join-Path $llvm "$triple$api-clang++.cmd")
Set-Item "env:AR_$t" (Join-Path $llvm 'llvm-ar.exe')
Set-Item "env:CARGO_TARGET_$($t.ToUpper())_LINKER" (Join-Path $llvm "$triple$api-clang.cmd")

# 1. Game library (libopenxiii.so)
Push-Location $repo
# Same target folder as the PC build: crates already built for Android stay there.
if (-not $env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR = Join-Path $repo 'target' }
try {
    $cargo = @('build', '--release', '--target', $triple, '--manifest-path', 'crates\openxiii-android\Cargo.toml')
    $features = @()
    # OXIII_CHEATS=1: test version with the cheats (off by default).
    if ((EnvOr 'OXIII_CHEATS') -eq '1') { $features += 'cheats' }
    # OXIII_IMPROVED=1: improved build (fixes beyond the original, Mods and Co-op pages, mods), as the PC's.
    if ((EnvOr 'OXIII_IMPROVED') -eq '1') { $features += 'improved' }
    if ($features) { $cargo += @('--features', ($features -join ',')) }
    cargo @cargo
    if ($LASTEXITCODE -ne 0) { throw "Rust build failed ($LASTEXITCODE)" }
} finally { Pop-Location }
$target = $env:CARGO_TARGET_DIR
$jniLibs = Join-Path $PSScriptRoot 'app\src\main\jniLibs'
if (Test-Path $jniLibs) { Remove-Item $jniLibs -Recurse -Force }
$jni = Join-Path $jniLibs $abi
New-Item -ItemType Directory -Force $jni | Out-Null
Copy-Item (Join-Path $target "$triple\release\libopenxiii.so") $jni -Force

# FFmpeg for the cutscenes, started by video.rs from nativeLibraryDir. LGPL 2.1: its license text
# and where to get its sources ship inside the APK with it.
$notices = Join-Path $PSScriptRoot 'app\src\main\assets\licenses'
if (Test-Path $notices) { Remove-Item $notices -Recurse -Force }
$ffmpegDir = Join-Path $tools "ffmpeg\$abi"
$ffmpeg = Join-Path $ffmpegDir 'libffmpeg.so'
if (Test-Path $ffmpeg) {
    $license = Join-Path $ffmpegDir 'COPYING.LGPLv2.1'
    if (-not (Test-Path $license)) { throw "FFmpeg license text missing next to $ffmpeg (COPYING.LGPLv2.1): rebuild it with android/ffmpeg/build-ffmpeg-android.cmd" }
    Copy-Item $ffmpeg $jni -Force
    $dest = Join-Path $notices 'ffmpeg'
    New-Item -ItemType Directory -Force $dest | Out-Null
    Copy-Item $license $dest -Force
    $notice = @(
        'This APK contains FFmpeg 7.1.2 (lib/<abi>/libffmpeg.so), used only to play the game''s .bik cutscenes.',
        'FFmpeg is licensed under the GNU Lesser General Public License version 2.1 or later (COPYING.LGPLv2.1).',
        'It was built without --enable-gpl and without --enable-nonfree.',
        'Sources: https://ffmpeg.org/releases/ffmpeg-7.1.2.tar.xz',
        'SHA-256: 089bc60fb59d6aecc5d994ff530fd0dcb3ee39aa55867849a2bbc4e555f9c304',
        'Build script with the exact configure options: android/ffmpeg/build-ffmpeg-android.sh in the OpenXIII repository.'
    )
    [IO.File]::WriteAllLines((Join-Path $dest 'NOTICE.txt'), $notice, (New-Object System.Text.UTF8Encoding($false)))
} else {
    Write-Host "No FFmpeg for ${abi}: the APK will have no cutscenes"
}
# Improved build: the example mods (examples/mods, one folder per mod, and its README.txt), copied
# by the launcher into OpenXIII\Mods\_Examples of the phone's storage.
$modExamples = Join-Path $PSScriptRoot 'app\src\main\assets\mod-examples'
if (Test-Path $modExamples) { Remove-Item $modExamples -Recurse -Force }
if ((EnvOr 'OXIII_IMPROVED') -eq '1') {
    New-Item -ItemType Directory -Force $modExamples | Out-Null
    Get-ChildItem (Join-Path $repo 'examples\mods') -Directory | ForEach-Object { Copy-Item $_.FullName $modExamples -Recurse -Force }
    Copy-Item (Join-Path $repo 'examples\mods\README.txt') $modExamples -Force
}
# Shared libc++ when the library depends on it (oboe audio).
$readelf = Join-Path $llvm 'llvm-readelf.exe'
if ((& $readelf -d (Join-Path $jni 'libopenxiii.so')) -match 'libc\+\+_shared') {
    Copy-Item (Join-Path $ndk "toolchains\llvm\prebuilt\windows-x86_64\sysroot\usr\lib\$triple\libc++_shared.so") $jni -Force
}

# 2. APK
Set-Content -Path (Join-Path $PSScriptRoot 'local.properties') -Value ("sdk.dir=" + ($env:ANDROID_HOME -replace '\\', '/')) -Encoding ASCII
$gradle = (Get-ChildItem (Join-Path $tools 'gradle') -Directory | Select-Object -First 1).FullName
Push-Location $PSScriptRoot
try {
    & (Join-Path $gradle 'bin\gradle.bat') --no-daemon -q assembleRelease
    if ($LASTEXITCODE -ne 0) { throw "Gradle failed ($LASTEXITCODE)" }
} finally { Pop-Location }
$output = Join-Path $PSScriptRoot 'output'
New-Item -ItemType Directory -Force $output | Out-Null
Copy-Item (Join-Path $PSScriptRoot 'app\build\outputs\apk\release\app-release.apk') (Join-Path $output $name) -Force
Write-Host "APK: $(Join-Path $output $name)"
