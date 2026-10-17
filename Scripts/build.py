from pathlib import Path
import subprocess,os,json,hashlib,tarfile,io,gzip
R=Path(__file__).resolve().parent.parent
subprocess.run(['npm.cmd' if os.name=='nt' else 'npm','run','build'],cwd=R/'frontend',check=True)
subprocess.run(['node','--check',str(R/'web/app.js')],check=True)
subprocess.run(['node','--test',str(R/'Scripts/ui-contract.test.mjs')],check=True)
subprocess.run(['cargo','test','--manifest-path',str(R/'backend/Cargo.toml'),'--locked'],check=True)
subprocess.run(['cargo','build','--manifest-path',str(R/'backend/Cargo.toml'),'--locked'],check=True)
subprocess.run(['python',str(R/'Scripts/ipc-test.py')],check=True)
root=Path(subprocess.check_output(['rustc','--print','sysroot'],text=True).strip());host=next(x.split(': ',1)[1] for x in subprocess.check_output(['rustc','-vV'],text=True).splitlines() if x.startswith('host: '));env=os.environ.copy();env['CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER']=str(root/'lib/rustlib'/host/'bin'/('rust-lld.exe' if os.name=='nt' else 'rust-lld'))
subprocess.run(['cargo','build','--manifest-path',str(R/'backend/Cargo.toml'),'--target','aarch64-unknown-linux-musl','--release','--locked'],env=env,check=True)
normalizer=R/'backend/target/debug'/('cpr-account-dialogue.exe' if os.name=='nt' else 'cpr-account-dialogue');manifest=json.loads(subprocess.check_output([str(normalizer),'--print-manifest']))
files={'bin/plugin':R/'backend/target/aarch64-unknown-linux-musl/release/cpr-account-dialogue',**{p:R/p for p in manifest['resources']}}
manifest['package']={'protocolVersion':4,'target':{'os':'linux','architecture':'aarch64'},'files':{n:hashlib.sha256(f.read_bytes()).hexdigest() for n,f in sorted(files.items())}}
name=f"donzy.account-dialogue-{manifest['version']}-aarch64-unknown-linux-gnu.tar.gz";out=R/'dist'/name
contents={'plugin.json':json.dumps(manifest,ensure_ascii=False,indent=2).encode(),**{n:f.read_bytes() for n,f in files.items()}}
with out.open('wb') as dst:
 with gzip.GzipFile(filename='',mode='wb',fileobj=dst,mtime=0) as gz:
  with tarfile.open(fileobj=gz,mode='w') as tar:
   for path,data in sorted(contents.items()):
    info=tarfile.TarInfo(path);info.size=len(data);info.mode=0o755 if path=='bin/plugin' else 0o644;info.mtime=0;tar.addfile(info,io.BytesIO(data))
sha=hashlib.sha256(out.read_bytes()).hexdigest();Path(str(out)+'.sha256').write_text(sha+'  '+name+'\n',encoding='ascii');print(out);print(sha)
