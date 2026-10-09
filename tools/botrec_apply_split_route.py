from pathlib import Path
import json
import math
import struct
import sys


BOTREC_PATH = Path("botrec_debug/decoded.bin")
OUTPUT_PATH = Path("botrec_debug/decoded_modified.bin")

HEADER_SIZE = 32
DEADZONE_SPEED = 10.0


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


def choose_reference_speed(samples, start):
    """
    90e percentile des vitesses horizontales.

    Ça nous permet de convertir automatiquement :
    vitesse SPLIT -> amplitude joystick 0..1
    """

    speeds = []

    for sample in samples[start:]:
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

    route_start = find_first_moving_sample(samples)

    speed_ref = choose_reference_speed(
        samples,
        route_start,
    )

    # -----------------------------
    # Charge template BOTREC
    # -----------------------------

    original = BOTREC_PATH.read_bytes()

    header, frames = load_frames(original)

    print()
    print("=== SPLIT -> BOTREC ===")
    print()
    print(f"Route       : {route_path}")
    print(f"Samples     : {len(samples)}")
    print(
        f"Sample rate : "
        f"{route_meta.get('targetSampleRateHz', '?')} Hz"
    )
    print(f"Début mouv. : sample {route_start}")
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

    available_route_samples = (
        len(samples) - route_start
    )

    usable = min(
        len(frames),
        available_route_samples,
    )

    if usable <= 0:
        raise RuntimeError(
            "Aucune frame utilisable"
        )

    print(
        f"Frames route appliquées : {usable}"
    )

    modified_frames = list(frames)

    preview_every = 60

    for frame_index in range(usable):
        sample_index = (
            route_start + frame_index
        )

        sample = samples[sample_index]

        vel = sample.get(
            "velocity",
            [0.0, 0.0, 0.0],
        )

        vx = float(vel[0])
        vy = float(vel[1])

        route_yaw = float(sample.get("yaw", 0.0))

        forward, left = movement_from_velocity(
            vx,
            vy,
            route_yaw,
            speed_ref,
        )

        modified_frames[frame_index] = (
            patch_frame_movement(
                frames[frame_index],
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
                f"sample={sample_index:<4} "
                f"vel=({vx:+7.1f},{vy:+7.1f}) "
                f"yaw={route_yaw:+7.2f} "
                f"=> "
                f"forward={forward:+.3f} "
                f"left={left:+.3f}"
            )

    # Après la partie route :
    # on force STOP sur les frames restantes.

    for frame_index in range(
        usable,
        len(modified_frames),
    ):
        modified_frames[frame_index] = (
            patch_frame_movement(
                frames[frame_index],
                0.0,
                0.0,
            )
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