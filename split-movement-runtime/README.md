# SPLIT Movement Runtime

Native engine-side foundation for SPLIT's Movement Lab.

## Live validation

The SPLIT-owned runtime has live validation for Source 2 startup, world lifecycle, server frames, client lifecycle and pawn pose/movement observation.

## Current milestone: real input capture + route recorder

The runtime now detours the server's current `CBasePlayerController::ProcessUsercmds` signature and captures the newest real command for each player slot:

- view angles
- forward axis
- left axis
- held button mask
- pressed button mask

Each route tick pairs those controls with the already validated server-side pawn sample:

- frame
- position
- velocity
- grounded
- pawn handle

The JSON route format is now `split-movement-route v2`. `status` / `stop` report how many recorded ticks also have an input sample, for example:

```text
[SPLIT][Recorder] stopped ticks=812 input=811/812 distance=... max_speed=...
```

## Runtime commands

```text
record
status
stop
save
clear
```

Routes are saved under `split-movement-runtime/routes/`.

## Native detour dependency

The runtime uses SafetyHook v0.7.0 at build time for the inline `ProcessUsercmds` detour. It is fetched by CMake and linked into the SPLIT runtime; users do not run a separate hook executable or Modlock runtime.

## Next

Once live input capture is validated, build the first playback path: create/control a bot and feed the recorded command stream back into Source 2, then compare the replayed pose to the recorded pose for drift correction.
