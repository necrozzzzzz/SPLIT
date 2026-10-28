# SPLIT Movement Lab prototype

This folder is intentionally isolated from the current SPLIT/Tauri application.

Goal: prove the core Hyperline-style loop before integrating anything into SPLIT:

1. record the local hero every Deadlock server tick;
2. keep the real movement command plus the resulting position/velocity/grounded/facing pose;
3. spawn a bot using the same hero;
4. replay each recorded command through Modlock's `Player.steer()`;
5. make the replay bot translucent and non-targetable as a ghost.

## Commands

- `/splitrec` — start recording.
- `/splitstop` — stop and keep the latest route in memory.
- `/splitghost` — spawn and replay that route.
- `/splitclear` — clear the route and remove the ghost.

The route is deliberately in-memory for this first proof of concept. Persistence, segments, PBs, leaderboards, sharing and SPLIT UI integration come only after record/replay is validated in Deadlock.

## Run

Install a current Modlock release, open this folder, then run:

```powershell
modlock dev --watch
```

Modlock builds the TypeScript mod, starts its local Deadlock server and launches Deadlock into it.

## What this prototype validates

This does **not** fake a ghost by interpolating XYZ positions. Each recorded tick stores Modlock's `MovementCommand` and replays it with `Player.steer()`. Modlock runs the buttons and movement axes through Deadlock so jumps, slides, mantles and other movement use the game's own animation/simulation, while the recorded pose keeps the replay aligned.

No current SPLIT feature depends on this folder.
