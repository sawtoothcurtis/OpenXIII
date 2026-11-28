# openxiii-update

At launch of the released game, before its window opens, OpenXIII asks GitHub which release is
the latest. When it is newer than this build, a small system window says so: **Open the download
page**, **Later** or **Don't ask again**. Nothing is downloaded or installed: the release page
opens in the browser, and the player updates by hand. Nothing here runs inside the game.

- One request per launch, to `github.com` only (the address `releases/latest` redirects to).
  Nothing is sent about the player or the machine.
- Turned off by "Don't ask again", or by `Auto=false` in `update.ini` (next to `OpenXIII.ini`).
  On Android: the launcher's "Say when a new version exists" button (open it with the icon's
  "Game folder" shortcut); the notice is `android/app/src/main/java/org/openxiii/UpdateNotice.java`.
- For a release to be noticed: tag it `v<version>` (for example `v0.2.0`), with a version above
  `[workspace.package] version` in `Cargo.toml` (Android: `versionName` in
  `android/app/build.gradle`), and mark it as the latest release on GitHub.
