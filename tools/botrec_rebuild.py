from pathlib import Path
import base64

import zstandard as zstd


DEBUG_DIR = Path("botrec_debug")

PACKED_PATH = DEBUG_DIR / "packed.bin"
DECODED_PATH = DEBUG_DIR / "decoded_modified.bin"
OUTPUT_PATH = DEBUG_DIR / "BOTREC_REBUILT.txt"


def encode_botrec(header: bytes, decoded: bytes) -> str:
    # Recompresse le payload décompressé.
    compressed = zstd.ZstdCompressor(level=3).compress(decoded)

    # Le BOTREC original contient un header de 5 octets
    # avant le flux Zstandard.
    packed = header + compressed

    # Base64 URL-safe, sans padding.
    encoded = base64.urlsafe_b64encode(packed).decode("ascii")
    encoded = encoded.rstrip("=")

    # Deadlock utilise "." à la place de "-"
    encoded = encoded.replace("-", ".")

    return "BOTREC." + encoded


def main():
    if not PACKED_PATH.exists():
        raise FileNotFoundError(
            "botrec_debug\\packed.bin introuvable.\n"
            "Lance d'abord botrec_decode.py sur un vrai BOTREC."
        )

    if not DECODED_PATH.exists():
        raise FileNotFoundError(
            "botrec_debug\\decoded.bin introuvable."
        )

    original_packed = PACKED_PATH.read_bytes()
    decoded = DECODED_PATH.read_bytes()

    if len(original_packed) < 5:
        raise ValueError("packed.bin est trop petit")

    header = original_packed[:5]

    rebuilt = encode_botrec(header, decoded)

    OUTPUT_PATH.write_text(
        rebuilt,
        encoding="utf-8",
    )

    print("[BOTREC REBUILD] OK")
    print(f"header       = {header.hex(' ')}")
    print(f"decoded      = {len(decoded)} bytes")
    print(f"output chars = {len(rebuilt)}")
    print()
    print(f"Créé : {OUTPUT_PATH}")


if __name__ == "__main__":
    main()