#!/usr/bin/env python3
"""Offline synthetic process-level baseline; no real stores or models."""
import argparse, hashlib, json, os, platform, statistics, subprocess, tempfile, time
from pathlib import Path

p = argparse.ArgumentParser()
p.add_argument('binary', type=Path)
p.add_argument('--output', type=Path, required=True)
p.add_argument('--commit', required=True)
p.add_argument('--samples', type=int, default=5)
a = p.parse_args()
binary = a.binary.resolve()
rows = []
def timed(args, cwd, env, payload=''):
    start = time.perf_counter_ns()
    r = subprocess.run([str(binary), *args], cwd=cwd, env=env, input=payload,
                       text=True, capture_output=True)
    elapsed = (time.perf_counter_ns()-start)/1e6
    if r.returncode: raise RuntimeError(r.stderr)
    return elapsed

with tempfile.TemporaryDirectory(prefix='mnemosyne-perf-') as directory:
    root = Path(directory)
    env = {'HOME':str(root/'home'), 'MNEMOSYNE_HOME':str(root/'global'), 'PATH':''}
    for size in [100,1000,10000]:
        project = root/str(size); project.mkdir(); (project/'.git').mkdir()
        timed(['init','--no-agent-files'],project,env)
        store=project/'.mnemosyne'; corpus=hashlib.sha256()
        for i in range(size):
            text = f'''---
id: codebase-2026-01-01-{i:08x}
type: codebase
source: synthetic
strength: 70
created: 2026-01-01
last_accessed: 2026-01-01
access_count: 0
tags: [service]
links: []
status: active
---
## Service {i}
Service {i} uses port {8000+i%1000} and backend/config.rs healthcheck /health.
'''
            corpus.update(text.encode()); (store/'working'/f'{i:08x}.md').write_text(text)
        query=['search','healthcheck','--format','json']
        for scenario in ['startup','cold_index','warm_query','single_edit','file_hook']:
            samples=[]
            for sample in range(a.samples):
                if scenario=='cold_index':
                    for cache in store.glob('rust-index.sqlite*'): cache.unlink()
                if scenario=='single_edit':
                    f=store/'working'/'00000000.md'
                    s=f.read_text(); f.write_text(s.replace('port 8000','port 8001') if 'port 8000' in s else s.replace('port 8001','port 8000'))
                args=['--version'] if scenario=='startup' else query
                payload=''
                if scenario=='file_hook':
                    args=['inject','--event','file_touch','--format','json']
                    payload=json.dumps({'files':['backend/config.rs']})
                samples.append(timed(args,project,env,payload))
            ordered=sorted(samples)
            rows.append({'records':size,'scenario':scenario,'samples_ms':samples,
                         'p50_ms':statistics.median(samples),'p95_ms':ordered[min(len(ordered)-1, int(len(ordered)*.95))],
                         'corpus_sha256':corpus.hexdigest()})
a.output.parent.mkdir(parents=True,exist_ok=True)
a.output.write_text(json.dumps({'commit':a.commit,'binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),
 'build':'release; onnx feature compiled, models disabled','platform':platform.platform(),
 'machine':platform.machine(),'processor':platform.processor(),'samples':a.samples,'clock':'perf_counter_ns',
 'models':'disabled; no assets read','effective_pipeline':'lexical; no network',
 'limitations':['process-level totals, not internal phase attribution','pending recovery measured by correctness tests only','no real-host/model E2E or lane ablation'], 'results':rows},indent=2)+'\n')
print(a.output)
