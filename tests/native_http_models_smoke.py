"""Protocol test using a local fake provider; no external API credits or data."""
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import threading


def main():
    calls = []

    class Handler(BaseHTTPRequestHandler):
        def do_POST(self):
            assert self.headers.get('Authorization') == 'Bearer local-test-only'
            body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
            calls.append(self.path)
            if self.path.endswith('/embeddings'):
                payload = {'data': [{'index': i, 'embedding': [1.0, 0.5]}
                                    for i in reversed(range(len(body['input'])))]}
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

            def run(args, text=None):
                r = subprocess.run([binary, *args], cwd=tmp, env=env, input=text,
                                   capture_output=True, text=True, timeout=15)
                assert r.returncode == 0, r.stderr
                return r.stdout

            run(['init', '--no-agent-files'])
            # A repository must not redirect model calls or choose a credential.
            (root / '.mnemosyne/config.toml').write_text(settings.replace(
                '[distill]', 'api_base="http://127.0.0.1:9/untrusted"\napi_key_env="MNE_UNTRUSTED_KEY"\n[distill]'
            ))
            for title in ['alpha', 'beta']:
                run(['write', '--type', 'codebase', '--importance', '70', '--title', title,
                     '--content', title, '--force'])
            assert 'Embedded 2' in run(['embed-backfill', '--scope', 'project'])
            results = json.loads(run(['search', 'semantic fixture', '--scope', 'project', '--format', 'json']))
            assert any('vec' in item['score_breakdown'] for item in results)
            output = run(['distill', '--stdin', '--format', 'role-jsonl', '--commit'],
                         json.dumps({'role': 'assistant', 'text': 'Use a lock to serialize writes.'}))
            assert 'HTTP fixture' in output
            assert '/v1/embeddings' in calls and '/v1/chat/completions' in calls
            print('HTTP embedding batch order, vector retrieval, LLM extraction: passed (local mock)')
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=5)


if __name__ == '__main__':
    main()
