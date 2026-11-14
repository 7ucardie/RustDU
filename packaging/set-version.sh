#!/bin/sh
# Write a version into Cargo.toml and the matching entry in Cargo.lock, so `--locked` builds keep working.
# Usage: packaging/set-version.sh <version>
set -eu
version="$1"
root="$(cd "$(dirname "$0")/.." && pwd)"
case "$version" in
  *[!0-9.]* | "") echo "not a plain version: $version" >&2; exit 2 ;;
esac
# Only the [package] version: the first line of that shape in the file.
perl -0pi -e 's/^version = "[^"]*"/version = "'"$version"'"/m' "$root/Cargo.toml"
perl -0pi -e 's/(name = "rustdu"\nversion = ")[^"]*/${1}'"$version"'/' "$root/Cargo.lock"
grep -qx "version = \"$version\"" "$root/Cargo.toml"
grep -A1 -x 'name = "rustdu"' "$root/Cargo.lock" | grep -qx "version = \"$version\""
