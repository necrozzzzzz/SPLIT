"""Extract only collision resources from dl_midtown.vpk into a diagnostic ZIP.

Python standard library only. Opens game archives read-only. Supports standard
Valve VPK versions 1 and 2; rejects other formats rather than guessing offsets.
"""
import argparse
import io
import json
from pathlib import Path, PurePosixPath
import struct
import sys
import zipfile
import zlib

DEFAULT_VPK = Path(r"C:\Program Files (x86)\Steam\steamapps\common\Deadlock\game\citadel\maps\dl_midtown.vpk")
MAX_TREE = 64 * 1024 * 1024
MAX_SELECTED = 160 * 1024 * 1024


def exact(stream, length):
    data = stream.read(length)
    if len(data) != length:
        raise ValueError("VPK tronque : lecture incomplete.")
    return data


def read_directory(path):
    with path.open("rb") as stream:
        magic, version, tree_size = struct.unpack("<III", exact(stream, 12))
        if magic != 0x55AA1234 or version not in (1, 2):
            raise ValueError(f"Format non pris en charge : signature={magic:#x}, version={version}. Envoyer ce message.")
        if not 0 < tree_size <= MAX_TREE:
            raise ValueError(f"Taille d'index inhabituelle : {tree_size}.")
        section_sizes = struct.unpack("<IIII", exact(stream, 16)) if version == 2 else None
        header_size = stream.tell()
        tree = io.BytesIO(exact(stream, tree_size))
    file_size = path.stat().st_size
    data_base = header_size + tree_size
    embedded_size = section_sizes[0] if section_sizes else file_size - data_base
    if embedded_size < 0 or data_base + embedded_size > file_size:
        raise ValueError("Section de donnees VPK invalide.")

    def string():
        parts = bytearray()
        while True:
            char = exact(tree, 1)
            if char == b"\0":
                return parts.decode("utf-8", errors="strict")
            parts.extend(char)
            if len(parts) > 16384:
                raise ValueError("Nom VPK anormalement long.")

    entries = []
    while True:
        extension = string()
        if not extension:
            break
        while True:
            folder = string()
            if not folder:
                break
            while True:
                name = string()
                if not name:
                    break
                crc, preload_size, archive, offset, length, terminator = struct.unpack("<IHHIIH", exact(tree, 18))
                if terminator != 0xFFFF:
                    raise ValueError("Entree VPK invalide : marqueur final incorrect.")
                preload = exact(tree, preload_size)
                filename = name + ("." + extension if extension != " " else "")
                full_name = (folder + "/" if folder != " " else "") + filename
                parts = PurePosixPath(full_name.replace("\\", "/"))
                if parts.is_absolute() or ".." in parts.parts or ":" in str(parts):
                    raise ValueError("Chemin interne VPK non sur.")
                if archive == 0x7FFF and offset + length > embedded_size:
                    raise ValueError(f"Donnees hors limites pour {full_name}.")
                entries.append(dict(path=str(parts), extension=extension, crc=crc, archive=archive,
                                    offset=offset, length=length, preload=preload))
    if tree.tell() != tree_size:
        raise ValueError("Index VPK avec donnees inattendues : extraction arretee.")
    return version, data_base, entries


def selected(entry):
    path = PurePosixPath(entry["path"])
    ext = entry["extension"].lower()
    if "vphys" in ext or path.stem.lower() == "world_physics":
        return True
    return ext in {"vwrld", "vwrld_c", "vmap", "vmap_c"} and entry["length"] + len(entry["preload"]) < 4 * 1024 * 1024


def archive_location(vpk_path, entry, data_base):
    if entry["archive"] == 0x7FFF:
        return vpk_path, data_base + entry["offset"]
    stem = vpk_path.stem
    if stem.endswith("_dir"):
        stem = stem[:-4]
    return vpk_path.with_name(f"{stem}_{entry['archive']:03d}.vpk"), entry["offset"]


def export(vpk_path, output):
    version, data_base, entries = read_directory(vpk_path)
    chosen = [entry for entry in entries if selected(entry)]
    total = sum(entry["length"] + len(entry["preload"]) for entry in chosen)
    if total > MAX_SELECTED:
        raise ValueError(f"Selection trop volumineuse ({total / 1024**2:.1f} Mio). Envoyer ce message avant extraction.")
    output = output.resolve()
    if output == vpk_path.resolve():
        raise ValueError("Le ZIP ne peut pas remplacer l'archive du jeu.")
    manifest = dict(vpkVersion=version, vpkFilename=vpk_path.name,
                    selectedBytes=total, selectedCount=len(chosen),
                    entries=[{k:v for k,v in e.items() if k != "preload"} |
                             {"preloadBytes":len(e["preload"]), "selected":selected(e)} for e in entries],
                    purpose="Static collision inspection only. No game files modified.")
    # Exclusive creation avoids silently replacing an earlier capture.
    created = False
    try:
        with output.open("xb") as destination:
            created = True
            with zipfile.ZipFile(destination, "w", zipfile.ZIP_DEFLATED) as bundle:
                bundle.writestr("CONTENU_VPK.json", json.dumps(manifest, indent=2, ensure_ascii=False))
                for entry in chosen:
                    archive, offset = archive_location(vpk_path, entry, data_base)
                    if offset + entry["length"] > archive.stat().st_size:
                        raise ValueError(f"Donnees hors limites : {archive.name}.")
                    crc = zlib.crc32(entry["preload"])
                    with archive.open("rb") as source, bundle.open("resources/"+entry["path"], "w") as target:
                        source.seek(offset)
                        target.write(entry["preload"])
                        remaining = entry["length"]
                        while remaining:
                            chunk = exact(source, min(1024*1024, remaining))
                            target.write(chunk)
                            crc = zlib.crc32(chunk, crc)
                            remaining -= len(chunk)
                    if crc & 0xFFFFFFFF != entry["crc"]:
                        raise ValueError(f"CRC32 incorrect : {entry['path']}. ZIP supprime.")
                    print(f"Extrait : {entry['path']} ({entry['length'] + len(entry['preload'])} octets)")
    except BaseException:
        if created:
            output.unlink(missing_ok=True)
        raise
    print(f"\nOK : {len(entries)} fichiers indexes, {len(chosen)} ressources extraites.")
    if not chosen:
        print("Aucune ressource physique trouvee : le ZIP contient l'index pour localiser la suite.")
    print(f"A ENVOYER : {output}")
    print(f"Taille ZIP : {output.stat().st_size / 1024**2:.2f} Mio")
    return output


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("vpk", nargs="?", type=Path, default=DEFAULT_VPK)
    parser.add_argument("--out", type=Path, default=Path("SPLIT_COLLISIONS_dl_midtown.zip"))
    args = parser.parse_args()
    try:
        export(args.vpk, args.out)
    except Exception as error:
        print(f"ERREUR : {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
