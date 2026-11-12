#!/usr/bin/env bash
# Fail unless every release binary in the given directory is fully static:
# no ELF interpreter and no shared-library dependency.
set -euo pipefail

dir=${1:?usage: check_static_binaries.sh TARGET_DIR}
for bin in computer-use-linux computer-use-linux-cosmic computer-use-linux-indicator; do
  path="$dir/$bin"
  if [ ! -f "$path" ]; then
    echo "::error::$path is missing"
    exit 1
  fi
  if readelf -d "$path" | grep -q NEEDED || readelf -l "$path" | grep -q INTERP; then
    echo "::error::$bin is not statically linked"
    exit 1
  fi
  echo "$bin is statically linked"
done
