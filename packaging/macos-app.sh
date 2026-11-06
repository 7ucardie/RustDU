#!/bin/sh
# Wrap the rustdu binary in RustDU.app with the app icon (macOS only: needs sips and iconutil).
# Usage: packaging/macos-app.sh <binary> <version> <dest-dir>
# SIGN_IDENTITY: a Developer ID in the keychain to sign with; unset signs ad hoc.
set -e
bin="$1"; version="$2"; dest="$3"
root="$(cd "$(dirname "$0")/.." && pwd)"
app="$dest/RustDU.app/Contents"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

rm -rf "$dest/RustDU.app"
mkdir -p "$app/MacOS" "$app/Resources" "$work/icon.iconset"
cp "$bin" "$app/MacOS/rustdu"

for s in 16 32 128 256 512; do
  sips -z "$s" "$s" "$root/assets/disk-explorer.png" --out "$work/icon.iconset/icon_${s}x${s}.png" >/dev/null
  d=$((s * 2))
  sips -z "$d" "$d" "$root/assets/disk-explorer.png" --out "$work/icon.iconset/icon_${s}x${s}@2x.png" >/dev/null
done
iconutil -c icns "$work/icon.iconset" -o "$app/Resources/rustdu.icns"

cat > "$app/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>RustDU</string>
  <key>CFBundleDisplayName</key><string>RustDU</string>
  <key>CFBundleIdentifier</key><string>io.github.evandervecht.disk-explorer</string>
  <key>CFBundleExecutable</key><string>rustdu</string>
  <key>CFBundleIconFile</key><string>rustdu</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$version</string>
  <key>CFBundleVersion</key><string>$version</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.utilities</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST
if [ -n "$SIGN_IDENTITY" ]; then
  # Notarization needs the hardened runtime and a secure timestamp.
  codesign --force --options runtime --timestamp --entitlements "$root/packaging/macos-entitlements.plist" \
    --sign "$SIGN_IDENTITY" "$dest/RustDU.app"
else
  # Ad-hoc signature: an unsigned bundle downloaded on Apple Silicon is reported as "damaged";
  # signed ad hoc it gets the normal "Open Anyway" prompt.
  codesign --force --deep --sign - "$dest/RustDU.app"
fi
echo "Built $dest/RustDU.app"
