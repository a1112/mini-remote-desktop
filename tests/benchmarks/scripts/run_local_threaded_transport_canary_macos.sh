#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../../.." && pwd)"
REPORT_DIR="${MRD_THREADED_TRANSPORT_REPORT_DIR:-${REPO_ROOT}/target/e2e-threaded-transport}"

mkdir -p "${REPORT_DIR}"
cd "${REPO_ROOT}"

echo "Running macOS threaded transport canary through QUIC media v3 loopback"
echo "Report dir: ${REPORT_DIR}"

MRD_THREADED_TRANSPORT_REPORT_DIR="${REPORT_DIR}" \
  cargo test -p integration-tests --test threaded_transport_e2e -- --nocapture
