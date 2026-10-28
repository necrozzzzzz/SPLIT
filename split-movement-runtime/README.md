# SPLIT Movement Runtime

Native engine-side foundation for SPLIT's Movement Lab.

## Architecture

SPLIT does **not** depend on `modlock.exe`.

The runtime is a SPLIT-owned Windows process. It loads the Deadlock Source 2 modules itself, resolves the engine interfaces it needs, then will install the movement-specific hooks used for recording and ghost playback.

Selected MIT-licensed Modlock engine-interoperability sources are preserved under `vendor/modlock/` as upstream reference / integration material. See `THIRD_PARTY_NOTICES.md`.

## Current milestone: live engine bridge

The runtime can now:

1. validate a Deadlock installation;
2. register the game's DLL directories;
3. load and pin `engine2.dll`;
4. load and pin `server.dll`;
5. resolve/load `schemasystem.dll`;
6. call its `CreateInterface` export;
7. resolve `SchemaSystem_001`.

This is deliberately a **probe** for now. It does not start a map or install movement hooks yet.

## Build

```powershell
cmake -S . -B build
cmake --build build --config Release
```

## Probe your Deadlock install

From `split-movement-runtime`:

```powershell
.\build\Release\split-movement-runtime.exe --game-dir "C:\Program Files (x86)\Steam\steamapps\common\Deadlock"
```

or:

```powershell
$env:DEADLOCK_DIR="C:\Program Files (x86)\Steam\steamapps\common\Deadlock"
.\build\Release\split-movement-runtime.exe
```

Expected milestone output:

```text
SPLIT Movement Runtime 0.2.0
  game layout: ready
  engine2.dll: ready
  server.dll: ready
  schemasystem.dll: ready
  SchemaSystem_001: ready
  movement hook: not ready
  command hook: not ready
  pawn observer: not ready
  bot control: not ready
```

If `SchemaSystem_001` is ready, our standalone SPLIT-owned process has successfully entered the same Source 2 engine/interface layer needed by the later hooks.

## Next

The next milestone is the real engine handoff + frame/world lifecycle. Once a world is live, we install pawn observation and then movement/command hooks.
