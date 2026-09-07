#!/usr/bin/env python3
"""Real-image checks for provider-owned data mounts; all fixtures are disposable."""
import json
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import time
import urllib.request
import uuid

IMAGE = sys.argv[1] if len(sys.argv) > 1 else "takomo-ci"
CONTAINERS = []
PREFIX = "takomo-volume-" + uuid.uuid4().hex[:10]


def docker(*args, check=True, timeout=120):
    result = subprocess.run(["docker", *args], capture_output=True, text=True, timeout=timeout)
    if check and result.returncode:
        raise AssertionError(f"docker {args[0]} failed: {result.stderr[-1500:]}")
    return result


def fixture(root, code, *args):
    return docker("run", "--rm", "--entrypoint", "python3", "--mount",
                  f"type=bind,source={root},target=/fixture", IMAGE, "-c", code, *args).stdout


def initialize(directory, check=True):
    # Version parsing exits before SQLite opens the DB. This exercises the actual
    # entrypoint's initializer while keeping every fixture byte unchanged.
    return docker("run", "--rm", "--mount", f"type=bind,source={directory},target=/var/data",
                  IMAGE, "--version", check=check)


def metadata(root, relative):
    return json.loads(fixture(root, """
import hashlib, json, os, stat, sys
from pathlib import Path
root = Path('/fixture') / sys.argv[1]
result = {}
for path in [root, *root.rglob('*')]:
    info = path.lstat()
    result[str(path.relative_to(root))] = {
        'uid': info.st_uid, 'gid': info.st_gid, 'mode': stat.S_IMODE(info.st_mode),
        'digest': hashlib.sha256(path.read_bytes()).hexdigest() if path.is_file() and not path.is_symlink() else None,
        'link': os.readlink(path) if path.is_symlink() else None,
    }
print(json.dumps(result))
""", relative))


def serve(directory, token=None, expected_project=None):
    name = f"{PREFIX}-{len(CONTAINERS)}"
    CONTAINERS.append(name)
    docker("run", "-d", "--name", name, "--memory", "3g", "--cpus", "2", "--pids-limit", "512",
           "--mount", f"type=bind,source={directory},target=/var/data",
           "-p", "127.0.0.1::8080", IMAGE)
    port = json.loads(docker("inspect", name).stdout)[0]["NetworkSettings"]["Ports"]["8080/tcp"][0]["HostPort"]
    base = f"http://127.0.0.1:{port}"
    deadline = time.monotonic() + 90
    while time.monotonic() < deadline:
        assert json.loads(docker("inspect", name).stdout)[0]["State"]["Running"], "mount startup exited"
        try:
            with urllib.request.urlopen(base + "/healthz", timeout=2) as response:
                if response.status == 200:
                    break
        except OSError:
            pass
        time.sleep(0.5)
    else:
        raise AssertionError("mount did not become healthy")
    docker("exec", name, "python3", "/usr/local/lib/takomo/container-healthcheck.py")
    docker("exec", name, "python3", "-c", """
from pathlib import Path
matches = []
for process in Path('/proc').iterdir():
    if not process.name.isdigit(): continue
    try:
        if (process / 'comm').read_text().strip() == 'takomo': matches.append(process.stat().st_uid)
    except FileNotFoundError: pass
assert matches == [10001], matches
""")
    for uid in (10002, 10003):
        docker("exec", "--user", str(uid), name, "python3", "-c", """
try: open('/var/data/takomo.db', 'rb')
except PermissionError: pass
else: raise AssertionError('renderer can read migrated database')
""")
    if token:
        request = urllib.request.Request(base + "/v1/projects", headers={"Authorization": "Bearer " + token})
        with urllib.request.urlopen(request, timeout=10) as response:
            assert any(item["id"] == expected_project for item in json.load(response)), "project or token was lost"
    docker("stop", "--time", "15", name, timeout=25)
    assert json.loads(docker("inspect", name).stdout)[0]["State"]["ExitCode"] == 0
    docker("rm", name)


