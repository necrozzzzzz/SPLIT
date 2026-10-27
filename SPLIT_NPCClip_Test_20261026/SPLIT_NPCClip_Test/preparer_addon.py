"""Diagnostic sandbox addon. Game files are opened read-only during preparation.
Remap npcclip to nodraw in one exact known map resource. Preserve geometry.
"""
import argparse
import hashlib
import json
from pathlib import Path
import struct
import sys
import zlib
from extract_midtown_collisions import DEFAULT_VPK, read_directory, archive_location, exact

RESOURCE = "maps/dl_midtown/world_physics.vmdl_c"
PROFILES = {
    "7a975ba6582c6e0c495ef08bd8a6a2023b585a334d7466cfc24cdf4ce001b4bc": {
        "capture": "2026-10-21", "hash_positions": [2188,2248], "string_lane_size": 1659,
    },
    "59c44b944e5e89f39aa28094ec27313fb300e8749e4b8c76938b3d532c36ff52": {
        "capture": "2026-10-26", "hash_positions": [2188,2256], "string_lane_size": 1659,
    },
}


def murmur_hash(text):
    data = text.encode("ascii")
    h, m = 0x31415926 ^ len(data), 0x5BD1E995
    end = len(data)//4*4
    for offset in range(0,end,4):
        k = struct.unpack_from("<I",data,offset)[0]*m & 0xFFFFFFFF
        k ^= k >> 24
        h = (h*m & 0xFFFFFFFF) ^ (k*m & 0xFFFFFFFF)
    for i,v in enumerate(data[end:]):
        h ^= v << (8*i)
    if end != len(data):
        h = h*m & 0xFFFFFFFF
    h ^= h >> 13
    h = h*m & 0xFFFFFFFF
    return h ^ (h >> 15)


def blocks(data):
    size, hv, rv, relative, count = struct.unpack_from("<IHHII",data)
    if (size,hv,rv,relative,count) != (len(data),12,1,8,4):
        raise ValueError("Disposition de modele inattendue.")
    result=[]
    for i in range(count):
        pos=16+i*12
        tag,offset,length=struct.unpack_from("<4sII",data,pos)
        start=pos+4+offset
        if start < 64 or start+length>len(data):
            raise ValueError("Bloc de modele hors limites.")
        result.append((tag,bytes(data[start:start+length])))
    return result


