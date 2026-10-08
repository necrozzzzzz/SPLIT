from pathlib import Path
import struct


PATH = Path("botrec_debug/decoded.bin")
HEADER_SIZE = 32


def read_varint(data, offset):
    value = 0
    shift = 0

    while True:
        b = data[offset]
        offset += 1

        value |= (b & 0x7F) << shift

        if not b & 0x80:
            return value, offset

        shift += 7


def parse_proto(data):
    out = []
    offset = 0

    while offset < len(data):
        key, offset = read_varint(data, offset)

        field = key >> 3
        wire = key & 7

        if wire == 0:
            value, offset = read_varint(data, offset)
            out.append((field, wire, value))

        elif wire == 1:
            value = data[offset:offset + 8]
            offset += 8
            out.append((field, wire, value))

        elif wire == 2:
            size, offset = read_varint(data, offset)
            value = data[offset:offset + size]
            offset += size
            out.append((field, wire, value))

        elif wire == 5:
            value = data[offset:offset + 4]
            offset += 4
            out.append((field, wire, value))

        elif wire == 7:
            # Delta encoder Valve :
            # clear/reset du champ.
            out.append((field, wire, None))

        else:
            raise ValueError(
                f"Unsupported protobuf wire={wire}"
            )

    return out


def f32(data):
    return struct.unpack("<f", data)[0]


def parse_frames(data):
    offset = HEADER_SIZE
    frames = []

    while offset < len(data):
        frame_offset = offset

        size, offset = read_varint(data, offset)

        payload = data[offset:offset + size]
        offset += size

        frames.append({
            "offset": frame_offset,
            "size": size,
            "payload": payload,
        })

    return frames


def parse_vec3(data):
    values = {}

    for field, wire, value in parse_proto(data):
        if wire == 5:
            values[field] = f32(value)

        elif wire == 7:
            values[field] = 0.0

    return values


def apply_vec3_delta(state, data):
    for field, wire, value in parse_proto(data):
        if wire == 5:
            state[field] = f32(value)

        elif wire == 7:
            state[field] = 0.0


def apply_buttons_delta(state, data):
    for field, wire, value in parse_proto(data):

        if wire == 0:
            state[field] = value

        elif wire == 7:
            state[field] = 0


def describe_buttons(state):
    held = state.get(1, 0)
    changed = state.get(2, 0)
    state3 = state.get(3, 0)

    pressed = state3 | (held & changed)
    released = state3 | ((~held) & changed)

    released &= 0xFFFFFFFFFFFFFFFF

    return held, changed, pressed, released


def names(mask):
    known = [
        (0x1, "ATTACK"),
        (0x2, "JUMP_STD"),
        (0x4, "DUCK"),
        (0x8, "FORWARD"),
        (0x10, "BACK"),
        (0x200, "LEFT"),
        (0x400, "RIGHT"),
    ]

    result = []

    for bit, name in known:
        if mask & bit:
            result.append(name)

    # On garde aussi les bits inconnus.
    known_mask = 0

    for bit, _ in known:
        known_mask |= bit

    unknown = mask & ~known_mask

    if unknown:
        result.append(f"UNKNOWN=0x{unknown:016X}")

    return "+".join(result) if result else "-"


def main():
    data = PATH.read_bytes()
    frames = parse_frames(data)

    print(f"File size : {len(data)}")
    print(f"Header    : {HEADER_SIZE} bytes")
    print(f"Frames    : {len(frames)}")
    print()

    # État persistant de la commande.
    forward = 0.0
    left = 0.0
    up = 0.0

    view_state = {}
    button_state = {}

    last_position = {
        1: None,
        2: None,
        3: None,
    }

    previous_z = None
    previous_held = 0

    for index, frame in enumerate(frames):
        outer = parse_proto(frame["payload"])

        command = None
        position_delta = None

        for field, wire, value in outer:
            if field == 1 and wire == 2:
                command = value

            elif field == 2 and wire == 2:
                position_delta = value

        if command is None:
            continue

        tick = None

        forward_changed = False
        left_changed = False
        up_changed = False
        view_changed = False
        buttons_changed = False

        for field, wire, value in parse_proto(command):

            if field == 2 and wire == 0:
                tick = value

            elif field == 3:
                if wire == 2:
                    apply_buttons_delta(
                        button_state,
                        value,
                    )
                    buttons_changed = True

                elif wire == 7:
                    button_state.clear()
                    buttons_changed = True

            elif field == 4:
                if wire == 2:
                    apply_vec3_delta(
                        view_state,
                        value,
                    )
                    view_changed = True

                elif wire == 7:
                    view_state.clear()
                    view_changed = True

            elif field == 5:
                if wire == 5:
                    forward = f32(value)
                    forward_changed = True

                elif wire == 7:
                    forward = 0.0
                    forward_changed = True

            elif field == 6:
                if wire == 5:
                    left = f32(value)
                    left_changed = True

                elif wire == 7:
                    left = 0.0
                    left_changed = True

            elif field == 7:
                if wire == 5:
                    up = f32(value)
                    up_changed = True

                elif wire == 7:
                    up = 0.0
                    up_changed = True

        position_changed = False

        if position_delta is not None:
            apply_vec3_delta(
                last_position,
                position_delta,
            )
            position_changed = True

        x = last_position.get(1)
        y = last_position.get(2)
        z = last_position.get(3)

        held, changed, pressed, released = describe_buttons(
            button_state
        )

        events = []

        if forward_changed:
            events.append(
                f"forward={forward:+.2f}"
            )

        if left_changed:
            events.append(
                f"left={left:+.2f}"
            )

        if up_changed:
            events.append(
                f"up={up:+.2f}"
            )

        if buttons_changed:
            events.append(
                "buttons "
                f"held=0x{held:016X} "
                f"[{names(held)}] "
                f"changed=0x{changed:016X} "
                f"pressed=0x{pressed:016X} "
                f"released=0x{released:016X}"
            )

        if view_changed:
            pitch = view_state.get(1)
            yaw = view_state.get(2)

            events.append(
                f"view=({pitch}, {yaw})"
            )

        if (
            position_changed
            and z is not None
            and previous_z is not None
        ):
            dz = z - previous_z

            if dz > 1.0:
                events.append(
                    f"Z_UP dz={dz:+.3f}"
                )

            elif dz < -1.0:
                events.append(
                    f"Z_DOWN dz={dz:+.3f}"
                )

        if held != previous_held:
            gained = held & ~previous_held
            lost = previous_held & ~held

            if gained:
                events.append(
                    f"HELD_GAIN 0x{gained:016X}"
                )

            if lost:
                events.append(
                    f"HELD_LOST 0x{lost:016X}"
                )

        if events:
            print(
                f"frame={index:<4} "
                f"tick={tick:<7} "
                f"pos=({x}, {y}, {z}) "
                f":: "
                + " | ".join(events)
            )

        if z is not None:
            previous_z = z

        previous_held = held


if __name__ == "__main__":
    main()