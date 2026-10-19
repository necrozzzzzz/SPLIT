# Compare two captures of the same controlled bot passage

This diagnostic records a manually controlled native-recorder bot through
SPLIT's existing route recorder. It does not fix mantle playback or establish
that a converted BOTREC is equivalent to native playback.

## Capture once

1. In Practice, start Deadlock's native bot recording and keep its bot idle.
   Start native recording first: if it spawns/replaces a bot, SPLIT must select
   the resulting entity, not an earlier bot.
2. In SPLIT, open Settings → About → Route Recorder. Select **Controlled bot
   (native recording)**. Enter that bot's pawn entity index from
   `cl_ent_find haze`, or use **Find bots (optional)** and choose the right
   entity. A detected entity is a non-local Citadel pawn, not a verified hero
   name. The local player is rejected.
3. Click **Start Recording**. Confirm **Recording bot #…**, return to Deadlock,
   leave a short idle interval, and perform one mantle passage while controlling
   the native recorder bot. Do not spawn another bot or switch the target.
4. Stop native recording, then stop SPLIT and use **Save JSON**. Export the
   native recording through **Share Recording**. Keep the JSON and native code
   together; both must come from this one execution.
5. Validate the original native playback with `citadel_hero_testing_enabled 0`
   and `citadel_mantle_max_height 134`, then `citadel_bot_pasterecording` and
   `citadel_bot_playrecording`. Retain whether the original succeeds.

Do not reconvert immediately: native recording and SPLIT start at different
times. The idle prefixes and shared movement/button transitions must first be
aligned; otherwise that timing difference becomes another uncontrolled input.
The native spawn header can also precede the SPLIT capture's first position.

## What the JSON contains

- `captureTarget`: `controlledBot`, pawn entity index and full serial-bearing
  handle, so the capture target is identifiable.
- Position, velocity and simulation clock of that selected pawn.
- `diagnostics.viewAngles`, top-level pitch/yaw and high-rate angle events from
  the bot's reflected `C_CitadelPlayerPawn::m_angEyeAngles`. These are replicated
  observations, potentially delayed/quantized, not raw user-command angles.
  `m_angClientCamera` remains separately in diagnostics.
- Physical keyboard transitions from the operator. These are useful only while
  manually controlling the native recorder bot. They are not bot-executed input
  evidence, and are not suitable for reconstructing autonomous playback.
- Separate optional movement-service observations and explicit schema/read
  errors. Missing or undecoded button fields are not silently replaced by
  guessed masks. Existing default AZERTY/QWERTY physical bindings still apply;
  custom game binds are not synchronized.

All route readers retain the selected controller and full pawn handle. If that
handle changes, recording stops instead of switching to another pawn. This is
read-only client telemetry; it neither injects commands nor changes game memory.
If reflected bot eye angles or a valid owning controller cannot be resolved,
starting fails explicitly. The ordinary **Local player** mode retains its
existing sources.
