#!/usr/bin/env python3
"""Exercise the shipping image with disposable containers and a persistent volume."""
import json
import re
import subprocess
import sys
import time
import tempfile
from pathlib import Path
import urllib.error
import urllib.request
import uuid


IMAGE = sys.argv[1] if len(sys.argv) > 1 else "takomo-ci"
PREFIX = f"takomo-smoke-{uuid.uuid4().hex[:10]}"
VOLUME = f"{PREFIX}-data"
CONTAINERS = []


def redact(text):
    return re.sub(r"tk_[A-Za-z0-9_-]+", "[REDACTED TOKEN]", text)


def docker(*arguments, timeout=120):
    result = subprocess.run(["docker", *arguments], capture_output=True, text=True,
                            timeout=timeout, check=False)
    if result.returncode:
        # Do not echo command output: admin commands may return credentials.
        detail = "" if "token" in arguments else redact(result.stderr[-2000:])
        raise AssertionError(f"docker {arguments[0]} failed ({result.returncode}): {detail}")
    return result.stdout.strip()


def inspect(name):
    return json.loads(docker("inspect", name))[0]


def start(suffix, *extra):
    name = f"{PREFIX}-{suffix}"
    CONTAINERS.append(name)
    docker("run", "-d", "--name", name, "--memory", "3g", "--cpus", "2", "--pids-limit", "512", "--mount", f"source={VOLUME},target=/var/data",
           "-p", "127.0.0.1::8080", "-e", "TAKOMO_TEST_SECRET=must-not-reach-renderers",
           *extra, IMAGE)
    port = inspect(name)["NetworkSettings"]["Ports"]["8080/tcp"][0]["HostPort"]
    return name, f"http://127.0.0.1:{port}"


def request(base, path, token=None, body=None):
    headers = {"Content-Type": "application/json"}
    if token:
        headers["Authorization"] = f"Bearer {token}"
    req = urllib.request.Request(base + path, headers=headers,
                                 data=None if body is None else json.dumps(body).encode())
    try:
        with urllib.request.urlopen(req, timeout=20) as response:
            return response.status, response.read().decode()
    except urllib.error.HTTPError as error:
        return error.code, error.read().decode()


def ready(name, base):
    deadline = time.monotonic() + 90
    while time.monotonic() < deadline:
        assert inspect(name)["State"]["Running"], "container exited during startup"
        try:
            if request(base, "/healthz")[0] == 200:
                docker("exec", name, "python3", "/usr/local/lib/takomo/container-healthcheck.py")
                return
        except (OSError, AssertionError):
            pass
        time.sleep(0.5)
    raise AssertionError("bundled health did not become ready within 90 seconds")


def mint(name, scopes):
    return json.loads(docker("exec", "--user", "takomo", name, "takomo", "--db", "/var/data/takomo.db",
                            "token", "create", "--actor", "human:container-smoke",
                            "--scopes", scopes, "--projects", "smoke", "--json"))["token"]


def assert_render(base, token, engine, source):
    status, raw = request(base, "/v1/diagrams/render", token,
                          {"project": "smoke", "engine": engine, "source": source})
    assert status == 200, f"{engine} rendering returned {status}: {raw}"
    svg = json.loads(raw)["svg"]
    assert "<svg" in svg and "</svg>" in svg, f"{engine} returned no complete SVG"


def stop(name):
    docker("stop", "--time", "15", name, timeout=25)
    state = inspect(name)["State"]
    assert state["ExitCode"] == 0, f"graceful stop exited {state['ExitCode']}"
    assert not state["Running"]


