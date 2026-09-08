#!/bin/sh
set -eu

if [ "$(uname -s)" != Darwin ]; then
    echo 'This packaging script requires macOS.' >&2
    exit 1
fi
cd "$(dirname "$0")/.."
"${OLIVE_CARGO:-cargo}" build --release --locked --features gui --bin olive-gui --target-dir target
olive_bundle='target/Olive Browser.app'
olive_version=$(awk -F '"' '/^version = / { print $2; exit }' Cargo.toml)
mkdir -p "$olive_bundle/Contents/MacOS" "$olive_bundle/Contents/Resources"
cp target/release/olive-gui "$olive_bundle/Contents/MacOS/olive-gui"
cp LICENSE "$olive_bundle/Contents/Resources/LICENSE"
cp assets/fonts/OFL.txt "$olive_bundle/Contents/Resources/Inter-OFL.txt"
cp assets/fonts/NotoSansHebrew-OFL.txt "$olive_bundle/Contents/Resources/NotoSansHebrew-OFL.txt"
cp assets/olive-browser.icns "$olive_bundle/Contents/Resources/olive-browser.icns"
cat > "$olive_bundle/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleName</key><string>Olive Browser</string>
  <key>CFBundleDisplayName</key><string>Olive Browser</string>
  <key>CFBundleIdentifier</key><string>org.olivebrowser.OliveBrowser</string>
  <key>CFBundleExecutable</key><string>olive-gui</string>
  <key>CFBundleIconFile</key><string>olive-browser</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$olive_version</string>
  <key>CFBundleVersion</key><string>$olive_version</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict></plist>
PLIST
printf 'Built %s/%s\n' "$PWD" "$olive_bundle"