def main(root):
    global IMAGE
    IMAGE = docker("image", "inspect", "--format", "{{.Id}}", IMAGE).stdout.strip()
    print(f"Testing volume initialization in {IMAGE}", flush=True)
    expected_gid = int(fixture(root, "import pwd; print(pwd.getpwnam('takomo').pw_gid)"))
    for uid in (0, 1000):
        relative = f"fresh-{uid}"
        fixture(root, """
import os, sys
from pathlib import Path
p = Path('/fixture') / sys.argv[1]
p.mkdir()
os.chown(p, int(sys.argv[2]), int(sys.argv[2]))
p.chmod(0o755)
""", relative, str(uid))
        serve(root / relative)
        after = metadata(root, relative)
        assert after["."]["uid"] == 10001 and after["."]["mode"] == 0o700
        assert after["takomo.db"]["uid"] == 10001
        print(f"ok - fresh UID {uid} bind mount initializes and runs app non-root", flush=True)

    # WAL/SHM are copied while the real SQLite connection is open, after a
    # committed write. PERSIST produces a real cold rollback journal; no forged
    # journal bytes are fed to SQLite. Version-only initialization must preserve
    # all bytes, then real app startup must recover the same project and token.
    for uid, mode in ((0, "WAL"), (1000, "WAL"), (1000, "PERSIST")):
        relative = f"existing-{uid}-{mode}"
        token = fixture(root, """
import json, os, shutil, sqlite3, subprocess, sys
from pathlib import Path
target = Path('/fixture') / sys.argv[1]
target.mkdir()
source = Path('/tmp/source.db')
subprocess.run(['takomo', '--db', str(source), 'project', 'create', '--id', 'preserved', '--name', 'Existing project'], check=True, capture_output=True)
result = subprocess.run(['takomo', '--db', str(source), 'token', 'create', '--actor', 'human:volume-test', '--scopes', 'read', '--projects', 'preserved', '--json'], check=True, capture_output=True, text=True)
connection = sqlite3.connect(source)
connection.execute('PRAGMA journal_mode=' + sys.argv[3])
connection.execute('CREATE TABLE volume_probe (value TEXT)')
connection.execute("INSERT INTO volume_probe VALUES ('committed before ownership migration')")
connection.commit()
for suffix in ('', '-wal', '-shm', '-journal'):
    path = Path(str(source) + suffix)
    if path.exists(): shutil.copyfile(path, target / ('takomo.db' + suffix))
connection.close()
(target / 'unrelated.txt').write_text('leave this file unchanged')
(target / 'other').mkdir()
(target / 'other' / 'nested.db').write_text('not the application database')
(target / '.takomo.db-litestream' / 'generation' / 'wal').mkdir(parents=True)
(target / '.takomo.db-litestream' / 'generation' / 'wal' / '00000000').write_bytes(b'backup metadata bytes')
for path in [target, *target.rglob('*')]:
    os.chown(path, int(sys.argv[2]), int(sys.argv[2]))
    path.chmod(0o755 if path.is_dir() else 0o640)
print(json.loads(result.stdout)['token'])
""", relative, str(uid), mode).strip()
        before = metadata(root, relative)
        expected = {"takomo.db", "takomo.db-wal", "takomo.db-shm"} if mode == "WAL" else {"takomo.db", "takomo.db-journal"}
        assert expected.issubset(before), "SQLite did not produce the expected real sidecars"
        initialize(root / relative)
        after = metadata(root, relative)
        for filename in expected:
            assert after[filename]["uid"] == 10001 and after[filename]["gid"] == expected_gid and after[filename]["mode"] == 0o600
            assert after[filename]["digest"] == before[filename]["digest"], f"initializer modified {filename} bytes"
        for filename in before:
            if filename.startswith('.takomo.db-litestream'):
                assert after[filename]['uid'] == 10001 and after[filename]['gid'] == expected_gid
                assert after[filename]['digest'] == before[filename]['digest']
        for filename in ("unrelated.txt", "other", "other/nested.db"):
            assert after[filename] == before[filename], f"initializer changed unrelated {filename}"
        serve(root / relative, token, "preserved")
        serve(root / relative, token, "preserved")
        # This committed record was in WAL before migration, proving recovery
        # preserved actual data rather than merely opening an empty database.
        fixture(root, """
import sqlite3, sys
connection = sqlite3.connect('/fixture/' + sys.argv[1] + '/takomo.db')
assert connection.execute('SELECT value FROM volume_probe').fetchone()[0] == 'committed before ownership migration'
""", relative)
        print(f"ok - UID {uid} {mode} database/sidecar bytes migrate, data/token survive restart", flush=True)

    fixture(root, """
import os, sqlite3
from pathlib import Path
p = Path('/fixture/selection'); p.mkdir()
for name in ('cli.db', 'environment.db', 'takomo.db'):
    c = sqlite3.connect(p / name)
    c.execute('CREATE TABLE untouched (value TEXT)'); c.commit(); c.close()
for path in [p, *p.iterdir()]: os.chown(path, 1000, 1000)
""")
    before = metadata(root, 'selection')
    docker('run', '--rm', '--mount', f'type=bind,source={root / "selection"},target=/var/data',
           '-e', 'TAKOMO_DB=/var/data/environment.db', IMAGE, '--db', 'cli.db', '--version')
    after = metadata(root, 'selection')
    assert after['cli.db']['uid'] == 10001
    assert after['cli.db']['digest'] == before['cli.db']['digest']
    assert after['environment.db'] == before['environment.db']
    assert after['takomo.db'] == before['takomo.db']
    docker('run', '--rm', '--mount', f'type=bind,source={root / "selection"},target=/var/data',
           IMAGE, '--db=/var/data/environment.db', '--version')
    assert metadata(root, 'selection')['environment.db']['uid'] == 10001
    print('ok - CLI DB selection overrides environment and leaves other databases untouched', flush=True)

    # Rejection must happen before mutating outside targets, including links to
    # another regular file on this same filesystem (hardlinks).
    for kind in ("symlink", "hardlink"):
        for filename in ("takomo.db", "takomo.db-wal", "takomo.db-shm", "takomo.db-journal", ".takomo.db-litestream/link"):
            relative = f"rejected-{kind}-{filename.replace('/', '-')}"
            fixture(root, """
import os, sys
from pathlib import Path
root = Path('/fixture')
target = root / sys.argv[1]
target.mkdir()
outside = root / (sys.argv[1] + '-outside')
outside.mkdir()
sentinel = outside / 'sentinel'
sentinel.write_text('external content must not change')
sentinel.chmod(0o640)
(target / sys.argv[3]).parent.mkdir(parents=True, exist_ok=True)
if sys.argv[2] == 'symlink': (target / sys.argv[3]).symlink_to('/fixture/' + outside.name + '/sentinel')
else: os.link(sentinel, target / sys.argv[3])
""", relative, kind, filename)
            baseline = metadata(root, relative + "-outside")
            inside = metadata(root, relative)
            result = docker("run", "--rm", "--mount", f"type=bind,source={root / relative},target=/var/data",
                            "--mount", f"type=bind,source={root},target=/fixture", IMAGE, "--version", check=False)
            assert result.returncode != 0, f"initializer accepted {kind} at {filename}"
            assert metadata(root, relative + "-outside") == baseline, "initializer mutated an outside link target"
            assert metadata(root, relative) == inside, "initializer partially changed rejected state"
    print("ok - DB and every sidecar reject symlinks/hardlinks without external mutation", flush=True)

    fixture(root, """
from pathlib import Path
p = Path('/fixture/nested-mount'); p.mkdir()
(p / '.takomo.db-litestream').mkdir()
outside = Path('/fixture/mounted-state'); outside.mkdir()
(outside / 'sentinel').write_text('nested mount must not change')
""")
    outside = metadata(root, 'mounted-state')
    inside = metadata(root, 'nested-mount')
    rejected = docker('run', '--rm', '--mount', f'type=bind,source={root / "nested-mount"},target=/var/data',
                      '--mount', f'type=bind,source={root / "mounted-state"},target=/var/data/.takomo.db-litestream',
                      IMAGE, '--version', check=False)
    assert rejected.returncode != 0, 'initializer crossed nested mount boundary'
    assert metadata(root, 'mounted-state') == outside
    assert metadata(root, 'nested-mount') == inside
    print('ok - nested Litestream mount refused without changing either mount', flush=True)



with tempfile.TemporaryDirectory(prefix=PREFIX + "-") as directory:
    root = Path(directory)
    try:
        main(root)
    except Exception:
        for name in CONTAINERS:
            logs = docker("logs", "--tail", "40", name, check=False, timeout=10)
            print(re.sub(r"tk_[A-Za-z0-9_-]+", "[REDACTED TOKEN]", (logs.stdout + logs.stderr)[-8000:]), file=sys.stderr)
        raise
    finally:
        for name in CONTAINERS:
            docker("rm", "-f", name, check=False, timeout=30)
        # The image deliberately changes fixture ownership away from the host
        # user. Remove only this test's children as root before tempfile cleanup.
        fixture(root, """
import shutil
from pathlib import Path
for path in Path('/fixture').iterdir():
    if path.is_dir() and not path.is_symlink(): shutil.rmtree(path)
    else: path.unlink()
""")
