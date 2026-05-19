#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="."
OUTPUT_DIR="target/codex-local-dual-process-canary-macos"
CHAIN_ID="local_dual_process/macos/videotoolbox_h264/quic_datagram_media_v3_or_v2/videotoolbox/web_preview"
PROFILE_IDS="1080p60"
DURATION_SECS=30
BITRATE_MBPS=20
DISPLAY_MODE_POLICY="none"
CODEC="h264"
CAPTURE_SOURCE_ID=""
CAPTURE_SOURCE_KIND="display"
NO_BUILD=0
KEEP_TAURI_OPEN=0

usage() {
  cat <<'EOF'
Usage: run_local_dual_process_lan_canary_macos.sh [options]

Options:
  --repo-root PATH
  --output-dir PATH
  --profile-id ID[,ID...]       Default: 1080p60
  --duration-secs SECONDS       Default: 30
  --duration SECONDS            Alias for --duration-secs
  --bitrate-mbps MBPS          Default: 20
  --display-mode-policy VALUE   none|temporary|required. Default: none
  --codec VALUE                 h264 only for macOS local dual-process canary
  --capture-source-id ID
  --capture-source-kind KIND    Default: display
  --no-build
  --keep-tauri-open
  --no-motion-stimulus          Accepted for parity with the PowerShell runner
  --help
EOF
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --repo-root) REPO_ROOT="$2"; shift 2 ;;
    --output-dir) OUTPUT_DIR="$2"; shift 2 ;;
    --profile-id) PROFILE_IDS="$2"; shift 2 ;;
    --duration-secs|--duration) DURATION_SECS="$2"; shift 2 ;;
    --bitrate-mbps) BITRATE_MBPS="$2"; shift 2 ;;
    --display-mode-policy) DISPLAY_MODE_POLICY="$2"; shift 2 ;;
    --codec) CODEC="$2"; shift 2 ;;
    --capture-source-id) CAPTURE_SOURCE_ID="$2"; shift 2 ;;
    --capture-source-kind) CAPTURE_SOURCE_KIND="$2"; shift 2 ;;
    --no-build) NO_BUILD=1; shift ;;
    --keep-tauri-open) KEEP_TAURI_OPEN=1; shift ;;
    --no-motion-stimulus) shift ;;
    --help|-h) usage; exit 0 ;;
    *) echo "Unknown option: $1" >&2; usage >&2; exit 2 ;;
  esac
done

if [ "$(uname -s)" != "Darwin" ]; then
  echo "This runner is macOS-only; use run_local_dual_process_lan_canary.ps1 on Windows." >&2
  exit 2
fi

CODEC="$(printf '%s' "$CODEC" | tr '[:upper:]' '[:lower:]')"
if [ "$CODEC" != "h264" ]; then
  echo "macOS local dual-process canary currently supports h264 only." >&2
  exit 2
fi

if [ "$DISPLAY_MODE_POLICY" != "none" ] && [ "$DISPLAY_MODE_POLICY" != "temporary" ] && [ "$DISPLAY_MODE_POLICY" != "required" ]; then
  echo "--display-mode-policy must be one of none, temporary, required." >&2
  exit 2
fi

if ! command -v python3 >/dev/null 2>&1; then
  echo "python3 is required for Unix-socket IPC health checks and report shaping." >&2
  exit 2
fi

if ! command -v pnpm >/dev/null 2>&1; then
  echo "pnpm is required to launch apps/Rdesk tauri:dev." >&2
  exit 2
fi

REPO="$(cd "$REPO_ROOT" && pwd)"
OUTPUT_ROOT="$REPO/$OUTPUT_DIR"
RAW_DIR="$OUTPUT_ROOT/raw"
mkdir -p "$RAW_DIR"

GIT_COMMIT="$(git -C "$REPO" rev-parse --short=12 HEAD)"

if [ "$NO_BUILD" -eq 0 ]; then
  cargo build -p app -p mrd-service
fi

SERVICE_BIN="$REPO/target/debug/mrd-service"
if [ ! -x "$SERVICE_BIN" ]; then
  echo "mrd-service executable was not found at $SERVICE_BIN" >&2
  exit 1
