from __future__ import annotations

import argparse
import base64
import csv
import json
import struct
from pathlib import Path

import zstandard as zstd


HEADER_SIZE = 32
BOTREC_HZ = 64.0

DONOR_JUMP_MS = 2406.0
DONOR_CROUCH_MS = 3266.0
DONOR_END_MS = 4828.0


def read_varint(data: bytes, offset: int):
    value = 0
    shift = 0

    while True:
        if offset >= len(data):
            raise RuntimeError("varint hors limites")

        b = data[offset]
        offset += 1
        value |= (b & 0x7F) << shift

        if not (b & 0x80):
            return value, offset

        shift += 7


def parse_fields(data: bytes):
    fields = []
    offset = 0

    while offset < len(data):
        key_start = offset
        key, after_key = read_varint(data, offset)
        field = key >> 3
        wire = key & 7
        offset = after_key

        if wire == 0:
            value_start = offset
            _, end = read_varint(data, offset)
        elif wire == 1:
            value_start = offset
            end = offset + 8
        elif wire == 2:
            size, payload_start = read_varint(data, offset)
            value_start = payload_start
            end = payload_start + size
        elif wire == 5:
            value_start = offset
            end = offset + 4
        elif wire == 7:
            value_start = offset
            end = offset
        else:
            raise RuntimeError(
                f"wire {wire} non supporté à 0x{key_start:X}"
            )

        fields.append(
            {
                "field": field,
                "wire": wire,
                "key_start": key_start,
                "value_start": value_start,
                "end": end,
            }
        )
        offset = end

    return fields


def load_frames(decoded: bytes):
    if len(decoded) < HEADER_SIZE:
        raise RuntimeError("BOTREC décodé trop court")

    header = decoded[:HEADER_SIZE]
    frames = []
    offset = HEADER_SIZE

    while offset < len(decoded):
        size, payload_start = read_varint(decoded, offset)
        payload_end = payload_start + size

        if payload_end > len(decoded):
            raise RuntimeError("frame tronquée")

        frames.append(decoded[payload_start:payload_end])
        offset = payload_end

    return header, frames


def decode_share_code(text: str):
    text = text.strip()

    if not text.startswith("BOTREC."):
        raise RuntimeError("Le fichier ne commence pas par BOTREC.")

    encoded = text[len("BOTREC."):].replace(".", "-")
    encoded += "=" * ((4 - len(encoded) % 4) % 4)
    packed = base64.urlsafe_b64decode(encoded)

    if len(packed) < 9:
        raise RuntimeError("BOTREC trop court")

    compressed = packed[5:]

    if compressed[:4] != bytes.fromhex("28 B5 2F FD"):
        raise RuntimeError("Flux Zstd BOTREC introuvable")

    return zstd.ZstdDecompressor().decompress(compressed)


def load_decoded(path: Path):
    if path.suffix.lower() == ".bin":
        return path.read_bytes()

    text = path.read_text(encoding="utf-8").strip()

    if text.startswith("BOTREC."):
        return decode_share_code(text)

    raise RuntimeError(
        f"{path} : attendu decoded.bin ou fichier texte BOTREC."
    )


def field_value_bytes(data: bytes, item):
    return data[item["key_start"]:item["end"]]


def get_command(frame: bytes):
    for item in parse_fields(frame):
        if item["field"] == 1 and item["wire"] == 2:
            return frame[item["value_start"]:item["end"]]
    return None


def decode_wire_value(data: bytes, item):
    wire = item["wire"]
    start = item["value_start"]
    end = item["end"]

    if wire == 0:
        value, _ = read_varint(data, start)
        return value
    if wire == 1:
        return struct.unpack_from("<Q", data, start)[0]
    if wire == 2:
        return data[start:end]
    if wire == 5:
        return struct.unpack_from("<f", data, start)[0]
    if wire == 7:
        return "<clear>"

    return None


def command_field_map(command: bytes):
    out = {}

    if command is None:
        return out

    for item in parse_fields(command):
        key = (item["field"], item["wire"])
        out.setdefault(key, []).append(
            {
                "value": decode_wire_value(command, item),
                "raw": field_value_bytes(command, item).hex(" "),
            }
        )

    return out


def extract_nested_floats(payload: bytes):
    values = {}

    for item in parse_fields(payload):
        if item["wire"] == 5:
            values[item["field"]] = struct.unpack_from(
                "<f",
                payload,
                item["value_start"],
            )[0]
        elif item["wire"] == 0:
            value, _ = read_varint(payload, item["value_start"])
            values[item["field"]] = value
        elif item["wire"] == 7:
            values[item["field"]] = "<clear>"

    return values


