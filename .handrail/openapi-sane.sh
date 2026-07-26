#!/usr/bin/env bash
# Instant, offline sanity check on the OpenAPI contract. Catches the two defect
# classes that actually bit this file — both of which parse cleanly, which is
# exactly why they survived unnoticed for so long:
#
#   1. A comma inside an unquoted description in a flow mapping. YAML reads it as
#      a separator, so `{ description: token id, e.g. "tok_1" }` silently becomes
#      description "token id" PLUS a junk key `e.g. "tok_1"`. The sentence you
#      wrote is gone and no parser objects.
#   2. `nullable: true` in a 3.1 document. It was removed in OpenAPI 3.1 (there
#      it is `type: [string, "null"]`), so every 3.1 tool ignores it and
#      generates a non-nullable client that breaks on the first real null.
#
# CI runs the full Redocly schema validation on top of this (see the `spec` job);
# this is the version fast enough for the inner loop. Missing PyYAML is a skip,
# not a red — a missing dev dependency is not a contract defect.
#
# Exit: 0 clean, 1 defect found, 2 nothing to check / cannot check.
set -uo pipefail
SPEC="${HR_SPEC:-spec/openapi.yaml}"
[ -f "$SPEC" ] || { echo "no $SPEC to check"; exit 2; }
python3 -c "import yaml" 2>/dev/null || {
  echo "skipped: python3 with PyYAML not available (CI still validates the spec)"
  exit 2
}
python3 - "$SPEC" <<'PY'
import sys, yaml

path = sys.argv[1]
try:
    with open(path, encoding="utf-8") as fh:
        doc = yaml.safe_load(fh)
except Exception as exc:
    print(f"{path} is not parseable YAML:\n  {exc}")
    sys.exit(1)

if not isinstance(doc, dict):
    print(f"{path} does not parse to a mapping")
    sys.exit(1)

problems = []
for key in ("openapi", "info", "paths"):
    if key not in doc:
        problems.append(f"missing top-level '{key}' — it parses, but it is not an OpenAPI document")
version = str(doc.get("openapi", ""))


def walk(node, trail=""):
    if isinstance(node, dict):
        for key, value in node.items():
            yield trail + "/" + str(key), key, value
            yield from walk(value, trail + "/" + str(key))
    elif isinstance(node, list):
        for index, value in enumerate(node):
            yield from walk(value, f"{trail}[{index}]")


# A key that carries no value and reads like prose is the comma artifact: the
# tail of a description that YAML split off into its own key. Deliberately
# conservative — a single-word fragment ("used") is indistinguishable from a
# legitimate `example: null`, so it is not flagged. Every *line* with the defect
# still trips this (a split sentence yields at least one multi-word fragment),
# and CI's Redocly pass catches whatever the heuristic leaves.
junk = [
    (trail, key)
    for trail, key, value in walk(doc)
    if value is None and isinstance(key, str) and (" " in key or key.endswith("."))
]
for trail, key in junk:
    problems.append(
        f"stray key {key!r} at {trail} — an unquoted description was split on a comma; "
        f"quote the whole description so the text survives"
    )

if version.startswith("3.1"):
    for trail, key, _ in walk(doc):
        if key == "nullable":
            problems.append(
                f"'nullable' at {trail} — removed in OpenAPI 3.1; write type: [<type>, \"null\"] "
                f"instead, or 3.1 tooling silently treats the field as non-nullable"
            )

if problems:
    print(f"{path}: {len(problems)} problem(s)")
    for problem in problems:
        print(f"  - {problem}")
    sys.exit(1)

print(f"ok — OpenAPI {version}, {len(doc['paths'])} paths, no stray keys")
PY
