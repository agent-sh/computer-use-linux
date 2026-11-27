#!/bin/sh
# Points the Claude Code and Codex plugin at a published release: the
# marketplace entry, both plugin manifests and the launcher's pinned version.
# Refuses until every asset the launcher downloads, and its .sha256, is
# published, so main never names a version that cannot be downloaded.
set -eu

if [ $# -ne 1 ] || ! echo "$1" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$'; then
  echo "usage: scripts/bump_plugin_version.sh X.Y.Z" >&2
  exit 64
fi
new=$1
root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"

crate=$(grep -m1 '^version = ' Cargo.toml | sed 's/.*"\(.*\)".*/\1/')
python3 - "$new" "$crate" <<'PY'
import sys
new, crate = (tuple(map(int, v.split("."))) for v in sys.argv[1:])
if new > crate:
    sys.exit(f"bump_plugin_version: {sys.argv[1]} is newer than the crate version {sys.argv[2]}")
PY
scripts/check_release_published.sh "$new"

python3 - "$new" <<'PY'
import json, pathlib, re, sys
version = sys.argv[1]
fields = {
    ".claude-plugin/marketplace.json": lambda d: d["plugins"][0]["version"],
    "plugins/computer-use-linux/.claude-plugin/plugin.json": lambda d: d["version"],
    "plugins/computer-use-linux/.codex-plugin/plugin.json": lambda d: d["version"],
}
for path, get in fields.items():
    file = pathlib.Path(path)
    text, count = re.subn(r'("version":\s*")[^"]*(")', rf"\g<1>{version}\g<2>", file.read_text(), count=1)
    if count != 1 or get(json.loads(text)) != version:
        sys.exit(f"bump_plugin_version: could not set the version in {path}")
    file.write_text(text)
PY
sed "s/^version=.*/version=$new/" plugins/computer-use-linux/bin/computer-use-linux >plugins/computer-use-linux/bin/computer-use-linux.new
chmod 0755 plugins/computer-use-linux/bin/computer-use-linux.new
mv plugins/computer-use-linux/bin/computer-use-linux.new plugins/computer-use-linux/bin/computer-use-linux
scripts/check_plugins.sh
echo "plugin points at v$new; commit it as: plugin: point at v$new"
