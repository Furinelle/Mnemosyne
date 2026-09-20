"""Protocol test using a local fake provider; no external API credits or data."""
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import subprocess
import sqlite3
import sys
import tempfile
import threading


def main():
    calls = []
    behavior = {"mode": "valid", "mutate": None}

    class Handler(BaseHTTPRequestHandler):
        def do_POST(self):
            assert self.headers.get('Authorization') == 'Bearer local-test-only'
            body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
            calls.append(self.path)
            if self.path.endswith('/embeddings'):
                if behavior['mutate'] is not None:
                    behavior['mutate']()
                    behavior['mutate'] = None
                if behavior['mode'] == 'offline':
                    self.send_error(503)
                    return
                payload = {'data': [{'index': i, 'embedding': [1.0, 0.5]}
                                    for i in reversed(range(len(body['input'])))]}
                mode = behavior['mode']
                if mode == 'duplicate':
                    payload['data'] = [{'index': 0, 'embedding': [1, 0]}] * max(2, len(body['input']))
                elif mode in ['nan', 'inf', 'zero', 'dimension']:
                    vector = {'nan': [float('nan'), 0], 'inf': [float('inf'), 0],
                              'zero': [0, 0], 'dimension': [1]}[mode]
                    for row in payload['data']:
                        row['embedding'] = vector
            elif self.path.endswith('/chat/completions'):
                payload = {'choices': [{'message': {'content': json.dumps([{
                    'type': 'pitfall', 'importance': 70, 'title': 'HTTP fixture',
                    'content': 'Use a lock to serialize writes.', 'tags': ['fixture'],
                }])}}]}
            else:
                self.send_error(404)
                return
            encoded = json.dumps(payload).encode()
            self.send_response(200)
            self.send_header('Content-Type', 'application/json')
            self.send_header('Content-Length', str(len(encoded)))
            self.end_headers()
            self.wfile.write(encoded)

        def log_message(self, *_args):
            pass

    server = ThreadingHTTPServer(('127.0.0.1', 0), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    binary = str(Path(sys.argv[1]).resolve())
    try:
        with tempfile.TemporaryDirectory(prefix='mnemosyne-http-models-') as tmp:
            root = Path(tmp)
            (root / '.git').mkdir()
            global_store = root / 'global'
            global_store.mkdir()
            url = f'http://127.0.0.1:{server.server_port}/v1'
            settings = ('[embedding]\nenabled=true\nbackend="openai"\nmodel="fixture"\n'
                        'dimensions=2\n[distill]\nenabled=true\nengine="llm"\n')
            (global_store / 'config.toml').write_text(
                settings.replace('[distill]', f'api_base="{url}"\napi_key_env="MNE_TEST_KEY"\n[distill]')
                + f'[distill.llm]\nbackend="openai"\nmodel="fixture"\napi_base="{url}"\napi_key_env="MNE_TEST_KEY"\n'
            )
            env = dict(os.environ, HOME=tmp, MNEMOSYNE_HOME=str(global_store), MNE_TEST_KEY='local-test-only')

            def run(args, text=None, success=True, stderr=False):
                r = subprocess.run([binary, *args], cwd=tmp, env=env, input=text,
                                   capture_output=True, text=True, timeout=15)
                assert (r.returncode == 0) == success, (r.returncode, r.stdout, r.stderr)
                return r.stderr if stderr else r.stdout

            run(['init', '--no-agent-files'])
            # A repository must not redirect model calls or choose a credential.
            (root / '.mnemosyne/config.toml').write_text(settings.replace(
                '[distill]', 'api_base="http://127.0.0.1:9/untrusted"\napi_key_env="MNE_UNTRUSTED_KEY"\n[distill]'
            ))
            for title in ['alpha', 'beta']:
                run(['write', '--type', 'codebase', '--importance', '70', '--title', title,
                     '--content', title, '--force'])
            assert 'Embedded 2' in run(['embed-backfill', '--scope', 'project'])
            before = len(calls)
            stats = json.loads(run(['embed-backfill', '--scope', 'project', '--format', 'json']))
            assert stats == dict(computed=0, written=0, skipped_stale=0, invalid=0, repaired=0, failed=0), stats
            assert len(calls) == before, 'fresh cache invoked provider'
            results = json.loads(run(['search', 'semantic fixture', '--scope', 'project', '--format', 'json']))
            assert any('vec' in item['score_breakdown'] for item in results)
            cache = root / '.mnemosyne/vectors-rust.sqlite'
            for raw in ['[0,0]', '[1]', '{broken']:
                with sqlite3.connect(cache) as db:
                    db.execute('UPDATE vectors SET vector_json=? WHERE path=(SELECT min(path) FROM vectors)', [raw])
                stats = json.loads(run(['embed-backfill', '--scope', 'project', '--format', 'json']))
                assert stats['written'] == stats['repaired'] == stats['computed'] == 1, stats
            project_config = root / '.mnemosyne/config.toml'
            original_config = project_config.read_text()
            for setting in ['dimensions=3', 'model="new-model"', 'revision="new-revision"']:
                def mutate(setting=setting):
                    project_config.write_text('[embedding]\nenabled=true\nbackend="openai"\n' + setting + '\n')
                behavior['mutate'] = mutate
                results = json.loads(run(['search', 'alpha', '--scope', 'project', '--format', 'json']))
                assert results and all('vec' not in item['score_breakdown'] for item in results), results
                assert results[0]['score_breakdown']['vector_diagnostic'] == {
                    'status': 'stale', 'reason': 'profile_changed_during_inference'}, results
                project_config.write_text(original_config)
            for mode in ['duplicate', 'nan', 'inf', 'zero', 'dimension', 'offline']:
                with sqlite3.connect(cache) as db:
                    db.execute('DELETE FROM vectors')
                behavior['mode'] = mode
                run(['embed-backfill', '--scope', 'project'], success=False)
                stats = json.loads(run(['embed-backfill', '--scope', 'project', '--format', 'json'], success=False))
                assert stats['failed' if mode == 'offline' else 'invalid'] == 2, (mode, stats)
                assert stats['written'] == 0, (mode, stats)
                with sqlite3.connect(cache) as db:
                    assert db.execute('SELECT count(*) FROM vectors').fetchone()[0] == 0, mode
                results = json.loads(run(['search', 'alpha', '--scope', 'project', '--format', 'json']))
                assert results and 'vector_diagnostic' in results[0]['score_breakdown'], mode
            behavior['mode'] = 'valid'
            output = run(['distill', '--stdin', '--format', 'role-jsonl', '--commit'],
                         json.dumps({'role': 'assistant', 'text': 'Use a lock to serialize writes.'}))
            assert 'HTTP fixture' in output
            assert '/v1/embeddings' in calls and '/v1/chat/completions' in calls
            # A broken derived cache and disabled/disconnected models cannot break base operations.
            db.close()
            cache.write_bytes(b'broken sqlite')
            results = json.loads(run(['search', 'alpha', '--scope', 'project', '--format', 'json']))
            assert results[0]['score_breakdown']['vector_diagnostic']['status'] == 'corrupt'
            cache.unlink()
            cache.mkdir()  # A cache path that cannot be opened or written as SQLite.
            results = json.loads(run(['search', 'alpha', '--scope', 'project', '--format', 'json']))
            assert results[0]['score_breakdown']['vector_diagnostic']['status'] == 'corrupt'
            project_config.write_text('[embedding]\nenabled=false\n')
            before = len(calls)
            run(['write', '--type', 'codebase', '--importance', '70', '--title', 'disabled',
                 '--content', 'disabled model remains writable', '--force'])
            results = json.loads(run(['search', 'disabled', '--scope', 'project', '--format', 'json']))
            assert results and len(calls) == before
            request = {'jsonrpc': '2.0', 'id': 1, 'method': 'tools/call', 'params': {
                'name': 'mnemosyne_search', 'arguments': {'query': 'disabled', 'scope': 'project'}}}
            reply = json.loads(run(['mcp', 'serve'], json.dumps(request) + '\n'))
            assert 'error' not in reply and not reply['result'].get('isError'), reply
            assert 'disabled' in json.dumps(reply) and len(calls) == before
            print('HTTP model protocol, cache repair, query profile CAS, invalid provider fallback, disabled model isolation: passed (local mock; no real model acceptance)')
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=5)


if __name__ == '__main__':
    main()
