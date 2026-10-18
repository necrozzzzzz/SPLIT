# BOTREC diagnostics

The existing Bot Replay Diagnostic action now appends 19 optional input columns
(to the 25 motion columns). No new UI action is needed. Update/restart SPLIT,
select the playback bot, and capture one playback using the existing action.

The columns contain separately timestamped **client observations** of movement
services and pawn/camera angles. They are not an authoritative CUserCmd stream.
Nonlocal bot fields may be stale or unavailable. Input errors do not erase
motion readings. `input_buffers_match`, `input_angles_stable` and
`input_sim_time_stable` are separate guards, not proof of atomic publication or
server execution. The input timestamps bound only the input read; `elapsed_ms`
continues to describe the earlier motion read. Extra reads can lower capture Hz;
inspect the actual timestamps and summary rather than assuming 60/64 Hz.

`button_block_hex` preserves the schema-bounded raw embedded member. When the
nested array is not reflected, `button_layout=rawEmbeddedMember` and decoded
button columns remain empty. Do not assume the first word is a button mask or
silently apply a fixed byte offset. `input_last_command` can repeat/regress;
these observations must not be turned into fictitious input transitions.

## Offline SourceTV extraction and comparison

Requires Node 24 and Python 3 (standard library only for comparison).

From this folder:

```powershell
npm ci --ignore-scripts
node extract_sourcetv.mjs "C:\path\split_server_test.dem" "C:\path\source.json"
py compare_trace.py "C:\path\source.json" "C:\path\bot.csv" --out "C:\path\comparison.json"
py -m unittest discover -p "test_*.py"
```

The source extractor pins deadem 4.0.2. It snapshots each command before the
parser's mutable accumulator advances. The parser handler wrapper is version
specific. No source attachment is changed. Failures are errors, not replaced
with invented input states. Some client demos cannot be decoded by this parser
and/or do not carry commands; use a SourceTV server demo.

Alignment estimates a constant offset from the early grounded movement window
(default 0.95–2.65 seconds of the demo). This is specific to the current test's
initial movement pattern. For other routes choose a suitable grounded window
with `--align-start` / `--align-end`. The later failure is not used to fit the
offset. The output contains positional errors and input coverage/quality counts;
it does not declare missing jumps or causal explanations from motion alone.
Old motion-only CSV files remain supported, with zero input observations.

This change adds diagnostics, not a fix to the recorder or BOTREC playback.
