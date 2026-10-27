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

## Route recorder — current limitations

SPLIT includes an experimental, read-only route diagnostic recorder. This is **not** a guaranteed frame-perfect BOTREC recorder or replay system.

- Position, velocity and camera telemetry are sampled, not a complete authoritative stream of game user commands.
- Physical keyboard press/release transitions are captured for default AZERTY/QWERTY movement bindings. Custom Deadlock key bindings are not synchronized.
- Engine-input observations are diagnostic and may miss commands; they are not yet used for BOTREC conversion.
- Bot replay can differ from player movement because Deadlock gives native bots an additional NPCClip collision interaction. This has been confirmed for a particular Haze wall-jump route; SPLIT does not alter game collision layers.
- Route recording and ghost playback are restricted to confirmed local practice contexts. Do not use them in live matches.

Do not advertise exact reproduction of every jump, dash, slide, melee or mantle sequence until independently validated in the shipping build.

## Release verification

Before publishing installers, verify both editions on Windows:

1. Run `npm run build` and `cargo test --manifest-path src-tauri/Cargo.toml`.
2. Build and launch both editions; verify installation, launch, update/clean uninstall, and that edition identities do not collide.
3. Verify save/load, all eight slots, presets, favorites, undo/redo, import/export, backups and restart persistence.
4. Verify that live-match safety restrictions still block actions when appropriate.
5. Record a route using default bindings; check input transitions, camera events, coherent telemetry, stop reasons and export integrity. Repeat with the keyboard-hook fallback if possible.
6. Verify BOTREC playback limitations are communicated without claiming frame-perfect replay.
7. Confirm no diagnostic VPK, modified map, or server replacement is required or packaged.

Passing the build commands alone is not sufficient for a public release.
