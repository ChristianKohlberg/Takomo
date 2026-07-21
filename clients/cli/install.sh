#!/usr/bin/env sh
# install.sh — put the takomo `takomo` CLI on your PATH.
#
# Two ways to run it:
#
#   # from a checkout (symlinks the repo copy so `git pull` updates takomo):
#   ./clients/cli/install.sh
#
#   # standalone (downloads takomo from GitHub):
#   curl -fsSL https://raw.githubusercontent.com/ChristianKohlberg/Takomo/main/clients/cli/install.sh | sh
#
# Env knobs:
#   TAKOMO_BIN_DIR   install dir            (default: ~/.local/bin)
#   TAKOMO_REF       git ref to fetch from  (default: main)
#   TAKOMO_CLI_URL   full raw URL to takomo (overrides REF)
#
# It installs the `takomo` script (plus a short `tk` alias symlink), and checks
# that its runtime deps (curl, python3) are present.
set -eu

REPO_SLUG="ChristianKohlberg/Takomo"
BIN_DIR="${TAKOMO_BIN_DIR:-$HOME/.local/bin}"
REF="${TAKOMO_REF:-main}"
CLI_URL="${TAKOMO_CLI_URL:-https://raw.githubusercontent.com/${REPO_SLUG}/${REF}/clients/cli/takomo}"
DEST="$BIN_DIR/takomo"
ALIAS="$BIN_DIR/tk"

say()  { printf '%s\n' "$*"; }
warn() { printf '%s\n' "$*" >&2; }
die()  { printf 'install: %s\n' "$*" >&2; exit 1; }

# 1. Runtime dependency check — takomo is bash + curl + python3.
missing=""
command -v curl    >/dev/null 2>&1 || missing="$missing curl"
command -v python3 >/dev/null 2>&1 || missing="$missing python3"
command -v bash    >/dev/null 2>&1 || missing="$missing bash"
if [ -n "$missing" ]; then
  die "missing required tool(s):$missing — install them first, then re-run."
fi

mkdir -p "$BIN_DIR" || die "cannot create $BIN_DIR"

# 2. Prefer a local checkout copy (symlink so pulls propagate); otherwise fetch.
SCRIPT_DIR=$( { CDPATH='' cd -- "$(dirname -- "$0")" 2>/dev/null && pwd; } || true )
LOCAL_TS=""
[ -n "$SCRIPT_DIR" ] && [ -f "$SCRIPT_DIR/takomo" ] && LOCAL_TS="$SCRIPT_DIR/takomo"

if [ -n "$LOCAL_TS" ]; then
  ln -sf "$LOCAL_TS" "$DEST"
  say "linked $DEST -> $LOCAL_TS"
else
  say "downloading takomo from $CLI_URL"
  tmp="$(mktemp)"
  curl -fsSL "$CLI_URL" -o "$tmp" || die "download failed from $CLI_URL"
  # Sanity-check we got the script, not an HTML error page.
  head -n1 "$tmp" | grep -q '^#!/usr/bin/env bash' || die "downloaded file is not the takomo script (check TAKOMO_REF/URL)"
  mv "$tmp" "$DEST"
  say "installed $DEST"
fi
chmod +x "$DEST" 2>/dev/null || true

# Short `tk` alias pointing at the installed `takomo`.
ln -sf "takomo" "$ALIAS"
say "linked $ALIAS -> takomo"

# 3. PATH hint.
case ":$PATH:" in
  *":$BIN_DIR:"*) : ;;
  *) warn ""
     warn "note: $BIN_DIR is not on your PATH. Add this to your shell profile:"
     warn "    export PATH=\"$BIN_DIR:\$PATH\"" ;;
esac

say ""
say "Done. Try:  takomo help"
say "Then onboard a repo:  TAKOMO_URL=... TAKOMO_TOKEN=tk_<admin> takomo init"
