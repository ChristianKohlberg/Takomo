#!/usr/bin/env python3
"""Exercise Render-style secret permissions with real container service users."""
import subprocess
import sys

image = sys.argv[1] if len(sys.argv) > 1 else "takomo-ci"
probe = r'''
import importlib.util
import os
from pathlib import Path
import subprocess
import sys

spec = importlib.util.spec_from_file_location("supervisor", "/usr/local/lib/takomo/container-supervisor.py")
supervisor = importlib.util.module_from_spec(spec)
spec.loader.exec_module(supervisor)

root = Path("/tmp/render-secret-fixture")
root.mkdir()
secret = root / "github.pem"
secret.write_text("fixture-only-not-a-real-key")
os.chown(root, 0, 1000)
root.chmod(0o750)
os.chown(secret, 0, 1000)
secret.chmod(0o640)
before = [(p.stat().st_uid, p.stat().st_gid, p.stat().st_mode) for p in (root, secret)]
# An unrelated supervisor group must not leak into any service.
os.setgroups([1000, 12345])
for name in ("takomo", "kroki", "mermaid"):
    child_code = """
import os, sys
from pathlib import Path
expected = sys.argv[1] == 'takomo'
assert os.geteuid() != 0
assert 12345 not in os.getgroups()
assert 'NoNewPrivs:\\t1' in Path('/proc/self/status').read_text()
try:
    Path('/tmp/render-secret-fixture/github.pem').read_bytes()
    readable = True
except PermissionError:
    readable = False
assert readable == expected, 'incorrect secret-file access for ' + sys.argv[1]
assert (1000 in os.getgroups()) == expected
""".replace('\\t', '\t')
    # Exercise the real spawn path, including setpriv and its group handling.
    supervisor.spawn(name, [sys.executable, "-c", child_code, name], {"PATH": os.environ["PATH"]}, "/tmp")
    child = supervisor.CHILDREN[-1][1]
    assert child.wait(timeout=10) == 0, name
assert before == [(p.stat().st_uid, p.stat().st_gid, p.stat().st_mode) for p in (root, secret)]
print("ok - app reads group-1000 secrets; renderers denied; no inherited groups or mount changes")
'''
subprocess.run(["docker", "run", "--rm", "--network", "none", "--entrypoint", "python3",
                image, "-c", probe], check=True, timeout=60)
