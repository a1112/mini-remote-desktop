#!/bin/bash
set -euo pipefail
IOS_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$IOS_ROOT"
SIMULATOR_ID="${SIMULATOR_ID:?Set SIMULATOR_ID from xcrun simctl list devices available}"
mkdir -p build
fixture_dir="$(mktemp -d "$IOS_ROOT/build/gateway.XXXXXX")"
cleanup() {
    if [[ -n ${FIXTURE_PID:-} ]]; then
        kill "$FIXTURE_PID" 2>/dev/null || true
        wait "$FIXTURE_PID" 2>/dev/null || true
    fi
    rm -r "$fixture_dir"
}
trap cleanup EXIT
sips -s format jpeg Resources/Assets.xcassets/AppIcon.appiconset/AppIcon.png --out "$fixture_dir/frame.jpg" >/dev/null
openssl ecparam -name prime256v1 -genkey -noout -out "$fixture_dir/key.pem"
openssl req -x509 -new -days 1 -key "$fixture_dir/key.pem" -out "$fixture_dir/cert.pem" \
  -subj '/CN=localhost' -addext 'subjectAltName=DNS:localhost'
python3 scripts/fixture_server.py --jpeg "$fixture_dir/frame.jpg" \
  --cert "$fixture_dir/cert.pem" --key "$fixture_dir/key.pem" >build/fixture.log 2>&1 &
FIXTURE_PID=$!
for attempt in {1..30}; do
  kill -0 "$FIXTURE_PID" || { cat build/fixture.log; exit 1; }
  if curl --noproxy '*' --max-time 2 --fail --silent http://127.0.0.1:19535/health >/dev/null; then break; fi
  sleep 0.1
done
curl --noproxy '*' --max-time 2 --fail --silent http://127.0.0.1:19535/health >/dev/null
xcodebuild -project RdeskMobile.xcodeproj -scheme RdeskMobile \
  -configuration Debug -destination "id=$SIMULATOR_ID" \
  -derivedDataPath build/simulator -parallel-testing-enabled NO \
  -collect-test-diagnostics never \
  CODE_SIGNING_ALLOWED=NO test
