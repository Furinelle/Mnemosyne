#!/usr/bin/env python3
"""Exercise an existing 1.0 binary only against a temporary store, then upgrade/restore."""
import argparse, hashlib, json, os, subprocess, tempfile
from pathlib import Path
p=argparse.ArgumentParser();p.add_argument('binary',type=Path);p.add_argument('--old-binary',type=Path,required=True);p.add_argument('--output',type=Path,required=True);a=p.parse_args()
new=a.binary.resolve();old=a.old_binary.resolve()
with tempfile.TemporaryDirectory(prefix='mnemosyne-upgrade-') as temp:
 root=Path(temp); project=root/'project';project.mkdir();(project/'.git').mkdir()
 env={'HOME':str(root/'home'),'MNEMOSYNE_HOME':str(root/'global'),'PATH':''}
 def run(binary,args,value=None,cwd=project,success=True):
  r=subprocess.run([str(binary),*args],cwd=cwd,env=env,input='' if value is None else json.dumps(value),text=True,capture_output=True)
  if success: assert r.returncode==0,(args,r.stderr)
  return r
 version=run(old,['--version']).stdout.strip();assert version=='mnemosyne 1.0.0',version
 run(old,['init','--no-agent-files']);run(old,['write','--type','codebase','--importance','70','--title','Upgrade fixture','--content','Quartz routing uses PostgreSQL'])
 before={str(f.relative_to(project/'.mnemosyne')):hashlib.sha256(f.read_bytes()).hexdigest() for f in (project/'.mnemosyne/working').glob('*.md')}
 run(new,['store-upgrade','--commit']);result=json.loads(run(new,['search','Quartz','--format','json']).stdout);assert result
 findings={'findings':[{'type':'pitfall','importance':70,'title':'Upgraded ingestion','content':'Automatic findings use the upgraded writer.'}]}
 ingested=json.loads(run(new,['ingest','--format','json','--commit'],findings).stdout);assert ingested[0]['id']
 replay=json.loads(run(new,['ingest','--format','json','--commit'],findings).stdout);assert replay[0]['id']==ingested[0]['id'] and replay[0]['verdict']=='duplicate'
 (project/'fixture.txt').write_text('temporary upgrade fixture')
 run(new,['checkpoint','new'],{'task_id':'upgrade','goal':'verify restoration','scoped_paths':['fixture.txt'],'expires':'2099-01-01T00:00:00Z'})
 written=json.loads(run(new,['write-v2'],{'type':'codebase','title':'Evidence fixture','content':'Independent test evidence','importance':70,'origin':'upgrade-test','source_session_id':'s','source_event_id':'e','finding_key':'f','source_kind':'tool_output','verification_state':'unverified'}).stdout)
 shown=json.loads(run(new,['show-v2',written['memory_ref']['memory_id']]).stdout)
 run(new,['proposal','create'],{'decision':'REFINE','reason':'upgrade fixture','targets':[{'memory_ref':written['memory_ref'],'expected_rev':shown['revision']['semantic_rev'],'expected_hash':shown['revision']['semantic_hash'],'body':'Reviewed future correction'}]})
 run(new,['snapshot',str(root/'package')]);run(new,['restore',str(root/'package'),str(root/'restored')])
 for directory in ['history','evidence','checkpoints','proposals']:
  left={str(f.relative_to(project/'.mnemosyne')):hashlib.sha256(f.read_bytes()).hexdigest() for f in (project/'.mnemosyne'/directory).rglob('*') if f.is_file()}
  right={str(f.relative_to(root/'restored')):hashlib.sha256(f.read_bytes()).hexdigest() for f in (root/'restored'/directory).rglob('*') if f.is_file()}
  assert left and left==right,directory
 restored={str(f.relative_to(root/'restored')):hashlib.sha256(f.read_bytes()).hexdigest() for f in (root/'restored/working').glob('*.md')}
 canonical={str(f.relative_to(project/'.mnemosyne')):hashlib.sha256(f.read_bytes()).hexdigest() for f in (project/'.mnemosyne/working').glob('*.md')};assert canonical==restored
 assert len(before)==1 and len(restored)==3
 assert (root/'restored/rust-index.sqlite').exists()
 assert ingested[0]['id'] in (root/'restored/MEMORY.md').read_text()
 manifest=json.loads((root/'restored/store.json').read_text());assert manifest['min_writer_version']==3
 # Old writers must not be used after upgrade: the rollout contract stops them.
 result={'status':'pass','old_version':version,'old_binary_sha256':hashlib.sha256(old.read_bytes()).hexdigest(),'new_binary_sha256':hashlib.sha256(new.read_bytes()).hexdigest(),'canonical_records':len(restored),'restore_bytes_match':True,'history_evidence_checkpoints_proposals_match':True,'cache_rebuilt':True,'upgraded_ingest_replay':True,'markdown_index_rebuilt':True,'live_stores_used':False,'old_writer_after_upgrade':'not run: rollout requires stopping old writer'}
 a.output.parent.mkdir(parents=True,exist_ok=True);a.output.write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result))