fi

profile_spec() {
  case "$1" in
    720p60) echo "1280 720 60 ${BITRATE_MBPS}" ;;
    1080p60) echo "1920 1080 60 ${BITRATE_MBPS}" ;;
    1080p120) echo "1920 1080 120 ${BITRATE_MBPS}" ;;
    1080p144) echo "1920 1080 144 ${BITRATE_MBPS}" ;;
    2k60) echo "2560 1440 60 ${BITRATE_MBPS}" ;;
    2k144) echo "2560 1440 144 80" ;;
    1600p120) echo "2560 1600 120 80" ;;
    1600p165) echo "2560 1600 165 80" ;;
    *) return 1 ;;
  esac
}

free_udp_port_pair() {
  python3 - <<'PY'
import random
import socket

for _ in range(100):
    base = 21216 + random.randrange(0, 1000) * 2
    sockets = []
    try:
        for port in (base, base + 1):
            sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
            sock.bind(("127.0.0.1", port))
            sockets.append(sock)
        print(f"{base} {base + 1}")
        raise SystemExit(0)
    except OSError:
        pass
    finally:
        for sock in sockets:
            sock.close()
raise SystemExit("could not find two free UDP discovery ports")
PY
}

free_tcp_port() {
  python3 - <<'PY'
import socket
sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
sock.bind(("127.0.0.1", 0))
print(sock.getsockname()[1])
sock.close()
PY
}

wait_ipc_service_health() {
  local socket_path="$1"
  local timeout_secs="$2"
  python3 - "$socket_path" "$timeout_secs" <<'PY'
import json
import socket
import struct
import sys
import time

path = sys.argv[1]
deadline = time.time() + float(sys.argv[2])
last_error = None
payload = b'{"type":"ServiceHealth"}'

while time.time() < deadline:
    client = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    try:
        client.settimeout(1.0)
        client.connect(path)
        client.sendall(struct.pack("<I", len(payload)) + payload)
        header = client.recv(4)
        if len(header) != 4:
            raise RuntimeError("short response header")
        length = struct.unpack("<I", header)[0]
        chunks = []
        remaining = length
        while remaining > 0:
            chunk = client.recv(remaining)
            if not chunk:
                raise RuntimeError("stream closed while reading response")
            chunks.append(chunk)
            remaining -= len(chunk)
        response = json.loads(b"".join(chunks).decode("utf-8"))
        if response.get("type") == "ServiceHealth":
            raise SystemExit(0)
        last_error = f"unexpected response type: {response.get('type')}"
    except Exception as exc:
        last_error = str(exc)
        time.sleep(0.25)
    finally:
        client.close()

raise SystemExit(f"IPC service health timed out for {path}. Last error: {last_error}")
PY
}

kill_tree() {
  local pid="${1:-}"
  if [ -z "$pid" ] || ! kill -0 "$pid" >/dev/null 2>&1; then
    return 0
  fi
  local children
  children="$(pgrep -P "$pid" 2>/dev/null || true)"
  for child in $children; do
    kill_tree "$child"
  done
  kill -TERM "$pid" >/dev/null 2>&1 || true
}

kill_tree_force() {
  local pid="${1:-}"
  if [ -z "$pid" ] || ! kill -0 "$pid" >/dev/null 2>&1; then
    return 0
  fi
  local children
  children="$(pgrep -P "$pid" 2>/dev/null || true)"
  for child in $children; do
    kill_tree_force "$child"
  done
  kill -KILL "$pid" >/dev/null 2>&1 || true
}

write_failure_report() {
  local report_path="$1"
  local message="$2"
  local controller_exit="$3"
  local peer_exit="$4"
  local tauri_exit="$5"
  python3 - "$report_path" "$message" "$controller_exit" "$peer_exit" "$tauri_exit" <<'PY'
import json
import sys

path, message, controller_exit, peer_exit, tauri_exit = sys.argv[1:6]
report = {
    "status": "failed",
    "failureReason": "runtime_error",
    "errorMessage": message,
    "controller_exit_code": None if controller_exit == "-" else controller_exit,
    "peer_exit_code": None if peer_exit == "-" else peer_exit,
    "tauri_exit_code": None if tauri_exit == "-" else tauri_exit,
    "probeSnapshot": None,
    "mediaPipelineSnapshot": None,
    "sessionSnapshot": None,
}
with open(path, "w", encoding="utf-8") as file:
    json.dump(report, file, indent=2)
PY
}

