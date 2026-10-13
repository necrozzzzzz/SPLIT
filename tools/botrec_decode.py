import base64
import struct
import sys
import tkinter as tk
from pathlib import Path

import zstandard as zstd


def get_clipboard():
    root = tk.Tk()
    root.withdraw()

    try:
        return root.clipboard_get().strip()
    finally:
        root.destroy()


def decode_botrec(code: str):
    if not code.startswith("BOTREC."):
        raise ValueError("Le presse-papiers ne commence pas par BOTREC.")

    encoded = code[len("BOTREC."):]

    # Alphabet utilisé par le share code Deadlock :
    # '.' correspond au '-' du Base64 URL-safe.
    encoded = encoded.replace(".", "-")
    encoded += "=" * ((4 - len(encoded) % 4) % 4)

    packed = base64.urlsafe_b64decode(encoded)

    if len(packed) < 9:
        raise ValueError("BOTREC trop court.")

    header = packed[:5]
    compressed = packed[5:]

    # Zstandard magic = 28 B5 2F FD
    if compressed[:4] != bytes.fromhex("28 B5 2F FD"):
        raise ValueError(
            f"Flux Zstd introuvable après le header. "
            f"Début={compressed[:8].hex(' ')}"
        )

    decompressed = zstd.ZstdDecompressor().decompress(compressed)

    return header, packed, decompressed


def hex_dump(data: bytes, width=16):
    lines = []

    for offset in range(0, len(data), width):
        chunk = data[offset:offset + width]
        hex_part = " ".join(f"{b:02X}" for b in chunk)
        ascii_part = "".join(
            chr(b) if 32 <= b <= 126 else "."
            for b in chunk
        )

        lines.append(
            f"{offset:08X}  "
            f"{hex_part:<{width * 3}} "
            f"{ascii_part}"
        )

    return "\n".join(lines)


def main():
    args = sys.argv[1:]
    out_dir = Path("botrec_debug")
    input_path = None

    i = 0
    while i < len(args):
        arg = args[i]

        if arg == "--out-dir":
            if i + 1 >= len(args):
                raise SystemExit("--out-dir attend un dossier")
            out_dir = Path(args[i + 1])
            i += 2
            continue

        if input_path is not None:
            raise SystemExit(
                "Usage: py tools\\botrec_decode.py "
                "[BOTREC.txt] [--out-dir dossier]"
            )

        input_path = Path(arg)
        i += 1

    if input_path is not None:
        code = input_path.read_text(
            encoding="utf-8"
        ).strip()
    else:
        code = get_clipboard()

    header, packed, decompressed = decode_botrec(code)

    out_dir.mkdir(parents=True, exist_ok=True)

    (out_dir / "packed.bin").write_bytes(packed)
    (out_dir / "decoded.bin").write_bytes(decompressed)

    (out_dir / "decoded_hex.txt").write_text(
        hex_dump(decompressed),
        encoding="utf-8",
    )

    print()
    print("[BOTREC] OK")
    print(f"[BOTREC] header       = {header.hex(' ')}")
    print(f"[BOTREC] packed       = {len(packed)} bytes")
    print(f"[BOTREC] decompressed = {len(decompressed)} bytes")
    print()
    print("Créé :")
    print(f"  {out_dir}\\packed.bin")
    print(f"  {out_dir}\\decoded.bin")
    print(f"  {out_dir}\\decoded_hex.txt")


if __name__ == "__main__":
    main()