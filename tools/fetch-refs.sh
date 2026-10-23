#!/usr/bin/env bash
# Shallow-clone the reference firmwares listed in tools/refs.txt into docs/vendor/_refs/
# (git-ignored; for reading only; mind each project's licence before copying anything).
set -euo pipefail
cd "$(dirname "$0")/.."
mkdir -p docs/vendor/_refs
grep -v '^#' tools/refs.txt | while read -r name url branch _commit; do
  [ -z "$name" ] && continue
  dest="docs/vendor/_refs/$name"
  if [ -d "$dest/.git" ]; then
    echo "update $name"; git -C "$dest" pull -q --ff-only || true
  else
    echo "clone  $name"; git clone -q --depth 1 --branch "$branch" "$url" "$dest"
  fi
done
