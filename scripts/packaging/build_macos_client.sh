#!/usr/bin/env bash
set -euo pipefail

workspace=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
explicit_identity=''
notary_profile=''
while [[ $# -gt 0 ]]; do
  case "$1" in
    --sign-identity)
      [[ $# -ge 2 && -n ${2:-} && ${2:-} != --* ]] || { echo '--sign-identity requires an identity.' >&2; exit 2; }
      explicit_identity=$2
      shift 2
      ;;
    --notary-profile)
      [[ $# -ge 2 && -n ${2:-} && ${2:-} != --* ]] || { echo '--notary-profile requires a Keychain profile.' >&2; exit 2; }
      notary_profile=$2
      shift 2
      ;;
    -h|--help)
      echo 'Usage: scripts/packaging/build_macos_client.sh [--sign-identity IDENTITY] [--notary-profile KEYCHAIN_PROFILE]'
      exit 0
      ;;
    *) echo "Unknown argument: $1" >&2; exit 2 ;;
  esac
done
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
if [[ -n $explicit_identity ]]; then
  export MRD_MACOS_SIGNING_IDENTITY="$explicit_identity"
fi
sign_identity=$(node "$workspace/apps/Rdesk/scripts/macos-signing-identity.mjs" "$workspace")
sign_options=()
if [[ -n $notary_profile ]]; then
  [[ $sign_identity != '-' ]] || { echo 'Notarization requires a Developer ID Application signing identity.' >&2; exit 2; }
  sign_options=(--options runtime --timestamp)
fi
export MRD_MACOS_SIGNING_IDENTITY="$sign_identity"
export APPLE_SIGNING_IDENTITY="$sign_identity"

cd "$workspace/apps/Rdesk"
# beforeBuildCommand already builds MrdService.app with its privacy metadata,
# then builds the frontend. Keep that single packaging entry point.
pnpm tauri build --ci --bundles app -- --locked --bin app

bundle="$workspace/target/release/bundle/macos/Rdesk.app"
service_bundle="$bundle/Contents/Resources/MrdService.app"
service="$service_bundle/Contents/MacOS/mrd-service"
/usr/bin/plutil -lint "$bundle/Contents/Info.plist" "$service_bundle/Contents/Info.plist"
executable=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleExecutable' "$bundle/Contents/Info.plist")
[[ $executable =~ ^[a-zA-Z0-9._-]+$ ]] || { echo 'Invalid client executable metadata.' >&2; exit 1; }
client_identifier=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$bundle/Contents/Info.plist")
service_identifier=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$service_bundle/Contents/Info.plist")
[[ $client_identifier =~ ^[a-zA-Z0-9][a-zA-Z0-9.-]*$ && $service_identifier =~ ^[a-zA-Z0-9][a-zA-Z0-9.-]*$ ]] || { echo 'Invalid bundle identifier metadata.' >&2; exit 1; }
node "$workspace/apps/Rdesk/scripts/verify-macos-bundle.mjs" "$bundle"
client="$bundle/Contents/MacOS/$executable"
test -x "$client"
test -x "$service"
/usr/bin/lipo "$client" -verify_arch "$macho_arch"
/usr/bin/lipo "$service" -verify_arch "$macho_arch"
# Seal the copied service first, then the final outer resource envelope. Each
# bundle uses its own identifier; neither bundle config supplies entitlements.
/usr/bin/codesign --force --sign "$sign_identity" "${sign_options[@]+"${sign_options[@]}"}" --identifier "$service_identifier" "$service_bundle"
/usr/bin/codesign --verify --strict "$service_bundle"
/usr/bin/codesign --force --sign "$sign_identity" "${sign_options[@]+"${sign_options[@]}"}" --identifier "$client_identifier" "$bundle"
/usr/bin/codesign --verify --deep --strict "$bundle"

output="$workspace/target/macos-client-artifacts"
mkdir -p "$output"
commit=$(git -C "$workspace" rev-parse HEAD)
stem="Rdesk-macos-$architecture-${commit:0:12}"
staging=$(mktemp -d "$output/.package.XXXXXX")
trap 'rm -rf -- "$staging"' EXIT
if [[ -n $notary_profile ]]; then
  authorities=$(/usr/bin/codesign -dvv "$bundle" 2>&1)
  if ! grep -q '^Authority=Developer ID Application:' <<< "$authorities"; then
    echo 'Notarization requires a Developer ID Application signing identity.' >&2
    exit 2
  fi
  submission="$staging/submission.zip"
  /usr/bin/ditto -c -k --sequesterRsrc --keepParent "$bundle" "$submission"
  /usr/bin/xcrun notarytool submit "$submission" --keychain-profile "$notary_profile" \
    --wait --timeout 30m --output-format json > "$staging/notary.json"
  node -e 'const info = require(process.argv[1]); if (info.status !== "Accepted") throw Error("Apple notarization status: " + info.status)' "$staging/notary.json"
  /usr/bin/xcrun stapler staple "$bundle"
  /usr/bin/xcrun stapler validate "$bundle"
  /usr/bin/codesign --verify --deep --strict "$bundle"
  stem="$stem-notarized"
  signing_description='Apple notarization accepted. A ticket is stapled to Rdesk.app.'
elif [[ $sign_identity == '-' ]]; then
  signing_description='This development package uses ad-hoc signing and is not notarized by Apple.'
else
  signing_description='This package is signed with the configured local identity; notarization is separate.'
fi
archive="$stem.zip"
/usr/bin/ditto -c -k --sequesterRsrc --keepParent "$bundle" "$staging/$archive"
(cd "$staging" && /usr/bin/shasum -a 256 "$archive" > "$archive.sha256")
cat > "$staging/README.txt" <<EOF
Rdesk macOS client
Architecture: $architecture
Minimum macOS: 13.0
Source commit: $commit

Unzip and copy Rdesk.app to Applications, then open it through macOS.
$signing_description
macOS may require you to explicitly allow the application in Privacy & Security.
Screen Recording and Accessibility permissions remain controlled by macOS.
Use your administrator-issued enrollment code to register this device.
The remote device code is assigned by the server as 10 unique decimal digits.
EOF
mv "$staging/$archive" "$staging/$archive.sha256" "$staging/README.txt" "$output/"
if [[ -n $notary_profile ]]; then
  mv "$staging/notary.json" "$output/$stem.notary.json"
fi
printf 'Verified macOS client package: %s\n' "$output/$archive"
