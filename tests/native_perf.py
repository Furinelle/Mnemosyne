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
if a.samples < 5:
    p.error('--samples must be at least 5 for phase attribution')
binary = a.binary.resolve()
rows = []
PHASES = ('file_enumeration', 'sqlite_sync', 'candidate_read', 'graph_expansion',
          'model_profile', 'model_fingerprint', 'model_initialize', 'context_render',
          'hook_total')

def timed(args, cwd, env, payload=''):
    start = time.perf_counter_ns()
    r = subprocess.run([str(binary), *args], cwd=cwd, env=env, input=payload,
                       text=True, capture_output=True)
    elapsed = (time.perf_counter_ns()-start)/1e6
    if r.returncode: raise RuntimeError(r.stderr)
    if args[0] in ('search', 'hook'):
        json.loads(r.stdout)
    events = []
    for line in r.stderr.splitlines():
        try:
            event = json.loads(line)
        except json.JSONDecodeError:
            continue
        if event.get('event') == 'mnemosyne_timing':
            events.append(event)
    phases = {}
    for phase in PHASES:
        measured = [e['duration_ms'] for e in events
                    if e['phase'] == phase and e['status'] == 'measured']
        statuses = [e['status'] for e in events if e['phase'] == phase]
        phases[phase] = {
            'status': 'measured' if measured else ('not_run' if 'not_run' in statuses else 'absent'),
            'inclusive_ms': sum(measured) if measured else None,
        }
    return elapsed, phases

with tempfile.TemporaryDirectory(prefix='mnemosyne-perf-') as directory:
    root = Path(directory)
    env = {'HOME':str(root/'home'), 'MNEMOSYNE_HOME':str(root/'global'), 'PATH':'',
           'MNEMOSYNE_TRACE_TIMING':'1'}
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
            samples=[]; phase_samples={phase:[] for phase in PHASES}; phase_statuses={phase:[] for phase in PHASES}
            for sample in range(a.samples):
                if scenario=='cold_index':
                    for cache in store.glob('rust-index.sqlite*'): cache.unlink()
                if scenario=='single_edit':
                    f=store/'working'/'00000000.md'
                    s=f.read_text(); f.write_text(s.replace('port 8000','port 8001') if 'port 8000' in s else s.replace('port 8001','port 8000'))
                args=['--version'] if scenario=='startup' else query
                payload=''
                if scenario=='file_hook':
                    args=['hook','PreToolUse']
                    payload=json.dumps({'session_id':f'perf-{size}-{sample}', 'host':'claude-code',
                                        'tool_name':'Edit', 'tool_input':{'file_path':'backend/config.rs'}})
                elapsed, phases = timed(args,project,env,payload)
                samples.append(elapsed)
                for phase, observation in phases.items():
                    phase_samples[phase].append(observation['inclusive_ms'])
                    phase_statuses[phase].append(observation['status'])
            ordered=sorted(samples)
            rows.append({'records':size,'scenario':scenario,'samples_ms':samples,
                         'p50_ms':statistics.median(samples),'p95_ms':ordered[min(len(ordered)-1, int(len(ordered)*.95))],
                         'phase_inclusive_samples_ms':phase_samples,
                         'phase_statuses':phase_statuses,
                         'corpus_sha256':corpus.hexdigest()})
a.output.parent.mkdir(parents=True,exist_ok=True)
a.output.write_text(json.dumps({'commit':a.commit,'binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest(),
 'build':'release; onnx feature compiled, models disabled','platform':platform.platform(),
 'machine':platform.machine(),'processor':platform.processor(),'samples':a.samples,'clock':'perf_counter_ns',
 'models':'disabled; no assets read','effective_pipeline':'lexical; no network',
 'limitations':['phase timings are inclusive and overlapping; do not sum them','pending recovery measured by correctness tests only','no real-host/model E2E or lane ablation'], 'results':rows},indent=2)+'\n')
print(a.output)
