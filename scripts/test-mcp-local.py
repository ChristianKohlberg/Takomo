#!/usr/bin/env python3
"""Run the existing stdio MCP E2E harness against a disposable local server.

Unlike an interactive Backlot lease this automated fixture must never target a
shared store. Pass the already built Takomo executable as the sole argument.
"""
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import time
from urllib.request import urlopen


def main():
    binary = str(Path(sys.argv[1]).resolve(strict=True))
    root = Path(__file__).resolve().parent.parent
    with tempfile.TemporaryDirectory(prefix="takomo-mcp-") as directory:
        command = [binary, "--db", str(Path(directory) / "test.db")]
        subprocess.run(command + ["project", "create", "--id", "mcptest", "--name", "MCP test"], check=True)
        token = json.loads(subprocess.check_output(command + [
            "token", "create", "--actor", "human:e2e", "--scopes",
            "read,write,human,admin", "--rate-limit", "10000", "--json",
        ]))["token"]
        with socket.socket() as listener:
            listener.bind(("127.0.0.1", 0))
            port = listener.getsockname()[1]
        base = f"http://127.0.0.1:{port}"
        with open(Path(directory) / "server.log", "w+") as log:
            server = subprocess.Popen(command + ["serve", "--bind", f"127.0.0.1:{port}"], stdout=log, stderr=log)
            try:
                for _ in range(100):
                    if server.poll() is not None:
                        raise RuntimeError("test server exited during startup")
                    try:
                        with urlopen(base + "/healthz", timeout=1) as response:
                            if response.status == 200:
                                break
                    except OSError:
                        time.sleep(0.1)
                else:
                    raise RuntimeError("test server did not become healthy")
                env = {**os.environ, "TAKOMO_URL": base + "/v1", "TAKOMO_TOKEN": token,
                       "TAKOMO_TEST_PROJECT": "mcptest"}
                subprocess.run(["npm", "test"], cwd=root / "clients/mcp", env=env, check=True, timeout=180)
            except Exception:
                log.seek(0)
                print(log.read(), file=sys.stderr)
                raise
            finally:
                server.terminate()
                try:
                    server.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    server.kill()
                    server.wait()


if __name__ == "__main__":
    main()
