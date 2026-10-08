from pathlib import Path
import struct


SRC = Path("botrec_debug/decoded.bin")
DST = Path("botrec_debug/decoded_modified.bin")

HEADER_SIZE = 32
TARGET_FRAME = 39
NEW_X = -99.9375


def read_varint(data, offset):
    value = 0
    shift = 0

    while True:
        b = data[offset]
        offset += 1

        value |= (b & 0x7F) << shift

        if not (b & 0x80):
            return value, offset

        shift += 7


def parse_proto_with_offsets(data):
    out = []
    offset = 0

    while offset < len(data):
        key_offset = offset
        key, offset = read_varint(data, offset)

        field = key >> 3
        wire = key & 7

        if wire == 0:
            value_offset = offset
            _, offset = read_varint(data, offset)
            out.append((field, wire, value_offset, offset))

        elif wire == 1:
            value_offset = offset
            offset += 8
            out.append((field, wire, value_offset, offset))

        elif wire == 2:
            size, offset = read_varint(data, offset)
            value_offset = offset
            offset += size
            out.append((field, wire, value_offset, offset))

        elif wire == 5:
            value_offset = offset
            offset += 4
            out.append((field, wire, value_offset, offset))

        elif wire == 7:
            out.append((field, wire, None, offset))

        else:
            raise RuntimeError(
                f"wire non supporté: {wire} à 0x{key_offset:X}"
            )

    return out


def main():
    data = bytearray(SRC.read_bytes())

    offset = HEADER_SIZE
    frame_index = 0

    while offset < len(data):
        frame_size, payload_offset = read_varint(data, offset)

        frame_start = payload_offset
        frame_end = frame_start + frame_size

        if frame_index == TARGET_FRAME:
            frame = bytes(data[frame_start:frame_end])

            outer = parse_proto_with_offsets(frame)

            position_start = None
            position_end = None

            for field, wire, value_start, value_end in outer:
                if field == 2 and wire == 2:
                    position_start = value_start
                    position_end = value_end
                    break

            if position_start is None:
                raise RuntimeError(
                    f"Frame {TARGET_FRAME}: pas de position"
                )

            position = frame[position_start:position_end]
            fields = parse_proto_with_offsets(position)

            x_offset_inside_position = None

            for field, wire, value_start, value_end in fields:
                if field == 1 and wire == 5:
                    x_offset_inside_position = value_start
                    break

            if x_offset_inside_position is None:
                raise RuntimeError(
                    f"Frame {TARGET_FRAME}: pas de X dans le delta position"
                )

            absolute_x_offset = (
                frame_start
                + position_start
                + x_offset_inside_position
            )

            old_x = struct.unpack_from(
                "<f",
                data,
                absolute_x_offset,
            )[0]

            struct.pack_into(
                "<f",
                data,
                absolute_x_offset,
                NEW_X,
            )

            print(
                f"Frame {TARGET_FRAME}: X "
                f"{old_x} -> {NEW_X}"
            )

            DST.write_bytes(data)

            print()
            print("[BOTREC PATCH POSITION] OK")
            print(f"size = {len(data)} bytes")
            print(f"Créé : {DST}")
            return

        offset = frame_end
        frame_index += 1

    raise RuntimeError(
        f"Frame {TARGET_FRAME} introuvable"
    )


if __name__ == "__main__":
    main()