from pathlib import Path
import base64
import zlib
import struct

import zstandard as zstd


DEBUG_DIR = Path("botrec_debug")

PACKED_PATH = DEBUG_DIR / "packed.bin"
DECODED_PATH = DEBUG_DIR / "decoded_modified.bin"
OUTPUT_PATH = DEBUG_DIR / "BOTREC_REBUILT.txt"
HEADER_SIZE = 32


def read_varint(data: bytes, offset: int):
    value = 0
    shift = 0
    while True:
        if offset >= len(data):
            raise ValueError("varint hors limites")
        b = data[offset]
        offset += 1
        value |= (b & 0x7F) << shift
        if not (b & 0x80):
            return value, offset
        shift += 7


def count_frames(decoded: bytes) -> int:
    if len(decoded) < HEADER_SIZE:
        raise ValueError("decoded_modified.bin est trop petit")

    offset = HEADER_SIZE
    count = 0
    while offset < len(decoded):
        size, payload_start = read_varint(decoded, offset)
        payload_end = payload_start + size
        if payload_end > len(decoded):
            raise ValueError("frame tronquée dans decoded_modified.bin")
        count += 1
        offset = payload_end

    return count


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

    # Vérifie que le payload à reconstruire est bien le nouveau fichier.
    frame_count = count_frames(decoded)
    header_frame_count = struct.unpack_from("<I", decoded, 0x1C)[0]

    if header_frame_count != frame_count:
        raise ValueError(
            "Incohérence decoded_modified.bin : "
            f"header={header_frame_count} frames, contenu={frame_count} frames"
        )

    # Byte 0 = version
    version = original_packed[0]

    # Bytes 1..4 = CRC32 du payload décompressé, little-endian
    crc = zlib.crc32(decoded) & 0xFFFFFFFF

    header = bytes([version]) + struct.pack("<I", crc)

    print(f"crc32        = 0x{crc:08X}")
    print(f"new header   = {header.hex(' ')}")

    rebuilt = encode_botrec(header, decoded)

    OUTPUT_PATH.write_text(
        rebuilt,
        encoding="utf-8",
    )

    print("[BOTREC REBUILD] OK")
    print(f"header       = {header.hex(' ')}")
    print(f"frames       = {frame_count}")
    print(f"decoded      = {len(decoded)} bytes")
    print(f"output chars = {len(rebuilt)}")
    print()
    print(f"Créé : {OUTPUT_PATH.resolve()}")


if __name__ == "__main__":
    main()