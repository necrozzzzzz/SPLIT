#!/usr/bin/env python3
"""Measure pre-crouch speed without relying on bot m_vecAbsVelocity.

Usage: py tools/botrec_slide_gate_report.py route.json bot_trace.csv
Reads only existing route and trace files; never modifies BOTREC or timings.
"""
import csv
import json
import math
import statistics
import sys
from pathlib import Path

def linear_speed(rows):
    if len(rows) < 3:
        return None
    t0 = rows[0][0]
    times = [(r[0] - t0) / 1000.0 for r in rows]
    mean_t = statistics.mean(times)
    den = sum((t - mean_t) ** 2 for t in times)
    if den <= 0:
        return None
    slopes = []
    for col in (1, 2):
        avg = statistics.mean(r[col] for r in rows)
        slopes.append(sum((t - mean_t) * (r[col] - avg) for t, r in zip(times, rows)) / den)
    return math.hypot(*slopes)

def main(route_path, trace_path):
    route = json.loads(Path(route_path).read_text(encoding="utf-8"))
    events = route.get("inputEvents", [])
    crouch = None
    previously = False
    for event in events:
        current = bool(event.get("buttons", {}).get("crouch"))
        if current and not previously:
            crouch = float(event["tMs"])
            break
        previously = current
    if crouch is None:
        raise ValueError("Route does not contain a crouch press event")
    samples = route["samples"]
    before = [s for s in samples if crouch - 50 <= s["tMs"] < crouch]
    player_speeds = [math.hypot(*s["velocity"][:2]) for s in before]
    if not player_speeds:
        raise ValueError("No player velocity samples before first crouch")
    with open(trace_path, newline="", encoding="utf-8-sig") as f:
        reader = csv.DictReader(f)
        if "entity_flags" not in reader.fieldnames:
            raise ValueError("Trace requires entity_flags column from newer diagnostic")
        rows = list(reader)
    trace = [(float(r["elapsed_ms"]), float(r["pos_x"]), float(r["pos_y"]), int(r["entity_flags"]))
             for r in rows if r.get("entity_flags") and r.get("pos_x") and r.get("pos_y")]
    transitions = [(prev, cur) for prev, cur in zip(trace, trace[1:])
                   if not (prev[3] & 2) and (cur[3] & 2)
                   and prev[3] & 1 and cur[3] & 1]
    if not transitions:
        raise ValueError("No ground-contact crouch flag transition in trace")
    prev, cur = transitions[0]
    print(f"Player first Ctrl: {crouch:.1f} ms; pre-Ctrl mean speed: {statistics.mean(player_speeds):.1f} u/s")
    print(f"Bot crouch transition: ({prev[0]:.1f}, {cur[0]:.1f}] ms; tick unknown")
    print(f"Ground flag before/after: {prev[3]&1} -> {cur[3]&1}; crouch flag: {prev[3]&2} -> {cur[3]&2}")
    print("Bot position-derived speed ending at the LAST pre-crouch sample:")
    for width in (100, 150, 200, 250):
        start = prev[0] - width
        segment = [p for p in trace if start <= p[0] <= prev[0]]
        speed = linear_speed(segment)
        if speed is not None:
            print(f"  {width:3d} ms window: {speed:6.1f} u/s ({len(segment)} samples)")
    print("Note: telemetry samples ~40ms apart can contain tearing; inspect multiple windows.")
    print("These numbers test the speed-entry hypothesis; they do not establish the game's slide threshold.")

if __name__ == "__main__":
    if len(sys.argv) != 3:
        raise SystemExit("Usage: py tools/botrec_slide_gate_report.py route.json bot_trace.csv")
    main(sys.argv[1], sys.argv[2])