def patch_resource(original):
    try:
        import zstandard
    except ImportError:
        raise RuntimeError("Module manquant : py -m pip install zstandard") from None
    source_sha=hashlib.sha256(original).hexdigest()
    profile=PROFILES.get(source_sha)
    if profile is None:
        raise ValueError("La carte a change depuis l'analyse. Arret : aucun addon produit pour cette version.")
    source_blocks=blocks(original)
    if [t for t,p in source_blocks]!=[b"PHYS",b"CTRL",b"RED2",b"DATA"]:
        raise ValueError("Ordre de blocs inattendu.")
    phys=source_blocks[0][1]
    if struct.unpack_from("<I",phys)[0]!=0x4B563305 or struct.unpack_from("<I",phys,20)[0]!=2:
        raise ValueError("Format KV3 v5 Zstandard attendu.")
    u1,c1,u2,c2=struct.unpack_from("<4i",phys,72)
    count1=struct.unpack_from("<i",phys,28)[0]
    decoder=zstandard.ZstdDecompressor()
    lane=decoder.decompress(phys[120:120+c1],max_output_size=u1)
    if len(lane)!=u1 or count1!=profile['string_lane_size']:
        raise ValueError("Disposition KV3 inattendue.")
    strings=lane[:count1]
    if strings.count(b"npcclip\0")!=1 or b"nodraw\0" in strings:
        raise ValueError("Noms de collision inattendus.")
    reduced=strings.replace(b"npcclip\0",b"nodraw\0")
    if (len(reduced)+3)//4 != (count1+3)//4:
        raise ValueError("Alignement de lane modifie.")
    # One additional alignment byte keeps the integer lanes at the same offset.
    modified=bytearray(reduced+b"\0"+lane[count1:])
    old_hash=struct.pack("<I",murmur_hash("npcclip"))
    new_hash=struct.pack("<I",murmur_hash("nodraw"))
    import re
    positions=[m.start() for m in re.finditer(re.escape(old_hash),lane)]
    if positions != profile['hash_positions']:
        raise ValueError("Les deux attributs de collision ne correspondent pas a la carte analysee.")
    for pos in positions:
        if modified[pos:pos+4]!=old_hash:
            raise ValueError("Decalage des attributs de collision.")
        modified[pos:pos+4]=new_hash
    modified=bytes(modified)
    assert len(modified)==len(lane)
    assert modified[max(positions)+4:]==lane[max(positions)+4:]
    compressed=zstandard.ZstdCompressor(level=3).compress(modified)
    header=bytearray(phys[:120])
    struct.pack_into("<i",header,28,count1-1)
    struct.pack_into("<i",header,52,struct.unpack_from("<i",phys,52)[0]+len(compressed)-c1)
    struct.pack_into("<i",header,76,len(compressed))
    replacement=bytes(header)+compressed+phys[120+c1:]
    assert decoder.decompress(replacement[120:120+len(compressed)],max_output_size=u1)==modified
    assert replacement[120+len(compressed):]==phys[120+c1:]
    new_blocks=[(b"PHYS",replacement)]+source_blocks[1:]
    output=bytearray(original[:64])
    for i,(tag,payload) in enumerate(new_blocks):
        output.extend(b"\0"*(-len(output)%16))
        start=len(output);output.extend(payload)
        struct.pack_into("<4sII",output,16+i*12,tag,start-(20+i*12),len(payload))
    struct.pack_into("<I",output,0,len(output))
    assert blocks(output)[1:]==source_blocks[1:]
    report={"sourceSha256":source_sha,"validatedCapture":profile['capture'],"patchedSha256":hashlib.sha256(output).hexdigest(),
            "tagChange":"npcclip -> nodraw","collisionAttributeIndices":[6,11],
            "geometryBlobAndSecondBufferByteIdentical":True,"otherResourceBlocksByteIdentical":True,
            "scope":"Attribute 6 retains playerclip; attribute 11 was npcclip only.",
            "runtimeValidated":False,"addonMountingValidated":False}
    return bytes(output),report


def write_addon(path,resource):
    tree=b"vmdl_c\0maps/dl_midtown\0world_physics\0"
    tree+=struct.pack("<IHHIIH",zlib.crc32(resource),0,0x7FFF,0,len(resource),0xFFFF)+b"\0\0\0"
    with path.open("xb") as destination:
        destination.write(struct.pack("<III",0x55AA1234,1,len(tree)))
        destination.write(tree);destination.write(resource)
    version,offset,entries=read_directory(path)
    assert version==1 and len(entries)==1 and entries[0]["path"]==RESOURCE
    with path.open("rb") as source:
        source.seek(offset)
        assert zlib.crc32(exact(source,len(resource)))==entries[0]["crc"]


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("vpk",nargs="?",type=Path,default=DEFAULT_VPK)
    args=parser.parse_args();folder=Path(__file__).resolve().parent
    out=folder/"SPLIT_NPCClip_Test.vpk"
    if out.exists():
        raise FileExistsError(f"Addon deja prepare : {out}. Il peut etre utilise tel quel.")
    version,base,entries=read_directory(args.vpk)
    matches=[e for e in entries if e["path"]==RESOURCE]
    if len(matches)!=1:
        raise ValueError("Ressource physique attendue absente ou dupliquee.")
    entry=matches[0];archive,offset=archive_location(args.vpk,entry,base)
    with archive.open("rb") as source:
        source.seek(offset);original=entry["preload"]+exact(source,entry["length"])
    if zlib.crc32(original)!=entry["crc"]:
        raise ValueError("CRC du modele original incorrect.")
    modified,report=patch_resource(original)
    try:
        write_addon(out,modified)
    except Exception:
        out.unlink(missing_ok=True);raise
    report["addonSha256"]=hashlib.sha256(out.read_bytes()).hexdigest()
    (folder/"VALIDATION.json").write_text(json.dumps(report,indent=2),encoding="utf-8")
    print(f"OK : addon cree, CRC verifie : {out}")
    print("Ferme Deadlock puis lance addon.py installer.")


if __name__=="__main__":
    try: main()
    except Exception as error:
        print(f"ERREUR : {error}",file=sys.stderr);raise SystemExit(1)