status_from_report() {
  local report_path="$1"
  python3 - "$report_path" <<'PY'
import json
import sys
try:
    with open(sys.argv[1], encoding="utf-8") as file:
        print(json.load(file).get("status", ""))
except Exception:
    print("")
PY
}

write_summary_report() {
  local output_root="$1"
  python3 - "$output_root" "$GIT_COMMIT" "$CHAIN_ID" <<'PY'
import glob
import json
import os
import sys

output_root, git_commit, chain_id = sys.argv[1:4]
rows = []
for raw_path in sorted(glob.glob(os.path.join(output_root, "raw", "local-dual-*.json"))):
    with open(raw_path, encoding="utf-8") as file:
        report = json.load(file)
    raw_name = os.path.basename(raw_path)
    profile_id = raw_name[len("local-dual-"):-len(".json")]
    probe = report.get("probeSnapshot") or {}
    pipeline = report.get("mediaPipelineSnapshot") or {}
    capture = report.get("captureSource") or {}
    fps = report.get("sampleObservedFps")
    if fps is None:
        fps = probe.get("current_fps", 0)
    row = {
        "id": profile_id,
        "mode": "local-dual-process",
        "chain": chain_id,
        "status": report.get("status", "failed"),
        "classification": "completed" if report.get("status") == "completed" else report.get("failureReason", "failed"),
        "fps_observed": fps or 0,
        "decoded_frames": probe.get("frames_decoded", 0),
        "dropped_frames": probe.get("frames_dropped", 0),
        "active_encoder": pipeline.get("active_encoder"),
        "active_decoder": pipeline.get("active_decoder"),
        "active_renderer": pipeline.get("active_renderer"),
        "active_width": pipeline.get("active_width"),
        "active_height": pipeline.get("active_height"),
        "active_fps": pipeline.get("active_fps"),
        "active_bitrate_mbps": pipeline.get("active_bitrate_mbps"),
        "capture_source_id": capture.get("id"),
        "capture_source_kind": capture.get("source_kind"),
        "raw_report_path": raw_path,
        "error_message": report.get("errorMessage"),
    }
    rows.append(row)

summary = {
    "mode": "local-dual-process",
    "platform": "macos",
    "git_commit": git_commit,
    "chain": chain_id,
    "rows": rows,
}

json_path = os.path.join(output_root, "local-dual-process-canary-report.json")
md_path = os.path.join(output_root, "local-dual-process-canary-report.md")
with open(json_path, "w", encoding="utf-8") as file:
    json.dump(summary, file, indent=2)

with open(md_path, "w", encoding="utf-8") as file:
    file.write("# macOS Local Dual-Process LAN Canary Report\n\n")
    file.write(f"- GitCommit: {git_commit}\n")
    file.write(f"- Chain: {chain_id}\n\n")
    file.write("| Profile | Status | FPS | Decoded | Encoder | Decoder | Renderer | Error |\n")
    file.write("| --- | --- | ---: | ---: | --- | --- | --- | --- |\n")
    for row in rows:
        error = (row.get("error_message") or "").replace("|", "\\|").replace("\n", " ")
        file.write(
            f"| {row['id']} | {row['status']} | {row['fps_observed']:.1f} | "
            f"{row['decoded_frames']} | {row.get('active_encoder') or '-'} | "
            f"{row.get('active_decoder') or '-'} | {row.get('active_renderer') or '-'} | {error or '-'} |\n"
        )

print(json_path)
print(md_path)
PY
}