def main():
    global IMAGE
    IMAGE = docker("image", "inspect", "--format", "{{.Id}}", IMAGE)
    print(f"Testing image {IMAGE}", flush=True)
    docker("volume", "create", VOLUME)
    name, base = start("bundled", "-e", "HTTP_PROXY=http://127.0.0.1:9",
                       "-e", "HTTPS_PROXY=http://127.0.0.1:9", "-e", "NO_PROXY=")
    ready(name, base)
    # Exercise the actual entrypoint's admin path on a separate short-lived
    # container: it must finish without starting renderer helpers.
    admin_container = f"{PREFIX}-admin"
    CONTAINERS.append(admin_container)
    admin_output = docker("run", "--rm", "--name", admin_container, "--mount", f"source={VOLUME},target=/var/data",
                          IMAGE, "token", "create", "--actor", "human:container-smoke",
                          "--scopes", "read,write,human,admin", "--projects", "*", "--json")
    admin = json.loads(admin_output)["token"]
    assert "Started " not in admin_output, "admin command launched service helpers"
    status, _ = request(base, "/v1/projects", admin, {"id": "smoke", "name": PREFIX})
    assert status == 201, "could not create project through bundled app"
    for page in ("board", "epics", "lanes", "inbox", "initiatives", "schedules", "verification", "environments"):
        status, body = request(base, "/" + page)
        assert status == 200 and 'id="root"' in body, f"/{page} did not serve the web build"
    for asset in ("app.js", "vendor.js", "runtime.js", "app.css"):
        status, body = request(base, "/assets/" + asset)
        assert status == 200 and body, f"/assets/{asset} did not serve"
    reader = mint(name, "read")
    assert request(base, "/v1/diagrams/render", body={})[0] == 401
    status, _ = request(base, "/v1/diagrams/render", reader,
                        {"project": "outside", "engine": "d2", "source": "a -> b"})
    assert status == 403, "rendering did not enforce project access"
    for engine, source in (
        ("plantuml", "@startsalt\n{ Codex integration | [Connect worker] }\n@endsalt"),
        ("mermaid", "flowchart LR\nTakomo --> Kroki"),
        ("d2", "takomo -> kroki: render"),
    ):
        assert_render(base, reader, engine, source)
    print("ok - default image health and authenticated Salt/Mermaid/D2 rendering", flush=True)

    # Renderer users must not inherit operator secrets or read the database.
    docker("exec", name, "python3", "-c", """
from pathlib import Path
import ipaddress
ports = {}
for table in ('tcp', 'tcp6'):
    for line in Path('/proc/net/' + table).read_text().splitlines()[1:]:
        fields = line.split()
        if fields[3] != '0A': continue
        address, port = fields[1].split(':')
        port = int(port, 16)
        if port not in (8000, 8002): continue
        raw = bytes.fromhex(address)
        address = ipaddress.ip_address(b''.join(raw[i:i+4][::-1] for i in range(0, len(raw), 4)))
        address = getattr(address, 'ipv4_mapped', None) or address
        assert address.is_loopback, (port, str(address))
        ports[port] = str(address)
assert set(ports) == {8000, 8002}, ports
found = set()
for process in Path('/proc').iterdir():
    if not process.name.isdigit(): continue
    try:
        uid = process.stat().st_uid
        if uid not in (10002, 10003): continue
        found.add(uid)
    except FileNotFoundError:
        pass
assert found == {10002, 10003}, found
""")
    for uid in ("10002", "10003"):
        docker("exec", "--user", uid, name, "python3", "-c", """
import os
from pathlib import Path
checked = False
for process in Path('/proc').iterdir():
    if not process.name.isdigit() or process.stat().st_uid != os.getuid(): continue
    command = (process / 'cmdline').read_bytes()
    if not command.startswith((b'java\\x00', b'node\\x00')): continue
    assert b'TAKOMO_TEST_SECRET=' not in (process / 'environ').read_bytes()
    checked = True
assert checked, 'renderer environment was not checked'
try:
    open('/var/data/takomo.db', 'rb')
except PermissionError:
    pass
else:
    raise AssertionError('renderer can read application database')
""")
    stop(name)
    docker("rm", name)
    name, base = start("recreated")
    ready(name, base)
    status, projects = request(base, "/v1/projects", admin)
    assert status == 200 and any(p["name"] == PREFIX for p in json.loads(projects)), \
        "project or token did not survive container recreation"
    assert_render(base, reader, "plantuml", "@startuml\nAlice -> Bob: after restart\n@enduml")
    print("ok - graceful shutdown and volume persistence across container recreation", flush=True)

    # A helper dying must stop the app rather than leave a healthy-looking
    # container whose diagram feature has silently stopped working.
    for uid, marker, label in ((10003, '/opt/kroki/mermaid/src/index.js', 'Mermaid'),
                               (10002, '/opt/kroki/kroki-server.jar', 'Kroki')):
        if uid == 10002:
            name, base = start("kroki-failure")
            ready(name, base)
        docker("exec", name, "python3", "-c", """
import os, signal, sys
from pathlib import Path
for process in Path('/proc').iterdir():
    if not process.name.isdigit(): continue
    try:
        if process.stat().st_uid == int(sys.argv[1]) and sys.argv[2].encode() in (process / 'cmdline').read_bytes():
            os.kill(int(process.name), signal.SIGKILL)
            break
    except FileNotFoundError:
        pass
else:
    raise AssertionError('renderer service was not running')
""", str(uid), marker)
        exit_code = int(docker("wait", name, timeout=20))
        assert exit_code != 0, f"{label} failure did not fail the container"
        print(f"ok - {label} failure stops the whole container", flush=True)

    external, external_base = start("external", "-e", "TAKOMO_KROKI_URL=http://127.0.0.1:9")
    ready(external, external_base)
    docker("exec", external, "python3", "-c", """
from pathlib import Path
assert not any(p.name.isdigit() and p.stat().st_uid in (10002, 10003) for p in Path('/proc').iterdir())
""")
    status, raw = request(external_base, "/v1/diagrams/render", reader,
                          {"project": "smoke", "engine": "d2", "source": "a -> b"})
    assert status == 502 and json.loads(raw)["code"] == "diagram.unavailable"
    stop(external)
    print("ok - external override skips bundled helpers and reports upstream failure", flush=True)

    # This checks the existing Litestream orchestration without cloud credentials
    # or a real backup upload. The replacement records both phases then execs the
    # real Takomo command, so shutdown still exercises the supervisor's app group.
    with tempfile.TemporaryDirectory(prefix="takomo-litestream-smoke-") as fixture:
        stub = Path(fixture) / "litestream"
        stub.write_text("""#!/usr/bin/env python3
import os, shlex, sys
from pathlib import Path
with Path('/var/data/litestream-smoke.log').open('a') as record:
    record.write(sys.argv[1] + '\\n')
if sys.argv[1] == 'replicate':
    command = shlex.split(sys.argv[sys.argv.index('-exec') + 1])
    os.execvp(command[0], command)
elif sys.argv[1] != 'restore':
    raise SystemExit(2)
""")
        stub.chmod(0o755)
        backup, backup_base = start("litestream", "-e", "LITESTREAM_BUCKET=smoke-only",
                                    "--mount", f"type=bind,source={stub},target=/usr/local/bin/litestream,readonly")
        ready(backup, backup_base)
        phases = docker("exec", backup, "cat", "/var/data/litestream-smoke.log").splitlines()
        assert phases == ["restore", "replicate"], "Litestream restore/replicate was not invoked"
        assert_render(backup_base, reader, "d2", "backup -> app: supervised")
        stop(backup)
        print("ok - Litestream startup and shutdown orchestration (local stub, no upload)", flush=True)


try:
    main()
except Exception:
    # Diagnose only long-running services, never the admin command's output or
    # Docker's Config/Env fields. Bound logs and redact credential-shaped text.
    for container in CONTAINERS:
        if container.endswith("-admin"):
            continue
        state = subprocess.run(["docker", "inspect", "--format",
                                "status={{.State.Status}} exit={{.State.ExitCode}}", container],
                               capture_output=True, text=True, timeout=10, check=False)
        if state.returncode:
            continue
        print(f"{container}: {state.stdout.strip()}", file=sys.stderr)
        logs = subprocess.run(["docker", "logs", "--tail", "60", container],
                              capture_output=True, text=True, timeout=10, check=False)
        print(redact((logs.stdout + logs.stderr)[-12000:]), file=sys.stderr)
    raise
finally:
    for container in CONTAINERS:
        subprocess.run(["docker", "rm", "-f", container], stdout=subprocess.DEVNULL,
                       stderr=subprocess.DEVNULL, timeout=30, check=False)
    subprocess.run(["docker", "volume", "rm", VOLUME], stdout=subprocess.DEVNULL,
                   stderr=subprocess.DEVNULL, timeout=30, check=False)
