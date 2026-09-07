#!/usr/bin/env python3
"""Supervise the single-container distribution; renderer children get no app secrets."""
import json
import os
from pathlib import Path
import pwd
import signal
import stat
import subprocess
import sys
import time
import urllib.request

CHILDREN = []
STOPPING = False


def stop(_signum, _frame):
    global STOPPING
    STOPPING = True


def spawn(name, command, environment, directory):
    account = pwd.getpwnam(name)
    child = subprocess.Popen(
        ["setpriv", "--no-new-privs", f"--reuid={account.pw_uid}",
         f"--regid={account.pw_gid}", "--clear-groups", "--", *command],
        env=environment, cwd=directory, start_new_session=True, umask=0o077,
    )
    CHILDREN.append((name, child))
    print(f"Started {name} (pid {child.pid})", flush=True)


def healthy(url):
    try:
        with urllib.request.build_opener(urllib.request.ProxyHandler({})).open(url, timeout=2) as response:
            return response.status == 200
    except (OSError, ValueError):
        return False


def failed():
    for name, child in CHILDREN:
        if child.poll() is not None:
            print(f"{name} exited ({child.returncode}); stopping container", flush=True)
            return True
    return False


def group_alive(child):
    try:
        os.killpg(child.pid, 0)
        return True
    except ProcessLookupError:
        return False


def shutdown():
    # Children can leave grandchildren after exit; signal their groups anyway.
    for sig, seconds in ((signal.SIGTERM, 7), (signal.SIGKILL, 1)):
        for _, child in CHILDREN:
            try:
                os.killpg(child.pid, sig)
            except ProcessLookupError:
                pass
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            for _, child in CHILDREN:
                child.poll()  # Reap direct children before checking group liveness.
            if not any(group_alive(child) for _, child in CHILDREN):
                break
            time.sleep(0.1)
    for _, child in CHILDREN:
        child.wait()


def database_arguments(arguments, environment):
    """Normalize the one global DB flag before the entrypoint adds it back."""
    result = []
    selected = None
    index = 0
    while index < len(arguments):
        argument = arguments[index]
        if argument == "--":
            result.extend(arguments[index:])
            break
        if argument == "--db" or argument.startswith("--db="):
            if selected is not None:
                raise RuntimeError("Specify --db only once")
            if argument == "--db":
                index += 1
                if index == len(arguments) or arguments[index].startswith("--"):
                    raise RuntimeError("--db requires a path")
                selected = arguments[index]
            else:
                selected = argument.split("=", 1)[1]
            if not selected:
                raise RuntimeError("--db requires a path")
        else:
            result.append(argument)
        index += 1
    selected = selected if selected is not None else environment.get("TAKOMO_DB") or "/var/data/takomo.db"
    # Both serve and admin commands use the persistent directory as their base.
    environment["TAKOMO_DB"] = os.path.abspath(os.path.join("/var/data", selected))
    return result


def mount_id(descriptor):
    for line in Path(f"/proc/self/fdinfo/{descriptor}").read_text().splitlines():
        if line.startswith("mnt_id:"):
            return line.split()[1]
    raise RuntimeError("Cannot determine data mount boundary")


def prepare_data(database):
    """Repair only the dedicated mount, selected DB and its own backup metadata."""
    account = pwd.getpwnam("takomo")
    flags = os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK
    directory = os.open("/var/data", flags | os.O_DIRECTORY)
    descriptors = []
    metadata = None
    try:
        boundary = mount_id(directory)

        def validate(descriptor, is_directory):
            info = os.fstat(descriptor)
            if mount_id(descriptor) != boundary:
                raise RuntimeError("Refusing a nested mount in Takomo database state")
            if is_directory:
                valid = stat.S_ISDIR(info.st_mode)
            else:
                valid = stat.S_ISREG(info.st_mode) and info.st_nlink == 1
            if not valid:
                raise RuntimeError("Takomo database state must contain only directories and regular files without hard links")

        def own(descriptor, is_directory):
            os.fchown(descriptor, account.pw_uid, account.pw_gid)
            os.fchmod(descriptor, 0o700 if is_directory else 0o600)

        def metadata_tree(descriptor, mutate=False, depth=0):
            # Litestream 0.3 metadata is shallow; reject pathological trees.
            if depth > 16:
                raise RuntimeError("Unexpectedly deep Litestream metadata tree")
            validate(descriptor, True)
            for name in os.listdir(descriptor):
                info = os.stat(name, dir_fd=descriptor, follow_symlinks=False)
                is_directory = stat.S_ISDIR(info.st_mode)
                child = os.open(name, flags | (os.O_DIRECTORY if is_directory else 0), dir_fd=descriptor)
                try:
                    validate(child, is_directory)
                    if is_directory:
                        metadata_tree(child, mutate, depth + 1)
                    elif mutate:
                        own(child, False)
                finally:
                    os.close(child)
            if mutate:
                own(descriptor, True)

        path = Path(database)
        if path.parent == Path("/var/data"):
            for suffix in ("", "-wal", "-shm", "-journal"):
                try:
                    descriptor = os.open(path.name + suffix, flags, dir_fd=directory)
                except FileNotFoundError:
                    continue
                descriptors.append(descriptor)
                validate(descriptor, False)
            try:
                metadata = os.open(f".{path.name}-litestream", flags | os.O_DIRECTORY, dir_fd=directory)
            except FileNotFoundError:
                pass
            if metadata is not None:
                metadata_tree(metadata)  # Validate every target before changing ownership.
        own(directory, True)
        for descriptor in descriptors:
            own(descriptor, False)
        if metadata is not None:
            metadata_tree(metadata, mutate=True)
    finally:
        for descriptor in descriptors:
            os.close(descriptor)
        if metadata is not None:
            os.close(metadata)
        os.close(directory)


