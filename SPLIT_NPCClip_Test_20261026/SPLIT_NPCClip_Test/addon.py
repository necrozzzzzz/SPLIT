"""Install/remove only the diagnostic addon owned by this package."""
import argparse
import csv
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys

DEFAULT_GAME = Path(r'C:\Program Files (x86)\Steam\steamapps\common\Deadlock\game\citadel')


def sha(path):
    digest=hashlib.sha256()
    with path.open('rb') as source:
        while chunk:=source.read(1048576):digest.update(chunk)
    return digest.hexdigest()


def require_game_closed():
    if os.name!='nt':return
    for name in ['deadlock.exe','citadel.exe']:
        result=subprocess.run(['tasklist','/FI',f'IMAGENAME eq {name}','/FO','CSV','/NH'],capture_output=True,text=True,encoding='utf-8',errors='replace',check=True)
        for row in csv.reader(result.stdout.splitlines()):
            if row and row[0].strip().lower()==name:
                raise RuntimeError('Ferme Deadlock avant cette operation, puis relance la commande.')


def state_file(folder):return folder/'installation.json'


def read_state(folder,game):
    state=json.loads(state_file(folder).read_text(encoding='utf-8'))
    target=Path(state['path']).resolve()
    if target.parent!=(game/'addons').resolve() or not re.fullmatch(r'pak\d{2}_dir\.vpk',target.name):
        raise ValueError('Chemin de retrait inattendu : aucun fichier supprime.')
    if target.exists() and sha(target)!=state['sha256']:
        raise ValueError('Le fichier installe a change. Aucun remplacement ni retrait automatique.')
    return state,target


def install(folder,game):
    require_game_closed()
    if state_file(folder).exists():raise ValueError('Installation deja enregistree : utilise le mode statut ou retirer.')
    source=folder/'SPLIT_NPCClip_Test.vpk'
    report=folder/'VALIDATION.json'
    if not source.is_file() or not report.is_file():raise ValueError('Lance preparer_addon.py avant installation.')
    source_hash=sha(source)
    if source_hash!=json.loads(report.read_text(encoding='utf-8'))['addonSha256']:
        raise ValueError('Le VPK ne correspond pas au rapport de validation.')
    info=(game/'gameinfo.gi').read_text(encoding='utf-8-sig')
    active=[line.split('//',1)[0] for line in info.splitlines()]
    if not any(re.fullmatch(r'\s*Game\s+"?citadel[/\\]addons(?:[/\\]\*)?"?\s*',line,re.I) for line in active):
        raise ValueError('Chargement citadel/addons desactive ou non reconnu dans gameinfo.gi. Aucun addon installe. Envoie ce message pour adapter le montage.')
    directory=game/'addons';directory.mkdir(exist_ok=True)
    target=None
    for number in range(99,69,-1):
        candidate=directory/f'pak{number:02d}_dir.vpk'
        try:
            destination=candidate.open('xb')
        except FileExistsError:continue
        target=candidate
        manifest_created=False
        try:
            with destination,source.open('rb') as source_stream:
                shutil.copyfileobj(source_stream,destination,1048576)
            if sha(target)!=source_hash:raise ValueError('Verification de la copie echouee.')
            state={'path':str(target.resolve()),'sha256':source_hash,'purpose':'SPLIT NPCClip sandbox diagnostic'}
            with state_file(folder).open('x',encoding='utf-8') as manifest:
                manifest_created=True
                json.dump(state,manifest,indent=2)
        except BaseException:
            target.unlink(missing_ok=True)
            if manifest_created:state_file(folder).unlink(missing_ok=True)
            raise
        break
    if target is None:raise ValueError('Aucun emplacement libre entre pak70 et pak99. Aucun addon remplace.')
    print(f'OK : addon installe dans {target}')
    print('Relance Deadlock et teste en sandbox avec le meme BOTREC.')
    print('Apres le test : ferme le jeu puis lance addon.py retirer.')
    return target


def remove(folder,game):
    require_game_closed()
    if not state_file(folder).exists():
        print('Aucune installation enregistree dans ce paquet.');return
    state,target=read_state(folder,game)
    target.unlink(missing_ok=True);state_file(folder).unlink()
    print('Addon de diagnostic retire. Relance Deadlock pour retrouver le chargement normal.')


def status(folder,game):
    if not state_file(folder).exists():print('Addon non installe par ce paquet.');return
    state,target=read_state(folder,game)
    print(f'Addon present : {target}' if target.exists() else 'Le fichier addon est absent du jeu.')
    print('La presence du fichier ne prouve pas encore son chargement par la carte.')


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('mode',choices=['installer','retirer','statut'])
    parser.add_argument('--jeu',type=Path,default=DEFAULT_GAME)
    args=parser.parse_args();folder=Path(__file__).resolve().parent
    {'installer':install,'retirer':remove,'statut':status}[args.mode](folder,args.jeu)


if __name__=='__main__':
    try:main()
    except Exception as error:
        print(f'ERREUR : {error}',file=sys.stderr);raise SystemExit(1)
