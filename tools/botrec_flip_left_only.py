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

    # leftmove uniquement.
    # On ne touche NI aux buttons NI aux positions.

    data = replace_exact(
        data,
        bytes.fromhex("35 00 00 40 BF"),
        bytes.fromhex("35 00 00 40 3F"),
        1,
        "-0.75 -> +0.75",
    )

    data = replace_exact(
        data,
        bytes.fromhex("35 00 00 80 BF"),
        bytes.fromhex("35 00 00 80 3F"),
        1,
        "-1.00 -> +1.00",
    )

    data = replace_exact(
        data,
        bytes.fromhex("35 00 00 80 BE"),
        bytes.fromhex("35 00 00 80 3E"),
        1,
        "-0.25 -> +0.25",
    )

    DST.write_bytes(data)

    print()
    print("[BOTREC FLIP LEFT ONLY] OK")
    print(f"size = {len(data)}")
    print(f"Créé : {DST}")


if __name__ == "__main__":
    main()