#!/bin/sh
# Pack the rustdu binary for Linux: a tarball laid out like an install prefix, and a .deb
# (Linux only: needs dpkg-deb, and ImageMagick to scale the icon).
# Usage: packaging/linux-package.sh <binary> <version> <amd64|arm64> <dest-dir>
set -eu
bin="$1"; version="$2"; arch="$3"; dest="$4"
root="$(cd "$(dirname "$0")/.." && pwd)"
name="rustdu-$version-linux-$arch"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

# The same tree serves both: <name>/ in the tarball, /usr in the package.
tree="$work/$name"
install -Dm755 "$bin" "$tree/bin/rustdu"
install -Dm644 "$root/packaging/linux/rustdu.desktop" "$tree/share/applications/rustdu.desktop"
install -Dm644 "$root/README.md" "$tree/share/doc/rustdu/README.md"
install -Dm644 "$root/LICENSE" "$tree/share/doc/rustdu/copyright"
# The hicolor theme has no folder for the 2048 px source; 512 is its largest.
icon="$tree/share/icons/hicolor/512x512/apps/rustdu.png"
mkdir -p "$(dirname "$icon")"
if command -v magick >/dev/null 2>&1; then
  magick "$root/assets/disk-explorer.png" -resize 512x512 "$icon"
elif command -v convert >/dev/null 2>&1; then
  convert "$root/assets/disk-explorer.png" -resize 512x512 "$icon"
else
  echo "ImageMagick not found: apt-get install imagemagick" >&2
  exit 1
fi

mkdir -p "$dest"
tar -czf "$dest/$name.tar.gz" --owner=0 --group=0 -C "$work" "$name"

deb="$work/deb"
mkdir -p "$deb/DEBIAN"
cp -R "$tree" "$deb/usr"
# A pre-release sorts before its release with ~ (1.0.0~rc.1), not with the tag's hyphen.
# Windowing and graphics libraries are loaded at run time, so dpkg-shlibdeps would not find them.
cat > "$deb/DEBIAN/control" <<CONTROL
Package: rustdu
Version: $(echo "$version" | sed 's/-/~/')
Architecture: $arch
Maintainer: RustDU <noreply@github.com>
Section: utils
Priority: optional
Homepage: https://github.com/7ucardie/RustDU
Installed-Size: $(du -sk "$deb/usr" | cut -f1)
Depends: libc6 (>= 2.35), libgcc-s1, libxkbcommon0, libegl1 | libvulkan1
Recommends: xdg-desktop-portal, xdg-utils, libwayland-client0, libxkbcommon-x11-0, git
Description: See where the disk space goes
 Folder sizes, permissions, owners, git state and last activity in an
 explorer view, with the largest files, the git repositories and the trash.
CONTROL
dpkg-deb --build --root-owner-group "$deb" "$dest/$name.deb" >/dev/null
echo "Built $dest/$name.tar.gz and $dest/$name.deb"
