#!/usr/bin/env python3
"""Compare a SPLIT route JSON against a read-only bot motion CSV.

Usage: python tools/botrec_compare_motion.py route.json bot.csv
No BOTREC commands, timings or positions are changed.
"""
import csv
import json
import math
import sys
from pathlib import Path

def interp(points, t):
    if t < points[0][0] or t > points[-1][0]:
        return None
    lo, hi = 0, len(points) - 1
    while hi - lo > 1:
        mid = (lo + hi) // 2
        if points[mid][0] <= t:
            lo = mid
        else:
            hi = mid
    a, b = points[lo], points[hi]
    f = (t - a[0]) / (b[0] - a[0]) if b[0] != a[0] else 0
    return tuple(a[i] + f * (b[i] - a[i]) for i in (1, 2))

def speed(points, t):
    a, b = interp(points, t - 50), interp(points, t + 50)
    return (math.dist(a, b) * 10) if a and b else None

def compare(route_path, trace_path):
    route = json.loads(Path(route_path).read_text(encoding="utf-8"))
    player = sorted((float(s["tMs"]), *map(float, s["position"][:2])) for s in route["samples"])
    with open(trace_path, newline="", encoding="utf-8-sig") as f:
        records = list(csv.DictReader(f))
    bot = sorted((float(r["elapsed_ms"]), float(r["pos_x"]), float(r["pos_y"]))
                 for r in records if not r.get("read_error") and r.get("pos_x") and r.get("pos_y"))
    if len(bot) < 20:
        raise ValueError("Not enough usable bot positions")
    # Find first long teleport back to the player's spawn position.
    start = player[0][1:]
    teleports = [t for t, x, y in bot
                 if math.dist((x, y), start) < 2]
    if not teleports:
        raise ValueError("BOTREC spawn teleport not found in trace")
    first_teleport = teleports[0]
    # Fit the clock offset only where bot and player are walking normally,
    # before the first Ctrl. This avoids aligning on the faulty slide itself.
    crouch_times = [float(e["tMs"]) for e in route.get("inputEvents", [])
                    if e.get("buttons", {}).get("crouch")]
    first_crouch = min(crouch_times) if crouch_times else 863.0
    early = [t for t in range(500, int(first_crouch) - 30, 20)]
    scored = []
    for offset in range(int(first_teleport) - 100, int(first_teleport) + 300):
        errors = [math.dist(interp(player, t), interp(bot, t + offset))
                  for t in early if interp(player, t) and interp(bot, t + offset)]
        if len(errors) >= 5:
            errors.sort()
            scored.append((errors[len(errors) // 2], offset))
    if not scored:
        raise ValueError("Cannot align early trajectories")
    residual, offset = min(scored)
    print(f"Early-walk aligned clock offset: +{offset} ms (median position error {residual:.1f} units)")
    print(f"First recorded Ctrl: {first_crouch:.1f} ms")
    print("t_ms,player_speed_xy,bot_speed_xy,position_error_xy")
    for t in (600, 750, 800, 850, round(first_crouch), 900, 950, 1000, 1100, 1200):
        p, b = interp(player, t), interp(bot, t + offset)
        if p and b:
            ps, bs = speed(player, t), speed(bot, t + offset)
            print(f"{t},{ps:.1f},{bs:.1f},{math.dist(p,b):.1f}")
    print("Note: velocities use 100-ms position differences; source telemetry may contain tearing.")
    print("Clock alignment is approximate, not a BOTREC tick synchronization.")

if __name__ == "__main__":
    if len(sys.argv) != 3:
        raise SystemExit("Usage: python tools/botrec_compare_motion.py route.json bot.csv")
    compare(sys.argv[1], sys.argv[2])
