#!/usr/bin/env python3
"""Offline lane matrix: mock vectors, real lexical/graph paths, no real model claim."""
import argparse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import threading


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("binary", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--skip-model-smoke", action="store_true")
    args = parser.parse_args()
    binary = args.binary.resolve()
    smoke = Path(__file__).with_name("native_http_models_smoke.py")
    if not args.skip_model_smoke:
        subprocess.run([sys.executable, smoke, binary], check=True, timeout=60)
    calls = []

    class Handler(BaseHTTPRequestHandler):
        def do_POST(self):
            assert self.headers.get("Authorization") == "Bearer local-test-only"
            body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
            assert self.path.endswith("/embeddings"), self.path
            calls.append(self.path)
            rows = []
            for index, text in enumerate(body["input"]):
                rows.append({"index": index, "embedding": [1.0, 0.0]
                             if "needlevector" in text else [0.0, 1.0]})
            encoded = json.dumps({"data": rows}).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(encoded)))
            self.end_headers()
            self.wfile.write(encoded)

        def log_message(self, *_args):
            pass

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        with tempfile.TemporaryDirectory(prefix="mnemosyne-eval-matrix-") as directory:
            root = Path(directory)
            (root / ".git").mkdir()
            global_store = root / "global"
            global_store.mkdir()
            url = f"http://127.0.0.1:{server.server_port}/v1"
            global_store.joinpath("config.toml").write_text(
                "[embedding]\nenabled=true\nbackend='openai'\nmodel='fixture'\ndimensions=2\n"
                f"api_base='{url}'\napi_key_env='MNE_TEST_KEY'\n"
            )
            env = dict(os.environ, HOME=str(root / "home"), MNEMOSYNE_HOME=str(global_store),
                       MNE_TEST_KEY="local-test-only")

            def run(command):
                result = subprocess.run([binary, *command], cwd=root, env=env, text=True,
                                        capture_output=True, timeout=20)
                assert result.returncode == 0, (command, result.stdout, result.stderr)
                return result.stdout

            def config(text):
                root.joinpath(".mnemosyne/config.toml").write_text(text)

            def write(title, content):
                return run(["write", "--type", "codebase", "--importance", "70", "--title", title,
                            "--content", content, "--force"]).strip().removeprefix("Wrote ")

            def search(query):
                return json.loads(run(["search", query, "--scope", "project", "--format", "json"]))

            run(["init", "--no-agent-files"])
            target = write("Target", "needlevector needlealpha needlegraph")
            for index in range(12):
                write(f"Noise {index}", f"irrelevant starlight {index}")

            records = []
            config("[embedding]\nenabled=false\n[fusion]\nlink_expansion=false\n[rerank]\nenabled=false\n")
            lexical = search("needlealpha")
            assert lexical and lexical[0]["id"] == target and "vec" not in lexical[0]["score_breakdown"]
            records.append({"requested": {"lexical": True, "vector": False, "graph": False, "rerank": False},
                            "effective": {"lexical": True, "vector": False, "graph": False, "rerank": False},
                            "diagnostic": "noise did not displace lexical target"})

            config("[embedding]\nenabled=true\nbackend='openai'\nmodel='fixture'\ndimensions=2\n"
                   "[fusion]\nlink_expansion=false\n[rerank]\nenabled=false\n")
            run(["embed-backfill", "--scope", "project"])
            vector = search("needlevector")
            assert vector and vector[0]["id"] == target and "vec" in vector[0]["score_breakdown"]
            records.append({"requested": {"lexical": True, "vector": True, "graph": False, "rerank": False},
                            "effective": {"lexical": True, "vector": True, "graph": False, "rerank": False},
                            "diagnostic": "local HTTP embedding mock; vec score observed"})

            config("[embedding]\nenabled=false\n[fusion]\nlink_expansion=true\nlink_expansion_max_hops=1\n"
                   "[rerank]\nenabled=false\n")
            linked = write("Linked", "unrelated linked evidence")
            run(["link", target, linked, "--rel", "related"])
            graph = search("needlegraph")
            graph_row = next(row for row in graph if row["id"] == linked)
            assert "link_boost" in graph_row["score_breakdown"]
            records.append({"requested": {"lexical": True, "vector": False, "graph": True, "rerank": False},
                            "effective": {"lexical": True, "vector": False, "graph": True, "rerank": False},
                            "diagnostic": "linked non-lexical record has link_boost"})

            config("[embedding]\nenabled=false\n[fusion]\nlink_expansion=false\n"
                   "[rerank]\nenabled=true\nbackend='onnx'\nmodel='missing-local-fixture'\ntop_n=5\n")
            rerank = search("needlealpha")
            assert rerank and rerank[0]["id"] == target
            assert all("rerank" not in row["score_breakdown"] for row in rerank)
            records.append({"requested": {"lexical": True, "vector": False, "graph": False, "rerank": True},
                            "effective": {"lexical": True, "vector": False, "graph": False, "rerank": False},
                            "diagnostic": "missing local ONNX asset kept lexical ordering; no real rerank claim"})

            path = root / ".mnemosyne/working" / f"{target}.md"
            original = path.read_text()
            edited = original.replace("needlealpha", "needleomega")
            assert len(edited) == len(original)
            path.write_text(edited)
            assert not search("needlealpha")
            assert search("needleomega")[0]["id"] == target
            renamed = path.with_name("renamed.md")
            path.rename(renamed)
            assert search("needleomega")[0]["path"].endswith("renamed.md")
            renamed.unlink()
            assert not search("needleomega")

            report = {
                "version": 1,
                "scope": "temporary project and loopback mock only",
                "cache_recovery": ("skipped; final runner executes native_http_models_smoke.py"
                                   if args.skip_model_smoke else
                                   "passed by tests/native_http_models_smoke.py before matrix"),
                "change_detection": ["same_size_edit", "rename", "delete"],
                "requested_effective_matrix": records,
                "mock_http_calls": len(calls),
                "limitations": [
                    "vector lane uses an in-process HTTP mock, not a real embedding provider",
                    "rerank is requested with missing local ONNX assets and verified only as lexical fallback",
                    "not a public-corpus retrieval result or cross-agent end-to-end evaluation",
                ],
            }
            args.output.parent.mkdir(parents=True, exist_ok=True)
            args.output.write_text(json.dumps(report, indent=2) + "\n")
            print(json.dumps(report, indent=2))
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=5)


if __name__ == "__main__":
    main()
