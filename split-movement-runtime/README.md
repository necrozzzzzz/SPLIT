# SPLIT Movement Runtime

Native engine-side foundation for SPLIT's Movement Lab.

SPLIT does **not** depend on `modlock.exe`. The runtime loads Source 2 itself and owns the movement-specific integration.

## Validated milestone

The standalone runtime already resolves:

- `engine2.dll`
- `server.dll`
- `schemasystem.dll`
- `SchemaSystem_001`

## Current milestone: Source 2 frame/world lifecycle

The runtime now also contains SPLIT-owned Windows vtable hooks for:

- `ISource2Server::GameFrame` (current slot 19)
- `INetworkServerService::StartupServer` (current slot 26)

and a Source 2 `Source2Main` dedicated-server handoff.

The slot values are pinned to the vendored 2026-10-28 Modlock game data for this prototype. The production runtime will verify/resolve them from SPLIT's own signature database rather than blindly trusting constants.

## Build

```powershell
cmake -S . -B build
cmake --build build --config Release
```

## Probe only

```powershell
.\build\Release\split-movement-runtime.exe --game-dir "C:\Program Files (x86)\Steam\steamapps\common\Deadlock"
```

## Start the local Movement Lab server

```powershell
.\build\Release\split-movement-runtime.exe --game-dir "C:\Program Files (x86)\Steam\steamapps\common\Deadlock" --server
```

Expected SPLIT-specific lines include:

```text
SPLIT Movement Runtime: engine bridge ready.
SPLIT Movement Runtime: frame/world hooks installed.
[SPLIT][Movement] loading world: dl_midtown
[SPLIT][Movement] world ready: dl_midtown
[SPLIT][Movement] server frame 1
```

The server is intentionally local/insecure. This is the isolated Movement Lab environment, not VAC-secured matchmaking.

## Next milestone

Once frame/world callbacks are validated live, wire:

1. connection tracking;
2. pawn observer;
3. movement command capture;
4. bot creation/control;
5. tick-perfect recorder and ghost replay.
