#!/usr/bin/env python3
"""Compare native BOTREC with identical commands serialized densely.

No route conversion, timing changes, or camera-position overrides.
Uses the SPLIT converter's view/movement rewriting helpers and verifies
that all effective view, axis and button states remain identical.
"""
import argparse
import base64
import struct
import zlib
from pathlib import Path

import zstandard as zstd
import botrec_apply_split_route as conv


def replace_fields(data, numbers, replacement=b""):
    result = bytearray()
    cursor = 0
    for number, wire, start, end in conv.parse_fields(data):
        if number not in numbers:
            result += data[cursor:end]
        cursor = end
    return bytes(result) + replacement


def raw_field(data, number):
    result = bytearray()
    cursor = 0
    for field, wire, start, end in conv.parse_fields(data):
        if field == number:
            result += data[cursor:end]
        cursor = end
    return bytes(result)


def replace_command(frame, command):
    return replace_fields(
        frame, {1}, conv.encode_varint(10)
        + conv.encode_varint(len(command)) + command,
    )


def states(frames):
    view = [0.0, 0.0, 0.0]
    axes = [0.0, 0.0, 0.0]
    buttons = [0, 0, 0]
    result = []
    for frame in frames:
        command = conv.get_command(frame)
        if command is None:
            raise ValueError("Frame without command")
        for field, wire, start, end in conv.parse_fields(command):
            if field in (3, 4):
                state = buttons if field == 3 else view
                if wire == 7:
                    state[:] = [0, 0, 0]
                elif wire == 2:
                    nested = command[start:end]
                    for n, w, s, e in conv.parse_fields(nested):
                        if n not in (1, 2, 3):
                            raise ValueError("Unknown nested state field")
                        if w == 7:
                            state[n - 1] = 0
                        elif field == 3 and w == 0:
                            state[n - 1] = conv.read_varint(nested, s)[0]
                        elif field == 4 and w == 5:
                            state[n - 1] = struct.unpack_from("<f", nested, s)[0]
                        else:
                            raise ValueError("Unexpected state wire type")
                else:
                    raise ValueError("Unexpected command state wire type")
            elif field in (5, 6, 7):
                if wire == 7:
                    axes[field - 5] = 0.0
                elif wire == 5:
                    axes[field - 5] = struct.unpack_from("<f", command, start)[0]
                else:
                    raise ValueError("Unexpected movement wire type")
        result.append((tuple(view), tuple(axes), tuple(buttons)))
    return result


def rewrite(frames):
    result = []
    original_states = states(frames)
    for frame, (view, axes, buttons) in zip(frames, original_states):
        original_command = conv.get_command(frame)
        changed = conv.patch_frame_view_and_buttons(frame, view[0], view[1])
        command = conv.get_command(changed)
        # The route helper removes button deltas. Restore the native bytes:
        # this test changes only how views/axes are serialized.
        command = replace_fields(command, {3}, raw_field(original_command, 3))
        # Preserve roll too, even for native recordings with nonzero roll.
        view_payload = conv.vec3_delta_payload(*view)
        command = replace_fields(command, {4}, b"\x22"
                                 + conv.encode_varint(len(view_payload))
                                 + view_payload)
        changed = conv.patch_frame_movement(replace_command(changed, command),
                                            axes[0], axes[1])
        if axes[2] != 0.0:
            command = replace_fields(conv.get_command(changed), {7},
                                     conv.float_field(7, axes[2]))
            changed = replace_command(changed, command)
        # Every other command/outer field must survive byte-for-byte.
        if replace_fields(original_command, {3, 4, 5, 6, 7}) != replace_fields(
                conv.get_command(changed), {3, 4, 5, 6, 7}):
            raise AssertionError("Unrelated command fields changed")
        if raw_field(original_command, 3) != raw_field(conv.get_command(changed), 3):
            raise AssertionError("Native button bytes changed")
        if replace_fields(frame, {1}) != replace_fields(changed, {1}):
            raise AssertionError("Outer camera/ability fields changed")
        result.append(changed)
    if original_states != states(result):
        raise AssertionError("Effective command states changed")
    return result


def build(header, frames):
    return header + b"".join(conv.encode_varint(len(f)) + f for f in frames)


def save(out_dir, name, decoded, version):
    crc = zlib.crc32(decoded) & 0xffffffff
    packed = bytes([version]) + struct.pack("<I", crc)
    packed += zstd.ZstdCompressor(level=3).compress(decoded)
    if zstd.ZstdDecompressor().decompress(packed[5:]) != decoded:
        raise AssertionError("Compression round trip failed")
    code = "BOTREC." + base64.urlsafe_b64encode(packed).decode("ascii").rstrip("=").replace("-", ".")
    (out_dir / (name + ".bin")).write_bytes(decoded)
    target = out_dir / (name + ".txt")
    target.write_text(code, encoding="utf-8")
    print(target.resolve())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("native_decoded", type=Path)
    parser.add_argument("--out-dir", type=Path, default=Path("botrec_native_ab"))
    parser.add_argument("--version", type=int, choices=range(256), default=1)
    args = parser.parse_args()
    decoded = args.native_decoded.read_bytes()
    header, frames = conv.load_frames(decoded)
    if len(header) != 32 or struct.unpack_from("<I", header, 28)[0] != len(frames):
        raise ValueError("Header/frame count mismatch")
    dense = rewrite(frames)
    rebuilt = build(header, frames)
    if rebuilt != decoded:
        raise AssertionError("Native control is not byte-identical")
    args.out_dir.mkdir(parents=True, exist_ok=True)
    save(args.out_dir, "NATIVE_CONTROL", decoded, args.version)
    save(args.out_dir, "NATIVE_DENSE_AXES_VIEW", build(header, dense), args.version)
    print(f"Verified: {len(frames)} frames, identical spawn/ticks/buttons/views/axes")
    print("Native camera position and all other fields preserved.")


if __name__ == "__main__":
    main()
