from pathlib import Path
import bisect
import json
import math
import struct
import sys


BOTREC_PATH = Path("botrec_debug/decoded.bin")
OUTPUT_PATH = Path("botrec_debug/decoded_modified.bin")
DEBUG_WINDOW_PATH = Path("botrec_debug/route_window_debug.csv")

HEADER_SIZE = 32
DEADZONE_SPEED = 10.0
BOTREC_HZ = 64.0
GROUND_POSITION_CHECKPOINT_MS = 250.0
GROUND_POSITION_MAX_ABS_VZ = 8.0

# Native donor supplied by the user (wall-jump -> mantle -> buffered crouch -> slide).
# These anchors were measured from the successful native BOTREC.
DONOR_CHAIN_JUMP_MS = 2406.0
DONOR_CHAIN_CROUCH_MS = 3266.0
DONOR_CHAIN_END_MS = 4828.0

# Button masks validated from native Deadlock BOTREC recordings.
BUTTON_CROUCH = 0x0000000000000004
BUTTON_FORWARD = 0x0000000000000008
BUTTON_BACK = 0x0000000000000010
BUTTON_LEFT = 0x0000000000000200
BUTTON_RIGHT = 0x0000000000000400
BUTTON_DASH = 0x0000100000000000
BUTTON_JUMP = 0x0101000000000000

# Internal BOTREC header layout inferred from native recordings:
# 0x00 u32 hero id
# 0x04 f32 start X
# 0x08 f32 start Y
# 0x0C f32 start Z
# 0x10 f32 start pitch
# 0x14 f32 start yaw
# 0x18 f32 start roll
# 0x1C u32 frame count
HEADER_START_X = 0x04
HEADER_START_Y = 0x08
HEADER_START_Z = 0x0C
HEADER_START_PITCH = 0x10
HEADER_START_YAW = 0x14
HEADER_START_ROLL = 0x18
HEADER_FRAME_COUNT = 0x1C


# ---------------------------------------------------------
# VARINT
# ---------------------------------------------------------

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


# ---------------------------------------------------------
# PROTO HELPERS
# ---------------------------------------------------------

def float_field(field_number, value):
    key = (field_number << 3) | 5
    return encode_varint(key) + struct.pack("<f", value)


def varint_field(field_number, value):
    key = (field_number << 3) | 0
    return encode_varint(key) + encode_varint(value)


def clear_field(field_number):
    return encode_varint((field_number << 3) | 7)


def parse_fields(data):
    """
    Retourne :
    (field, wire, value_start, value_end)
    """

    fields = []
    offset = 0

    while offset < len(data):
        key_start = offset
        key, offset = read_varint(data, offset)

        field = key >> 3
        wire = key & 7

        if wire == 0:
            value_start = offset
            _, offset = read_varint(data, offset)

        elif wire == 1:
            value_start = offset
            offset += 8

        elif wire == 2:
            size, payload_start = read_varint(data, offset)
            value_start = payload_start
            offset = payload_start + size

        elif wire == 5:
            value_start = offset
            offset += 4

        elif wire == 7:
            value_start = offset

        else:
            raise RuntimeError(
                f"wire {wire} non supporté à 0x{key_start:X}"
            )

        fields.append(
            (
                field,
                wire,
                value_start,
                offset,
            )
        )

    return fields


# ---------------------------------------------------------
# BOTREC
# ---------------------------------------------------------

def load_frames(data):
    header = data[:HEADER_SIZE]

    frames = []
    offset = HEADER_SIZE

    while offset < len(data):
        size, payload_start = read_varint(data, offset)
        payload_end = payload_start + size

        if payload_end > len(data):
            raise RuntimeError("frame tronquée")

        frames.append(
            bytes(data[payload_start:payload_end])
        )

        offset = payload_end

    return header, frames



def extract_command_tick(command):
    for field, wire, start, end in parse_fields(command):
        if field == 2 and wire == 0:
            value, _ = read_varint(command, start)
            return value
    return None


def patch_frame_tick(frame, tick):
    """
    Remplace command.field2 (client tick) par une nouvelle valeur.
    Sert uniquement aux frames synthétiques ajoutées au-delà du template.
    """
    out = bytearray()
    offset = 0
    patched = False

    while offset < len(frame):
        key_start = offset
        key, after_key = read_varint(frame, offset)
        field = key >> 3
        wire = key & 7
        offset = after_key

        if wire == 0:
            _, end = read_varint(frame, offset)
            out += frame[key_start:end]
            offset = end

        elif wire == 1:
            end = offset + 8
            out += frame[key_start:end]
            offset = end

        elif wire == 5:
            end = offset + 4
            out += frame[key_start:end]
            offset = end

        elif wire == 7:
            out += frame[key_start:offset]

        elif wire == 2:
            size, payload_start = read_varint(frame, offset)
            payload_end = payload_start + size
            payload = frame[payload_start:payload_end]

            if field == 1 and not patched:
                command_out = bytearray()
                command_offset = 0
                tick_replaced = False

                while command_offset < len(payload):
                    cmd_key_start = command_offset
                    cmd_key, cmd_after_key = read_varint(payload, command_offset)
                    cmd_field = cmd_key >> 3
                    cmd_wire = cmd_key & 7
                    command_offset = cmd_after_key

                    if cmd_wire == 0:
                        _, cmd_end = read_varint(payload, command_offset)

                        if cmd_field == 2 and not tick_replaced:
                            command_out += encode_varint((2 << 3) | 0)
                            command_out += encode_varint(int(tick))
                            tick_replaced = True
                        else:
                            command_out += payload[cmd_key_start:cmd_end]

                        command_offset = cmd_end

                    elif cmd_wire == 1:
                        cmd_end = command_offset + 8
                        command_out += payload[cmd_key_start:cmd_end]
                        command_offset = cmd_end

                    elif cmd_wire == 5:
                        cmd_end = command_offset + 4
                        command_out += payload[cmd_key_start:cmd_end]
                        command_offset = cmd_end

                    elif cmd_wire == 7:
                        command_out += payload[cmd_key_start:command_offset]

                    elif cmd_wire == 2:
                        cmd_size, cmd_payload_start = read_varint(payload, command_offset)
                        cmd_payload_end = cmd_payload_start + cmd_size
                        command_out += payload[cmd_key_start:cmd_payload_end]
                        command_offset = cmd_payload_end

                    else:
                        raise RuntimeError(
                            f"wire command {cmd_wire} non supporté"
                        )

                if not tick_replaced:
                    command_out += encode_varint((2 << 3) | 0)
                    command_out += encode_varint(int(tick))

                out += encode_varint(key)
                out += encode_varint(len(command_out))
                out += command_out
                patched = True

            else:
                out += frame[key_start:payload_end]

            offset = payload_end

        else:
            raise RuntimeError(
                f"wire {wire} non supporté"
            )

    if not patched:
        raise RuntimeError(
            "command message introuvable dans une frame"
        )

    return bytes(out)


def has_outer_position(frame):
    """
    True when the native BOTREC frame contains outer.field2 position state.
    Native recordings use this field sparsely/statefully rather than on
    every single frame.
    """
    for field, wire, start, end in parse_fields(frame):
        if field == 2 and wire == 2:
            return True
    return False


def strip_outer_position(frame):
    """
    Supprime outer.field2 (position optionnelle) d'une frame synthétique.
    Les frames natives existantes restent inchangées.
    """
    out = bytearray()
    offset = 0

    while offset < len(frame):
        key_start = offset
        key, after_key = read_varint(frame, offset)
        field = key >> 3
        wire = key & 7
        offset = after_key

        if wire == 0:
            _, end = read_varint(frame, offset)

        elif wire == 1:
            end = offset + 8

        elif wire == 5:
            end = offset + 4

        elif wire == 7:
            end = offset

        elif wire == 2:
            size, payload_start = read_varint(frame, offset)
            end = payload_start + size

        else:
            raise RuntimeError(
                f"wire {wire} non supporté"
            )

        if field != 2:
            out += frame[key_start:end]

        offset = end

    return bytes(out)


