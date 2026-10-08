#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
python3 scripts/test_launcher_checks.py
python3 scripts/generate_launcher_icons.py --check
python3 scripts/verify_launcher_icons.py
: "${ANDROID_HOME:=${ANDROID_SDK_ROOT:-}}"
: "${ANDROID_HOME:?Set ANDROID_HOME to your installed, licensed Android SDK}"
export ANDROID_HOME
GRADLE=${GRADLE:-gradle}
"$GRADLE" --no-daemon :app:testDebugUnitTest :app:assembleDebug
# AGP 8.13 location. Fall back to its older merged_manifests layout.
MANIFEST=app/build/intermediates/merged_manifest/debug/processDebugMainManifest/AndroidManifest.xml
if [[ ! -f "$MANIFEST" ]]; then
    MANIFEST=app/build/intermediates/merged_manifests/debug/processDebugManifest/AndroidManifest.xml
fi
test -f "$MANIFEST"
AAPT2=${AAPT2:-$ANDROID_HOME/build-tools/35.0.0/aapt2}
test -x "$AAPT2"
python3 scripts/verify_launcher_icons.py \
    --merged-manifest "$MANIFEST" \
    --apk app/build/outputs/apk/debug/app-debug.apk \
    --aapt2 "$AAPT2"
