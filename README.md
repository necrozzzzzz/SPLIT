# SPLIT
Deadlock Practice Tool
SPLIT is built for Deadlock players.

Save and load positions instantly,
Switch presets quickly,
Keep favorite saves separated,
And retry movement routes faster

## Features

- 8 Save state slots
- 4 Presets
- ☆ Favorite slots
- ↶ Undo / ↷ Redo
- Import / Export presets
- Tag colors
- Automatic backups

## Build

```bash
npm run build
npm run tauri build -- --no-bundle
```

Both historical commands default to the Borderless edition.

### Edition builds

`SPLIT_EDITION` is the single build-time selector shared by Vite and Rust. Use
the dedicated commands instead of setting it manually. Each Tauri command also
merges its small edition override after the shared `tauri.conf.json`; conflicting
values from the override win, while all common configuration remains inherited.

```bash
npm run build:borderless
npm run build:fullscreen
npm run tauri:build:borderless
npm run tauri:build:fullscreen
npm run tauri:bundle:borderless
npm run tauri:bundle:fullscreen
npm run build:editions
npm run bundle:editions
```

- `borderless` keeps the current Windows/Tauri Quick Access and native Win32
  notifications.
- `fullscreen` keeps the same keyboard-controlled save-state core and Discord
  Presence, without Quick Access or native in-game notification overlays.

The application builds are retained side by side:

```text
artifacts/borderless/SPLIT-Borderless.exe
artifacts/fullscreen/SPLIT-Fullscreen.exe
```

The bundle commands additionally retain these NSIS installers:

```text
artifacts/borderless/SPLIT-Borderless-Setup-2.0.0-1.exe
artifacts/fullscreen/SPLIT-Fullscreen-Setup-2.0.0-1.exe
```

The two Tauri identities are deliberately distinct, so Windows installation,
shortcuts, autostart entries, WebView data, window state and single-instance
mutexes do not collide. SPLIT's explicitly managed savestates, screenshots and
configuration remain shared under `%APPDATA%\\SPLIT`.