def extract_command_summary(frame: bytes):
    command = get_command(frame)
    fmap = command_field_map(command)

    summary = {
        "tick": "",
        "buttons_raw": "",
        "buttons_held": "",
        "buttons_changed": "",
        "buttons_extra": "",
        "pitch": "",
        "yaw": "",
        "forward": "",
        "left": "",
        "up": "",
        "field10": "",
        "field17": "",
        "command_fields": "",
    }

    if command is None:
        return summary

    field_names = []

    for (field, wire), values in sorted(fmap.items()):
        field_names.append(f"{field}/{wire}x{len(values)}")

    summary["command_fields"] = " ".join(field_names)

    tick_entries = fmap.get((2, 0))
    if tick_entries:
        summary["tick"] = tick_entries[-1]["value"]

    buttons_entries = fmap.get((3, 2))
    if buttons_entries:
        payload = buttons_entries[-1]["value"]
        summary["buttons_raw"] = payload.hex(" ")
        nested = extract_nested_floats(payload)
        summary["buttons_held"] = nested.get(1, "")
        summary["buttons_changed"] = nested.get(2, "")
        summary["buttons_extra"] = nested.get(3, "")
    elif fmap.get((3, 7)):
        summary["buttons_raw"] = "<clear>"

    view_entries = fmap.get((4, 2))
    if view_entries:
        nested = extract_nested_floats(view_entries[-1]["value"])
        summary["pitch"] = nested.get(1, "")
        summary["yaw"] = nested.get(2, "")

    for field, name in ((5, "forward"), (6, "left"), (7, "up")):
        entries = fmap.get((field, 5))
        if entries:
            summary[name] = entries[-1]["value"]
        elif fmap.get((field, 7)):
            summary[name] = "<clear>"

    for field, name in ((10, "field10"), (17, "field17")):
        entries = fmap.get((field, 0))
        if entries:
            summary[name] = entries[-1]["value"]
        elif fmap.get((field, 7)):
            summary[name] = "<clear>"

    return summary


def extract_position(frame: bytes):
    for item in parse_fields(frame):
        if item["field"] == 2 and item["wire"] == 2:
            payload = frame[item["value_start"]:item["end"]]
            nested = extract_nested_floats(payload)
            return (
                nested.get(1, ""),
                nested.get(2, ""),
                nested.get(3, ""),
                payload.hex(" "),
            )

    return "", "", "", ""


def find_route_chain(route_path: Path):
    route = json.loads(route_path.read_text(encoding="utf-8"))
    events = route.get("inputEvents")
    samples = route.get("samples")

    if not isinstance(events, list) or not isinstance(samples, list):
        raise RuntimeError("inputEvents/samples manquants dans la route")

    first_sample = samples[0] if samples else {}
    route_zero = float(first_sample.get("tMs", 0.0))

    sample_times = [
        float(sample.get("tMs", 0.0))
        for sample in samples
    ]

    def prior_vertical_speed(t_ms):
        if not sample_times:
            return 0.0

        import bisect
        index = max(
            0,
            bisect.bisect_right(sample_times, t_ms) - 1,
        )
        velocity = samples[index].get(
            "velocity",
            [0.0, 0.0, 0.0],
        )
        return float(velocity[2]) if len(velocity) >= 3 else 0.0

    previous_jump = False
    previous_crouch = False
    jump_ms = None
    crouch_ms = None
    end_ms = None

    for event in events:
        buttons = event.get("buttons") or {}
        raw_t_ms = float(event.get("tMs", 0.0))
        t_ms = raw_t_ms - route_zero

        jump = bool(buttons.get("jump"))
        crouch = bool(buttons.get("crouch"))

        if jump_ms is None and jump and not previous_jump:
            if abs(prior_vertical_speed(raw_t_ms)) >= 50.0:
                jump_ms = t_ms

        if (
            jump_ms is not None
            and crouch_ms is None
            and crouch
            and not previous_crouch
            and t_ms > jump_ms
        ):
            crouch_ms = t_ms

        if (
            crouch_ms is not None
            and end_ms is None
            and previous_crouch
            and not crouch
            and t_ms > crouch_ms
        ):
            end_ms = t_ms
            break

        previous_jump = jump
        previous_crouch = crouch

    if jump_ms is None or crouch_ms is None or end_ms is None:
        raise RuntimeError(
            "Impossible de détecter wall-jump airborne -> crouch -> release"
        )

    return jump_ms, crouch_ms, end_ms


def lerp_time(value, src_a, src_b, dst_a, dst_b):
    if src_b <= src_a:
        return dst_a

    alpha = (value - src_a) / (src_b - src_a)
    return dst_a + alpha * (dst_b - dst_a)


def donor_time_for_generated_ms(generated_ms, route_anchors):
    route_jump, route_crouch, route_end = route_anchors

    if generated_ms <= route_jump:
        return DONOR_JUMP_MS + (generated_ms - route_jump)

    if generated_ms <= route_crouch:
        return lerp_time(
            generated_ms,
            route_jump,
            route_crouch,
            DONOR_JUMP_MS,
            DONOR_CROUCH_MS,
        )

    if generated_ms <= route_end:
        return lerp_time(
            generated_ms,
            route_crouch,
            route_end,
            DONOR_CROUCH_MS,
            DONOR_END_MS,
        )

    return DONOR_END_MS + (generated_ms - route_end)


def nearest_frame(frames, t_ms):
    index = int(round(t_ms * BOTREC_HZ / 1000.0))
    index = max(0, min(len(frames) - 1, index))
    return index, frames[index]


