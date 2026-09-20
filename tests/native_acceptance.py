#!/usr/bin/env python3
"""Repeatable offline native evolution checks; runtime state and endpoints are isolated."""
import argparse, hashlib, json, os, subprocess, tarfile, tempfile
from pathlib import Path

p=argparse.ArgumentParser();p.add_argument('--output',type=Path,required=True);p.add_argument('--old-binary',type=Path);a=p.parse_args()
root=Path(__file__).resolve().parent.parent;output=a.output.resolve();output.mkdir(parents=True,exist_ok=True)
home=Path.home();base_env={k:v for k,v in os.environ.items() if k in ('PATH','TMPDIR','SDKROOT','DEVELOPER_DIR')}
base_env.update(CARGO_HOME=os.environ.get('CARGO_HOME',str(home/'.cargo')),RUSTUP_HOME=os.environ.get('RUSTUP_HOME',str(home/'.rustup')))
results=[]
with tempfile.TemporaryDirectory(prefix='mnemosyne-acceptance-') as temp:
    temp=Path(temp);env=dict(base_env,HOME=str(temp/'home'),MNEMOSYNE_HOME=str(temp/'global'))
    def run(name,command,cwd=root,extra=None):
        current=dict(env);current.update(extra or {})
        r=subprocess.run(command,cwd=cwd,env=current,capture_output=True,text=True)
        (output/(name+'.txt')).write_text(r.stdout+r.stderr+'\nEXIT_CODE='+str(r.returncode)+'\n')
        results.append({'name':name,'command':command,'exit_code':r.returncode})
        print(name,r.returncode,flush=True)
        (output/'results.json').write_text(json.dumps(results,indent=2)+'\n')
        if r.returncode: raise SystemExit(r.returncode)
    run('fmt',['cargo','fmt','--all','--','--check'])
    run('clippy',['cargo','clippy','--offline','--locked','--all-targets','--all-features','--','-D','warnings'])
    run('tests',['cargo','test','--offline','--locked','--all-targets'])
    run('all-features-tests',['cargo','test','--offline','--locked','--all-targets','--all-features'])
    binary=str(root/'target/debug/mnemosyne')
    run('release-build',['cargo','build','--offline','--locked','--release','--features','onnx'])
    run('transport',['python3','tests/native_transport_smoke.py',binary])
    run('http-models',['python3','tests/native_http_models_smoke.py',binary])
    run('eval-matrix',['python3','tests/native_eval_matrix.py',binary,'--skip-model-smoke','--output',str(output/'eval-matrix.json')])
    if a.old_binary:
        run('upgrade-restore',['python3','tests/native_upgrade_smoke.py',binary,'--old-binary',str(a.old_binary.resolve()),'--output',str(output/'upgrade-restore.json')])
    for name,options in [('bm25',[]),('full',['--pipeline','full']),('longmemeval-sample',['--longmemeval','--pipeline','full'])]:
        run('eval-'+name,[binary,'eval','run',*options,'--min-recall','0.95'],cwd=temp)
    run('package-list',['cargo','package','--offline','--list','--allow-dirty'])
    listing=(output/'package-list.txt').read_text().splitlines()
    for path in listing:
        assert not any(part in ('.mnemosyne','.venv','.pytest_cache','credentials','target') for part in Path(path).parts),path
    run('package',['cargo','package','--offline','--locked','--allow-dirty','--no-verify'])
    package=root/'target/package/mnemosyne-1.0.0.crate'
    unpack=temp/'package';unpack.mkdir()
    with tarfile.open(package) as archive:archive.extractall(unpack,filter='data')
    run('package-offline-build',['cargo','build','--offline','--locked'],cwd=unpack/'mnemosyne-1.0.0',extra={'CARGO_TARGET_DIR':str(root/'target/package-check')})
    (output/'source-sha256.txt').write_text(hashlib.sha256(package.read_bytes()).hexdigest()+'  mnemosyne-1.0.0.crate\n')
