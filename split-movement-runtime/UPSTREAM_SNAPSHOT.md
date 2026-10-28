# Upstream snapshot

This directory records the first Modlock engine-interoperability import used to bootstrap SPLIT Movement Runtime.

Imported areas include selected files from:

- `include/modlock/gameinterop/`
- `src/gameinterop/`
- `src/gameinterop/windows/`
- `data/game_signatures.txtpb`
- `docs/engine-access.md`
- `include/modlock/export.h`
- `LICENSE`

The import intentionally starts with the movement-critical surface rather than the full Modlock WASM/runtime/backend stack.

Not imported as SPLIT architecture:

- Hyperline backend/publishing;
- Modlock CLI;
- WASM sandbox/Wasmtime host;
- language runtimes;
- WebRTC lobby system;
- Modlock UI/metrics/settings systems.

Those are outside the SPLIT Movement Runtime goal.
