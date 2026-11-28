# OpenXIII

An open-source reimplementation of **XIII** (Ubisoft, 2003), reverse-engineered from the original PC game.
Runs on Windows and Android.

You need your own copy of the PC version (Steam or GOG): OpenXIII contains none of the game's files.

Not affiliated with or endorsed by Ubisoft.

**Download:** [Windows](https://github.com/sawtoothcurtis/OpenXIII/releases/latest/download/OpenXIII-v0.1.0-windows.zip) ·
[Android](https://github.com/sawtoothcurtis/OpenXIII/releases/latest/download/OpenXIII-v0.1.0-android.apk)

## Features

- Modern resolutions and aspect ratios (widescreen, ultrawide).
- New Enhanced graphics settings, with an Original mode to play the game as it was.
- A split-screen co-op mod for the campaign, and multiplayer (split screen and LAN).
- Controller support, with its own settings and the console versions' aim assist.
- Android port, with touch controls or a controller.
- Modding API (TOML and Lua), with examples and [documentation](docs/modding/README.md).

## Install

**Windows**

1. [Download the Windows zip](https://github.com/sawtoothcurtis/OpenXIII/releases/latest/download/OpenXIII-v0.1.0-windows.zip) and unzip it anywhere.
2. Run `XIII.exe` and pick your original game's `.exe` or folder.

**Android** (64-bit)

1. Copy your original game's folder to the phone.
2. [Download the APK](https://github.com/sawtoothcurtis/OpenXIII/releases/latest/download/OpenXIII-v0.1.0-android.apk) on the phone and install it.
3. Open OpenXIII and pick that folder.

Settings and saves are in `%APPDATA%\OpenXIII` (Windows) or the app's storage (Android). The game checks for
updates at startup.

## Build from source

With Rust (https://rustup.rs), on Windows:

```
cargo build --release -p openxiii-viewer --features improved,dx11
```

This gives `target\release\oxiii-view.exe` (the released `XIII.exe`). Without `--features`, you get the original
game only, without Enhanced graphics or mods.

Android: run `android/build-apk.ps1` (see the comments at its top).

Tests: `cargo test`, with `OXIII_GAME` set to the game's folder.

## Credits

OpenXIII was developed with Claude Code.

## License

OpenXIII is free software under the GNU General Public License v3.0 or later (see `LICENSE`). Third-party
code keeps its own licence: see `THIRD-PARTY.md`.