def fmt(value):
    if isinstance(value, float):
        return f"{value:.6f}"
    return str(value)


def comparable_value(summary, key):
    value = summary.get(key, "")
    return fmt(value)


def main():
    parser = argparse.ArgumentParser(
        description=(
            "Compare un BOTREC natif donneur à un BOTREC SPLIT généré, "
            "alignés sur la première chaîne jump -> crouch -> release."
        )
    )
    parser.add_argument("donor")
    parser.add_argument("generated")
    parser.add_argument("route")
    parser.add_argument(
        "--before-ms",
        type=float,
        default=250.0,
        help="marge avant le premier jump (défaut: 250)",
    )
    parser.add_argument(
        "--after-ms",
        type=float,
        default=500.0,
        help="marge après la release crouch (défaut: 500)",
    )
    parser.add_argument(
        "--out",
        default="botrec_debug/native_vs_generated.csv",
    )
    args = parser.parse_args()

    donor_path = Path(args.donor)
    generated_path = Path(args.generated)
    route_path = Path(args.route)
    out_path = Path(args.out)

    donor_decoded = load_decoded(donor_path)
    generated_decoded = load_decoded(generated_path)

    _, donor_frames = load_frames(donor_decoded)
    _, generated_frames = load_frames(generated_decoded)

    route_anchors = find_route_chain(route_path)
    route_jump, route_crouch, route_end = route_anchors

    start_ms = max(0.0, route_jump - args.before_ms)
    end_ms = route_end + args.after_ms

    keys = [
        "buttons_raw",
        "buttons_held",
        "buttons_changed",
        "buttons_extra",
        "pitch",
        "yaw",
        "forward",
        "left",
        "up",
        "field10",
        "field17",
        "command_fields",
    ]

    rows = []
    differing_frames = 0

    start_frame = int(start_ms * BOTREC_HZ / 1000.0)
    end_frame = int(end_ms * BOTREC_HZ / 1000.0) + 1

    for gen_index in range(start_frame, min(end_frame, len(generated_frames))):
        generated_ms = gen_index * (1000.0 / BOTREC_HZ)
        donor_ms = donor_time_for_generated_ms(
            generated_ms,
            route_anchors,
        )
        donor_index, donor_frame = nearest_frame(
            donor_frames,
            donor_ms,
        )
        generated_frame = generated_frames[gen_index]

        native = extract_command_summary(donor_frame)
        generated = extract_command_summary(generated_frame)

        native_pos = extract_position(donor_frame)
        generated_pos = extract_position(generated_frame)

        diff_fields = []

        for key in keys:
            if comparable_value(native, key) != comparable_value(generated, key):
                diff_fields.append(key)

        if native_pos[:3] != generated_pos[:3]:
            diff_fields.append("position")

        if diff_fields:
            differing_frames += 1

        row = {
            "generated_frame": gen_index,
            "generated_ms": f"{generated_ms:.3f}",
            "donor_frame": donor_index,
            "donor_ms": f"{donor_ms:.3f}",
            "phase": (
                "pre_jump"
                if generated_ms < route_jump
                else "jump_to_crouch"
                if generated_ms < route_crouch
                else "crouch_to_release"
                if generated_ms <= route_end
                else "post_release"
            ),
            "diff_fields": "|".join(diff_fields),
        }

        for key in keys:
            row[f"native_{key}"] = comparable_value(native, key)
            row[f"generated_{key}"] = comparable_value(generated, key)

        for axis, native_value, generated_value in zip(
            ("x", "y", "z"),
            native_pos[:3],
            generated_pos[:3],
        ):
            row[f"native_pos_{axis}"] = fmt(native_value)
            row[f"generated_pos_{axis}"] = fmt(generated_value)

        row["native_pos_raw"] = native_pos[3]
        row["generated_pos_raw"] = generated_pos[3]
        rows.append(row)

    out_path.parent.mkdir(parents=True, exist_ok=True)

    fieldnames = list(rows[0].keys()) if rows else []

    with out_path.open("w", newline="", encoding="utf-8") as handle:
        writer = csv.DictWriter(handle, fieldnames=fieldnames)
        writer.writeheader()
        writer.writerows(rows)

    print()
    print("=== BOTREC NATIVE VS GENERATED ===")
    print()
    print(f"Donor       : {donor_path}")
    print(f"Generated   : {generated_path}")
    print(f"Route       : {route_path}")
    print(
        "Route chain : "
        f"jump={route_jump:.3f} ms, "
        f"crouch={route_crouch:.3f} ms, "
        f"release={route_end:.3f} ms"
    )
    print(
        "Donor chain : "
        f"jump={DONOR_JUMP_MS:.1f} ms, "
        f"crouch={DONOR_CROUCH_MS:.1f} ms, "
        f"release={DONOR_END_MS:.1f} ms"
    )
    print(f"Frames CSV  : {len(rows)}")
    print(f"Frames diff : {differing_frames}")
    print(f"CSV         : {out_path}")
    print()
    print(
        "Compare notamment : buttons, forward/left/up, field10, "
        "field17, viewangles et position."
    )


if __name__ == "__main__":
    main()
