#!/bin/zsh
# Builds Disk Explorer.app into ./build. Run ./build-app.sh, then sign the result.
set -euo pipefail
cd "$(dirname "$0")"

APP_NAME="Disk Explorer"
EXECUTABLE="DiskExplorer"
BUNDLE_ID="${BUNDLE_ID:-com.example.DiskExplorer}"   # override: BUNDLE_ID=com.you.DiskExplorer ./build-app.sh
VERSION="${VERSION:-1.0.0}"
BUILD_NUMBER="${BUILD_NUMBER:-1}"
ARCHS=(${=ARCHS:-arm64 x86_64})                        # override: ARCHS=arm64 ./build-app.sh
MIN_OS="14.0"

ARCH_FLAGS=()
for a in $ARCHS; do ARCH_FLAGS+=(--arch "$a"); done

echo "▸ Building release binary for: $ARCHS"
swift build -c release "${ARCH_FLAGS[@]}"
BIN_DIR="$(swift build -c release "${ARCH_FLAGS[@]}" --show-bin-path)"

APP="build/$APP_NAME.app"
CONTENTS="$APP/Contents"
rm -rf "$APP"
mkdir -p "$CONTENTS/MacOS" "$CONTENTS/Resources"
cp "$BIN_DIR/$EXECUTABLE" "$CONTENTS/MacOS/$EXECUTABLE"

echo "▸ Writing Info.plist"
cat > "$CONTENTS/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleDevelopmentRegion</key><string>en</string>
    <key>CFBundleExecutable</key><string>$EXECUTABLE</string>
    <key>CFBundleIconFile</key><string>AppIcon</string>
    <key>CFBundleIdentifier</key><string>$BUNDLE_ID</string>
    <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
    <key>CFBundleName</key><string>$APP_NAME</string>
    <key>CFBundleDisplayName</key><string>$APP_NAME</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>$VERSION</string>
    <key>CFBundleVersion</key><string>$BUILD_NUMBER</string>
    <key>LSMinimumSystemVersion</key><string>$MIN_OS</string>
    <key>LSApplicationCategoryType</key><string>public.app-category.utilities</string>
    <key>NSHighResolutionCapable</key><true/>
    <key>NSHumanReadableCopyright</key><string>Copyright © $(date +%Y). All rights reserved.</string>
    <key>NSPrincipalClass</key><string>NSApplication</string>
    <key>NSSupportsAutomaticTermination</key><false/>
    <key>NSSupportsSuddenTermination</key><false/>
</dict>
</plist>
PLIST
printf 'APPL????' > "$CONTENTS/PkgInfo"

echo "▸ Generating app icon"
ICON_TMP="$(mktemp -d)"
if swift Tools/make-icon.swift "$ICON_TMP/icon_1024.png" >/dev/null 2>&1; then
    ICONSET="$ICON_TMP/AppIcon.iconset"
    mkdir -p "$ICONSET"
    for s in 16 32 128 256 512; do
        sips -z $s $s "$ICON_TMP/icon_1024.png" --out "$ICONSET/icon_${s}x${s}.png" >/dev/null
        d=$((s * 2))
        sips -z $d $d "$ICON_TMP/icon_1024.png" --out "$ICONSET/icon_${s}x${s}@2x.png" >/dev/null
    done
    iconutil -c icns "$ICONSET" -o "$CONTENTS/Resources/AppIcon.icns"
else
    echo "  (icon generation skipped)"
fi
rm -rf "$ICON_TMP"

echo "▸ Ad-hoc signing so the bundle runs locally (re-sign with your identity later)"
codesign --force --sign - --entitlements DiskExplorer.entitlements "$APP"

echo
echo "Built: $PWD/$APP"
echo
echo "To sign for distribution:"
echo "  codesign --force --deep --options runtime --timestamp \\"
echo "    --entitlements DiskExplorer.entitlements \\"
echo "    --sign 'Developer ID Application: Your Name (TEAMID)' '$APP'"
echo "  codesign --verify --deep --strict --verbose=2 '$APP'"
echo "Then notarize: ditto -c -k --keepParent '$APP' DiskExplorer.zip && xcrun notarytool submit DiskExplorer.zip --keychain-profile <profile> --wait && xcrun stapler staple '$APP'"
