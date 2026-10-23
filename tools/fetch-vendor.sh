#!/usr/bin/env bash
# Download the vendor files listed in tools/vendor-files.txt into docs/vendor/ (git-ignored)
# and verify their SHA-256. Files already present are only verified.
# Exit status 1 if any file is missing or has a different hash.
set -euo pipefail
cd "$(dirname "$0")/.."
status=0
while read -r path sha url; do
  case "$path" in ''|'#'*) continue ;; esac
  dest="docs/vendor/$path"
  if [ ! -f "$dest" ]; then
    if [ "$url" = "-" ]; then
      echo "manual  $dest  (no public URL; see the comment in tools/vendor-files.txt)"; status=1; continue
    fi
    mkdir -p "$(dirname "$dest")"
    if ! curl -fsSL --retry 2 -o "$dest.part" "$url"; then
      echo "FAILED  $dest  <- $url"; rm -f "$dest.part"; status=1; continue
    fi
    mv "$dest.part" "$dest"
  fi
  got=$(sha256sum "$dest" | cut -d' ' -f1)
  if [ "$got" = "$sha" ]; then echo "ok      $dest"; else echo "CHANGED $dest  (sha256 $got)"; status=1; fi
done < tools/vendor-files.txt
echo "Vendor web pages (read online): tools/vendor-pages.txt"
exit $status