run_profile() {
  local profile_id="$1"
  local spec
  if ! spec="$(profile_spec "$profile_id")"; then
    echo "Unknown profile id: $profile_id" >&2
    return 2
  fi

  local width height fps bitrate
  read -r width height fps bitrate <<EOF
$spec
EOF

  local run_stamp run_uuid run_id run_dir logs_dir run_bin_dir run_service_bin
  run_stamp="$(date '+%Y%m%d-%H%M%S')"
  run_uuid="$(python3 - <<'PY'
import uuid
print(uuid.uuid4().hex[:8])
PY
)"
  run_id="local-dual-${profile_id}-${run_stamp}-${GIT_COMMIT}-${run_uuid}"
  run_dir="$OUTPUT_ROOT/runs/$run_id"
  logs_dir="$run_dir/logs"
  run_bin_dir="$run_dir/bin"
  mkdir -p "$logs_dir/controller" "$logs_dir/peer" "$run_bin_dir"
  cp "$SERVICE_BIN" "$run_bin_dir/mrd-service"
  run_service_bin="$run_bin_dir/mrd-service"

  local ports controller_port peer_port
  ports="$(free_udp_port_pair)"
  read -r controller_port peer_port <<EOF
$ports
EOF

  local controller_socket peer_socket controller_device peer_device report_path
  controller_socket="/tmp/mrd-service-local-controller-${run_id}.sock"
  peer_socket="/tmp/mrd-service-local-peer-${run_id}.sock"
  controller_device="lan-local-controller-${run_id}"
  peer_device="lan-local-peer-${run_id}"
  report_path="$RAW_DIR/local-dual-${profile_id}.json"
  rm -f "$controller_socket" "$peer_socket" "$report_path"

  local controller_pid="" peer_pid="" tauri_pid=""
  trap 'kill_tree "$tauri_pid"; kill_tree "$controller_pid"; kill_tree "$peer_pid"; sleep 1; kill_tree_force "$tauri_pid"; kill_tree_force "$controller_pid"; kill_tree_force "$peer_pid"; rm -f "$controller_socket" "$peer_socket"' RETURN

  echo "Running macOS local dual-process LAN canary ${profile_id}"

  env \
    MRD_SERVICE_IPC_ENDPOINT="$controller_socket" \
    MRD_LAN_DEVICE_ID="$controller_device" \
    MRD_LAN_DEVICE_NAME="Local Dual Controller" \
    MRD_LAN_DISCOVERY_PORT="$controller_port" \
    MRD_LAN_DISCOVERY_PROBE_ENDPOINTS="127.0.0.1:${peer_port}" \
    MRD_SERVICE_BUILD_ID="$GIT_COMMIT" \
    MRD_LAN_RECEIVER_DECODER="videotoolbox" \
    RUST_LOG="info" \
    "$run_service_bin" >"$logs_dir/controller/controller.stdout.log" 2>"$logs_dir/controller/controller.stderr.log" &
  controller_pid="$!"

  env \
    MRD_SERVICE_IPC_ENDPOINT="$peer_socket" \
    MRD_LAN_DEVICE_ID="$peer_device" \
    MRD_LAN_DEVICE_NAME="Local Dual Peer" \
    MRD_LAN_DISCOVERY_PORT="$peer_port" \
    MRD_LAN_DISCOVERY_PROBE_ENDPOINTS="127.0.0.1:${controller_port}" \
    MRD_SERVICE_BUILD_ID="$GIT_COMMIT" \
    RUST_LOG="info" \
    "$run_service_bin" >"$logs_dir/peer/peer.stdout.log" 2>"$logs_dir/peer/peer.stderr.log" &
  peer_pid="$!"

  wait_ipc_service_health "$controller_socket" 20
  wait_ipc_service_health "$peer_socket" 20

  local single_instance_port timeout_ms min_fps
  single_instance_port="$(free_tcp_port)"
  timeout_ms=$((DURATION_SECS * 1000 + 2500))
  min_fps=$((fps / 2))
  if [ "$min_fps" -lt 1 ]; then
    min_fps=1
  fi

  (
    cd "$REPO/apps/Rdesk"
    env \
      MRD_SERVICE_IPC_ENDPOINT="$controller_socket" \
      MRD_SERVICE_BOOTSTRAP_DISABLED="1" \
      MRD_RDESK_SINGLE_INSTANCE_ADDR="127.0.0.1:${single_instance_port}" \
      MRD_LAN_E2E_AUTORUN="1" \
      MRD_LAN_E2E_TARGET_DEVICE_ID="$peer_device" \
      MRD_LAN_E2E_TRANSPORT="quic" \
      MRD_LAN_E2E_TIMEOUT_MS="$timeout_ms" \
      MRD_LAN_E2E_MIN_SAMPLE_DURATION_MS="$((DURATION_SECS * 1000))" \
      MRD_LAN_E2E_MIN_DECODED_FRAMES="20" \
      MRD_LAN_E2E_MIN_FPS="$min_fps" \
      MRD_LAN_E2E_STOP_ON_COMPLETE="true" \
      MRD_LAN_E2E_REPORT_PATH="$report_path" \
      MRD_LAN_E2E_PROFILE_WIDTH="$width" \
      MRD_LAN_E2E_PROFILE_HEIGHT="$height" \
      MRD_LAN_E2E_PROFILE_FPS="$fps" \
      MRD_LAN_E2E_PROFILE_BITRATE_MBPS="$bitrate" \
      MRD_LAN_E2E_PROFILE_CODEC="$CODEC" \
      MRD_LAN_E2E_PROFILE_HDR_ENABLED="false" \
      MRD_LAN_E2E_DISPLAY_MODE_POLICY="$DISPLAY_MODE_POLICY" \
      MRD_LAN_E2E_CAPTURE_SOURCE_ID="$CAPTURE_SOURCE_ID" \
      MRD_LAN_E2E_CAPTURE_SOURCE_KIND="$CAPTURE_SOURCE_KIND" \
      MRD_LAN_E2E_EXPECTED_PEER_BUILD_ID="$GIT_COMMIT" \
      pnpm tauri:dev
  ) >"$logs_dir/tauri.stdout.log" 2>"$logs_dir/tauri.stderr.log" &
  tauri_pid="$!"

  local deadline status controller_exit peer_exit tauri_exit
  deadline=$((SECONDS + DURATION_SECS + 240))
  status=""
  while [ "$SECONDS" -lt "$deadline" ]; do
    if [ -s "$report_path" ]; then
      status="$(status_from_report "$report_path")"
      if [ "$status" = "completed" ] || [ "$status" = "failed" ] || [ "$status" = "skipped" ]; then
        break
      fi
    fi
    if ! kill -0 "$controller_pid" >/dev/null 2>&1 || ! kill -0 "$peer_pid" >/dev/null 2>&1 || ! kill -0 "$tauri_pid" >/dev/null 2>&1; then
      break
    fi
    sleep 1
  done

  if [ ! -s "$report_path" ]; then
    controller_exit="-"
    peer_exit="-"
    tauri_exit="-"
    if ! kill -0 "$controller_pid" >/dev/null 2>&1; then controller_exit="exited"; fi
    if ! kill -0 "$peer_pid" >/dev/null 2>&1; then peer_exit="exited"; fi
    if ! kill -0 "$tauri_pid" >/dev/null 2>&1; then tauri_exit="exited"; fi
    write_failure_report "$report_path" "macOS local dual-process LAN E2E did not produce a completed report before timeout or process exit. Logs: $logs_dir" "$controller_exit" "$peer_exit" "$tauri_exit"
  fi

  if [ "$KEEP_TAURI_OPEN" -eq 0 ]; then
    kill_tree "$tauri_pid"
  fi
  kill_tree "$controller_pid"
  kill_tree "$peer_pid"
  sleep 1
  if [ "$KEEP_TAURI_OPEN" -eq 0 ]; then
    kill_tree_force "$tauri_pid"
  fi
  kill_tree_force "$controller_pid"
  kill_tree_force "$peer_pid"
  rm -f "$controller_socket" "$peer_socket"
  trap - RETURN
}

IFS=',' read -r -a REQUESTED_PROFILES <<<"$PROFILE_IDS"
for profile in "${REQUESTED_PROFILES[@]}"; do
  trimmed="$(printf '%s' "$profile" | xargs)"
  if [ -n "$trimmed" ]; then
    run_profile "$trimmed"
  fi
done

write_summary_report "$OUTPUT_ROOT" >/dev/null
echo "macOS local dual-process LAN canary report written to $OUTPUT_ROOT"
