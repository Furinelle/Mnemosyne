"""Real local ONNX embedding smoke; all memory writes stay in a temp store.

Usage: python3 tests/native_onnx_smoke.py BINARY MODEL_ONNX ORT_LIBRARY [DIMENSION] [RERANK_ONNX]
Model and vocabulary must already exist; this test never downloads them.
"""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile


def main():
    binary, model = [str(Path(p).resolve()) for p in sys.argv[1:3]]
    library = str(Path(sys.argv[3]).resolve()) if sys.argv[3] != '-' else None
    dimension = int(sys.argv[4]) if len(sys.argv) > 4 else 512
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

        def run(args):
            result = subprocess.run([binary, *args], cwd=tmp, env=env, text=True,
                                    capture_output=True, timeout=60)
            assert result.returncode == 0, result.stderr
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
        if len(sys.argv)>5:
            reranker=str(Path(sys.argv[5]).resolve())
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


if __name__ == "__main__":
    main()
