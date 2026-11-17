#!/bin/sh
set -eu

if [ "$(uname -s)" != Darwin ]; then
    echo 'This packaging script requires macOS.' >&2
    exit 1
fi
cd "$(dirname "$0")/.."
olive_applications_dir=${OLIVE_APPLICATIONS_DIR:-/Applications}
olive_installed_bundle="$olive_applications_dir/Olive Browser.app"
olive_build_number=0
if [ -f "$olive_installed_bundle/Contents/Info.plist" ]; then
    olive_previous_build=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleVersion' "$olive_installed_bundle/Contents/Info.plist" 2>/dev/null || true)
    case "$olive_previous_build" in
        ''|*[!0-9.]*) ;;
        *)
            olive_previous_build=${olive_previous_build##*.}
            olive_previous_build=$(printf '%s' "$olive_previous_build" | sed 's/^0*//')
            olive_build_number=${olive_previous_build:-0}
            ;;
    esac
fi
olive_build_number=$((olive_build_number + 1))

"${OLIVE_CARGO:-cargo}" build --release --locked --features gui --bin olive-gui --target-dir target
olive_bundle='target/Olive Browser.app'
olive_version=$(awk -F '"' '/^version = / { print $2; exit }' Cargo.toml)
mkdir -p "$olive_bundle/Contents/MacOS" "$olive_bundle/Contents/Resources"
# Replace the executable's inode instead of overwriting a running app. macOS
# can otherwise retain the old code-signature cache and kill new workers.
olive_executable=$(mktemp "$olive_bundle/Contents/MacOS/.olive-gui.XXXXXX")
trap 'rm -f "$olive_executable"' 0
cp target/release/olive-gui "$olive_executable"
chmod 755 "$olive_executable"
mv -f "$olive_executable" "$olive_bundle/Contents/MacOS/olive-gui"
trap - 0
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
  <key>CFBundleVersion</key><string>$olive_build_number</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict></plist>
PLIST
# Sign with the hardened runtime, which keeps other processes from injecting
# code into the browser or reading its memory, and keeps it from loading
# libraries that are not Apple's or its own. Without a Developer ID in
# OLIVE_SIGNING_IDENTITY, the signature is ad hoc.
if [ -n "${OLIVE_SIGNING_IDENTITY:-}" ]; then
    codesign --force --options runtime --timestamp --sign "$OLIVE_SIGNING_IDENTITY" "$olive_bundle"
else
    codesign --force --options runtime --timestamp=none --sign - "$olive_bundle"
fi
codesign --verify --strict "$olive_bundle"

# CI only packages the app under target/. Local builds also replace the
# installed app after signing and verification have succeeded.
if [ "${CI:-}" != true ]; then
    olive_stage="$olive_applications_dir/.Olive Browser.app.new.$$"
    olive_backup="$olive_applications_dir/.Olive Browser.app.previous.$$"
    olive_run_install_command() {
        if [ -w "$olive_applications_dir" ]; then
            "$@"
        else
            sudo "$@"
        fi
    }
    if [ -e "$olive_stage" ] || [ -e "$olive_backup" ]; then
        echo 'Temporary app install path already exists.' >&2
        exit 1
    fi
    olive_run_install_command ditto "$olive_bundle" "$olive_stage"
    if [ -e "$olive_installed_bundle" ]; then
        olive_run_install_command mv "$olive_installed_bundle" "$olive_backup"
    fi
    if ! olive_run_install_command mv "$olive_stage" "$olive_installed_bundle"; then
        if [ -e "$olive_backup" ]; then
            olive_run_install_command mv "$olive_backup" "$olive_installed_bundle"
        fi
        exit 1
    fi
    olive_run_install_command rm -rf "$olive_backup"
    printf 'Installed %s\n' "$olive_installed_bundle"
fi
printf 'Built %s/%s\n' "$PWD" "$olive_bundle"
