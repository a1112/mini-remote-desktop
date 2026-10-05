#!/usr/bin/env bash
set -euo pipefail

workspace=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
[[ $(uname -s) == Darwin ]] || { echo 'This package requires a native macOS build host.' >&2; exit 1; }
case $(uname -m) in
  arm64) architecture=apple-silicon; macho_arch=arm64 ;;
  x86_64) architecture=intel; macho_arch=x86_64 ;;
  *) echo 'Unsupported macOS architecture.' >&2; exit 1 ;;
esac

# The existing Tauri platform config embeds the bundle from this exact directory.
if [[ -n ${CARGO_TARGET_DIR:-} && $CARGO_TARGET_DIR != "$workspace/target" ]]; then
  echo 'macOS packaging requires CARGO_TARGET_DIR to be the repository target directory.' >&2
  exit 1
fi
if [[ -n ${CARGO_BUILD_TARGET:-} || -n ${TARGET:-} ]]; then
  echo 'Use a native Mac host without a cross-compilation target for this package.' >&2
  exit 1
fi
export CARGO_TARGET_DIR="$workspace/target"
export MACOSX_DEPLOYMENT_TARGET=13.0

cd "$workspace/apps/Rdesk"
# beforeBuildCommand already builds MrdService.app with its privacy metadata,
# then builds the frontend. Keep that single packaging entry point.
pnpm tauri build --ci --bundles app -- --locked

bundle="$workspace/target/release/bundle/macos/Rdesk.app"
service_bundle="$bundle/Contents/Resources/MrdService.app"
service="$service_bundle/Contents/MacOS/mrd-service"
/usr/bin/plutil -lint "$bundle/Contents/Info.plist" "$service_bundle/Contents/Info.plist"
executable=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleExecutable' "$bundle/Contents/Info.plist")
[[ $executable =~ ^[a-zA-Z0-9._-]+$ ]] || { echo 'Invalid client executable metadata.' >&2; exit 1; }
client_identifier=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$bundle/Contents/Info.plist")
service_identifier=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$service_bundle/Contents/Info.plist")
[[ $client_identifier =~ ^[a-zA-Z0-9][a-zA-Z0-9.-]*$ && $service_identifier =~ ^[a-zA-Z0-9][a-zA-Z0-9.-]*$ ]] || { echo 'Invalid bundle identifier metadata.' >&2; exit 1; }
client="$bundle/Contents/MacOS/$executable"
test -x "$client"
test -x "$service"
/usr/bin/lipo "$client" -verify_arch "$macho_arch"
/usr/bin/lipo "$service" -verify_arch "$macho_arch"
# Seal the copied service first, then the final outer resource envelope. Each
# bundle uses its own identifier; neither bundle config supplies entitlements.
/usr/bin/codesign --force --sign - --identifier "$service_identifier" "$service_bundle"
/usr/bin/codesign --verify --strict "$service_bundle"
/usr/bin/codesign --force --sign - --identifier "$client_identifier" "$bundle"
/usr/bin/codesign --verify --deep --strict "$bundle"

output="$workspace/target/macos-client-artifacts"
mkdir -p "$output"
commit=$(git -C "$workspace" rev-parse HEAD)
archive="$output/Rdesk-macos-$architecture-${commit:0:12}.zip"
/usr/bin/ditto -c -k --sequesterRsrc --keepParent "$bundle" "$archive"
(cd "$output" && /usr/bin/shasum -a 256 "$(basename "$archive")" > "$(basename "$archive").sha256")
cat > "$output/README.txt" <<EOF
Rdesk macOS client
Architecture: $architecture
Minimum macOS: 13.0
Source commit: $commit

Unzip and copy Rdesk.app to Applications, then open it through macOS.
This development package uses ad-hoc signing and is not notarized by Apple.
macOS may require you to explicitly allow the application in Privacy & Security.
Screen Recording and Accessibility permissions remain controlled by macOS.
Use your administrator-issued enrollment code to register this device.
The remote device code is assigned by the server as 10 unique decimal digits.
EOF
printf 'Verified macOS client package: %s\n' "$archive"
