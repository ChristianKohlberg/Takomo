#!/usr/bin/env python3
"""Check the app and, when bundled, both private renderer services."""
import json
from pathlib import Path
import sys
import urllib.request

try:
    state = json.loads(Path("/run/takomo-health.json").read_text())
    urls = [state["url"]]
    if state["bundled"]:
        urls += ["http://127.0.0.1:8000/health", "http://127.0.0.1:8002/health"]
    for url in urls:
        with urllib.request.build_opener(urllib.request.ProxyHandler({})).open(url, timeout=4) as response:
            if response.status != 200:
                sys.exit(1)
except (OSError, ValueError, KeyError):
    sys.exit(1)
