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
# The plugin points at a published release, which can trail the crate while a
# new release publishes (scripts/bump_plugin_version.sh moves it).
launcher_version=$(sed -n 's/^version=//p' "$launcher")

for manifest in \
  .claude-plugin/marketplace.json \
  "$plugin/.claude-plugin/plugin.json" \
  "$plugin/.codex-plugin/plugin.json" \
  "$plugin/codex-mcp.json"; do
  python3 -m json.tool "$manifest" >/dev/null || fail "invalid JSON: $manifest"
done

python3 - "$launcher_version" "$cargo_version" <<'PY'
import json, re, sys
version, crate = sys.argv[1:]
if not re.fullmatch(r"\d+\.\d+\.\d+", version):
    sys.exit(f"check_plugins: launcher version {version!r} is not X.Y.Z")
if tuple(map(int, version.split("."))) > tuple(map(int, crate.split("."))):
    sys.exit(f"check_plugins: plugin version {version} is newer than the crate version {crate}")
checks = {
    "plugins/computer-use-linux/.claude-plugin/plugin.json": lambda d: d["version"],
    "plugins/computer-use-linux/.codex-plugin/plugin.json": lambda d: d["version"],
    ".claude-plugin/marketplace.json": lambda d: d["plugins"][0]["version"],
}
for path, get in checks.items():
    with open(path) as f:
        found = get(json.load(f))
    if found != version:
        sys.exit(f"check_plugins: {path} version {found} != launcher version {version}")
PY

diff -r skills/computer-use-linux "$plugin/skills/computer-use-linux" >/dev/null ||
  fail "plugins/computer-use-linux/skills is out of sync; run: rm -rf plugins/computer-use-linux/skills/computer-use-linux && cp -r skills/computer-use-linux plugins/computer-use-linux/skills/"

sh -n "$launcher"
[ -x "$launcher" ] || fail "launcher is not executable"

# Codex and Pi pass the server only the variables they list.
python3 scripts/check_env_forwarding.py

if [ "${1:-}" != "--binary" ]; then
  echo "plugin manifests ok: plugin version $launcher_version, crate version $cargo_version"
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

plugin_cache="$tmp/cache/computer-use-linux/plugin"
pinned="$plugin_cache/v$launcher_version"
fetch_log="$plugin_cache/.partial-v$launcher_version/fetch.log"

# True while a background download holds its version's lock.
fetchers_busy() {
  for held in "$plugin_cache"/.partial-v*.flock; do
    [ -e "$held" ] || continue
    flock -s -n "$held" true || return 0
  done
  return 1
}

# Waits, at most 60 seconds, for the launcher's background downloads to end.
wait_fetchers() {
  i=0
  while fetchers_busy; do
    i=$((i + 1))
    [ "$i" -le 600 ] || fail "a background download did not finish"
    sleep 0.1
  done
}

# Serves $tmp/release over HTTP at RATE bytes per second (0: no cap), logging
# requests to LOG, and points the launcher at it. A third argument passes on a
# server flag such as --no-ranges.
server_pid=
start_server() {
  stop_server
  rm -f "$tmp/port"
  python3 scripts/release_fixture_server.py --dir "$tmp/release" \
    --port-file "$tmp/port" --log "$2" --rate "$1" ${3:+"$3"} &
  server_pid=$!
  i=0
  until [ -s "$tmp/port" ]; do
    i=$((i + 1))
    [ "$i" -le 100 ] || fail "release fixture server did not start"
    sleep 0.1
  done
  COMPUTER_USE_LINUX_DOWNLOAD_BASE="http://127.0.0.1:$(cat "$tmp/port")"
}
stop_server() {
  if [ -n "$server_pid" ]; then
    kill "$server_pid" 2>/dev/null || true
    wait "$server_pid" 2>/dev/null || true
    server_pid=
  fi
}
trap 'stop_server; rm -rf "$tmp"' EXIT INT TERM

# Fills an older version's cache, as an earlier plugin release would have.
seed_older() {
  mkdir -p "$plugin_cache/v0.0.9"
  cp "$binary" "$plugin_cache/v0.0.9/computer-use-linux"
  cp "$helper" "$plugin_cache/v0.0.9/computer-use-linux-cosmic"
  cp "$indicator" "$plugin_cache/v0.0.9/computer-use-linux-indicator"
}

main_asset="$tmp/release/computer-use-linux-$target"
helper_sha="$tmp/release/computer-use-linux-cosmic-$target.sha256"
good_main_sha=$(cat "$main_asset.sha256")
good_helper_sha=$(cat "$helper_sha")
bad_sha="0000000000000000000000000000000000000000000000000000000000000000  x"

