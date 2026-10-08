from pathlib import Path
import struct


SRC = Path("botrec_debug/decoded.bin")
DST = Path("botrec_debug/decoded_modified.bin")

HEADER_SIZE = 32

# On démarre au début du premier mouvement de ton recording.
START_FRAME = 107

# ~1 seconde par direction à 60 Hz.
SEGMENT = 60


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


def encode_varint(value):
    out = bytearray()

    while True:
        b = value & 0x7F
        value >>= 7

        if value:
            out.append(b | 0x80)
        else:
            out.append(b)
            break

    return bytes(out)


def float_field(field_number, value):
    key = (field_number << 3) | 5
    return bytes([key]) + struct.pack("<f", value)


def clear_field(field_number):
    # wire 7 = reset/default dans ce format
    key = (field_number << 3) | 7
    return bytes([key])


def patch_command(command, extra):
    return command + extra


def patch_frame(frame_payload, extra_command_bytes):
    out = bytearray()
    offset = 0
    patched = False

    while offset < len(frame_payload):
        key_start = offset
        key, offset = read_varint(frame_payload, offset)

        field = key >> 3
        wire = key & 7

        if wire == 0:
            value_start = offset
            _, offset = read_varint(frame_payload, offset)
            out += frame_payload[key_start:offset]

        elif wire == 1:
            offset += 8
            out += frame_payload[key_start:offset]

        elif wire == 5:
            offset += 4
            out += frame_payload[key_start:offset]

        elif wire == 7:
            out += frame_payload[key_start:offset]

        elif wire == 2:
            size, payload_start = read_varint(frame_payload, offset)
            payload_end = payload_start + size

            payload = frame_payload[payload_start:payload_end]

            if field == 1 and not patched:
                payload = patch_command(
                    payload,
                    extra_command_bytes,
                )

                out += encode_varint(key)
                out += encode_varint(len(payload))
                out += payload

                patched = True
            else:
                out += frame_payload[key_start:payload_end]

            offset = payload_end

        else:
            raise RuntimeError(
                f"wire {wire} non supporté"
            )

    if not patched:
        raise RuntimeError("command field introuvable")

    return bytes(out)


def main():
    data = SRC.read_bytes()

    header = data[:HEADER_SIZE]

    frames = []
    offset = HEADER_SIZE

    while offset < len(data):
        size, payload_start = read_varint(data, offset)
        payload_end = payload_start + size

        frames.append(
            bytes(data[payload_start:payload_end])
        )

        offset = payload_end

    print(f"Frames trouvées : {len(frames)}")

    f0 = START_FRAME
    f1 = f0 + SEGMENT
    f2 = f1 + SEGMENT
    f3 = f2 + SEGMENT
    f4 = f3 + SEGMENT

    if f4 >= len(frames):
        raise RuntimeError("Recording trop court")

    # field5 = forwardmove
    # field6 = leftmove

    # 1) AVANT
    frames[f0] = patch_frame(
        frames[f0],
        float_field(5, +1.0)
        + clear_field(6)
    )

    # 2) GAUCHE
    frames[f1] = patch_frame(
        frames[f1],
        clear_field(5)
        + float_field(6, +1.0)
    )

    # 3) ARRIÈRE
    frames[f2] = patch_frame(
        frames[f2],
        float_field(5, -1.0)
        + clear_field(6)
    )

    # 4) DROITE
    frames[f3] = patch_frame(
        frames[f3],
        clear_field(5)
        + float_field(6, -1.0)
    )

    # STOP
    frames[f4] = patch_frame(
        frames[f4],
        clear_field(5)
        + clear_field(6)
    )

    rebuilt = bytearray(header)

    for frame in frames:
        rebuilt += encode_varint(len(frame))
        rebuilt += frame

    DST.write_bytes(rebuilt)

    print()
    print("[BOTREC MAKE SQUARE] OK")
    print(f"Avant : {len(data)} bytes")
    print(f"Après : {len(rebuilt)} bytes")
    print()
    print(f"{f0}-{f1 - 1} : AVANT")
    print(f"{f1}-{f2 - 1} : GAUCHE")
    print(f"{f2}-{f3 - 1} : ARRIERE")
    print(f"{f3}-{f4 - 1} : DROITE")
    print(f"{f4} : STOP")
    print()
    print(f"Créé : {DST}")


if __name__ == "__main__":
    main()