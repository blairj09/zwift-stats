#!/bin/bash
# Builds "Zwift Stats.app" and copies it to ~/Applications (or the folder given as $1).
set -euo pipefail

cd "$(dirname "$0")/.."
dest="${1:-$HOME/Applications}"
app="target/release/Zwift Stats.app"
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)

cargo build --release

rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp target/release/zwift-hub "$app/Contents/MacOS/zwift-hub"

# Icon: every size macOS asks for, from the 1024px source
iconset=$(mktemp -d)/AppIcon.iconset
mkdir -p "$iconset"
for size in 16 32 128 256 512; do
  sips -z $size $size macos/icon.png --out "$iconset/icon_${size}x${size}.png" >/dev/null
  sips -z $((size * 2)) $((size * 2)) macos/icon.png --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$iconset" -o "$app/Contents/Resources/AppIcon.icns"

cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>Zwift Stats</string>
    <key>CFBundleDisplayName</key><string>Zwift Stats</string>
    <key>CFBundleIdentifier</key><string>com.github.blairj09.zwift-hub</string>
    <key>CFBundleExecutable</key><string>zwift-hub</string>
    <key>CFBundleIconFile</key><string>AppIcon</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>${version}</string>
    <key>CFBundleVersion</key><string>${version}</string>
    <key>LSMinimumSystemVersion</key><string>11.0</string>
    <key>LSApplicationCategoryType</key><string>public.app-category.healthcare-fitness</string>
    <key>NSHighResolutionCapable</key><true/>
    <key>NSDocumentsFolderUsageDescription</key><string>Zwift Stats reads your Zwift logs and rides from Documents/Zwift.</string>
</dict>
</plist>
PLIST

# Ad-hoc signature: enough for a Mac to run an app built on it, and gives macOS a stable
# identity to remember the Documents permission by
codesign --force --deep --sign - "$app"

mkdir -p "$dest"
# The app was called Zwift Hub before; drop that copy so only one is installed
rm -rf "$dest/Zwift Stats.app" "$dest/Zwift Hub.app"
cp -R "$app" "$dest/"
echo "Installed $dest/Zwift Stats.app"
