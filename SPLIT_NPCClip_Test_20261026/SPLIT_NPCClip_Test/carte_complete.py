"""Diagnostic local : copie complete dans addons/maps, original jamais modifie."""
import argparse, collections, hashlib, json, os, shutil, struct, sys, zlib
from pathlib import Path, PurePosixPath
from extract_midtown_collisions import read_directory, exact, DEFAULT_VPK
from preparer_addon import patch_resource, RESOURCE
from addon import require_game_closed

def sha(p):
    h=hashlib.sha256()
    with p.open('rb') as f:
        for b in iter(lambda:f.read(4*1024*1024),b''):h.update(b)
    return h.hexdigest()

def tree_bytes(entries):
    groups=collections.defaultdict(lambda:collections.defaultdict(list))
    for e in entries:
        p=PurePosixPath(e['path']); ext=e['extension']; name=p.name[:-len(ext)-1] if ext!=' ' else p.name
        groups[ext][str(p.parent) if str(p.parent)!='.' else ' '].append((name,e))
    out=bytearray()
    def s(v):out.extend(v.encode()+b'\0')
    for ext,folders in groups.items():
        s(ext)
        for folder,items in folders.items():
            s(folder)
            for name,e in items:
                s(name);out.extend(struct.pack('<IHHIIH',e['crc'],len(e['preload']),e['archive'],e['offset'],e['length'],65535));out.extend(e['preload'])
            s('')
        s('')
    s('');return out

def build(source,out,patch=patch_resource):
    _,base,entries=read_directory(source)
    if any(e['archive']!=32767 for e in entries):raise ValueError('Archive externe : format non pris en charge.')
    selected=[e for e in entries if e['path']==RESOURCE]
    if len(selected)!=1:raise ValueError('Ressource physique absente ou dupliquee.')
    e=selected[0]
    with source.open('rb') as f:
        f.seek(base+e['offset']);original=e['preload']+exact(f,e['length'])
    if zlib.crc32(original)!=e['crc']:raise ValueError('CRC original invalide.')
    modified,report=patch(original)
    tail_size=source.stat().st_size-base
    new=[dict(x) for x in entries]
    new[entries.index(e)].update(preload=b'',offset=tail_size,length=len(modified),crc=zlib.crc32(modified))
    tree=tree_bytes(new)
    with out.open('xb') as dest:
        dest.write(struct.pack('<III',0x55aa1234,1,len(tree)));dest.write(tree)
        with source.open('rb') as f:
            f.seek(base);shutil.copyfileobj(f,dest,4*1024*1024)
        dest.write(modified);dest.flush();os.fsync(dest.fileno())
    _,newbase,check=read_directory(out)
    if {x['path']:x for x in check}!={x['path']:x for x in new}:raise ValueError('Index reconstruit incorrect.')
    with source.open('rb') as f,out.open('rb') as g:
        f.seek(base);g.seek(newbase);remaining=tail_size
        while remaining:
            n=min(4*1024*1024,remaining)
            if exact(f,n)!=exact(g,n):raise ValueError('Les donnees originales ont change.')
            remaining-=n
        if exact(g,len(modified))!=modified:raise ValueError('Physique modifiee incorrecte.')
    return report

def main():
    ap=argparse.ArgumentParser(description=__doc__);ap.add_argument('mode',choices=['installer','retirer']);ap.add_argument('--jeu',type=Path,default=DEFAULT_VPK.parent.parent);a=ap.parse_args()
    require_game_closed();game=a.jeu.resolve();target=game/'addons/maps/dl_midtown.vpk';state=Path(__file__).resolve().parent/'installation_carte_complete.json'
    if a.mode=='retirer':
        if not state.exists():raise ValueError('Aucune installation de carte enregistree.')
        data=json.loads(state.read_text())
        if Path(data['path']).resolve()!=target or not target.is_file() or sha(target)!=data['sha256']:raise ValueError('Copie absente ou modifiee : aucun retrait automatique.')
        target.unlink();state.unlink();print('OK : copie de diagnostic retiree ; carte originale inchangee.');return
    if state.exists() or target.exists():raise ValueError('Une copie existe deja : aucun fichier remplace.')
    info=(game/'gameinfo.gi').read_text(encoding='utf-8-sig')
    import re
    active='\n'.join(line.split('//',1)[0] for line in info.splitlines())
    if not re.search(r'(?im)^\s*Game\s+"?citadel[/\\]addons"?\s*$',active):raise ValueError('Chemin addons absent.')
    source=game/'maps/dl_midtown.vpk';target.parent.mkdir(parents=True,exist_ok=True)
    if shutil.disk_usage(target.parent).free<2*source.stat().st_size+400*1024*1024:raise ValueError('Espace libre insuffisant (environ 2,6 Go necessaire).')
    tmp=target.with_name('dl_midtown.vpk.split-preparation')
    if tmp.exists():raise ValueError('Preparation precedente presente : aucun ecrasement.')
    print('Construction et verification de la copie complete...',flush=True)
    created=False
    try:
        report=build(source,tmp);digest=sha(tmp)
        # Exclusive destination avoids overwriting another mod, including a race.
        with target.open('xb') as dest:
            created=True
            with tmp.open('rb') as src:shutil.copyfileobj(src,dest,4*1024*1024)
        if sha(target)!=digest:raise ValueError('Copie installee incorrecte.')
        with state.open('x',encoding='utf-8') as f:json.dump({'path':str(target),'sha256':digest,'report':report},f,indent=2)
    except Exception:
        if created and not state.exists():target.unlink(missing_ok=True)
        raise
    finally:
        tmp.unlink(missing_ok=True)
    print('OK : copie complete installee :',target)
    print('La carte originale est inchangee. Test en sandbox uniquement, puis retirer avant les parties normales.')
    print('Retour arriere : py .\\carte_complete.py retirer')
if __name__=='__main__':
    try:main()
    except Exception as e:print('ERREUR :',e,file=sys.stderr);sys.exit(1)