# A tampered main binary never reaches the cache, and the launcher fails.
echo "$bad_sha" >"$main_asset.sha256"
if "$launcher" --help >/dev/null 2>"$tmp/tamper.err"; then
  fail "launcher accepted a tampered sha256"
fi
grep -q 'sha256 mismatch' "$tmp/tamper.err" ||
  fail "tampered download failed for another reason: $(cat "$tmp/tamper.err")"
wait_fetchers
[ ! -e "$pinned/computer-use-linux" ] || fail "tampered download reached the cache"
echo "$good_main_sha" >"$main_asset.sha256"

# The server starts once its own binary is verified; a helper that fails its
# sha256 stays out of the cache.
# The MCP safety check runs part of its checks with a PATH that has only what
# the launcher needs for a complete cache, so it runs once the cache is whole.
echo "$bad_sha" >"$helper_sha"
"$launcher" --help >/dev/null 2>"$tmp/helper.err" ||
  fail "launcher did not start while a helper failed its sha256: $(cat "$tmp/helper.err")"
wait_fetchers
[ -x "$pinned/computer-use-linux" ] || fail "launcher did not cache the main binary"
[ ! -e "$pinned/computer-use-linux-cosmic" ] || fail "tampered helper reached the cache"
[ -x "$pinned/computer-use-linux-indicator" ] || fail "a failed helper held back the indicator"
grep -q 'sha256 mismatch' "$fetch_log" || fail "helper mismatch was not logged"
echo "$good_helper_sha" >"$helper_sha"

# The next start runs at once and fetches the missing helpers behind it.
"$launcher" --help >/dev/null 2>"$tmp/helpers.err" ||
  fail "launcher did not start with only its main binary cached: $(cat "$tmp/helpers.err")"
wait_fetchers
for name in computer-use-linux-cosmic computer-use-linux-indicator; do
  [ -x "$pinned/$name" ] || fail "background download did not cache $name"
done
[ ! -e "$plugin_cache/.partial-v$launcher_version" ] || fail "launcher kept a finished partial dir"
scripts/mcp_safety_check.py --binary "$launcher"

export COMPUTER_USE_LINUX_DOWNLOAD_BASE="file://$tmp/missing"
scripts/mcp_safety_check.py --binary "$launcher" >/dev/null ||
  fail "launcher did not reuse the cached binaries"
[ ! -e "$plugin_cache/.partial-v$launcher_version" ] ||
  fail "launcher started a download for a complete cache"
stale="$plugin_cache/v0.0.0"
recent="$plugin_cache/v0.0.1"
staging="$plugin_cache/.download.killed"
stale_partial="$plugin_cache/.partial-v0.0.0"
mkdir -p "$stale" "$recent" "$staging" "$stale_partial"
touch -d '40 days ago' "$stale" "$stale_partial"
touch -d '2 hours ago' "$staging"
scripts/mcp_safety_check.py --binary "$launcher" >/dev/null
[ ! -e "$stale" ] || fail "launcher kept a version unused for 40 days"
[ -e "$recent" ] || fail "launcher pruned a recently used version"
[ ! -e "$staging" ] || fail "launcher kept a stale download staging dir"
[ ! -e "$stale_partial" ] || fail "launcher kept a partial download abandoned for 40 days"

# A pinned release that is not published (404) runs the newest cached
# version, even one unused for 40 days, instead of failing.
rm -rf "$plugin_cache"
seed_older
mkdir -p "$plugin_cache/v0.0.3"
cp "$binary" "$plugin_cache/v0.0.3/computer-use-linux"
touch -d '40 days ago' "$plugin_cache/v0.0.9"
"$launcher" --help >/dev/null 2>"$tmp/missing.err" ||
  fail "launcher failed on an unpublished release with a cache present: $(cat "$tmp/missing.err")"
grep -q 'starting the cached v0.0.9' "$tmp/missing.err" ||
  fail "launcher did not start the newest cached version: $(cat "$tmp/missing.err")"
wait_fetchers
[ -d "$plugin_cache/v0.0.9" ] || fail "launcher pruned the version it fell back to"

# A slow link: the cached version starts within the wait, the download goes
# on after the launcher has exec'd, and the next start runs the new version.
size=$(wc -c <"$main_asset")
start_server $((size / 6)) "$tmp/slow.log"
export COMPUTER_USE_LINUX_DOWNLOAD_BASE
COMPUTER_USE_LINUX_PLUGIN_UPDATE_WAIT=1 "$launcher" --help >/dev/null 2>"$tmp/slow.err" ||
  fail "launcher failed on a slow link: $(cat "$tmp/slow.err")"
