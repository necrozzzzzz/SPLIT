#!/usr/bin/env python3
"""Diagnostic only: compare wall-clock and recorded local-tick timelines.

The input JSON is never modified. Both variants use the same native template
and the same prefix of recorded inputs/views. RECORDED_TICKS is an experiment,
not a validated replacement for real user-command timing: local ticks can
include client prediction. Do not use this as a general replay fix yet.
"""
import argparse
import base64
import bisect
import copy
import json
import math
import struct
import sys
import tempfile
import zlib
from pathlib import Path

import zstandard as zstd
import botrec_apply_split_route as conv


def clock_mapper(samples):
    times = [float(s["tMs"]) for s in samples]
    ticks = [s["diagnostics"]["simulationTick"] for s in samples]
    if any(not math.isfinite(t) for t in times):
        raise ValueError("Invalid sample timestamp")
    if any(not isinstance(k, int) or not 0 <= k < 0xffffffff for k in ticks):
        raise ValueError("Missing/invalid recorded local tick")
    if any(b <= a for a, b in zip(times, times[1:])):
        raise ValueError("Sample timestamps must increase strictly")
    if any(b < a for a, b in zip(ticks, ticks[1:])):
        raise ValueError("Local tick clock regresses; do not repair it silently")

    def mapped_ms(ms):
        ms = float(ms)
        if not times[0] <= ms <= times[-1]:
            raise ValueError("Event outside sampled clock range")
        i = min(bisect.bisect_right(times, ms) - 1, len(times) - 2)
        fraction = (ms - times[i]) / (times[i + 1] - times[i])
        tick = ticks[i] + fraction * (ticks[i + 1] - ticks[i])
        return (tick - ticks[0]) * 1000.0 / conv.BOTREC_HZ

    return mapped_ms


def first_pair(events):
    forward = next(e["tMs"] for e in events if e["buttons"].get("forward"))
    crouch = next(e["tMs"] for e in events if e["buttons"].get("crouch"))
    return forward, crouch


def make_prefix(route, prefix_ms, transform):
    result = copy.deepcopy(route)
    selected = [s for s in result["samples"] if s["tMs"] <= prefix_ms]
    if len(selected) < 2:
        raise ValueError("Prefix has fewer than two samples")
    end_ms = selected[-1]["tMs"]
    result["samples"] = selected
    for key in ("inputEvents", "cameraEvents"):
        result[key] = [e for e in result.get(key, [])
                       if selected[0]["tMs"] <= e["tMs"] <= end_ms]
        for event in result[key]:
            event["tMs"] = transform(event["tMs"])
    for sample in selected:
        sample["tMs"] = transform(sample["tMs"])
    result["sampleCount"] = len(selected)
    result["inputEventCount"] = len(result["inputEvents"])
    result["cameraEventCount"] = len(result["cameraEvents"])
    result["durationMs"] = selected[-1]["tMs"] - selected[0]["tMs"]
    return result


def encode(decoded):
    packed = b"\x01" + struct.pack("<I", zlib.crc32(decoded) & 0xffffffff)
    packed += zstd.ZstdCompressor(level=3).compress(decoded)
    assert zstd.ZstdDecompressor().decompress(packed[5:]) == decoded
    return "BOTREC." + base64.urlsafe_b64encode(packed).decode().rstrip("=").replace("-", ".")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("route", type=Path)
    parser.add_argument("--template", type=Path, required=True)
    parser.add_argument("--prefix-ms", type=float, default=2000.0)
    parser.add_argument("--out-dir", type=Path, default=Path("botrec_clock_ab"))
    args = parser.parse_args()
    route = json.loads(args.route.read_text(encoding="utf-8"))
    mapping = clock_mapper(route["samples"])
    forward, crouch = first_pair(route["inputEvents"])
    print(f"Original Z -> Ctrl: {crouch - forward:.3f} ms")
    print(f"Local ticks Z -> Ctrl: {mapping(crouch) - mapping(forward):.3f} ms")
    print("DIAGNOSTIC ONLY: local ticks may reflect client prediction.")
    if args.prefix_ms <= crouch:
        raise ValueError("Prefix must include the first crouch")
    original_globals = conv.OUTPUT_PATH, conv.DEBUG_WINDOW_PATH
    original_argv = sys.argv[:]
    try:
        for name, transform in [("WALL_CLOCK", float), ("RECORDED_TICKS", mapping)]:
            target = args.out_dir / name
            target.mkdir(parents=True, exist_ok=True)
            prefix = make_prefix(route, args.prefix_ms, transform)
            with tempfile.TemporaryDirectory(prefix="split_clock_ab_") as temp:
                temporary_route = Path(temp) / "route.json"
                temporary_route.write_text(json.dumps(prefix), encoding="utf-8")
                conv.OUTPUT_PATH = target / "decoded.bin"
                conv.DEBUG_WINDOW_PATH = target / "route_window_debug.csv"
                sys.argv = ["botrec_apply_split_route.py", str(temporary_route),
                            "--template", str(args.template), "--no-position-checkpoints"]
                conv.main()
            decoded = conv.OUTPUT_PATH.read_bytes()
            header, frames = conv.load_frames(decoded)
            assert struct.unpack_from("<I", header, 28)[0] == len(frames)
            (target / "BOTREC.txt").write_text(encode(decoded), encoding="utf-8")
            print(f"{name}: {(target / 'BOTREC.txt').resolve()}")
    finally:
        conv.OUTPUT_PATH, conv.DEBUG_WINDOW_PATH = original_globals
        sys.argv = original_argv


if __name__ == "__main__":
    main()
