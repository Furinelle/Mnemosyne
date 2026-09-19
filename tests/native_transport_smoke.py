"""Offline native SSE regression. No Mnemosyne Python kernel is imported."""
import http.client
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import time


def main():
    binary = str(Path(sys.argv[1]).resolve())
    with tempfile.TemporaryDirectory(prefix="mnemosyne-sse-") as tmp:
        root = Path(tmp)
        global_store = root / "global"
        global_store.mkdir()
        with socket.socket() as sock:
            sock.bind(("127.0.0.1", 0))
            port = sock.getsockname()[1]
        (global_store / "config.toml").write_text(
            f'[mcp.sse]\nhost = "127.0.0.1"\nport = {port}\n'
        )
        env = dict(os.environ, HOME=tmp, MNEMOSYNE_HOME=str(global_store))
        process = subprocess.Popen(
            [binary, "mcp", "serve", "--sse"], cwd=tmp, env=env,
            stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        )
        stream = http.client.HTTPConnection("127.0.0.1", port, timeout=5)
        try:
            for attempt in range(100):
                try:
                    with socket.create_connection(("127.0.0.1", port), timeout=0.1):
                        break
                except OSError:
                    if process.poll() is not None:
                        raise AssertionError(process.stderr.read().decode())
                    time.sleep(0.02)
            stream.request("GET", "/sse")
            response = stream.getresponse()
            assert response.status == 200
            assert response.readline().strip() == b"event: endpoint"
            endpoint = response.readline().decode().strip().removeprefix("data: ")
            assert endpoint.startswith("/messages?session_id=")

            def post(data):
                client = http.client.HTTPConnection("127.0.0.1", port, timeout=5)
                try:
                    client.request("POST", endpoint, body=data)
                    reply = client.getresponse()
                    reply.read()
                    return reply.status
                finally:
                    client.close()

            assert post(b"\xff") == 400
            assert process.poll() is None
            assert post(json.dumps({"jsonrpc": "2.0", "id": 9, "method": "ping"})) == 202
            while True:
                line = response.readline().decode().strip()
                if line.startswith("data: {"):
                    assert json.loads(line.removeprefix("data: "))["id"] == 9
                    break
            print("SSE handshake, malformed UTF-8 isolation, and ping: passed")
        finally:
            stream.close()
            process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()


if __name__ == "__main__":
    main()