grep -q 'still downloading; starting the cached v0.0.9' "$tmp/slow.err" ||
  fail "launcher waited for a slow download: $(cat "$tmp/slow.err")"
[ ! -x "$pinned/computer-use-linux" ] || fail "slow download finished too early to test"
wait_fetchers
[ -x "$pinned/computer-use-linux" ] || fail "background download did not finish: $(cat "$fetch_log" 2>/dev/null)"
"$launcher" --help >/dev/null 2>"$tmp/after.err" ||
  fail "launcher failed after the download: $(cat "$tmp/after.err")"
! grep -q 'starting the cached' "$tmp/after.err" ||
  fail "launcher kept the cached version after the download: $(cat "$tmp/after.err")"

# A download killed part way, launcher and fetcher both, resumes from the
# partial file at the next start.
rm -rf "$plugin_cache"
start_server $((size / 20)) "$tmp/killed.log"
"$launcher" --help >/dev/null 2>"$tmp/killed.err" &
launcher_pid=$!
part="$plugin_cache/.partial-v$launcher_version/computer-use-linux.part"
i=0
until [ -s "$part" ] && [ "$(wc -c <"$part")" -gt $((size / 10)) ]; do
  i=$((i + 1))
  [ "$i" -le 100 ] || fail "slow download did not start"
  sleep 0.1
done
kill -9 "$launcher_pid"
wait "$launcher_pid" 2>/dev/null || true
fetchers_busy || fail "the download did not outlive the launcher"
# The fetcher leads its own session; kill its download with it.
kill -9 "-$(cat "$plugin_cache/.partial-v$launcher_version/fetch.pid")"
i=0
while fetchers_busy; do
  i=$((i + 1))
  [ "$i" -le 100 ] || fail "fetcher did not exit"
  sleep 0.1
done
offset=$(wc -c <"$part")
start_server 0 "$tmp/resume.log"
"$launcher" --help >/dev/null 2>"$tmp/resume.err" ||
  fail "launcher did not start after a killed download: $(cat "$tmp/resume.err")"
grep -q "^GET /computer-use-linux-$target bytes=$offset-" "$tmp/resume.log" ||
  fail "download did not resume at byte $offset: $(cat "$tmp/resume.log")"
wait_fetchers
cmp -s "$pinned/computer-use-linux" "$main_asset" || fail "resumed binary differs from the release"
# Launchers started together share one download.
rm -rf "$plugin_cache"
start_server $((size / 4)) "$tmp/concurrent.log"
pids=
for n in 1 2 3 4 5 6; do
  "$launcher" --help >/dev/null 2>"$tmp/concurrent.$n.err" &
  pids="$pids $!"
done
for pid in $pids; do
  wait "$pid" || fail "a concurrent start failed: $(cat "$tmp"/concurrent.*.err)"
done
wait_fetchers
[ "$(grep -c "^GET /computer-use-linux-$target " "$tmp/concurrent.log")" = 1 ] ||
  fail "concurrent starts downloaded more than once: $(cat "$tmp/concurrent.log")"
cmp -s "$pinned/computer-use-linux" "$main_asset" || fail "concurrent download differs from the release"
for name in computer-use-linux-cosmic computer-use-linux-indicator; do
  [ -x "$pinned/$name" ] || fail "concurrent starts did not cache $name"
done

# A partial file the server cannot continue starts over: one longer than the
# asset (416), and one for a server that ignores Range.
for mode in oversized no-ranges; do
  rm -rf "$plugin_cache"
  mkdir -p "$plugin_cache/.partial-v$launcher_version"
  if [ "$mode" = oversized ]; then
    { cat "$main_asset"; echo garbage; } >"$part"
    start_server 0 "$tmp/$mode.log"
  else
    head -c $((size / 2)) "$main_asset" >"$part"
    start_server 0 "$tmp/$mode.log" --no-ranges
  fi
  "$launcher" --help >/dev/null 2>"$tmp/$mode.err" ||
    fail "launcher did not recover from a $mode partial: $(cat "$tmp/$mode.err")"
  wait_fetchers
  cmp -s "$pinned/computer-use-linux" "$main_asset" || fail "$mode partial produced a wrong binary"
done
stop_server
echo "plugin launcher ok: download, tamper rejection, background helpers, cache reuse, pruning, unpublished-release fallback, slow link, resume, concurrent starts, unresumable partials"
