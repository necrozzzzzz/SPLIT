# SPLIT Movement Runtime

Native engine-side foundation for SPLIT's Movement Lab.

SPLIT does **not** depend on `modlock.exe`.

## Validated live

The SPLIT-owned runtime has been validated live through:

- Source 2 module loading and `SchemaSystem_001`
- `Source2Main` dedicated server startup
- world lifecycle on `dl_midtown`
- real server `GameFrame` callbacks
- client lifecycle: enter, fully connected and disconnect

## Current milestone: pawn movement probe

After a human reaches `ClientFullyConnect`, SPLIT now attaches a minimal pawn probe to that authoritative slot.

The probe resolves its field offsets from the live server schema:

- `CBasePlayerController::m_hPawn`
- `CBaseEntity::m_CBodyComponent`
- `CBodyComponent::m_pSceneNode`
- `CGameSceneNode::m_vecAbsOrigin`
- `CBaseEntity::m_vecAbsVelocity`
- `CBaseEntity::m_hGroundEntity`

It then walks:

```text
slot + 1
→ player controller
→ pawn handle
→ serial-validated pawn entity
→ scene node
→ position / velocity / grounded
```

No pawn field offset is hardcoded. The prototype still pins a few stable Source 2 ABI/vtable facts from the vendored 2026-10-28 upstream snapshot; production will verify these from SPLIT-owned game data.

Expected live output after joining and moving:

```text
[SPLIT][Movement] pawn attached slot=0 handle=0x...
[SPLIT][Movement] pose pos=(..., ..., ...) vel=(..., ..., ...) grounded=yes
```

## Next

Once this live pawn probe is validated, replace periodic logging with a per-tick movement recorder and add command/button capture for route recording.
