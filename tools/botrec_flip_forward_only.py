from pathlib import Path


SRC = Path("botrec_debug/decoded.bin")
DST = Path("botrec_debug/decoded_modified.bin")


def replace_exact(data, old, new, expected_count, name):
    count = data.count(old)

    print(f"{name}: {count} occurrence(s)")

    if count != expected_count:
        raise RuntimeError(
            f"{name}: attendu {expected_count}, trouvé {count}"
        )

    return data.replace(old, new)


def main():
    data = SRC.read_bytes()

    # field 5 / wire 5 = forwardmove
    #
    # +0.75 <-> -0.75
    # +1.00 <-> -1.00
    # +0.25 <-> -0.25

    plus_075  = bytes.fromhex("2D 00 00 40 3F")
    minus_075 = bytes.fromhex("2D 00 00 40 BF")

    plus_100  = bytes.fromhex("2D 00 00 80 3F")
    minus_100 = bytes.fromhex("2D 00 00 80 BF")

    plus_025  = bytes.fromhex("2D 00 00 80 3E")
    minus_025 = bytes.fromhex("2D 00 00 80 BE")

    # On utilise des placeholders pour éviter
    # qu'un replace inverse soit re-remplacé ensuite.

    tmp_075 = bytes.fromhex("2D 01 02 03 04")
    tmp_100 = bytes.fromhex("2D 05 06 07 08")
    tmp_025 = bytes.fromhex("2D 09 0A 0B 0C")

    data = replace_exact(
        data,
        plus_075,
        tmp_075,
        1,
        "+0.75 -> TEMP",
    )

    data = replace_exact(
        data,
        minus_075,
        plus_075,
        1,
        "-0.75 -> +0.75",
    )

    data = replace_exact(
        data,
        tmp_075,
        minus_075,
        1,
        "TEMP -> -0.75",
    )

    data = replace_exact(
        data,
        plus_100,
        tmp_100,
        1,
        "+1.00 -> TEMP",
    )

    data = replace_exact(
        data,
        minus_100,
        plus_100,
        1,
        "-1.00 -> +1.00",
    )

    data = replace_exact(
        data,
        tmp_100,
        minus_100,
        1,
        "TEMP -> -1.00",
    )

    data = replace_exact(
        data,
        plus_025,
        tmp_025,
        1,
        "+0.25 -> TEMP",
    )

    data = replace_exact(
        data,
        minus_025,
        plus_025,
        1,
        "-0.25 -> +0.25",
    )

    data = replace_exact(
        data,
        tmp_025,
        minus_025,
        1,
        "TEMP -> -0.25",
    )

    DST.write_bytes(data)

    print()
    print("[BOTREC FLIP FORWARD ONLY] OK")
    print(f"size = {len(data)}")
    print(f"Créé : {DST}")


if __name__ == "__main__":
    main()