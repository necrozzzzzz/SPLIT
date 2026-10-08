from pathlib import Path
import struct


SRC = Path("botrec_debug/decoded.bin")
DST = Path("botrec_debug/decoded_modified.bin")


def replace_exact(data, old, new, expected_count, name):
    count = data.count(old)

    print(f"{name}: {count} occurrence(s)")

    if count != expected_count:
        raise RuntimeError(
            f"{name}: attendu {expected_count}, trouvé {count}. "
            "J'arrête pour ne pas modifier le mauvais endroit."
        )

    return data.replace(old, new)


def main():
    data = SRC.read_bytes()

    # Change le héros enregistré dans le BOTREC.
    # Haze = 13
    # Bebop = 15
    data = bytearray(data)
    struct.pack_into("<I", data, 0, 15)
    data = bytes(data)

    #
    # 1. Inverser leftmove
    #

    minus_075 = bytes.fromhex("35 00 00 40 BF")
    plus_075  = bytes.fromhex("35 00 00 40 3F")

    minus_100 = bytes.fromhex("35 00 00 80 BF")
    plus_100  = bytes.fromhex("35 00 00 80 3F")

    minus_025 = bytes.fromhex("35 00 00 80 BE")
    plus_025  = bytes.fromhex("35 00 00 80 3E")

    data = replace_exact(
        data,
        minus_075,
        plus_075,
        1,
        "left -0.75 -> +0.75",
    )

    data = replace_exact(
        data,
        minus_100,
        plus_100,
        1,
        "left -1.00 -> +1.00",
    )

    data = replace_exact(
        data,
        minus_025,
        plus_025,
        1,
        "left -0.25 -> +0.25",
    )

    #
    # 2. Inverser le bouton logique
    #
    # RIGHT = 0x400
    # protobuf varint : 80 08
    #
    # LEFT = 0x200
    # protobuf varint : 80 04
    #

    right_field1 = bytes.fromhex("08 80 08")
    left_field1  = bytes.fromhex("08 80 04")

    right_field2 = bytes.fromhex("10 80 08")
    left_field2  = bytes.fromhex("10 80 04")

    # field1 = held
    # Il apparaît au PRESS.
    data = replace_exact(
        data,
        right_field1,
        left_field1,
        1,
        "held RIGHT 0x400 -> LEFT 0x200",
    )

    # field2 = changed
    # Il apparaît au PRESS + au RELEASE.
    data = replace_exact(
        data,
        right_field2,
        left_field2,
        2,
        "changed RIGHT 0x400 -> LEFT 0x200",
    )

    DST.write_bytes(data)

    print()
    print("[BOTREC MODIFY] OK")
    print(f"Créé : {DST}")


if __name__ == "__main__":
    main()