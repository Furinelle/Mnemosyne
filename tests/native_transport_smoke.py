"""Offline native SSE boundaries. Python is a test driver, never the kernel."""
import http.client
import json
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
        (root / "empty-bin").mkdir()
        with socket.socket() as sock:
            sock.bind(("127.0.0.1", 0))
            port = sock.getsockname()[1]
        config = global_store / "config.toml"
        config.write_text(f'[mcp.sse]\nhost = "127.0.0.1"\nport = {port}\n')
        env = {
            "HOME": tmp, "MNEMOSYNE_HOME": str(global_store),
            "PATH": str(root / "empty-bin"), "MNEMOSYNE_MAX_INPUT_BYTES": "4096",
        }
        process = subprocess.Popen(
            [binary, "mcp", "serve", "--sse"], cwd=tmp, env=env,
            stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        )
        stream = http.client.HTTPConnection("127.0.0.1", port, timeout=5)
        try:
            for _ in range(100):
                try:
                    with socket.create_connection(("127.0.0.1", port), timeout=0.1):
                        break
                except OSError:
                    if process.poll() is not None:
                        raise AssertionError(process.stderr.read().decode())
                    time.sleep(0.02)

            def request(method, path, data=None, headers=None):
                client = http.client.HTTPConnection("127.0.0.1", port, timeout=5)
                try:
                    client.request(method, path, body=data, headers=headers or {})
                    reply = client.getresponse()
                    reply.read()
                    return reply.status
                finally:
                    client.close()

            assert request("GET", "/sse", headers={"Origin": "https://untrusted.example"}) == 403
            assert request("GET", "/sse", headers={"Host": f"rebind.example:{port}"}) == 403
            stream.request("GET", "/sse", headers={"Origin": f"http://localhost:{port}"})
            response = stream.getresponse()
            assert response.status == 200
            assert response.readline().strip() == b"event: endpoint"
            endpoint = response.readline().decode().strip().removeprefix("data: ")
            assert endpoint.startswith("/messages?session_id=")

            def post(payload, headers=None):
                if isinstance(payload, dict):
                    payload = json.dumps(payload)
                return request("POST", endpoint, payload, headers)

            def receive(expected_id):
                while True:
                    line = response.readline().decode().strip()
                    if line.startswith("data: {"):
                        result = json.loads(line.removeprefix("data: "))
                        assert result["id"] == expected_id, result
                        assert result["jsonrpc"] == "2.0", result
                        return result

            assert post(b"\xff") == 400
            assert post(b"x" * 4097) == 413
            assert post({"jsonrpc": "2.0", "id": 50, "method": "ping"},
                        {"Origin": "null"}) == 403
            assert process.poll() is None
            # Notifications produce no SSE response, including unknown ones.
            assert post({"jsonrpc": "2.0", "method": "notifications/future"}) == 202
            assert post({"jsonrpc": "2.0", "method": "ping"}) == 202
            assert post({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
                "protocolVersion": "2024-11-05", "capabilities": {},
                "clientInfo": {"name": "isolated-sse-fixture", "version": "1"},
            }}) == 202
            assert receive(1)["result"]["protocolVersion"] == "2024-11-05"
            assert post({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}) == 202
            assert receive(2)["error"]["code"] == -32002
            assert post({"jsonrpc": "2.0", "method": "notifications/initialized"}) == 202
            assert post({"jsonrpc": "2.0", "id": 3, "method": "tools/list"}) == 202
            assert len(receive(3)["result"]["tools"]) == 8
            assert post({"jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": {
                "name": "mnemosyne_read_core", "arguments": {"scope": "global"},
            }}) == 202
            assert receive(4)["result"]["structuredContent"] == {"global": ""}
            assert post({"jsonrpc": "2.0", "id": 5, "method": "tools/call", "params": {
                "name": "mnemosyne_show", "arguments": {"id": "missing"},
            }}) == 202
            assert receive(5)["result"]["isError"] is True
            assert post({"jsonrpc": "2.0", "id": 6, "method": "ping"}) == 202
            assert receive(6)["result"] == {}
            print("SSE initialization, tools, notifications, isError, UTF-8/size and Origin/Host boundaries: passed")
        finally:
            stream.close()
            process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()

        config.write_text(f'[mcp.sse]\nhost = "0.0.0.0"\nport = {port}\n')
        rejected = subprocess.run([binary, "mcp", "serve", "--sse"], cwd=tmp, env=env,
                                  capture_output=True, timeout=5)
        assert rejected.returncode != 0
        assert b"Non-loopback SSE is disabled" in rejected.stderr
        assert not rejected.stdout
        print("Non-loopback SSE rejects unauthenticated exposure: passed")


if __name__ == "__main__":
    main()
