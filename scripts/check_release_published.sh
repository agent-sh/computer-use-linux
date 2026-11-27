#!/bin/sh
# Fails when a release has no published binaries: by default the one the
# plugin launcher pins, or the version given. The marketplace serves main, so
# main must not name a version that cannot be downloaded.
set -eu

root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
version=${1:-$(sed -n 's/^version=//p' "$root/plugins/computer-use-linux/bin/computer-use-linux")}
base="https://github.com/agent-sh/computer-use-linux/releases/download/v$version"

missing=
for target in x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu; do
  for name in computer-use-linux computer-use-linux-cosmic computer-use-linux-indicator; do
    for suffix in "" .sha256; do
      asset="$name-$target$suffix"
      curl -fsIL --retry 3 -o /dev/null "$base/$asset" || missing="$missing $asset"
    done
  done
done

if [ -n "$missing" ]; then
  echo "check_release_published: release v$version is missing:$missing" >&2
  echo "check_release_published: the plugin may point at v$version only once its tag's run has published it (CONTRIBUTING.md, Releases)" >&2
  exit 1
fi
echo "release v$version is published"
