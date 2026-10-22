#!/bin/bash
set -euo pipefail
IOS_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$IOS_ROOT"
MODE="${1:-simulator}"
case "$MODE" in
  simulator)
    xcodebuild -project RdeskMobile.xcodeproj -scheme RdeskMobile \
      -configuration Debug -sdk iphonesimulator -destination 'generic/platform=iOS Simulator' \
      -derivedDataPath build/simulator CODE_SIGNING_ALLOWED=NO build
    printf '\nSimulator app: %s\n' "$IOS_ROOT/build/simulator/Build/Products/Debug-iphonesimulator/RdeskMobile.app"
    ;;
  device)
    : "${APPLE_DEVELOPMENT_TEAM:?Set APPLE_DEVELOPMENT_TEAM to your Apple development team ID}"
    xcodebuild -project RdeskMobile.xcodeproj -scheme RdeskMobile \
      -configuration Release -sdk iphoneos -destination 'generic/platform=iOS' \
      -derivedDataPath build/device "DEVELOPMENT_TEAM=$APPLE_DEVELOPMENT_TEAM" \
      -allowProvisioningUpdates -allowProvisioningDeviceRegistration build
    PACKAGE_DIR="$(mktemp -d "$IOS_ROOT/build/package.XXXXXX")"
    trap 'rm -rf "$PACKAGE_DIR"' EXIT
    mkdir -p "$PACKAGE_DIR/Payload"
    ditto build/device/Build/Products/Release-iphoneos/RdeskMobile.app "$PACKAGE_DIR/Payload/RdeskMobile.app"
    (cd "$PACKAGE_DIR" && /usr/bin/zip -qry Rdesk-iOS.ipa Payload)
    mv "$PACKAGE_DIR/Rdesk-iOS.ipa" "$IOS_ROOT/build/Rdesk-iOS.ipa"
    printf '\nDevice app: %s\nIPA: %s\n' "$IOS_ROOT/build/device/Build/Products/Release-iphoneos/RdeskMobile.app" "$IOS_ROOT/build/Rdesk-iOS.ipa"
    ;;
  *) printf 'Usage: %s [simulator|device]\n' "$0" >&2; exit 2 ;;
esac