def main():
    if os.geteuid() != 0:
        raise RuntimeError("Bundled startup needs root to assign separate service users; application processes run non-root")
    os.umask(0o077)
    app_environment = dict(os.environ)
    arguments = database_arguments(sys.argv[1:], app_environment)
    prepare_data(app_environment["TAKOMO_DB"])
    app_environment["HOME"] = "/var/data"
    server = bool(arguments and arguments[0] == "serve")
    bundled = server and not app_environment.get("TAKOMO_KROKI_URL", "").strip()
    if not server:
        account = pwd.getpwnam("takomo")
        os.execvpe("setpriv", ["setpriv", "--no-new-privs", f"--reuid={account.pw_uid}",
                   f"--regid={account.pw_gid}", "--clear-groups", "--",
                   "/usr/local/bin/docker-entrypoint.sh", *arguments], app_environment)
    for sig in (signal.SIGTERM, signal.SIGINT):
        signal.signal(sig, stop)
    if bundled:
        app_environment["TAKOMO_KROKI_URL"] = "http://127.0.0.1:8000"
        # Reqwest respects proxy environment variables; the private renderer
        # must stay local even when the application uses an outbound proxy.
        exclusions = ",".join(filter(None, (app_environment.get("NO_PROXY"),
                                          app_environment.get("no_proxy"),
                                          "127.0.0.1,localhost,::1")))
        app_environment["NO_PROXY"] = exclusions
        app_environment["no_proxy"] = exclusions
        app_environment.setdefault("TAKOMO_KROKI_VERSION", "0.32.1-bundled-v1")
        # Explicit allowlist: no inherited database paths, cloud credentials,
        # proxy variables, Java options or user-provided renderer relaxations.
        common = {"PATH": "/usr/local/bin:/usr/bin:/bin", "LANG": "C.UTF-8"}
        mermaid = dict(common, HOME="/var/lib/mermaid", TMPDIR="/var/lib/mermaid",
                       XDG_CONFIG_HOME="/var/lib/mermaid/config", XDG_CACHE_HOME="/var/lib/mermaid/cache",
                       PUPPETEER_EXECUTABLE_PATH="/usr/bin/chromium",
                       KROKI_MERMAID_PAGE_URL="file:///opt/kroki/mermaid/assets/index.html",
                       KROKI_MERMAID_MAX_CONCURRENCY="4", KROKI_MERMAID_CONVERT_TIMEOUT="8000",
                       KROKI_MERMAID_PROTOCOL_TIMEOUT="8000", KROKI_MERMAID_MAX_TEXT_SIZE="50000",
                       KROKI_MAX_BODY_SIZE="64kb", LEVEL="info")
        spawn("mermaid", ["node", "--max-old-space-size=256", "/opt/kroki/mermaid/src/index.js"], mermaid, "/var/lib/mermaid")
        kroki = dict(common, HOME="/var/lib/kroki", TMPDIR="/var/lib/kroki",
                     KROKI_LISTEN="127.0.0.1:8000", KROKI_SAFE_MODE="SECURE",
                     KROKI_PLANTUML_SECURITY_PROFILE="SANDBOX", D2_BUNDLE="false",
                     KROKI_PLANTUML_BIN_PATH="/opt/plantuml/plantuml",
                     KROKI_MERMAID_HOST="127.0.0.1", KROKI_COMMAND_TIMEOUT="8s",
                     KROKI_CONVERT_TIMEOUT="8s", KROKI_DELEGATE_TIMEOUT_MS="9000",
                     KROKI_DELEGATE_MAX_POOL_SIZE="4")
        spawn("kroki", ["java", "-Xmx512m", "-XX:ActiveProcessorCount=2",
              "-Djava.io.tmpdir=/var/lib/kroki", "-Dlogback.configurationFile=/etc/kroki/logback.xml",
              "-jar", "/opt/kroki/kroki-server.jar"], kroki, "/var/lib/kroki")
        deadline = time.monotonic() + 60
        while not STOPPING:
            if failed():
                return 1
            if healthy("http://127.0.0.1:8000/health") and healthy("http://127.0.0.1:8002/health"):
                break
            if time.monotonic() >= deadline:
                print("Bundled renderers did not become ready within 60 seconds", flush=True)
                return 1
            time.sleep(0.2)
    if STOPPING:
        return 0
    bind = app_environment.get("TAKOMO_BIND", "127.0.0.1:8080")
    for index, argument in enumerate(arguments):
        if argument == "--bind" and index + 1 < len(arguments):
            bind = arguments[index + 1]
        elif argument.startswith("--bind="):
            bind = argument.split("=", 1)[1]
    bind = bind.replace("0.0.0.0:", "127.0.0.1:").replace("[::]:", "[::1]:")
    Path("/run/takomo-health.json").write_text(json.dumps({"url": f"http://{bind}/healthz", "bundled": bundled}))
    spawn("takomo", ["/usr/local/bin/docker-entrypoint.sh", *arguments], app_environment, "/var/data")
    while not STOPPING:
        if failed():
            return 1
        time.sleep(0.1)
    return 0


if __name__ == "__main__":
    try:
        result = main()
    except Exception as error:
        print(f"Container startup failed: {error}", file=sys.stderr, flush=True)
        result = 1
    finally:
        shutdown()
    sys.exit(result)
