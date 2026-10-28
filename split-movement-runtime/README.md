# SPLIT Movement Runtime

This is the native engine-side foundation for SPLIT's Movement Lab.

It is intentionally separate from the Tauri application. The desktop app will eventually launch/control this runtime; the runtime will own the Deadlock-specific engine integration needed for tick-perfect movement recording and ghost playback.

## Direction

We are **not** depending on `modlock.exe`.

Instead, SPLIT vendors the MIT-licensed engine-interoperability pieces we need from `paralin/modlock`, then integrates and maintains them inside a SPLIT-owned runtime.

The initial imported scope contains the core pieces needed for the movement path:

- movement hook;
- command/input hooks;
- pawn observer;
- schema/entity access;
- signature and virtual-slot resolution;
- connection tracking;
- bot creation;
- frame hook;
- Windows module/native-memory support.

The upstream snapshot lives under:

`vendor/modlock/`

See `THIRD_PARTY_NOTICES.md` and the vendored `LICENSE`.

## Current phase

Phase 1 deliberately keeps the SPLIT-owned executable buildable while the vendored engine code is still isolated.

The capability sequence is:

1. engine/module + SchemaSystem bootstrap;
2. frame hook;
3. pawn observer;
4. movement/command capture;
5. bot creation/control;
6. tick recorder;
7. steer-style ghost replay;
8. IPC to the Tauri app;
9. route persistence, PBs, segments and community backend.

The old `movement-lab-prototype/` remains a reference proof of the Modlock API idea, but it is **not** the architecture we are shipping.

## Build the SPLIT-owned runtime

From this directory:

```powershell
cmake -S . -B build
cmake --build build --config Release
.\build\Release\split-movement-runtime.exe
```

At this stage the executable reports which native capabilities have been wired. They intentionally start as `not wired`; we enable them one by one as their dependency closure is integrated and tested.
