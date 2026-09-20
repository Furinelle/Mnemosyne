"""Real local ONNX embedding smoke; all memory writes stay in a temp store.

Usage: python3 tests/native_onnx_smoke.py BINARY MODEL_ONNX ORT_LIBRARY [DIMENSION] [RERANK_ONNX]
Model and vocabulary must already exist; this test never downloads them.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import subprocess
import sys
import tempfile
import time


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('binary')
    parser.add_argument('model_onnx')
    parser.add_argument('ort_library')
    parser.add_argument('dimension', nargs='?', type=int, default=512)
    parser.add_argument('rerank_onnx', nargs='?')
    parser.add_argument('--timing-output', type=Path)
    args = parser.parse_args()
    binary, model = [str(Path(p).resolve()) for p in (args.binary, args.model_onnx)]
    library = str(Path(args.ort_library).resolve()) if args.ort_library != '-' else None
    dimension = args.dimension
    reranker = str(Path(args.rerank_onnx).resolve()) if args.rerank_onnx else None
    if args.timing_output and not reranker:
        parser.error('--timing-output requires RERANK_ONNX')

    def sha256(path):
        digest = hashlib.sha256()
        with Path(path).open('rb') as file:
            for chunk in iter(lambda: file.read(65536), b''):
                digest.update(chunk)
        return digest.hexdigest()

    with tempfile.TemporaryDirectory(prefix="mnemosyne-onnx-") as tmp:
        root = Path(tmp)
        (root / ".git").mkdir()
        store = root / "global"
        store.mkdir()
        settings = (
            '[embedding]\nenabled=true\nbackend="onnx"\n'
            f'model="local-smoke"\ndimensions={dimension}\n'
        )
        (store / "config.toml").write_text(settings + f"onnx_path={json.dumps(model)}\n")
        env = dict(os.environ, HOME=tmp, MNEMOSYNE_HOME=str(store))
        if library:
            env['ORT_DYLIB_PATH']=library
        else:
            env.pop('ORT_DYLIB_PATH',None)

        def run(command, trace=False):
            result = subprocess.run([binary, *command], cwd=tmp, env=env, text=True,
                                    capture_output=True, timeout=60)
            assert result.returncode == 0, result.stderr
            if not trace:
                assert not result.stderr.strip(), result.stderr
            return result.stdout, result.stderr

        run(["init", "--no-agent-files"])
        (root / ".mnemosyne/config.toml").write_text(settings)
        for title, body in [("storage", "数据库通过文件锁避免并发写入冲突"),
                            ("fruit", "苹果和香蕉是常见水果"),
                            ("network", "网络连接失败后可以检查代理配置")]:
            run(["write", "--type", "codebase", "--importance", "70", "--title", title,
                 "--content", body, "--force"])
        assert "Embedded 3" in run(["embed-backfill", "--scope", "project"])[0]
        output, error = run(["search", "防止同时修改数据", "--scope", "project", "--format", "json"])
        rows = json.loads(output)
        assert rows and any("vec" in row["score_breakdown"] for row in rows)
        assert not error.strip(), error
        assert "Embedded 0" in run(["embed-backfill", "--scope", "project"])[0]
        print("Real ONNX embedding, vector retrieval, incremental cache: passed")
        if reranker:
            rerank='\n[rerank]\nenabled=true\nmodel="local-cross-encoder"\ntop_n=3\n'
            with (store / "config.toml").open('a') as config:
                config.write(rerank+f'onnx_path={json.dumps(reranker)}\n')
            with (root / '.mnemosyne/config.toml').open('a') as config:
                config.write(rerank)
            output,error=run(["search","database lock","--scope","project","--format","json"])
            rows=json.loads(output)
            assert rows and any('rerank' in row['score_breakdown'] for row in rows)
            assert not error.strip(),error
            print('Real ONNX cross-encoder reranking: passed')

        if args.timing_output:
            samples = []
            required = {'file_enumeration', 'sqlite_sync', 'candidate_read', 'graph_expansion',
                        'model_profile', 'model_fingerprint', 'model_initialize'}
            trace_env = dict(env, MNEMOSYNE_TRACE_TIMING='1')
            for _ in range(5):
                started = time.perf_counter_ns()
                result = subprocess.run(
                    [binary, 'search', 'database lock', '--scope', 'project', '--format', 'json'],
                    cwd=tmp, env=trace_env, text=True, capture_output=True, timeout=60,
                )
                elapsed_ms = (time.perf_counter_ns() - started) / 1e6
                assert result.returncode == 0, result.stderr
                rows = json.loads(result.stdout)
                assert rows and any('vec' in row['score_breakdown'] for row in rows)
                assert any('rerank' in row['score_breakdown'] for row in rows)
                phases = []
                for line in result.stderr.splitlines():
                    try:
                        event = json.loads(line)
                    except json.JSONDecodeError as error:
                        raise AssertionError(f'non-timing stderr: {line}') from error
                    assert event.get('event') == 'mnemosyne_timing', event
                    assert event.get('inclusive') is True, event
                    assert event.get('status') == 'measured', event
                    assert isinstance(event.get('duration_ms'), (int, float)), event
                    phases.append(event)
                assert required <= {event['phase'] for event in phases}, phases
                samples.append({'elapsed_ms': elapsed_ms, 'phases': phases})
            output = {
                'binary': {'path': binary, 'sha256': sha256(binary)},
                'models': {
                    'embedding': {'path': model, 'sha256': sha256(model)},
                    'rerank': {'path': reranker, 'sha256': sha256(reranker)},
                },
                'ort_library': {'path': library, 'sha256': sha256(library)} if library else None,
                'environment': {'platform': platform.platform(), 'machine': platform.machine(),
                                'python': sys.version, 'trace': 'MNEMOSYNE_TRACE_TIMING=1'},
                'samples': samples,
                'limitations': ['phase durations are inclusive and overlapping; do not sum them'],
            }
            args.timing_output.parent.mkdir(parents=True, exist_ok=True)
            args.timing_output.write_text(json.dumps(output, indent=2) + '\n')
            print(args.timing_output)


if __name__ == "__main__":
    main()
