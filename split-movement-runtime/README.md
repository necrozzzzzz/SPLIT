# SPLIT Movement Runtime

Native engine-side foundation for SPLIT's Movement Lab.

## Live validation

The SPLIT-owned runtime now has live validation for Source 2 startup, world lifecycle, server frames, client lifecycle and pawn pose/movement observation.

## Current milestone: tick recorder

The runtime now contains a first SPLIT-owned route recorder. While the local Movement Lab server is running, type commands directly in its PowerShell:

```text
record
status
stop
save
clear
```

`record` starts a fresh buffer. Every observed server frame then stores:

- server frame number
- pawn handle
- position
- velocity
- grounded state

`stop` freezes the current recording and prints tick count, accumulated 3D distance and maximum observed speed.

`save` writes the current recording under:

```text
split-movement-runtime/routes/
```

using the first local route schema:

```text
split-movement-route v1
```

This is intentionally local and simple. Tauri/IPC, named routes, PBs, segments and community publishing come after the movement command stream is captured.

## Next

Capture the player's actual movement command/buttons each tick and pair them with these pose samples. That gives SPLIT enough information for deterministic-ish bot steering / ghost replay instead of merely interpolating XYZ positions.
