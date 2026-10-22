#!/usr/bin/env python3
import pathlib,tarfile,hashlib,os,json,stat
from fixture_paths import fixture_root
ROOT=fixture_root();COMMIT='de0c9b28f22281a0251d7e98aa8a097895a5b185';PREFIX='coturn-'+COMMIT
assert os.geteuid()==0 and not ROOT.is_symlink() and ROOT.stat().st_uid==0 and stat.S_IMODE(ROOT.stat().st_mode)==0o700
archive=ROOT/'official-source.tar.gz';m=archive.lstat();assert stat.S_ISREG(m.st_mode) and m.st_uid==0 and m.st_nlink==1 and stat.S_IMODE(m.st_mode)==0o600
assert hashlib.sha256(archive.read_bytes()).hexdigest()=='3d6fa86bb713379e735342ef640b93f64272b79af0318ad70a6a871cff0ccc93'
dest=ROOT/'source';dest.mkdir(mode=0o700);allowed=[];omitted=[]
with tarfile.open(archive,'r:gz') as t:
 for member in t.getmembers():
  p=pathlib.PurePosixPath(member.name);assert not p.is_absolute() and '..' not in p.parts and (member.name==PREFIX or member.name.startswith(PREFIX+'/'))
  if member.issym():omitted.append({'name':member.name,'type':'symlink'});continue
  assert member.isdir() or member.isfile();assert not(member.islnk() or member.isdev() or member.isfifo());allowed.append(member)
 t.extractall(dest,members=allowed,filter='data')
expected={'src/apps/relay/prom_server.c':'606e4d956ae912c4b589115530391813b1581f579f6cf1a05fb3e8469e5a26b5','src/prometheus/prom.c':'4026e55301150263df36ac6bd548ebfb2e3d4bd162d2154c0fa318ae3f6a4677'}
for rel,digest in expected.items():assert hashlib.sha256((dest/PREFIX/rel).read_bytes()).hexdigest()==digest
print(json.dumps({'source_commit':COMMIT,'extracted':len(allowed),'omitted_symlinks':omitted,'core_pins_verified':True}))
