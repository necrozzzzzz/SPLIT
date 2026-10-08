from pathlib import Path


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

    # field 6 / wire 5 = leftmove
    #
    # 0x35 = (6 << 3) | 5

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

    DST.write_bytes(data)

    print()
    print("[BOTREC MODIFY] OK")
    print(f"Créé : {DST}")


if __name__ == "__main__":
    main()