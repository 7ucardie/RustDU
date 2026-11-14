#!/bin/sh
# Print the version the next release gets, from the Conventional Commits since the last v<version> tag.
# Prints nothing when those commits call for no release (only docs, ci, chore, build, test, refactor).
# Usage: packaging/next-version.sh [patch|minor|major]   (an argument forces at least that bump)
#   feat:                      minor
#   fix: perf:                 patch
#   type!: or BREAKING CHANGE  major, or minor while the version is still 0.x
set -eu
force="${1:-}"

# Pre-release tags (v1.2.0-rc.1) are not a base to count from.
last="$(git describe --tags --abbrev=0 --match 'v[0-9]*' --exclude '*-*' 2>/dev/null || true)"
if [ -n "$last" ]; then
  range="$last..HEAD"
  current="${last#v}"
else
  range="HEAD"
  current="0.0.0"
fi

log="$(git log --no-merges --format='%s%n%b' "$range")"
bump="$force"
rank() { case "$1" in major) echo 3 ;; minor) echo 2 ;; patch) echo 1 ;; *) echo 0 ;; esac; }
raise() { if [ "$(rank "$1")" -gt "$(rank "$bump")" ]; then bump="$1"; fi; }
if printf '%s\n' "$log" | grep -Eq '^(fix|perf)(\([^)]*\))?: '; then raise patch; fi
if printf '%s\n' "$log" | grep -Eq '^feat(\([^)]*\))?: '; then raise minor; fi
if printf '%s\n' "$log" | grep -Eq '^[a-z]+(\([^)]*\))?!: |^BREAKING[ -]CHANGE: '; then raise major; fi
[ -n "$bump" ] || exit 0

IFS=. read -r major minor patch <<VERSION
$current
VERSION
if [ "$bump" = major ] && [ "$major" -eq 0 ]; then bump=minor; fi
case "$bump" in
  major) major=$((major + 1)); minor=0; patch=0 ;;
  minor) minor=$((minor + 1)); patch=0 ;;
  patch) patch=$((patch + 1)) ;;
  *) echo "unknown bump: $bump" >&2; exit 2 ;;
esac
echo "$major.$minor.$patch"
