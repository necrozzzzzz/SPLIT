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
the dedicated commands instead of setting it manually:

```bash
npm run build:borderless
npm run build:panorama
npm run tauri:build:borderless
npm run tauri:build:panorama
```

- `borderless` keeps the current Windows/Tauri Quick Access and native Win32
  notifications.
- `panorama` builds the shared application and backend without those Windows
  renderer services. The future Panorama bridge is intentionally not present
  yet.
