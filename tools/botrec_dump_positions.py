from pathlib import Path
import csv
import struct

BOTREC_PATH = Path("botrec_debug/decoded.bin")
OUTPUT_PATH = Path("botrec_debug/native_positions.csv")

HEADER_SIZE = 32
HEADER_START_X = 0x04
HEADER_START_Y = 0x08
HEADER_START_Z = 0x0C
BOTREC_HZ = 64.0


def read_varint(data, offset):
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


def parse_fields(data):
    fields = []
    offset = 0
    while offset < len(data):
        key, offset = read_varint(data, offset)
        field = key >> 3
        wire = key & 7

        if wire == 0:
            start = offset
            _, offset = read_varint(data, offset)
        elif wire == 1:
            start = offset
            offset += 8
        elif wire == 2:
            size, payload_start = read_varint(data, offset)
            start = payload_start
            offset = payload_start + size
        elif wire == 5:
            start = offset
            offset += 4
        elif wire == 7:
            start = offset
        else:
            raise RuntimeError(f"wire {wire} non supporté")

        fields.append((field, wire, start, offset))

    return fields


def load_frames(data):
    header = data[:HEADER_SIZE]
    frames = []
    offset = HEADER_SIZE

    while offset < len(data):
        size, payload_start = read_varint(data, offset)
        payload_end = payload_start + size
        if payload_end > len(data):
            raise RuntimeError("frame tronquée")
        frames.append(bytes(data[payload_start:payload_end]))
        offset = payload_end

    return header, frames


def extract_command_tick(frame):
    for field, wire, start, end in parse_fields(frame):
        if field != 1 or wire != 2:
            continue
        command = frame[start:end]
        for cf, cw, cs, ce in parse_fields(command):
            if cf == 2 and cw == 0:
                value, _ = read_varint(command, cs)
                return value
    return None


def extract_position_state(frame):
    """
    outer.field2 = état de position natif.
    Sous-champs float32:
      field1 = X absolu
      field2 = Y absolu
      field3 = Z absolu

    Un axe absent conserve sa dernière valeur.
    """
    for field, wire, start, end in parse_fields(frame):
        if field != 2 or wire != 2:
            continue

        payload = frame[start:end]
        values = [0.0, 0.0, 0.0]
        present = [False, False, False]

        for pf, pw, ps, pe in parse_fields(payload):
            if pw == 5 and pf in (1, 2, 3):
                values[pf - 1] = struct.unpack_from("<f", payload, ps)[0]
                present[pf - 1] = True

        return tuple(values), tuple(present), payload.hex(" ")

    return None, (False, False, False), ""


def main():
    if not BOTREC_PATH.exists():
        raise RuntimeError(
            f"BOTREC décodé introuvable : {BOTREC_PATH}\n"
            "Lance d'abord : "
            "py tools\\botrec_decode.py BOTREC_FORWARD_JUMP_BACK_JUMP.txt"
        )

    data = BOTREC_PATH.read_bytes()
    header, frames = load_frames(data)

    if len(header) < HEADER_SIZE:
        raise RuntimeError("header BOTREC trop court")

    start_position = [
        struct.unpack_from("<f", header, HEADER_START_X)[0],
        struct.unpack_from("<f", header, HEADER_START_Y)[0],
        struct.unpack_from("<f", header, HEADER_START_Z)[0],
    ]

    current = list(start_position)
    rows = []
    correction_count = 0
    nonzero_count = 0

    for frame_index, frame in enumerate(frames):
        tick = extract_command_tick(frame)
        values, present, raw_hex = extract_position_state(frame)
        has_position = values is not None

        if has_position:
            correction_count += 1
            if any(present):
                nonzero_count += 1

            for axis in range(3):
                if present[axis]:
                    current[axis] = values[axis]

        rows.append({
            "frame": frame_index,
            "t_ms": f"{frame_index * (1000.0 / BOTREC_HZ):.3f}",
            "tick": "" if tick is None else tick,
            "has_position": int(has_position),
            "has_x": int(present[0]),
            "has_y": int(present[1]),
            "has_z": int(present[2]),
            "value_x": "" if not has_position else f"{values[0]:.6f}",
            "value_y": "" if not has_position else f"{values[1]:.6f}",
            "value_z": "" if not has_position else f"{values[2]:.6f}",
            "reconstructed_x": f"{current[0]:.6f}",
            "reconstructed_y": f"{current[1]:.6f}",
            "reconstructed_z": f"{current[2]:.6f}",
            "raw_position_payload_hex": raw_hex,
        })

    OUTPUT_PATH.parent.mkdir(exist_ok=True)

    fieldnames = [
        "frame", "t_ms", "tick",
        "has_position", "has_x", "has_y", "has_z",
        "value_x", "value_y", "value_z",
        "reconstructed_x", "reconstructed_y", "reconstructed_z",
        "raw_position_payload_hex",
    ]

    with OUTPUT_PATH.open("w", newline="", encoding="utf-8") as handle:
        writer = csv.DictWriter(handle, fieldnames=fieldnames)
        writer.writeheader()
        writer.writerows(rows)

    print()
    print("=== BOTREC NATIVE POSITION DUMP ===")
    print()
    print(
        "Spawn header : "
        f"({start_position[0]:.3f}, "
        f"{start_position[1]:.3f}, "
        f"{start_position[2]:.3f})"
    )
    print(f"Frames       : {len(frames)}")
    print(f"Frames pos   : {correction_count}")
    print(f"Pos non-zero : {nonzero_count}")
    print(f"CSV          : {OUTPUT_PATH}")
    print()
    print(
        "Le CSV traite outer.field2 comme un état ABSOLU par axe. "
        "Un axe absent conserve sa dernière valeur."
    )


if __name__ == "__main__":
    main()
