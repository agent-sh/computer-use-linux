#!/bin/sh
# Contract check for the Claude Code and Codex plugin under plugins/.
# With --binary, also serves a fake release from that build and runs the MCP
# safety check through the plugin launcher, covering download, sha256
# verification, tamper rejection, and cache reuse.
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
plugin="$root/plugins/computer-use-linux"
launcher="$plugin/bin/computer-use-linux"
cd "$root"

fail() {
  echo "check_plugins: $*" >&2
  exit 1
}

cargo_version=$(grep -m1 '^version = ' Cargo.toml | sed 's/.*"\(.*\)".*/\1/')
launcher_version=$(sed -n 's/^version=//p' "$launcher")
[ "$launcher_version" = "$cargo_version" ] ||
  fail "launcher version $launcher_version != Cargo version $cargo_version"

for manifest in \
  .claude-plugin/marketplace.json \
  "$plugin/.claude-plugin/plugin.json" \
  "$plugin/.codex-plugin/plugin.json" \
  "$plugin/codex-mcp.json"; do
  python3 -m json.tool "$manifest" >/dev/null || fail "invalid JSON: $manifest"
done

python3 - "$cargo_version" <<'PY'
import json, sys
version = sys.argv[1]
checks = {
    "plugins/computer-use-linux/.claude-plugin/plugin.json": lambda d: d["version"],
    "plugins/computer-use-linux/.codex-plugin/plugin.json": lambda d: d["version"],
    ".claude-plugin/marketplace.json": lambda d: d["plugins"][0]["version"],
}
for path, get in checks.items():
    with open(path) as f:
        found = get(json.load(f))
    if found != version:
        sys.exit(f"check_plugins: {path} version {found} != Cargo version {version}")
PY

diff -r skills/computer-use-linux "$plugin/skills/computer-use-linux" >/dev/null ||
  fail "plugins/computer-use-linux/skills is out of sync; run: rm -rf plugins/computer-use-linux/skills/computer-use-linux && cp -r skills/computer-use-linux plugins/computer-use-linux/skills/"

sh -n "$launcher"
[ -x "$launcher" ] || fail "launcher is not executable"

if [ "${1:-}" != "--binary" ]; then
  echo "plugin manifests ok: version $cargo_version"
  exit 0
fi
binary=$2
helper="$(dirname -- "$binary")/computer-use-linux-cosmic"
indicator="$(dirname -- "$binary")/computer-use-linux-indicator"

case "$(uname -m)" in
  x86_64 | amd64) target=x86_64-unknown-linux-gnu ;;
  aarch64 | arm64) target=aarch64-unknown-linux-gnu ;;
  *) fail "unsupported CPU architecture: $(uname -m)" ;;
esac

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT INT TERM
mkdir "$tmp/release"
cp "$binary" "$tmp/release/computer-use-linux-$target"
cp "$helper" "$tmp/release/computer-use-linux-cosmic-$target"
cp "$indicator" "$tmp/release/computer-use-linux-indicator-$target"
(cd "$tmp/release" && for asset in *; do sha256sum "$asset" >"$asset.sha256"; done)

export XDG_CACHE_HOME="$tmp/cache"
export COMPUTER_USE_LINUX_DOWNLOAD_BASE="file://$tmp/release"
unset COMPUTER_USE_LINUX_BIN COMPUTER_USE_LINUX_COSMIC_HELPER COMPUTER_USE_LINUX_INDICATOR_BIN

good_sha=$(cat "$tmp/release/computer-use-linux-cosmic-$target.sha256")
echo "0000000000000000000000000000000000000000000000000000000000000000  x" \
  >"$tmp/release/computer-use-linux-cosmic-$target.sha256"
if "$launcher" windows >/dev/null 2>"$tmp/tamper.err"; then
  fail "launcher accepted a tampered sha256"
fi
grep -q 'sha256 mismatch' "$tmp/tamper.err" ||
  fail "tampered download failed for another reason: $(cat "$tmp/tamper.err")"
[ ! -e "$tmp/cache/computer-use-linux/plugin/v$cargo_version/computer-use-linux" ] ||
  fail "tampered download reached the cache"
echo "$good_sha" >"$tmp/release/computer-use-linux-cosmic-$target.sha256"

scripts/mcp_safety_check.py --binary "$launcher"

export COMPUTER_USE_LINUX_DOWNLOAD_BASE="file://$tmp/missing"
scripts/mcp_safety_check.py --binary "$launcher" >/dev/null ||
  fail "launcher did not reuse the cached binaries"
stale="$tmp/cache/computer-use-linux/plugin/v0.0.0"
recent="$tmp/cache/computer-use-linux/plugin/v0.0.1"
staging="$tmp/cache/computer-use-linux/plugin/.download.killed"
mkdir -p "$stale" "$recent" "$staging"
touch -d '40 days ago' "$stale"
touch -d '2 hours ago' "$staging"
scripts/mcp_safety_check.py --binary "$launcher" >/dev/null
[ ! -e "$stale" ] || fail "launcher kept a version unused for 40 days"
[ -e "$recent" ] || fail "launcher pruned a recently used version"
[ ! -e "$staging" ] || fail "launcher kept a stale download staging dir"
echo "plugin launcher ok: download, tamper rejection, cache reuse, pruning"
