#!/usr/bin/env python3
"""Print native versus SPLIT BOTREC command state around first crouch.
Read-only. Usage:
py tools/botrec_compare_native_crouch.py botrec_donor/decoded.bin botrec_debug/decoded_modified.bin
"""
import sys
import struct
from pathlib import Path
from botrec_inspect import parse_frames, parse_proto

def decode(data):
    rows = []
    held = 0
    fields = {}
    for index, frame in enumerate(parse_frames(data)):
        outer = parse_proto(frame["payload"])
        cmd = next((v for n, w, v in outer if n == 1 and w == 2), None)
        if cmd is None:
            continue
        updated = {}
        for n, w, v in parse_proto(cmd):
            updated.setdefault(n, []).append((w, v))
            if n == 3 and w == 7:
                held = 0
            elif n == 3 and w == 2:
                for bn, bw, bv in parse_proto(v):
                    if bn == 1 and bw == 0:
                        held = bv
                    if bn == 1 and bw == 7:
                        held = 0
            if n in (5, 6, 7):
                if w == 5:
                    fields[n] = struct.unpack("<f", v)[0]
                elif w == 7:
                    fields[n] = 0.0
        rows.append((index, held, fields.get(5, 0.0), fields.get(6, 0.0), fields.get(7, 0.0),
                     tuple(sorted((n, tuple(w for w, _ in values)) for n, values in updated.items())),
                     tuple(sorted((n, w) for n, w, _ in outer))))
    return rows

def show(title, path):
    rows = decode(path.read_bytes())
    presses = [i for i in range(1, len(rows))
               if rows[i][1] & 4 and not (rows[i-1][1] & 4)]
    print(f"\n=== {title} === frames={len(rows)} crouch_presses={len(presses)}")
    if not presses:
        return
    for hit in presses[:5]:
        print(f"First crouch at frame {rows[hit][0]}, t~{rows[hit][0]*1000/64:.1f} ms")
        print("frame held forward left up command_fields(field:wire_types) outer_fields")
        for row in rows[max(0, hit-8):min(len(rows), hit+9)]:
            idx, held, fwd, left, up, cmd_fields, outer_fields = row
            print(f"{idx:4d} 0x{held:016X} {fwd:+.3f} {left:+.3f} {up:+.3f} {cmd_fields} {outer_fields}")
        print("---")

if __name__ == "__main__":
    if len(sys.argv) != 3:
        raise SystemExit("Usage: py tools/botrec_compare_native_crouch.py donor_decoded.bin generated_decoded.bin")
    show("NATIVE", Path(sys.argv[1]))
    show("SPLIT GENERATED", Path(sys.argv[2]))
