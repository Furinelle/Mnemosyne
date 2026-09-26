#!/usr/bin/env python3
"""Offline process baselines for legacy and formal V2 synthetic stores."""
import argparse
import hashlib
import json
import platform
import shutil
import statistics
import subprocess
import tempfile
import time
from pathlib import Path

PHASES = ('file_enumeration', 'sqlite_sync', 'candidate_read', 'graph_expansion',
          'model_profile', 'model_fingerprint', 'model_initialize', 'context_render', 'hook_total')
SCENARIOS = ('cache_absent', 'warm_search', 'revise_then_search', 'prep_task',
             'file_hook', 'stop_new_replay')


class Blocked(Exception):
    pass


def sha(data):
    return hashlib.sha256(data).hexdigest()


def recipe(profile, n):
    if profile == 'legacy':
        return dict(memories=n, support=0, post_revision_support=0, double_revise=0,
                    supersede=0, checkpoints=0, pending=0)
    if profile == 'v2-current':
        return dict(memories=n, support=0, post_revision_support=0, double_revise=0,
                    supersede=0, checkpoints=1, pending=0)
    return dict(memories=n, support=n // 5, post_revision_support=int(n >= 10),
                double_revise=n // 10, supersede=n // 10,
                checkpoints=min(5, max(1, n // 20)), pending=min(5, max(1, n // 20)))


def corpus_hash(project):
    """Hash canonical files; exclude SQLite caches, locks and transient host state."""
    store = project / '.mnemosyne'
    files = [store / name for name in ('store.json', 'config.toml')]
    files += [p for dirname in ('working', 'evidence', 'history', 'checkpoints', 'proposals')
              for p in (store / dirname).rglob('*') if p.is_file()]
    digest = hashlib.sha256()
    for path in sorted(p for p in files if p.is_file()):
        name, data = str(path.relative_to(store)).encode(), path.read_bytes()
        digest.update(name + b'\0' + len(data).to_bytes(8, 'big') + data)
    return digest.hexdigest()


class Runner:
    def __init__(self, binary, env, timeout, deadline):
        self.binary, self.env, self.timeout, self.deadline = binary, env, timeout, deadline

    def call(self, args, cwd, payload=None):
        remaining = self.deadline - time.monotonic()
        if remaining <= 0:
            raise Blocked('overall deadline exceeded')
        started = time.perf_counter_ns()
        try:
            out = subprocess.run([str(self.binary), *args], cwd=cwd, env=self.env,
                                 input=json.dumps(payload) if payload is not None else '',
                                 text=True, capture_output=True, timeout=min(self.timeout, remaining))
        except subprocess.TimeoutExpired as exc:
            raise Blocked(f'command timed out: {args}') from exc
        elapsed = (time.perf_counter_ns() - started) / 1e6
        # The hook CLI reports errors on stderr but still exits zero.
        if out.returncode or 'mnemosyne hook:' in out.stderr:
            raise RuntimeError(f'{args}: exit={out.returncode}: {out.stderr[:800]}')
        events = []
        for line in out.stderr.splitlines():
            try:
                event = json.loads(line)
            except json.JSONDecodeError:
                continue
            if event.get('event') == 'mnemosyne_timing':
                events.append(event)
        phases = {}
        for phase in PHASES:
            hits = [e for e in events if e.get('phase') == phase]
            measures = [e['duration_ms'] for e in hits if e.get('status') == 'measured']
            phases[phase] = dict(status='measured' if measures else
                                 ('not_run' if any(e.get('status') == 'not_run' for e in hits) else 'absent'),
                                 inclusive_ms=sum(measures) if measures else None)
        return dict(command=args, elapsed_ms=elapsed, stdout=out.stdout, phases=phases)

    def json(self, args, cwd, payload=None):
        result = self.call(args, cwd, payload)
        result['value'] = json.loads(result['stdout'])
        return result


def request(i, event='primary'):
    return dict(type='codebase', title=f'Service {i}',
                content=f'Service {i} uses port {8000 + i % 1000} and backend/config.rs healthcheck /health.',
                importance=70, tags=['service'], source='synthetic', origin=f'synthetic-{event}',
                source_session_id='perf', source_event_id=f'{i}-{event}', finding_key='service',
                source_kind='code_observation', verification_state='unverified',
                fact_key=f'perf-service-{i}')


def show(r, project, ref):
    return r.json(['show-v2', ref['memory_id'], '--store-id', ref['store_id']], project)['value']


def target(r, project, ref):
    revision = show(r, project, ref)['revision']
    return dict(memory_ref=ref, expected_rev=revision['semantic_rev'],
                expected_hash=revision['semantic_hash'])


def build(r, project, profile, n, max_disk_bytes):
    project.mkdir()
    (project / '.git').mkdir()
    (project / 'backend').mkdir()
    (project / 'backend/config.rs').write_text('pub const HEALTH: &str = "/health";\n')
    r.call(['init', '--no-agent-files'], project)
    store, spec = project / '.mnemosyne', recipe(profile, n)
    if profile == 'legacy':
        for i in range(n):
            md = (f'---\nid: codebase-2026-01-01-{i:08x}\ntype: codebase\nsource: synthetic\n'
                  'strength: 70\ncreated: 2026-01-01\nlast_accessed: 2026-01-01\naccess_count: 0\n'
                  'tags: [service]\nlinks: []\nstatus: active\n---\n'
                  f'## Service {i}\n{request(i)["content"]}\n')
            (store / 'working' / f'codebase-2026-01-01-{i:08x}.md').write_text(md)
        return spec
    manifest = r.json(['store-upgrade', '--commit'], project)['value']
    assert manifest['schema_version'] == 2 and manifest['min_writer_version'] == 3
    (store / 'config.toml').write_text("[distill]\nenabled = true\nengine = 'host'\n")
    refs = []
    for i in range(n):
        outcome = r.json(['write-v2'], project, request(i))['value']
        assert outcome['status'] == 'created' and outcome['evidence_count'] == 1
        refs.append(outcome['memory_ref'])
        if (i + 1) % 100 == 0:
            used = sum(p.stat().st_size for p in store.rglob('*') if p.is_file())
            if used > max_disk_bytes:
                raise Blocked(f'construction exceeded disk budget after {i + 1} memories')
    if profile == 'v2-evolution':
        for i in range(spec['double_revise']):
            for revision in (1, 2):
                payload = {**target(r, project, refs[i]), 'changes': {
                    'body': f'## Service {i}\n\nRevision {revision} for service {i}; backend/config.rs healthcheck /health.'}}
                outcome = r.json(['revise-v2'], project, payload)['value']
                assert outcome['revision']['semantic_rev'] == revision + 1
        if spec['post_revision_support']:
            supported = request(0, 'after-revision')
            supported['content'] = 'Revision 2 for service 0; backend/config.rs healthcheck /health.'
            outcome = r.json(['write-v2'], project, supported)['value']
            assert outcome['status'] == 'supported' and outcome['memory_ref'] == refs[0]
            assert outcome['evidence_count'] == 2
        start = 2 * spec['supersede']
        for i in range(start, start + spec['support']):
            outcome = r.json(['write-v2'], project, request(i, 'secondary'))['value']
            assert outcome['status'] == 'supported' and outcome['memory_ref'] == refs[i]
            assert outcome['evidence_count'] == 2
        for i in range(spec['supersede']):
            old = refs[spec['double_revise'] + i]['memory_id']
            new = refs[n - spec['supersede'] + i]['memory_id']
            r.call(['link', new, old, '--rel', 'supersedes'], project)
            assert show(r, project, refs[spec['double_revise'] + i])['memory']['status'] == 'superseded'
    for i in range(spec['checkpoints']):
        payload = dict(task_id=f'perf-task-{i}', goal='Check the health route',
                       scoped_paths=['backend/config.rs'], next_action=f'inspect health route {i} before restart',
                       source_session='perf', source_agent='synthetic',
                       unresolved=['health route not yet verified'], expires='2099-01-01T00:00:00Z')
        assert r.json(['checkpoint', 'new'], project, payload)['value']['state'] == 'active'
    for i in range(spec['pending']):
        ref = refs[n - 1 - i]
        payload = dict(decision='REFINE', reason=f'pending synthetic review {i}', evidence=[],
                       targets=[{**target(r, project, ref),
                                 'body': f'## Service {n - 1 - i}\n\nReviewed route {i}.'}])
        assert r.json(['proposal', 'create'], project, payload)['value']['state'] == 'pending'
    return spec


def verify(project, profile, spec):
    store = project / '.mnemosyne'
    files = sorted((store / 'working').glob('*.md'))
    assert len(files) == spec['memories']
    if profile == 'legacy':
        assert not (store / 'store.json').exists()
        assert all(f'id: {path.stem}\n' in path.read_text() for path in files)
        return dict(schema=1, memories=len(files), sources=0, revisions=0,
                    checkpoints=0, pending=0, superseded=0)
    manifest = json.loads((store / 'store.json').read_text())
    assert manifest['schema_version'] == 2 and manifest['min_writer_version'] == 3
    sources = revisions = superseded = 0
    for path in files:
        text = path.read_text()
        assert f'id: {path.stem}\n' in text
        superseded += 'status: superseded\n' in text
        evidence = json.loads((store / 'evidence' / f'{path.stem}.json').read_text())
        history = json.loads((store / 'history' / path.stem / 'manifest.json').read_text())
        assert evidence['schema_version'] == 2 and evidence['memory']['id'] == path.stem
        assert evidence['markdown_published'] is True and history['memory_id'] == path.stem
        bindings = evidence['memory']['extra']['source_revisions']
        entries = history['entries']
        assert len(evidence['events']) == len(bindings) and entries
        for entry in entries:
            revision = entry['revision']
            image = store / 'history' / path.stem / f'{revision}.md'
            assert image.is_file() and sha(image.read_bytes()) == entry['raw_hash']
        for revision in bindings:
            assert isinstance(revision, int) and 1 <= revision <= len(entries)
            assert entries[revision - 1]['revision'] == revision
        sources += len(evidence['events'])
        revisions += len(history['entries'])
    assert len(list((store / 'evidence').glob('*.json'))) == spec['memories']
    assert len(list((store / 'history').glob('*/manifest.json'))) == spec['memories']
    checkpoints = list((store / 'checkpoints').glob('*.json'))
    proposals = list((store / 'proposals').glob('*.json'))
    assert len(checkpoints) == spec['checkpoints'] and len(proposals) == spec['pending']
    for path in checkpoints:
        data = json.loads(path.read_text())
        assert data['id'] == path.stem and data['store_id'] == manifest['store_id']
        assert data['data']['next_action']
    for path in proposals:
        data = json.loads(path.read_text())
        assert data['store_id'] == manifest['store_id'] and data['state'] == 'pending'
    assert sources == spec['memories'] + spec['support'] + spec['post_revision_support']
    assert superseded == spec['supersede']
    assert revisions >= (spec['memories'] + 2 * spec['double_revise'] + spec['support'] +
                         spec['post_revision_support'] + 2 * spec['supersede'])
    return dict(schema=2, writer=3, store_id=manifest['store_id'], memories=len(files),
                sources=sources, revisions=revisions, checkpoints=len(checkpoints),
                pending=len(proposals), superseded=superseded)


def lane(rows):
    breakdowns = [row.get('score_breakdown', {}) for row in rows]
    if any('vector_diagnostic' in b for b in breakdowns):
        return 'lexical_fallback'
    if any('vector' in b or 'rerank' in b for b in breakdowns):
        return 'model'
    if any({'fts', 'bm25', 'cjk_like'} & b.keys() for b in breakdowns):
        return 'lexical'
    return 'unknown'


def measure(r, project, profile, scenario, sample):
    store = project / '.mnemosyne'
    query = ['search', 'healthcheck', '--format', 'json', '--scope', 'project']
    if scenario == 'cache_absent':
        for path in store.glob('rust-index.sqlite*'):
            path.unlink()
        result = r.json(query, project)
        assert result['value']
        actual = lane(result['value'])
    elif scenario == 'warm_search':
        for _ in range(2):
            assert r.json(query, project)['value']
        result = r.json(query, project)
        assert result['value']
        actual = lane(result['value'])
    elif scenario == 'revise_then_search':
        if profile == 'legacy':
            path = store / 'working/codebase-2026-01-01-00000000.md'
            path.write_text(path.read_text().replace('healthcheck /health', 'revisionmarker0 /health'))
            expected_id = path.stem
        else:
            first = next(p.stem for p in (store / 'working').glob('*.md')
                         if '## Service 0\n' in p.read_text())
            ref = dict(store_id=json.loads((store / 'store.json').read_text())['store_id'], memory_id=first)
            payload = {**target(r, project, ref), 'changes': {
                'body': '## Revised route\n\nrevisionmarker0 backend/config.rs healthcheck /health'}}
            r.json(['revise-v2'], project, payload)
            expected_id = first
        result = r.json(['search', 'revisionmarker0', '--format', 'json', '--scope', 'project'], project)
        assert any(row['id'] == expected_id for row in result['value'])
        actual = lane(result['value'])
    elif scenario == 'prep_task':
        action = 'inspect health route 0 before restart'
        result = r.json(['prep', 'resume health route', '--task-id', 'perf-task-0',
                         '--format', 'json', '--budget', '8000'], project)
        assert action in result['value']['context']
        assert any(item.get('kind') == 'checkpoint' and action in item.get('text', '')
                   for item in result['value']['items'])
        actual = 'lexical+checkpoint'
    elif scenario == 'file_hook':
        payload = dict(session_id=f'perf-{sample}', host='claude-code', tool_name='Edit',
                       tool_input={'file_path': 'backend/config.rs'})
        result = r.json(['hook', 'PreToolUse'], project, payload)
        hook = result['value']['hookSpecificOutput']
        assert hook['hookEventName'] == 'PreToolUse' and hook['additionalContext']
        actual = 'lexical_hook'
    else:
        before = {p.stem for p in (store / 'working').glob('*.md')}
        transcript = project / f'perf-stop-{sample}.jsonl'
        block = (f'**Findings:**\n- type: pitfall\n- importance: 70\n- title: Stop benchmark {sample}\n'
                 f'- content: |\n    Stop persisted synthetic fact {sample} for healthcheck.\n')
        transcript.write_text(json.dumps({'type': 'assistant', 'message': {'role': 'assistant',
                                    'content': block}}) + '\n')
        payload = {'transcript_path': str(transcript), 'session_id': f'perf-stop-{sample}'}
        result = r.json(['hook', 'Stop'], project, payload)
        assert 'auto-saved' in result['value']['systemMessage']
        after = {p.stem for p in (store / 'working').glob('*.md')}
        assert len(after - before) == 1
        new_id = (after - before).pop()
        evidence = json.loads((store / 'evidence' / f'{new_id}.json').read_text())
        assert evidence['memory']['id'] == new_id and evidence['events']
        assert (store / 'history' / new_id / 'manifest.json').is_file()
        replay = r.call(['hook', 'Stop'], project, payload)
        assert not replay['stdout'].strip()
        assert {p.stem for p in (store / 'working').glob('*.md')} == after
        result['replay_elapsed_ms'] = replay['elapsed_ms']
        result['replay_phases'] = replay['phases']
        actual = 'host_distill_v2'
    observation = dict(command=result['command'], elapsed_ms=result['elapsed_ms'],
                       phases=result['phases'], actual_lane=actual)
    if 'replay_elapsed_ms' in result:
        observation['replay_elapsed_ms'] = result['replay_elapsed_ms']
        observation['replay_phases'] = result['replay_phases']
    return observation


def run(args):
    binary = args.binary.resolve()
    if not binary.is_file():
        raise ValueError(f'binary not found: {binary}')
    if args.samples < 5:
        raise ValueError('--samples must be at least 5')
    if args.command_timeout <= 0 or args.overall_timeout <= 0 or args.max_disk_mib <= 0:
        raise ValueError('timeouts and disk budget must be positive')
    sizes = args.records or ([100, 1000, 10000] if args.profile == 'legacy' else [100, 1000])
    if any(n < 1 for n in sizes):
        raise ValueError('record counts must be positive')
    deadline = time.monotonic() + args.overall_timeout
    report = dict(commit=args.commit, binary_sha256=sha(binary.read_bytes()),
                  runner_sha256=sha(Path(__file__).read_bytes()), profile=args.profile,
                  platform=platform.platform(), machine=platform.machine(), samples=args.samples,
                  clock='perf_counter_ns', models='disabled; no model assets or network',
                  environment={'MNEMOSYNE_TRACE_TIMING': '1', 'MNEMOSYNE_AUTO_INIT': '0',
                               'HOME': 'isolated', 'MNEMOSYNE_HOME': 'isolated', 'PATH': ''},
                  limitations=['phase durations are inclusive and overlapping; do not sum',
                               'five-sample p95 is descriptive, not a reliable tail estimate',
                               'filesystem page cache is not evicted'], results=[])

    def save():
        report['execution_complete'] = bool(report['results']) and all(
            row['status'] == 'PASS' for row in report['results'])
        report['acceptance_passed'] = report['execution_complete']
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(report, indent=2) + '\n')

    with tempfile.TemporaryDirectory(prefix='mnemosyne-perf-') as directory:
        root = Path(directory)
        env = dict(HOME=str(root / 'home'), MNEMOSYNE_HOME=str(root / 'global'), PATH='',
                   MNEMOSYNE_TRACE_TIMING='1', MNEMOSYNE_AUTO_INIT='0')
        runner = Runner(binary, env, args.command_timeout, deadline)
        for n in sizes:
            row = dict(profile=args.profile, records_requested=n, status='BLOCKED', scenarios=[])
            report['results'].append(row)
            save()
            project = root / f'{args.profile}-{n}'
            try:
                construction_started = time.monotonic()
                spec = build(runner, project, args.profile, n, args.max_disk_mib * 1024 * 1024)
                row['construction_elapsed_ms'] = (time.monotonic() - construction_started) * 1000
                row['recipe'] = spec
                row['recipe_sha256'] = sha(json.dumps({'profile': args.profile, **spec,
                    'generator_sha256': report['runner_sha256']}, sort_keys=True).encode())
                row['recipe_hash_basis'] = 'profile + recipe counts + generator SHA256'
                row['counts'] = verify(project, args.profile, spec)
                row['corpus_sha256'] = corpus_hash(project)
                row['corpus_bytes'] = sum(p.stat().st_size for p in (project / '.mnemosyne').rglob('*') if p.is_file())
                if row['corpus_bytes'] > args.max_disk_mib * 1024 * 1024:
                    raise Blocked(f'corpus exceeds {args.max_disk_mib} MiB disk budget')
                save()
                for scenario in SCENARIOS:
                    entry = dict(scenario=scenario, status='BLOCKED', samples=[])
                    row['scenarios'].append(entry)
                    if args.profile == 'legacy' and scenario in ('prep_task', 'stop_new_replay'):
                        entry['status'] = 'NOT_APPLICABLE'
                        save()
                        continue
                    for sample in range(args.samples):
                        if time.monotonic() >= deadline:
                            raise Blocked('overall deadline exceeded')
                        copy = root / f'sample-{scenario}-{sample}'
                        shutil.copytree(project, copy)
                        try:
                            entry['samples'].append(measure(runner, copy, args.profile, scenario, sample))
                            save()
                        finally:
                            shutil.rmtree(copy)
                    values = [item['elapsed_ms'] for item in entry['samples']]
                    entry.update(status='PASS', p50_ms=statistics.median(values),
                                 p95_ms=sorted(values)[min(len(values)-1, int(len(values)*.95))],
                                 actual_lanes=sorted({item['actual_lane'] for item in entry['samples']}))
                    save()
                row['status'] = 'PASS'
            except Blocked as exc:
                row['blocked_reason'] = str(exc)
                row.setdefault('counts', {'memories_created': len(list((project / '.mnemosyne/working').glob('*.md')))
                                          if project.exists() else 0})
            except Exception as exc:
                row['status'], row['error'] = 'FAIL', f'{type(exc).__name__}: {exc}'
            save()
    return report


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('binary', type=Path)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--commit', required=True)
    parser.add_argument('--profile', choices=('legacy', 'v2-current', 'v2-evolution'), default='legacy')
    parser.add_argument('--records', type=int, action='append', help='repeat for multiple corpus sizes')
    parser.add_argument('--samples', type=int, default=5)
    parser.add_argument('--command-timeout', type=float, default=120)
    parser.add_argument('--overall-timeout', type=float, default=1800)
    parser.add_argument('--max-disk-mib', type=int, default=1024)
    args = parser.parse_args()
    try:
        report = run(args)
    except ValueError as exc:
        parser.error(str(exc))
    print(args.output)
    if any(row['status'] == 'FAIL' for row in report['results']):
        raise SystemExit(1)
    if not report['execution_complete']:
        raise SystemExit(2)


if __name__ == '__main__':
    main()
