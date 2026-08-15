#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
BASE_RUNNER="$SCRIPT_DIR/run_local_dual_process_lan_canary_macos.sh"
REALISTIC_RUNNER="$SCRIPT_DIR/run_local_realistic_lan_e2e_macos.sh"

bash -n "$BASE_RUNNER"
bash -n "$REALISTIC_RUNNER"

base_help="$(bash "$BASE_RUNNER" --help)"
realistic_help="$(bash "$REALISTIC_RUNNER" --help)"

for option in \
  "--min-fps-ratio" \
  "--max-sample-drop-ratio" \
  "--loss-pct" \
  "--base-delay-ms" \
  "--jitter-ms" \
  "--mtu-bytes" \
  "--impairment-seed" \
  "--force-datagram-media" \
  "--allow-aspect-preserving-profile"; do
  if ! grep -q -- "$option" <<<"$base_help"; then
    echo "base runner help is missing $option" >&2
    exit 1
  fi
done

for scenario in clean lan stress; do
  if ! grep -q -- "$scenario" <<<"$realistic_help"; then
    echo "realistic runner help is missing $scenario" >&2
    exit 1
  fi
done

if ! grep -q 'MRD_LAN_TEST_IMPAIRMENT_LOSS_PCT' "$BASE_RUNNER"; then
  echo "base runner does not forward sender impairment configuration" >&2
  exit 1
fi
if ! grep -q 'exit "$overall_status"' "$BASE_RUNNER"; then
  echo "base runner does not propagate failed report status" >&2
  exit 1
fi

echo "macOS realistic LAN E2E runner contract checks passed"
