#!/bin/bash
set -euo pipefail
IOS_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$IOS_ROOT"
SIMULATOR_ID="${SIMULATOR_ID:?Set SIMULATOR_ID from xcrun simctl list devices available}"
mkdir -p build
sips -s format jpeg Resources/Assets.xcassets/AppIcon.appiconset/AppIcon.png --out build/fixture.jpg >/dev/null
python3 scripts/fixture_server.py --jpeg build/fixture.jpg >build/fixture.log 2>&1 &
FIXTURE_PID=$!
trap 'kill "$FIXTURE_PID" 2>/dev/null || true' EXIT
for attempt in {1..30}; do
  kill -0 "$FIXTURE_PID" || { cat build/fixture.log; exit 1; }
  if curl --noproxy '*' --max-time 2 --fail --silent http://127.0.0.1:19534/health >/dev/null; then break; fi
  sleep 0.1
done
xcodebuild -project RdeskMobile.xcodeproj -scheme RdeskMobile \
  -configuration Debug -destination "id=$SIMULATOR_ID" \
  -derivedDataPath build/simulator -parallel-testing-enabled NO \
  -collect-test-diagnostics never \
  CODE_SIGNING_ALLOWED=NO test