def patch_outer_position_xy(frame, x, y):
    """
    Write only absolute X/Y state and deliberately omit Z.

    This lets BOTREC keep Deadlock's vertical wall-jump/mantle physics while
    constraining the horizontal path to the recorded SPLIT route.
    """
    frame = strip_outer_position(frame)

    payload = (
        float_field(1, x)
        + float_field(2, y)
    )

    out = bytearray(frame)
    out += encode_varint((2 << 3) | 2)
    out += encode_varint(len(payload))
    out += payload
    return bytes(out)


def patch_outer_position(frame, x, y, z):
    """
    Legacy experiment: writes XYZ to outer.field2 CAMERA position.
    This is CCitadelUserCmdPB.vec_camera_position, NOT pawn origin.
    Kept for explicit A/B compatibility; disabled by default.
    """
    frame = strip_outer_position(frame)

    payload = (
        float_field(1, x)
        + float_field(2, y)
        + float_field(3, z)
    )

    out = bytearray(frame)
    out += encode_varint((2 << 3) | 2)
    out += encode_varint(len(payload))
    out += payload
    return bytes(out)



def infer_tick_step(frames):
    ticks = []

    for frame in frames:
        command = get_command(frame)
        if command is None:
            continue

        tick = extract_command_tick(command)
        if tick is not None:
            ticks.append(tick)

    if len(ticks) < 2:
        return 1, ticks[-1] if ticks else 0

    positive_diffs = [
        b - a
        for a, b in zip(ticks, ticks[1:])
        if b > a
    ]

    if not positive_diffs:
        return 1, ticks[-1]

    positive_diffs.sort()
    step = positive_diffs[len(positive_diffs) // 2]

    return max(1, int(step)), ticks[-1]


def get_command(frame):
    for field, wire, start, end in parse_fields(frame):
        if field == 1 and wire == 2:
            return frame[start:end]

    return None


def extract_view_yaw(command, current_yaw):
    """
    command field 4 = viewangles
    viewangles field 2 = yaw float32
    """

    for field, wire, start, end in parse_fields(command):
        if field != 4 or wire != 2:
            continue

        view = command[start:end]

        for vf, vw, vs, ve in parse_fields(view):
            if vf == 2 and vw == 5:
                current_yaw = struct.unpack_from(
                    "<f",
                    view,
                    vs,
                )[0]

    return current_yaw



def vec3_delta_payload(pitch, yaw, roll=0.0):
    return (
        float_field(1, pitch)
        + float_field(2, yaw)
        + float_field(3, roll)
    )


def patch_frame_view_and_buttons(frame, pitch, yaw):
    """
    Nettoie les inputs hérités du template :
    - field 3 (buttons) => reset complet
    - field 4 (viewangles) => yaw/pitch de la route SPLIT

    On garde les autres champs de command intacts.
    """
    out = bytearray()
    offset = 0
    patched = False

    while offset < len(frame):
        key_start = offset
        key, after_key = read_varint(frame, offset)

        field = key >> 3
        wire = key & 7
        offset = after_key

        if wire == 0:
            _, end = read_varint(frame, offset)
            out += frame[key_start:end]
            offset = end

        elif wire == 1:
            end = offset + 8
            out += frame[key_start:end]
            offset = end

        elif wire == 5:
            end = offset + 4
            out += frame[key_start:end]
            offset = end

        elif wire == 7:
            out += frame[key_start:offset]

        elif wire == 2:
            size, payload_start = read_varint(frame, offset)
            payload_end = payload_start + size
            payload = frame[payload_start:payload_end]

            if field == 1 and not patched:
                command_out = bytearray()
                command_offset = 0

                while command_offset < len(payload):
                    cmd_key_start = command_offset
                    cmd_key, cmd_after_key = read_varint(payload, command_offset)
                    cmd_field = cmd_key >> 3
                    cmd_wire = cmd_key & 7
                    command_offset = cmd_after_key

                    if cmd_wire == 0:
                        _, cmd_end = read_varint(payload, command_offset)
                        if cmd_field not in (3, 4):
                            command_out += payload[cmd_key_start:cmd_end]
                        command_offset = cmd_end

                    elif cmd_wire == 1:
                        cmd_end = command_offset + 8
                        if cmd_field not in (3, 4):
                            command_out += payload[cmd_key_start:cmd_end]
                        command_offset = cmd_end

                    elif cmd_wire == 5:
                        cmd_end = command_offset + 4
                        if cmd_field not in (3, 4):
                            command_out += payload[cmd_key_start:cmd_end]
                        command_offset = cmd_end

                    elif cmd_wire == 7:
                        if cmd_field not in (3, 4):
                            command_out += payload[cmd_key_start:command_offset]

                    elif cmd_wire == 2:
                        cmd_size, cmd_payload_start = read_varint(payload, command_offset)
                        cmd_payload_end = cmd_payload_start + cmd_size
                        if cmd_field not in (3, 4):
                            command_out += payload[cmd_key_start:cmd_payload_end]
                        command_offset = cmd_payload_end

                    else:
                        raise RuntimeError(
                            f"wire command {cmd_wire} non supporté"
                        )

                # Do not reset buttons here. patch_frame_buttons() emits
                # sparse button deltas; held state must persist between ticks.
                # A field-3 clear on every frame would cancel that state.

                # Replace viewangles (field 4 / wire 2)
                view_payload = vec3_delta_payload(pitch, yaw, 0.0)
                command_out += encode_varint((4 << 3) | 2)
                command_out += encode_varint(len(view_payload))
                command_out += view_payload

                out += encode_varint(key)
                out += encode_varint(len(command_out))
                out += command_out
                patched = True

            else:
                out += frame[key_start:payload_end]

            offset = payload_end

        else:
            raise RuntimeError(
                f"wire {wire} non supporté"
            )

    if not patched:
        raise RuntimeError(
            "command message introuvable dans une frame"
        )

    return bytes(out)


def patch_frame_buttons(frame, mode, held_mask=0, changed_mask=0):
    """
    Remplace command.field3 par un delta buttons natif.

    Modes:
    - None: aucun delta buttons sur cette frame.
    - "change": field1=held, field2=changed.
    - "initial": explicit held/changed plus a cleared third state.
    - "clear_changed": clear field2 après une transition maintenue.
    - "reset": clear complet après une release vers held=0.
    """
    out = bytearray()
    offset = 0
    patched = False

    while offset < len(frame):
        key_start = offset
        key, after_key = read_varint(frame, offset)
        field = key >> 3
        wire = key & 7
        offset = after_key

        if wire == 0:
            _, end = read_varint(frame, offset)
            out += frame[key_start:end]
            offset = end

        elif wire == 1:
            end = offset + 8
            out += frame[key_start:end]
            offset = end

        elif wire == 5:
            end = offset + 4
            out += frame[key_start:end]
            offset = end

        elif wire == 7:
            out += frame[key_start:offset]

        elif wire == 2:
            size, payload_start = read_varint(frame, offset)
            payload_end = payload_start + size
            payload = frame[payload_start:payload_end]

            if field == 1 and not patched:
                command_out = bytearray()
                command_offset = 0

                while command_offset < len(payload):
                    cmd_key_start = command_offset
                    cmd_key, cmd_after_key = read_varint(payload, command_offset)
                    cmd_field = cmd_key >> 3
                    cmd_wire = cmd_key & 7
                    command_offset = cmd_after_key

                    if cmd_wire == 0:
                        _, cmd_end = read_varint(payload, command_offset)
                        if cmd_field != 3:
                            command_out += payload[cmd_key_start:cmd_end]
                        command_offset = cmd_end

                    elif cmd_wire == 1:
                        cmd_end = command_offset + 8
                        if cmd_field != 3:
                            command_out += payload[cmd_key_start:cmd_end]
                        command_offset = cmd_end

                    elif cmd_wire == 5:
                        cmd_end = command_offset + 4
                        if cmd_field != 3:
                            command_out += payload[cmd_key_start:cmd_end]
                        command_offset = cmd_end

                    elif cmd_wire == 7:
                        if cmd_field != 3:
                            command_out += payload[cmd_key_start:command_offset]

                    elif cmd_wire == 2:
                        cmd_size, cmd_payload_start = read_varint(payload, command_offset)
                        cmd_payload_end = cmd_payload_start + cmd_size
                        if cmd_field != 3:
                            command_out += payload[cmd_key_start:cmd_payload_end]
                        command_offset = cmd_payload_end

                    else:
                        raise RuntimeError(
                            f"wire command {cmd_wire} non supporté"
                        )

                if mode in ("change", "initial"):
                    buttons_payload = bytearray()

                    if held_mask:
                        buttons_payload += varint_field(1, held_mask)
                    else:
                        buttons_payload += clear_field(1)

                    buttons_payload += varint_field(2, changed_mask)
                    if mode == "initial":
                        # Start independently of the preceding playback delta state.
                        buttons_payload += clear_field(3)

                    command_out += encode_varint((3 << 3) | 2)
                    command_out += encode_varint(len(buttons_payload))
                    command_out += buttons_payload

                elif mode == "clear_changed":
                    buttons_payload = clear_field(2)
                    command_out += encode_varint((3 << 3) | 2)
                    command_out += encode_varint(len(buttons_payload))
                    command_out += buttons_payload

                elif mode == "reset":
                    command_out += clear_field(3)

                elif mode is not None:
                    raise RuntimeError(
                        f"mode buttons inconnu : {mode}"
                    )

                out += encode_varint(key)
                out += encode_varint(len(command_out))
                out += command_out
                patched = True

            else:
                out += frame[key_start:payload_end]

            offset = payload_end

        else:
            raise RuntimeError(
                f"wire {wire} non supporté"
            )

    if not patched:
        raise RuntimeError(
            "command message introuvable dans une frame"
        )

    return bytes(out)




def patch_frame_movement(frame, forward, left):
    """
    Remplace complètement les axes de mouvement du template.

    command.field5 = forwardmove
    command.field6 = leftmove
    command.field7 = upmove

    Les anciennes valeurs 5/6/7 du BOTREC natif sont supprimées avant
    d'écrire les axes SPLIT. Ça évite qu'un upmove ou un axe hérité du
    template FORWARD_JUMP_BACK_JUMP pollue la route reconstruite.
    """
    out = bytearray()
    offset = 0
    patched = False

    while offset < len(frame):
        key_start = offset
        key, after_key = read_varint(frame, offset)

        field = key >> 3
        wire = key & 7
        offset = after_key

        if wire == 0:
            _, end = read_varint(frame, offset)
            out += frame[key_start:end]
            offset = end

        elif wire == 1:
            end = offset + 8
            out += frame[key_start:end]
            offset = end

        elif wire == 5:
            end = offset + 4
            out += frame[key_start:end]
            offset = end

        elif wire == 7:
            out += frame[key_start:offset]

        elif wire == 2:
            size, payload_start = read_varint(frame, offset)
            payload_end = payload_start + size
            payload = frame[payload_start:payload_end]

            if field == 1 and not patched:
                command_out = bytearray()
                command_offset = 0

                while command_offset < len(payload):
                    cmd_key_start = command_offset
                    cmd_key, cmd_after_key = read_varint(payload, command_offset)
                    cmd_field = cmd_key >> 3
                    cmd_wire = cmd_key & 7
                    command_offset = cmd_after_key

                    if cmd_wire == 0:
                        _, cmd_end = read_varint(payload, command_offset)
                    elif cmd_wire == 1:
                        cmd_end = command_offset + 8
                    elif cmd_wire == 2:
                        cmd_size, cmd_payload_start = read_varint(payload, command_offset)
                        cmd_end = cmd_payload_start + cmd_size
                    elif cmd_wire == 5:
                        cmd_end = command_offset + 4
                    elif cmd_wire == 7:
                        cmd_end = command_offset
                    else:
                        raise RuntimeError(
                            f"wire command {cmd_wire} non supporté"
                        )

                    if cmd_field not in (5, 6, 7):
                        command_out += payload[cmd_key_start:cmd_end]

                    command_offset = cmd_end

                command_out += float_field(5, forward)
                command_out += float_field(6, left)
                # Explicitly clear upmove so no vertical axis from the
                # template survives.
                command_out += clear_field(7)

                out += encode_varint(key)
                out += encode_varint(len(command_out))
                out += command_out
                patched = True

            else:
                out += frame[key_start:payload_end]

            offset = payload_end

        else:
            raise RuntimeError(
                f"wire {wire} non supporté"
            )

    if not patched:
        raise RuntimeError(
            "command message introuvable dans une frame"
        )

    return bytes(out)


# ---------------------------------------------------------
# SPLIT ROUTE
# ---------------------------------------------------------

def load_route(path):
    obj = json.loads(path.read_text(encoding="utf-8"))

    if "samples" not in obj:
        raise RuntimeError(
            "Le JSON n'a pas de tableau 'samples'"
        )

    samples = obj["samples"]

    if not samples:
        raise RuntimeError(
            "La route ne contient aucun sample"
        )

    return obj, samples


def route_button_mask(sample):
    buttons = sample.get("buttons")

    if not isinstance(buttons, dict):
        return 0

    mask = 0

    if buttons.get("forward"):
        mask |= BUTTON_FORWARD

    if buttons.get("back"):
        mask |= BUTTON_BACK

    if buttons.get("left"):
        mask |= BUTTON_LEFT

    if buttons.get("right"):
        mask |= BUTTON_RIGHT

    if buttons.get("jump"):
        mask |= BUTTON_JUMP

    if buttons.get("crouch"):
        mask |= BUTTON_CROUCH

    if buttons.get("dash"):
        mask |= BUTTON_DASH

    return mask


def route_sample_at_ms(samples, times_ms, target_ms):
    if target_ms <= times_ms[0]:
        return 0, samples[0]

    if target_ms >= times_ms[-1]:
        return len(samples) - 1, samples[-1]

    index = max(
        0,
        bisect.bisect_right(times_ms, target_ms) - 1,
    )
    return index, samples[index]


def prepare_input_events(route_meta, route_time_zero):
    raw_events = route_meta.get("inputEvents")

    if not isinstance(raw_events, list) or not raw_events:
        return None, None

    events = [
        event
        for event in raw_events
        if isinstance(event, dict)
        and isinstance(event.get("buttons"), dict)
        and isinstance(event.get("tMs"), (int, float))
    ]

    if not events:
        return None, None

    events.sort(key=lambda event: float(event["tMs"]))
    event_times_ms = [
        float(event["tMs"]) - route_time_zero
        for event in events
    ]

    return events, event_times_ms


def prepare_camera_events(route_meta, route_time_zero):
    raw_events = route_meta.get("cameraEvents")

    if not isinstance(raw_events, list) or not raw_events:
        return None, None

    events = [
        event
        for event in raw_events
        if isinstance(event, dict)
        and isinstance(event.get("tMs"), (int, float))
        and isinstance(event.get("yaw"), (int, float))
        and isinstance(event.get("pitch"), (int, float))
    ]

    if not events:
        return None, None

    events.sort(key=lambda event: float(event["tMs"]))
    event_times_ms = [
        float(event["tMs"]) - route_time_zero
        for event in events
    ]

    return events, event_times_ms


def camera_at_ms(events, event_times_ms, target_ms):
    if not events:
        return None

    if target_ms <= event_times_ms[0]:
        event = events[0]
        return float(event["yaw"]), float(event["pitch"])

    if target_ms >= event_times_ms[-1]:
        event = events[-1]
        return float(event["yaw"]), float(event["pitch"])

    right = bisect.bisect_right(event_times_ms, target_ms)
    left_index = max(0, right - 1)
    right_index = min(len(events) - 1, right)

    left_t = event_times_ms[left_index]
    right_t = event_times_ms[right_index]

    if right_t <= left_t:
        alpha = 0.0
    else:
        alpha = (target_ms - left_t) / (right_t - left_t)

    left_event = events[left_index]
    right_event = events[right_index]

    yaw = interpolate_angle_degrees(
        float(left_event["yaw"]),
        float(right_event["yaw"]),
        alpha,
    )
    pitch = interpolate_angle_degrees(
        float(left_event["pitch"]),
        float(right_event["pitch"]),
        alpha,
    )

    return yaw, pitch


def command_view_at_ms(samples, times_ms, target_ms):
    """
    Interpolate diagnostics.viewAngles (pawn/user-command view), not
    m_angClientCamera.

    During wall-jump/mantle the client camera can diverge by 10-20+ degrees
    from the command/pawn view. BOTREC field4 drives command view, so using
    camera yaw here can rotate camera-relative movement into the wrong world
    direction even when the recorded keys are correct.
    """
    valid = []

    for index, sample in enumerate(samples):
        diagnostics = sample.get("diagnostics", {})
        view = diagnostics.get("viewAngles")

        if (
            isinstance(view, list)
            and len(view) >= 2
            and isinstance(view[0], (int, float))
            and isinstance(view[1], (int, float))
        ):
            valid.append(
                (
                    times_ms[index],
                    float(view[0]),
                    float(view[1]),
                )
            )

    if not valid:
        return None

    valid_times = [item[0] for item in valid]

    if target_ms <= valid_times[0]:
        _, pitch, yaw = valid[0]
        return yaw, pitch

    if target_ms >= valid_times[-1]:
        _, pitch, yaw = valid[-1]
        return yaw, pitch

    right = bisect.bisect_right(valid_times, target_ms)
    left_index = max(0, right - 1)
    right_index = min(len(valid) - 1, right)

    left_t, left_pitch, left_yaw = valid[left_index]
    right_t, right_pitch, right_yaw = valid[right_index]

    if right_t <= left_t:
        alpha = 0.0
    else:
        alpha = (target_ms - left_t) / (right_t - left_t)

    yaw = interpolate_angle_degrees(
        left_yaw,
        right_yaw,
        alpha,
    )
    pitch = interpolate_angle_degrees(
        left_pitch,
        right_pitch,
        alpha,
    )

    return yaw, pitch


def input_event_at_ms(events, event_times_ms, target_ms):
    if not events:
        return None

    if target_ms <= event_times_ms[0]:
        return events[0]

    index = max(
        0,
        bisect.bisect_right(event_times_ms, target_ms) - 1,
    )
    return events[index]


def input_button_mask_at_ms(
    samples,
    times_ms,
    events,
    event_times_ms,
    target_ms,
):
    if events:
        event = input_event_at_ms(
            events,
            event_times_ms,
            target_ms,
        )
        return route_button_mask(event)

    return route_button_mask_at_ms(
        samples,
        times_ms,
        target_ms,
    )


def input_movement_at_ms(
    samples,
    times_ms,
    events,
    event_times_ms,
    target_ms,
):
    if events:
        event = input_event_at_ms(
            events,
            event_times_ms,
            target_ms,
        )
        buttons = event.get("buttons", {})
        forward = (
            (1.0 if buttons.get("forward") else 0.0)
            - (1.0 if buttons.get("back") else 0.0)
        )
        left = (
            (1.0 if buttons.get("left") else 0.0)
            - (1.0 if buttons.get("right") else 0.0)
        )
        return forward, left

    return route_movement_at_ms(
        samples,
        times_ms,
        target_ms,
    )


def route_button_mask_at_ms(samples, times_ms, target_ms):
    if target_ms <= times_ms[0]:
        return route_button_mask(samples[0])

    if target_ms >= times_ms[-1]:
        return route_button_mask(samples[-1])

    index = max(
        0,
        bisect.bisect_right(times_ms, target_ms) - 1,
    )

    return route_button_mask(samples[index])


def route_movement_at_ms(samples, times_ms, target_ms):
    if target_ms <= times_ms[0]:
        sample = samples[0]
    elif target_ms >= times_ms[-1]:
        sample = samples[-1]
    else:
        index = max(
            0,
            bisect.bisect_right(times_ms, target_ms) - 1,
        )
        sample = samples[index]

    buttons = sample.get("buttons")

    if not isinstance(buttons, dict):
        return None

    movement_keys_present = any(
        key in buttons
        for key in ("forward", "back", "left", "right")
    )

    if not movement_keys_present:
        return None

    forward = (
        (1.0 if buttons.get("forward") else 0.0)
        - (1.0 if buttons.get("back") else 0.0)
    )

    left = (
        (1.0 if buttons.get("left") else 0.0)
        - (1.0 if buttons.get("right") else 0.0)
    )

    return forward, left


def horizontal_speed(sample):
    vel = sample.get("velocity")

    if not vel or len(vel) < 2:
        return 0.0

    return math.hypot(
        float(vel[0]),
        float(vel[1]),
    )


def find_first_moving_sample(samples):
    for i, sample in enumerate(samples):
        if horizontal_speed(sample) > DEADZONE_SPEED:
            return i

    raise RuntimeError(
        "Aucun déplacement détecté dans la route"
    )


def choose_reference_speed(samples):
    """
    90e percentile des vitesses horizontales.

    Ça nous permet de convertir automatiquement :
    vitesse SPLIT -> amplitude joystick 0..1
    """

    speeds = []

    for sample in samples:
        speed = horizontal_speed(sample)

        if speed > DEADZONE_SPEED:
            speeds.append(speed)

    if not speeds:
        return 1.0

    speeds.sort()

    index = int(
        (len(speeds) - 1) * 0.90
    )

    reference = speeds[index]

    return max(reference, 1.0)


def interpolate_angle_degrees(a, b, alpha):
    """
    Interpolation angulaire par le chemin le plus court.
    Évite un grand tour quand le yaw traverse -180/180.
    """
    delta = (b - a + 180.0) % 360.0 - 180.0
    return a + delta * alpha


def should_apply_ground_position_checkpoint(
    samples,
    times_ms,
    target_ms,
    last_checkpoint_ms,
):
    """
    Only correct position when the route is stably grounded.

    A single |vz| ~= 0 sample is NOT enough: wall contacts, mantles and the
    apex of a jump can all briefly report near-zero vertical speed. Those
    false positives were causing mid-air teleports immediately after the
    first wall-jump.

    Require a short window around the target where:
    - vertical speed stays near zero for every sample;
    - Z stays essentially flat.
    """
    if (
        last_checkpoint_ms is not None
        and target_ms - last_checkpoint_ms < GROUND_POSITION_CHECKPOINT_MS
    ):
        return False

    window_ms = 70.0
    z_tolerance = 0.75

    left = bisect.bisect_left(
        times_ms,
        target_ms - window_ms,
    )
    right = bisect.bisect_right(
        times_ms,
        target_ms + window_ms,
    )

    window = samples[left:right]

    # At ~60 Hz we want several consecutive telemetry samples, not a
    # one-frame zero caused by collision/mantle/apex state.
    if len(window) < 5:
        return False

    z_values = []

    for sample in window:
        velocity = sample.get("velocity", [0.0, 0.0, 0.0])
        position = sample.get("position", [0.0, 0.0, 0.0])

        if len(velocity) < 3 or len(position) < 3:
            return False

        vz = float(velocity[2])
        z = float(position[2])

        if abs(vz) > GROUND_POSITION_MAX_ABS_VZ:
            return False

        z_values.append(z)

    if max(z_values) - min(z_values) > z_tolerance:
        return False

    return True

def best_position_anchor_near_ms(
    samples,
    times_ms,
    target_ms,
    window_ms=80.0,
):
    """
    Pick the least suspicious absolute-position sample near target_ms.

    During wall-jump/mantle telemetry, m_vecAbsOrigin can transiently jump
    between prediction/scene-node states even though velocity remains smooth.
    Prefer the sample with the smallest positionVelocityResidual instead of
    blindly interpolating across those discontinuities.
    """
    left = bisect.bisect_left(times_ms, target_ms - window_ms)
    right = bisect.bisect_right(times_ms, target_ms + window_ms)

    best = None
    best_score = None

    for index in range(left, right):
        sample = samples[index]
        position = sample.get("position", [0.0, 0.0, 0.0])
        diagnostics = sample.get("diagnostics", {})
        residual = diagnostics.get("positionVelocityResidual")

        if not isinstance(position, list) or len(position) < 2:
            continue

        if isinstance(residual, list) and len(residual) >= 2:
            score = math.hypot(
                float(residual[0]),
                float(residual[1]),
            )
        else:
            score = abs(times_ms[index] - target_ms) + 1000.0

        # Small timing penalty only breaks ties between equally coherent samples.
        score += abs(times_ms[index] - target_ms) * 0.001

        if best_score is None or score < best_score:
            best_score = score
            best = (
                float(position[0]),
                float(position[1]),
            )

    if best is not None:
        return best

    x, y, _ = resample_position_at_ms(
        samples,
        times_ms,
        target_ms,
    )
    return x, y


def resample_position_at_ms(samples, times_ms, target_ms):
    if target_ms <= times_ms[0]:
        position = samples[0].get("position", [0.0, 0.0, 0.0])
        return tuple(float(v) for v in position[:3])

    if target_ms >= times_ms[-1]:
        position = samples[-1].get("position", [0.0, 0.0, 0.0])
        return tuple(float(v) for v in position[:3])

    right = bisect.bisect_right(times_ms, target_ms)
    left_index = max(0, right - 1)
    right_index = min(len(samples) - 1, right)

    left_t = times_ms[left_index]
    right_t = times_ms[right_index]

    if right_t <= left_t:
        alpha = 0.0
    else:
        alpha = (target_ms - left_t) / (right_t - left_t)

    left_pos = samples[left_index].get("position", [0.0, 0.0, 0.0])
    right_pos = samples[right_index].get("position", left_pos)

    return tuple(
        float(left_pos[axis])
        + (float(right_pos[axis]) - float(left_pos[axis])) * alpha
        for axis in range(3)
    )


def resample_route_at_ms(samples, times_ms, target_ms):
    """
    Échantillonne la route SPLIT au temps demandé.

    Position n'est pas nécessaire ici : pour le BOTREC on convertit
    vitesse monde + yaw caméra en forwardmove/leftmove.
    """
    if target_ms <= times_ms[0]:
        sample = samples[0]
        vel = sample.get("velocity", [0.0, 0.0, 0.0])
        return (
            float(vel[0]),
            float(vel[1]),
            float(sample.get("yaw", 0.0)),
            float(sample.get("pitch", 0.0)),
        )

    if target_ms >= times_ms[-1]:
        return (
            0.0,
            0.0,
            float(samples[-1].get("yaw", 0.0)),
            float(samples[-1].get("pitch", 0.0)),
        )

    right = bisect.bisect_right(times_ms, target_ms)
    left_index = max(0, right - 1)
    right_index = min(len(samples) - 1, right)

    left_sample = samples[left_index]
    right_sample = samples[right_index]

    left_t = times_ms[left_index]
    right_t = times_ms[right_index]

    if right_t <= left_t:
        alpha = 0.0
    else:
        alpha = (target_ms - left_t) / (right_t - left_t)

    left_vel = left_sample.get("velocity", [0.0, 0.0, 0.0])
    right_vel = right_sample.get("velocity", [0.0, 0.0, 0.0])

    vx = float(left_vel[0]) + (float(right_vel[0]) - float(left_vel[0])) * alpha
    vy = float(left_vel[1]) + (float(right_vel[1]) - float(left_vel[1])) * alpha

    left_yaw = float(left_sample.get("yaw", 0.0))
    right_yaw = float(right_sample.get("yaw", left_yaw))
    yaw = interpolate_angle_degrees(left_yaw, right_yaw, alpha)

    left_pitch = float(left_sample.get("pitch", 0.0))
    right_pitch = float(right_sample.get("pitch", left_pitch))
    pitch = interpolate_angle_degrees(left_pitch, right_pitch, alpha)

    return vx, vy, yaw, pitch


# ---------------------------------------------------------
# WORLD VECTOR -> FORWARD / LEFT
# ---------------------------------------------------------

def movement_from_velocity(vx, vy, yaw_degrees, speed_ref):
    speed = math.hypot(vx, vy)

    if speed <= DEADZONE_SPEED:
        return 0.0, 0.0

    #
    # Intensité globale.
    #
    # Une vitesse proche du 90e percentile = input 1.0
    #

    magnitude = min(
        speed / speed_ref,
        1.0,
    )

    nx = vx / speed
    ny = vy / speed

    yaw = math.radians(yaw_degrees)

    #
    # Source / Deadlock :
    #
    # forward world =
    #   (cos(yaw), sin(yaw))
    #
    # left world =
    #   (sin(yaw), -cos(yaw))
    #

    forward_x = math.cos(yaw)
    forward_y = math.sin(yaw)

    # Deadlock's BOTREC leftmove sign is opposite to the
    # world-space left basis used above. The previous conversion produced
    # forward -> right -> back -> left for a SPLIT route recorded as
    # forward -> left -> back -> right, so invert the lateral basis.
    left_x = -math.sin(yaw)
    left_y = math.cos(yaw)

    forward = (
        nx * forward_x
        + ny * forward_y
    ) * magnitude

    left = (
        nx * left_x
        + ny * left_y
    ) * magnitude

    forward = max(-1.0, min(1.0, forward))
    left = max(-1.0, min(1.0, left))

    return forward, left


def patch_header_frame_count(header, frame_count):
    patched = bytearray(header)
    struct.pack_into("<I", patched, HEADER_FRAME_COUNT, int(frame_count))
    return bytes(patched)


def patch_header_spawn(header, sample):
    """
    Remplace le point de départ natif du BOTREC par le premier
    sample de la route SPLIT. On conserve hero id et frame count.
    """
    position = sample.get("position")
    if not position or len(position) < 3:
        raise RuntimeError(
            "Le premier sample SPLIT n'a pas de position XYZ"
        )

    x = float(position[0])
    y = float(position[1])
    z = float(position[2])

    pitch = float(sample.get("pitch", 0.0))
    yaw = float(sample.get("yaw", 0.0))
    roll = 0.0

    patched = bytearray(header)
    struct.pack_into("<f", patched, HEADER_START_X, x)
    struct.pack_into("<f", patched, HEADER_START_Y, y)
    struct.pack_into("<f", patched, HEADER_START_Z, z)
    struct.pack_into("<f", patched, HEADER_START_PITCH, pitch)
    struct.pack_into("<f", patched, HEADER_START_YAW, yaw)
    struct.pack_into("<f", patched, HEADER_START_ROLL, roll)

    print(
        "Spawn BOTREC : "
        f"({x:.2f}, {y:.2f}, {z:.2f}) "
        f"pitch={pitch:.2f} yaw={yaw:.2f}"
    )

    return bytes(patched)


def find_route_first_chain_anchors(route_meta):
    """
    Detect the first AIRBORNE jump (our wall-jump candidate), then the
    first crouch press and crouch release after it.

    Previous code incorrectly anchored the donor chain to the first jump
    of the route, which is usually the take-off jump before the wall jump.
    """
    raw_events = route_meta.get("inputEvents")
    samples = route_meta.get("samples")

    if not isinstance(raw_events, list) or not isinstance(samples, list):
        return None

    sample_times = [
        float(sample.get("tMs", 0.0))
        for sample in samples
        if isinstance(sample, dict)
    ]

    def prior_vertical_speed(t_ms):
        if not sample_times:
            return 0.0

        index = max(
            0,
            bisect.bisect_right(sample_times, t_ms) - 1,
        )
        sample = samples[index]
        velocity = sample.get("velocity", [0.0, 0.0, 0.0])
        if not isinstance(velocity, list) or len(velocity) < 3:
            return 0.0
        return float(velocity[2])

    jump_ms = None
    crouch_ms = None
    end_ms = None
    previous = {
        "jump": False,
        "crouch": False,
    }

    for event in raw_events:
        if not isinstance(event, dict):
            continue

        buttons = event.get("buttons")
        if not isinstance(buttons, dict):
            continue

        t_ms = float(event.get("tMs", 0.0))
        jump = bool(buttons.get("jump"))
        crouch = bool(buttons.get("crouch"))

        if jump and not previous["jump"] and jump_ms is None:
            vz_before = prior_vertical_speed(t_ms)

            # A wall jump is an airborne jump edge. Ignore the initial
            # ground take-off jump that previously confused the donor warp.
            if abs(vz_before) >= 50.0:
                jump_ms = t_ms

        if (
            jump_ms is not None
            and crouch_ms is None
            and crouch
            and not previous["crouch"]
            and t_ms > jump_ms
        ):
            crouch_ms = t_ms

        if (
            crouch_ms is not None
            and end_ms is None
            and previous["crouch"]
            and not crouch
            and t_ms > crouch_ms
        ):
            end_ms = t_ms
            break

        previous["jump"] = jump
        previous["crouch"] = crouch

    if jump_ms is None or crouch_ms is None or end_ms is None:
        return None

    return jump_ms, crouch_ms, end_ms


def piecewise_map_time(value, src_a, src_b, dst_a, dst_b):
    if src_b <= src_a:
        return dst_a
    alpha = (value - src_a) / (src_b - src_a)
    return dst_a + alpha * (dst_b - dst_a)


def donor_time_for_route_ms(route_ms, route_anchors):
    """
    Time-warp the donor's successful mechanical chain onto the route's first
    jump/crouch/release chain. This is the key difference from the previous
    donor mode, which kept donor metadata at its original absolute frame
    numbers even though the route performed the wall-jump at a different time.
    """
    if route_anchors is None:
        return route_ms

    route_jump, route_crouch, route_end = route_anchors

    if route_ms <= route_jump:
        return DONOR_CHAIN_JUMP_MS + (route_ms - route_jump)

    if route_ms <= route_crouch:
        return piecewise_map_time(
            route_ms,
            route_jump,
            route_crouch,
            DONOR_CHAIN_JUMP_MS,
            DONOR_CHAIN_CROUCH_MS,
        )

    if route_ms <= route_end:
        return piecewise_map_time(
            route_ms,
            route_crouch,
            route_end,
            DONOR_CHAIN_CROUCH_MS,
            DONOR_CHAIN_END_MS,
        )

    return DONOR_CHAIN_END_MS + (route_ms - route_end)


def frame_at_time(frames, target_ms):
    if not frames:
        raise RuntimeError("Template BOTREC vide")
    index = int(round(target_ms * BOTREC_HZ / 1000.0))
    index = max(0, min(len(frames) - 1, index))
    return frames[index]


# ---------------------------------------------------------
# MAIN
# ---------------------------------------------------------

def main():
    args = sys.argv[1:]

    if not args:
        print(
            "Usage :\n"
            "py tools\\botrec_apply_split_route.py "
            "\"chemin\\route.json\" "
            "[--template chemin\\decoded.bin]"
        )
        raise SystemExit(1)

    route_path = Path(args[0])
    template_path = BOTREC_PATH
    warp_native_chain = False
    airborne_xy_guide = False
    mechanics_command_view = False
    client_camera_view = False
    no_position_checkpoints = True
    force_dash_left = False
    delay_first_crouch_ms = 0.0

    i = 1
    while i < len(args):
        arg = args[i]

        if arg == "--template":
            if i + 1 >= len(args):
                raise SystemExit("--template attend un decoded.bin")
            template_path = Path(args[i + 1])
            i += 2
            continue

        if arg == "--warp-native-chain":
            warp_native_chain = True
            i += 1
            continue

        if arg == "--airborne-xy-guide":
            airborne_xy_guide = True
            i += 1
            continue

        if arg == "--mechanics-command-view":
            mechanics_command_view = True
            i += 1
            continue

        if arg == "--client-camera-view":
            client_camera_view = True
            i += 1
            continue

        if arg == "--legacy-position-checkpoints":
            # Compatibility experiment only: outer.field2 is CAMERA position,
            # not pawn origin (CCitadelUserCmdPB.vec_camera_position).
            no_position_checkpoints = False
            i += 1
            continue

        if arg == "--no-position-checkpoints":
            no_position_checkpoints = True
            i += 1
            continue

        if arg == "--delay-first-crouch-ms":
            if i + 1 >= len(args):
                raise SystemExit("--delay-first-crouch-ms attend un nombre de ms")
            delay_first_crouch_ms = float(args[i + 1])
            if not math.isfinite(delay_first_crouch_ms) or delay_first_crouch_ms < 0:
                raise SystemExit("Délai crouch invalide")
            i += 2
            continue

        if arg == "--force-dash-left":
            force_dash_left = True
            i += 1
            continue

        raise SystemExit(f"Argument inconnu : {arg}")

    if not route_path.exists():
        raise RuntimeError(
            f"Route introuvable : {route_path}"
        )

    if not template_path.exists():
        raise RuntimeError(
            f"BOTREC décodé introuvable : {template_path}"
        )

    # -----------------------------
    # Charge route SPLIT
    # -----------------------------

    route_meta, samples = load_route(route_path)

    if any("tMs" not in sample for sample in samples):
        raise RuntimeError(
            "La route SPLIT ne contient pas tMs sur tous les samples"
        )

    times_ms = [float(sample["tMs"]) for sample in samples]
    route_time_zero = times_ms[0]
    times_ms = [value - route_time_zero for value in times_ms]

    route_duration_ms = times_ms[-1]
    timing_source = "tMs"

    input_events, input_event_times_ms = prepare_input_events(
        route_meta,
        route_time_zero,
    )
    camera_events, camera_event_times_ms = prepare_camera_events(
        route_meta,
        route_time_zero,
    )

    first_crouch_start_ms = None
    first_crouch_release_ms = None
    if delay_first_crouch_ms:
        if not input_events:
            raise RuntimeError("Le test exige des inputEvents.")
        was_crouching = False
        for event, event_ms in zip(input_events, input_event_times_ms):
            crouching = bool(route_button_mask(event) & BUTTON_CROUCH)
            if first_crouch_start_ms is None and crouching and not was_crouching:
                first_crouch_start_ms = event_ms
            elif first_crouch_start_ms is not None and was_crouching and not crouching:
                first_crouch_release_ms = event_ms
                break
            was_crouching = crouching
        if first_crouch_start_ms is None or first_crouch_release_ms is None:
            raise RuntimeError("Premier appui/relâchement Ctrl introuvable.")
        if first_crouch_start_ms + delay_first_crouch_ms >= first_crouch_release_ms:
            raise RuntimeError("Le délai dépasse la durée du premier Ctrl.")


    detected_chain_anchors = (
        find_route_first_chain_anchors(route_meta)
        if (
            warp_native_chain
            or airborne_xy_guide
            or mechanics_command_view
        )
        else None
    )

    if (
        (
            warp_native_chain
            or airborne_xy_guide
            or mechanics_command_view
        )
        and detected_chain_anchors is None
    ):
        raise RuntimeError(
            "Impossible de détecter wall-jump -> crouch -> release "
            "dans inputEvents"
        )

    detected_chain_anchors = (
        tuple(
            value - route_time_zero
            for value in detected_chain_anchors
        )
        if detected_chain_anchors is not None
        else None
    )

    route_chain_anchors = (
        detected_chain_anchors
        if warp_native_chain
        else None
    )

    xy_guide_anchors = (
        detected_chain_anchors
        if airborne_xy_guide
        else None
    )

    mechanics_view_anchors = (
        detected_chain_anchors
        if mechanics_command_view
        else None
    )

    speed_ref = choose_reference_speed(samples)
    has_button_data = bool(input_events) or any(
        isinstance(sample.get("buttons"), dict)
        for sample in samples
    )
    has_movement_input_data = bool(input_events) or any(
        isinstance(sample.get("buttons"), dict)
        and any(
            key in sample["buttons"]
            for key in ("forward", "back", "left", "right")
        )
        for sample in samples
    )

    # -----------------------------
    # Charge template BOTREC
    # -----------------------------

    original = template_path.read_bytes()

    header, frames = load_frames(original)

    # Le BOTREC natif contient son point de spawn dans son header.
    # On le remplace par la position/caméra du début de la route SPLIT.
    header = patch_header_spawn(header, samples[0])

    print()
    print("=== SPLIT -> BOTREC ===")
    print()
    print(f"Route       : {route_path}")
    print(f"Template    : {template_path}")
    if route_chain_anchors is not None:
        print(
            "Donor warp  : "
            f"route {route_chain_anchors[0]:.1f}/"
            f"{route_chain_anchors[1]:.1f}/"
            f"{route_chain_anchors[2]:.1f} ms -> "
            f"donor {DONOR_CHAIN_JUMP_MS:.1f}/"
            f"{DONOR_CHAIN_CROUCH_MS:.1f}/"
            f"{DONOR_CHAIN_END_MS:.1f} ms"
        )
    if xy_guide_anchors is not None:
        print(
            "Airborne XY : "
            f"{xy_guide_anchors[0] + (2.0 * 1000.0 / BOTREC_HZ):.1f} -> "
            f"{xy_guide_anchors[1]:.1f} ms "
            "(velocity-integrated X/Y each BOTREC frame, Z left to native physics)"
        )
    if mechanics_view_anchors is not None:
        print(
            "Mechanics view: diagnostics.viewAngles from "
            f"{max(0.0, mechanics_view_anchors[0] - 100.0):.1f} -> "
            f"{mechanics_view_anchors[2]:.1f} ms "
            "(client camera ignored during wall-jump/mantle chain)"
        )
    if force_dash_left:
        print(
            "Dash lateral: force LEFT/Q while DASH is held "
            "(RIGHT/D -> LEFT/Q)"
        )
    if delay_first_crouch_ms:
        print(
            f"TEST first Ctrl: {first_crouch_start_ms:.1f} -> "
            f"{first_crouch_start_ms + delay_first_crouch_ms:.1f} ms "
            f"(release unchanged at {first_crouch_release_ms:.1f} ms)"
        )
    print(
        "Command view: "
        + ("legacy client camera (--client-camera-view)"
           if client_camera_view else "recorded pawn diagnostics.viewAngles where available")
    )
    print("Legacy camera-position overrides: "
          + ("disabled (default)" if no_position_checkpoints else "ENABLED (legacy experiment)"))
    print(f"Samples     : {len(samples)}")
    print(
        f"Sample rate : "
        f"{route_meta.get('targetSampleRateHz', '?')} Hz"
    )
    print(f"Durée route : {route_duration_ms / 1000.0:.3f} s")
    print(f"BOTREC Hz   : {BOTREC_HZ:.1f}")
    print(f"Timing      : {timing_source}")
    print(
        "Position    : grounded checkpoints only "
        f"(>= {GROUND_POSITION_CHECKPOINT_MS:.0f} ms apart, "
        f"|vz| <= {GROUND_POSITION_MAX_ABS_VZ:.1f})"
    )
    print(
        "Input timing: "
        + (
            f"high-rate events ({len(input_events)} transitions)"
            if input_events
            else "legacy telemetry-sample states"
        )
    )
    print(
        "Camera timing: "
        + (
            f"high-rate events ({len(camera_events)} transitions)"
            if camera_events
            else "legacy telemetry samples"
        )
    )
    print(f"Speed ref   : {speed_ref:.2f}")
    print(
        "Buttons     : "
        + (
            "movement+jump/crouch/dash"
            if has_movement_input_data
            else (
                "jump/crouch/dash"
                if has_button_data
                else "aucun (ancienne route)"
            )
        )
    )
    print()
    print(f"BOTREC      : {len(frames)} frames")

    # -----------------------------
    # Reconstitue yaw du template
    # -----------------------------

    yaw_states = []

    current_yaw = None

    for frame in frames:
        command = get_command(frame)

        if command is not None:
            current_yaw = extract_view_yaw(
                command,
                current_yaw,
            )

        yaw_states.append(current_yaw)

    first_valid_yaw = next(
        (
            yaw
            for yaw in yaw_states
            if yaw is not None
        ),
        None,
    )

    if first_valid_yaw is None:
        raise RuntimeError(
            "Impossible de trouver le yaw du BOTREC"
        )

    # Au cas où les toutes premières frames
    # n'auraient pas encore de viewangle.
    for i in range(len(yaw_states)):
        if yaw_states[i] is None:
            yaw_states[i] = first_valid_yaw
        else:
            break

    # -----------------------------
    # Applique la route
    # -----------------------------

    route_frame_count = int(
        math.floor(route_duration_ms * BOTREC_HZ / 1000.0)
    ) + 1

    if route_frame_count <= 0:
        raise RuntimeError(
            "Aucune frame utilisable"
        )

    # Construit exactement le nombre de frames demandé.
    #
    # - route <= template : on tronque comme avant.
    # - route > template  : on ajoute des frames synthétiques à partir
    #   d'une frame native propre, avec ticks continus.
    if route_chain_anchors is not None:
        modified_frames = []
        tick_step, _ = infer_tick_step(frames)
        first_tick = extract_command_tick(get_command(frames[0])) or 0

        for frame_index in range(route_frame_count):
            route_ms = frame_index * (1000.0 / BOTREC_HZ)
            donor_ms = donor_time_for_route_ms(
                route_ms,
                route_chain_anchors,
            )
            donor_frame = strip_outer_position(
                frame_at_time(frames, donor_ms)
            )
            modified_frames.append(
                patch_frame_tick(
                    donor_frame,
                    first_tick + tick_step * frame_index,
                )
            )

        synthetic_count = 0
    else:
        modified_frames = [
            strip_outer_position(frame)
            for frame in frames[:min(route_frame_count, len(frames))]
        ]

        synthetic_count = max(
            0,
            route_frame_count - len(modified_frames),
        )

    if synthetic_count:
        tick_step, last_tick = infer_tick_step(frames)

        synthetic_base = strip_outer_position(frames[-1])

        for extra_index in range(synthetic_count):
            synthetic_tick = (
                last_tick
                + tick_step * (extra_index + 1)
            )

            synthetic_frame = patch_frame_tick(
                synthetic_base,
                synthetic_tick,
            )

            modified_frames.append(
                synthetic_frame
            )

        print(
            f"Frames synthétiques : {synthetic_count} "
            f"(tick step={tick_step})"
        )

    header = patch_header_frame_count(
        header,
        route_frame_count,
    )

    print(
        f"Frames finales : {route_frame_count} "
        f"(template original : {len(frames)})"
    )

    preview_every = int(BOTREC_HZ)

    debug_window_start_ms = 0.0
    debug_window_end_ms = 2000.0
    debug_rows = [
        "frame,t_ms,sample_index,sample_t_ms,pos_x,pos_y,pos_z,"
        "vel_x,vel_y,vel_z,yaw,pitch,forward,left,"
        "forward_key,back_key,left_key,right_key,jump,crouch,dash,"
        "button_mask,changed_mask"
    ]

    previous_button_mask = 0
    clear_changed_next = False
    button_transition_count = 0
    last_position_checkpoint_ms = None
    early_checkpoint_ms = []

    xy_guide_x = None
    xy_guide_y = None
    xy_guide_prev_ms = None
    xy_guide_prev_vx = None
    xy_guide_prev_vy = None

    for frame_index in range(route_frame_count):
        target_ms = frame_index * (1000.0 / BOTREC_HZ)

        vx, vy, route_yaw, route_pitch = resample_route_at_ms(
            samples,
            times_ms,
            target_ms,
        )

        camera_state = camera_at_ms(
            camera_events,
            camera_event_times_ms,
            target_ms,
        )
        if camera_state is not None:
            route_yaw, route_pitch = camera_state

        # BOTREC field 4 is the command/pawn view, not the visual client
        # camera. Use recorded pawn view for every frame where available,
        # including ground movement and the very first crouch/slide.
        # --client-camera-view preserves the previous converter output for A/B.
        if not client_camera_view:
            command_view = command_view_at_ms(
                samples,
                times_ms,
                target_ms,
            )
            if command_view is not None:
                route_yaw, route_pitch = command_view

        movement_input = input_movement_at_ms(
            samples,
            times_ms,
            input_events,
            input_event_times_ms,
            target_ms,
        )

        if movement_input is not None:
            forward, left = movement_input
        else:
            forward, left = movement_from_velocity(
                vx,
                vy,
                route_yaw,
                speed_ref,
            )

        current_button_mask = input_button_mask_at_ms(
            samples,
            times_ms,
            input_events,
            input_event_times_ms,
            target_ms,
        )

        # Isolated timing experiment: postpone only the first Ctrl press.
        if (
            delay_first_crouch_ms
            and first_crouch_start_ms <= target_ms
            < first_crouch_start_ms + delay_first_crouch_ms
        ):
            current_button_mask &= ~BUTTON_CROUCH

        # Diagnostic/fix for the final air-dash in this route.
        # The recorded JSON itself contains RIGHT + DASH even though the
        # intended physical input was Z+Q+Shift. Swap only the lateral
        # component while dash is held; do not touch any wall-jump inputs.
        if (
            force_dash_left
            and (current_button_mask & BUTTON_DASH)
            and (current_button_mask & BUTTON_RIGHT)
            and not (current_button_mask & BUTTON_LEFT)
        ):
            current_button_mask &= ~BUTTON_RIGHT
            current_button_mask |= BUTTON_LEFT

            if movement_input is not None:
                # Preserve forward/back exactly; replace D (-1) with Q (+1).
                left = 1.0

        changed_button_mask = (
            previous_button_mask ^ current_button_mask
        )

        if frame_index == 0:
            # Explicitly initialize all three button states on every playback.
            button_mode = "initial"
            clear_changed_next = True
            if changed_button_mask:
                button_transition_count += 1
        elif changed_button_mask:
            button_mode = "change"
            clear_changed_next = True
            button_transition_count += 1
        elif clear_changed_next:
            button_mode = (
                "reset"
                if current_button_mask == 0
                else "clear_changed"
            )
            clear_changed_next = False
        else:
            button_mode = None

        cleaned_frame = patch_frame_view_and_buttons(
            modified_frames[frame_index],
            route_pitch,
            route_yaw,
        )

        cleaned_frame = patch_frame_buttons(
            cleaned_frame,
            button_mode,
            current_button_mask,
            changed_button_mask,
        )

        positioned_frame = patch_frame_movement(
            cleaned_frame,
            forward,
            left,
        )

        # Hybrid guidance:
        # - preserve native Z physics for wall-jump / mantle;
        # - after the wall-jump has had two BOTREC ticks to establish contact
        #   and impulse, constrain only X/Y to the recorded route until the
        #   buffered crouch begins;
        # - otherwise use only conservative stable-ground XYZ checkpoints.
        xy_guide_active = False

        if xy_guide_anchors is not None:
            wall_jump_ms, crouch_ms, _ = xy_guide_anchors
            guide_start_ms = wall_jump_ms + (
                2.0 * 1000.0 / BOTREC_HZ
            )
            xy_guide_active = (
                guide_start_ms <= target_ms < crouch_ms
            )

        if xy_guide_active:
            # IMPORTANT: do NOT use raw route X/Y every frame here.
            # Around the wall-jump, recorded m_vecAbsOrigin contains large
            # one-frame discontinuities even while velocity is continuous.
            # Start from the cleanest nearby position sample, then integrate
            # the recorded world velocity at BOTREC cadence.
            if xy_guide_x is None or xy_guide_y is None:
                xy_guide_x, xy_guide_y = best_position_anchor_near_ms(
                    samples,
                    times_ms,
                    target_ms,
                )
                xy_guide_prev_ms = target_ms
                xy_guide_prev_vx = vx
                xy_guide_prev_vy = vy
            else:
                dt_seconds = (
                    target_ms - xy_guide_prev_ms
                ) / 1000.0

                # Trapezoidal integration smooths normal velocity changes
                # without copying the torn absolute-origin samples.
                xy_guide_x += (
                    (xy_guide_prev_vx + vx)
                    * 0.5
                    * dt_seconds
                )
                xy_guide_y += (
                    (xy_guide_prev_vy + vy)
                    * 0.5
                    * dt_seconds
                )

                xy_guide_prev_ms = target_ms
                xy_guide_prev_vx = vx
                xy_guide_prev_vy = vy

            modified_frames[frame_index] = patch_outer_position_xy(
                positioned_frame,
                xy_guide_x,
                xy_guide_y,
            )
        else:
            # Reset integration state once the airborne guide window ends.
            if xy_guide_prev_ms is not None:
                xy_guide_x = None
                xy_guide_y = None
                xy_guide_prev_ms = None
                xy_guide_prev_vx = None
                xy_guide_prev_vy = None
            use_position_checkpoint = (
                not no_position_checkpoints
                and frame_index < len(frames)
                and has_outer_position(frames[frame_index])
                and should_apply_ground_position_checkpoint(
                    samples,
                    times_ms,
                    target_ms,
                    last_position_checkpoint_ms,
                )
            )

            if use_position_checkpoint:
                route_x, route_y, route_z = resample_position_at_ms(
                    samples,
                    times_ms,
                    target_ms,
                )
                modified_frames[frame_index] = patch_outer_position(
                    positioned_frame,
                    route_x,
                    route_y,
                    route_z,
                )
                last_position_checkpoint_ms = target_ms
                if target_ms <= 2000.0:
                    early_checkpoint_ms.append(round(target_ms, 3))
            else:
                modified_frames[frame_index] = strip_outer_position(
                    positioned_frame
                )

        previous_button_mask = current_button_mask

        if (
            debug_window_start_ms
            <= target_ms
            <= debug_window_end_ms
        ):
            source_index, source_sample = route_sample_at_ms(
                samples,
                times_ms,
                target_ms,
            )
            source_buttons = source_sample.get("buttons", {})
            source_position = source_sample.get(
                "position",
                [0.0, 0.0, 0.0],
            )
            source_velocity = source_sample.get(
                "velocity",
                [0.0, 0.0, 0.0],
            )
            debug_rows.append(
                ",".join(
                    [
                        str(frame_index),
                        f"{target_ms:.3f}",
                        str(source_index),
                        f"{times_ms[source_index]:.3f}",
                        f"{float(source_position[0]):.4f}",
                        f"{float(source_position[1]):.4f}",
                        f"{float(source_position[2]):.4f}",
                        f"{float(source_velocity[0]):.4f}",
                        f"{float(source_velocity[1]):.4f}",
                        f"{float(source_velocity[2]):.4f}",
                        f"{route_yaw:.4f}",
                        f"{route_pitch:.4f}",
                        f"{forward:.3f}",
                        f"{left:.3f}",
                        str(int(bool(source_buttons.get("forward")))),
                        str(int(bool(source_buttons.get("back")))),
                        str(int(bool(source_buttons.get("left")))),
                        str(int(bool(source_buttons.get("right")))),
                        str(int(bool(source_buttons.get("jump")))),
                        str(int(bool(source_buttons.get("crouch")))),
                        str(int(bool(source_buttons.get("dash")))),
                        f"0x{current_button_mask:X}",
                        f"0x{changed_button_mask:X}",
                    ]
                )
            )

        if (
            frame_index % preview_every == 0
            or frame_index == route_frame_count - 1
        ):
            print(
                f"frame={frame_index:<4} "
                f"t={target_ms / 1000.0:6.3f}s "
                f"vel=({vx:+7.1f},{vy:+7.1f}) "
                f"yaw={route_yaw:+7.2f} "
                f"=> "
                f"forward={forward:+.3f} "
                f"left={left:+.3f}"
            )

    print(
        "Ground checkpoints in first 2s (ms): "
        + (", ".join(str(t) for t in early_checkpoint_ms)
           if early_checkpoint_ms else "none")
    )
    print(
        f"Transitions buttons : {button_transition_count}"
    )

    DEBUG_WINDOW_PATH.write_text(
        "\n".join(debug_rows) + "\n",
        encoding="utf-8",
    )
    print(
        "Debug 0.0-2.0s : "
        f"{DEBUG_WINDOW_PATH}"
    )

    # -----------------------------
    # Reconstruit decoded_modified
    # -----------------------------

    rebuilt = bytearray(header)

    for frame in modified_frames:
        rebuilt += encode_varint(len(frame))
        rebuilt += frame

    OUTPUT_PATH.write_bytes(rebuilt)

    print()
    print("[SPLIT -> BOTREC] OK")
    print(
        f"Original : {len(original)} bytes"
    )
    print(
        f"Modifié  : {len(rebuilt)} bytes"
    )
    print(
        f"Créé     : {OUTPUT_PATH}"
    )
    print()
    print(
        "Maintenant : "
        "py tools\\botrec_rebuild.py"
    )


if __name__ == "__main__":
    main()