from pathlib import Path
import math
import struct


SRC = Path("botrec_debug/decoded.bin")
DST = Path("botrec_debug/decoded_modified.bin")

HEADER_SIZE = 32


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


def write_varint(value):
    out = bytearray()

    while True:
        b = value & 0x7F
        value >>= 7

        if value:
            out.append(b | 0x80)
        else:
            out.append(b)
            return bytes(out)


def proto_key(field, wire):
    return write_varint((field << 3) | wire)


def encode_field(field, wire, value):
    out = bytearray()
    out += proto_key(field, wire)

    if wire == 0:
        out += write_varint(value)

    elif wire == 1:
        out += value

    elif wire == 2:
        out += write_varint(len(value))
        out += value

    elif wire == 5:
        out += value

    elif wire == 7:
        pass

    else:
        raise ValueError(f"wire={wire} non supporté")

    return bytes(out)


def parse_proto_raw(data):
    """
    Retourne :
    (field, wire, value)

    value :
      wire 0 -> int
      wire 1 -> 8 bytes
      wire 2 -> bytes
      wire 5 -> 4 bytes
      wire 7 -> None
    """
    out = []
    offset = 0

    while offset < len(data):
        key, offset = read_varint(data, offset)

        field = key >> 3
        wire = key & 7

        if wire == 0:
            value, offset = read_varint(data, offset)

        elif wire == 1:
            value = data[offset:offset + 8]
            offset += 8

        elif wire == 2:
            size, offset = read_varint(data, offset)
            value = data[offset:offset + size]
            offset += size

        elif wire == 5:
            value = data[offset:offset + 4]
            offset += 4

        elif wire == 7:
            value = None

        else:
            raise ValueError(
                f"wire protobuf non supporté : {wire}"
            )

        out.append((field, wire, value))

    return out


def f32(raw):
    return struct.unpack("<f", raw)[0]


def pack_f32(value):
    return struct.pack("<f", value)


def parse_position(data, state):
    for field, wire, value in parse_proto_raw(data):
        if wire == 5:
            state[field] = f32(value)

        elif wire == 7:
            state[field] = 0.0


def encode_position(x, y, z):
    out = bytearray()

    out += encode_field(
        1,
        5,
        pack_f32(x),
    )

    out += encode_field(
        2,
        5,
        pack_f32(y),
    )

    out += encode_field(
        3,
        5,
        pack_f32(z),
    )

    return bytes(out)


def mirror_xy(x, y, start_x, start_y, yaw_deg):
    """
    Miroir gauche/droite autour de l'axe FORWARD
    défini par le yaw de départ.
    """

    yaw = math.radians(yaw_deg)

    # Axe avant
    fx = math.cos(yaw)
    fy = math.sin(yaw)

    # Axe gauche
    lx = -math.sin(yaw)
    ly = math.cos(yaw)

    dx = x - start_x
    dy = y - start_y

    forward_amount = dx * fx + dy * fy
    left_amount = dx * lx + dy * ly

    # miroir latéral
    left_amount = -left_amount

    new_x = (
        start_x
        + forward_amount * fx
        + left_amount * lx
    )

    new_y = (
        start_y
        + forward_amount * fy
        + left_amount * ly
    )

    return new_x, new_y


def modify_command(command):
    """
    Inverse uniquement leftmove.
    Ne touche PAS aux boutons.
    """

    out = bytearray()

    for field, wire, value in parse_proto_raw(command):

        # field 6 = leftmove
        if field == 6 and wire == 5:
            old = f32(value)
            new = -old

            print(
                f"leftmove {old:+.2f} -> {new:+.2f}"
            )

            value = pack_f32(new)

        out += encode_field(
            field,
            wire,
            value,
        )

    return bytes(out)


def main():
    data = SRC.read_bytes()

    if len(data) < HEADER_SIZE:
        raise RuntimeError("decoded.bin trop petit")

    header = bytearray(data[:HEADER_SIZE])

    # Header BOTREC observé :
    #
    # 0x00 hero id
    # 0x04 start X
    # 0x08 start Y
    # 0x0C start Z
    # 0x10 pitch
    # 0x14 yaw

    start_x = struct.unpack_from(
        "<f",
        header,
        0x04,
    )[0]

    start_y = struct.unpack_from(
        "<f",
        header,
        0x08,
    )[0]

    start_z = struct.unpack_from(
        "<f",
        header,
        0x0C,
    )[0]

    yaw = struct.unpack_from(
        "<f",
        header,
        0x14,
    )[0]

    print()
    print("[BOTREC MIRROR]")
    print(
        f"start = ({start_x}, {start_y}, {start_z})"
    )
    print(f"yaw   = {yaw}")
    print()

    src_offset = HEADER_SIZE
    output = bytearray(header)

    position_state = {
        1: start_x,
        2: start_y,
        3: start_z,
    }

    frame_index = 0
    position_count = 0

    while src_offset < len(data):
        frame_size, src_offset = read_varint(
            data,
            src_offset,
        )

        frame = data[
            src_offset:
            src_offset + frame_size
        ]

        src_offset += frame_size

        rebuilt_frame = bytearray()

        for field, wire, value in parse_proto_raw(frame):

            #
            # Outer field 1 = usercmd
            #
            if field == 1 and wire == 2:
                value = modify_command(value)

            #
            # Outer field 2 = position
            #
            elif field == 2 and wire == 2:
                parse_position(
                    value,
                    position_state,
                )

                x = position_state.get(1)
                y = position_state.get(2)
                z = position_state.get(3)

                if (
                    x is not None
                    and y is not None
                    and z is not None
                ):
                    new_x, new_y = mirror_xy(
                        x,
                        y,
                        start_x,
                        start_y,
                        yaw,
                    )

                    value = encode_position(
                        new_x,
                        new_y,
                        z,
                    )

                    position_count += 1

                    if position_count <= 5:
                        print(
                            f"pos "
                            f"({x:.2f}, {y:.2f})"
                            f" -> "
                            f"({new_x:.2f}, {new_y:.2f})"
                        )

            rebuilt_frame += encode_field(
                field,
                wire,
                value,
            )

        output += write_varint(
            len(rebuilt_frame)
        )

        output += rebuilt_frame

        frame_index += 1

    DST.write_bytes(output)

    print()
    print("[BOTREC MIRROR] OK")
    print(f"frames    = {frame_index}")
    print(f"positions = {position_count}")
    print(f"size old  = {len(data)}")
    print(f"size new  = {len(output)}")
    print()
    print(f"Créé : {DST}")


if __name__ == "__main__":
    main()