# SPLIT Movement Runtime

Native engine-side foundation for SPLIT's Movement Lab.

SPLIT does **not** depend on `modlock.exe`. The runtime loads Source 2 itself and owns the movement-specific integration.

## Validated live

On the current Deadlock build the SPLIT-owned runtime has been validated live through:

- `engine2.dll`
- `server.dll`
- `schemasystem.dll`
- `SchemaSystem_001`
- `Source2Main`
- `INetworkServerService::StartupServer`
- `ISource2Server::GameFrame`

A local `dl_midtown` server starts, a stock client can join it, and SPLIT receives real server frames.

## Current milestone: connection lifecycle

The runtime now also hooks `Source2GameClients001`:

- `ClientPutInServer` — slot 13
- `ClientFullyConnect` — slot 15
- `ClientDisconnect` — slot 16

This gives Movement Lab the authoritative human slot/XUID/name and the exact point where the client has completed sign-on. That is the prerequisite for attaching the pawn observer and for any one-shot client initialization workaround such as the camera refresh.

The slot values are pinned to the vendored 2026-10-28 game data for the prototype. Production will verify them from SPLIT's own signature database.

## Start

```powershell
.\build\Release\split-movement-runtime.exe --game-dir "C:\Program Files (x86)\Steam\steamapps\common\Deadlock" --server
```

After joining, expected SPLIT lines include:

```text
[SPLIT][Movement] client entered slot=0 xuid=... bot=no ready=no name="..."
[SPLIT][Movement] client fully connected slot=0 xuid=... bot=no ready=yes name="..."
[SPLIT][Movement] human session ready on slot 0; pawn observation can now attach to this slot.
```

## Next

Resolve the live entity system + schema field offsets, attach to that human slot's controller/pawn, then emit position, velocity and grounded state from the real server frame.
