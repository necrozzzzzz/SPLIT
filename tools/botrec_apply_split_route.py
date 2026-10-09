from pathlib import Path
import bisect
import json
import math
import struct
import sys


BOTREC_PATH = Path("botrec_debug/decoded.bin")
OUTPUT_PATH = Path("botrec_debug/decoded_modified.bin")

HEADER_SIZE = 32
DEADZONE_SPEED = 10.0
BOTREC_HZ = 64.0

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

                # Reset buttons (field 3 / wire 7)
                command_out += encode_varint((3 << 3) | 7)

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


def patch_frame_movement(frame, forward, left):
    """
    Ajoute forwardmove/leftmove à la FIN du command message.

    Le moteur utilise les dernières valeurs présentes :
    c'est exactement le principe validé avec notre test carré.
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
                # field 5 = forwardmove
                # field 6 = leftmove

                payload += float_field(5, forward)
                payload += float_field(6, left)

                out += encode_varint(key)
                out += encode_varint(len(payload))
                out += payload

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


# ---------------------------------------------------------
# MAIN
# ---------------------------------------------------------

def main():
    if len(sys.argv) < 2:
        print(
            "Usage :\n"
            "py tools\\botrec_apply_split_route.py "
            "\"chemin\\route.json\""
        )
        raise SystemExit(1)

    route_path = Path(sys.argv[1])

    if not route_path.exists():
        raise RuntimeError(
            f"Route introuvable : {route_path}"
        )

    if not BOTREC_PATH.exists():
        raise RuntimeError(
            f"BOTREC décodé introuvable : {BOTREC_PATH}"
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

    speed_ref = choose_reference_speed(samples)

    # -----------------------------
    # Charge template BOTREC
    # -----------------------------

    original = BOTREC_PATH.read_bytes()

    header, frames = load_frames(original)

    # Le BOTREC natif contient son point de spawn dans son header.
    # On le remplace par la position/caméra du début de la route SPLIT.
    header = patch_header_spawn(header, samples[0])

    print()
    print("=== SPLIT -> BOTREC ===")
    print()
    print(f"Route       : {route_path}")
    print(f"Samples     : {len(samples)}")
    print(
        f"Sample rate : "
        f"{route_meta.get('targetSampleRateHz', '?')} Hz"
    )
    print(f"Durée route : {route_duration_ms / 1000.0:.3f} s")
    print(f"BOTREC Hz   : {BOTREC_HZ:.1f}")
    print(f"Speed ref   : {speed_ref:.2f}")
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

    if route_frame_count > len(frames):
        missing = route_frame_count - len(frames)
        raise RuntimeError(
            "Le BOTREC template est encore trop court pour cette route : "
            f"{missing} frame(s) manquante(s) "
            f"(~{missing / BOTREC_HZ:.2f} s). "
            "Pour l'instant, utilise une route plus courte que le template."
        )

    # On garde exactement le nombre de frames correspondant à la durée SPLIT.
    # Les frames de fin du vieux template ne sont plus conservées.
    modified_frames = list(frames[:route_frame_count])
    header = patch_header_frame_count(header, route_frame_count)

    print(
        f"Frames finales : {route_frame_count} "
        f"(template original : {len(frames)})"
    )

    preview_every = int(BOTREC_HZ)

    for frame_index in range(route_frame_count):
        target_ms = frame_index * (1000.0 / BOTREC_HZ)

        vx, vy, route_yaw, route_pitch = resample_route_at_ms(
            samples,
            times_ms,
            target_ms,
        )

        forward, left = movement_from_velocity(
            vx,
            vy,
            route_yaw,
            speed_ref,
        )

        cleaned_frame = patch_frame_view_and_buttons(
            modified_frames[frame_index],
            route_pitch,
            route_yaw,
        )

        modified_frames[frame_index] = (
            patch_frame_movement(
                cleaned_frame,
                forward,
                left,
            )
        )

        if (
            frame_index % preview_every == 0
            or frame_index == usable - 1
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