#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="."
OUTPUT_DIR="target/codex-local-dual-process-canary-macos"
CHAIN_ID="local_dual_process/macos/videotoolbox_h264/quic_datagram_media_v3_or_v2/videotoolbox/macos_native_render_proxy"
PROFILE_IDS="1080p60"
DURATION_SECS=30
BITRATE_MBPS=20
BITRATE_MBPS_SET=0
DISPLAY_MODE_POLICY="none"
CODEC="h264"
CAPTURE_SOURCE_ID=""
CAPTURE_SOURCE_KIND="display"
SOURCE_FIT_PROFILE=0
SYNTHETIC_CV_CAPTURE=0
NO_BUILD=0
KEEP_TAURI_OPEN=0
RENDER_DISPLAY=1
RENDER_MAX_FPS=""
MAX_STEADY_STAGE_P95_MS=10
MAX_REPEAT_LATEST_RATIO="0.25"
MIN_CAPTURE_DIRECT_RATIO="0.99"
MAX_RENDER_PRESENT_SKIP_RATIO="0.02"
RECEIVER_DECODER="videotoolbox"
RENDER_PROXY_ASYNC_PRESENT=""
HEVC_RAW_DECODE_ASYNC=""
HEVC_RAW_DECODE_MAX_PENDING_INPUTS=""

usage() {
  cat <<'EOF'
Usage: run_local_dual_process_lan_canary_macos.sh [options]

Options:
  --repo-root PATH
  --output-dir PATH
  --profile-id ID[,ID...]       Default: 1080p60
  --duration-secs SECONDS       Default: 30
  --duration SECONDS            Alias for --duration-secs
  --bitrate-mbps MBPS          Override the profile default bitrate.
  --display-mode-policy VALUE   none|temporary|required. Default: none
  --codec VALUE                 h264|hevc|av1 for macOS local dual-process canary
  --synthetic-cv-capture        Use the deterministic test:synthetic-cv source for pipeline diagnostics
  --capture-source-id ID
  --capture-source-kind KIND    Default: display
  --source-fit-profile          Fit the requested profile to the selected capture source aspect.
  --min-capture-direct-ratio R  Minimum macOS CVPixelBuffer capture ratio. Default: 0.99
  --max-render-present-skip-ratio R
                                Maximum native present skip ratio. Default: 0.02
  --render-max-fps FPS          Override MRD_LAN_RENDER_MAX_FPS for render pacing experiments.
  --render-proxy-async-present VALUE
                                Override MRD_MACOS_RENDER_PROXY_ASYNC_PRESENT for fallback proxy.
  --hevc-raw-decode-async VALUE Override MRD_VIDEOTOOLBOX_HEVC_RAW_DECODE_ASYNC for the app.
  --hevc-raw-decode-max-pending-inputs VALUE
                                Override MRD_VIDEOTOOLBOX_HEVC_RAW_DECODE_MAX_PENDING_INPUTS for the app.
  --no-render-display           Skip opening the receiver render window for diagnostics
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
    --bitrate-mbps) BITRATE_MBPS="$2"; BITRATE_MBPS_SET=1; shift 2 ;;
    --display-mode-policy) DISPLAY_MODE_POLICY="$2"; shift 2 ;;
    --codec) CODEC="$2"; shift 2 ;;
    --synthetic-cv-capture) SYNTHETIC_CV_CAPTURE=1; shift ;;
    --capture-source-id) CAPTURE_SOURCE_ID="$2"; shift 2 ;;
    --capture-source-kind) CAPTURE_SOURCE_KIND="$2"; shift 2 ;;
    --source-fit-profile) SOURCE_FIT_PROFILE=1; shift ;;
    --min-capture-direct-ratio) MIN_CAPTURE_DIRECT_RATIO="$2"; shift 2 ;;
    --max-render-present-skip-ratio) MAX_RENDER_PRESENT_SKIP_RATIO="$2"; shift 2 ;;
    --render-max-fps) RENDER_MAX_FPS="$2"; shift 2 ;;
    --render-proxy-async-present) RENDER_PROXY_ASYNC_PRESENT="$2"; shift 2 ;;
    --hevc-raw-decode-async) HEVC_RAW_DECODE_ASYNC="$2"; shift 2 ;;
    --hevc-raw-decode-max-pending-inputs) HEVC_RAW_DECODE_MAX_PENDING_INPUTS="$2"; shift 2 ;;
    --no-render-display) RENDER_DISPLAY=0; shift ;;
    --no-build) NO_BUILD=1; shift ;;
    --keep-tauri-open) KEEP_TAURI_OPEN=1; shift ;;
    --no-motion-stimulus) shift ;;
    --help|-h) usage; exit 0 ;;
    *) echo "Unknown option: $1" >&2; usage >&2; exit 2 ;;
  esac
done

if [ "$SYNTHETIC_CV_CAPTURE" -eq 1 ]; then
  export MRD_LAN_TEST_SYNTHETIC_CAPTURE="${MRD_LAN_TEST_SYNTHETIC_CAPTURE:-1}"
  if [ -z "$CAPTURE_SOURCE_ID" ]; then
    CAPTURE_SOURCE_ID="test:synthetic-cv"
  fi
  CAPTURE_SOURCE_KIND="display"
fi

if [ "$(uname -s)" != "Darwin" ]; then
  echo "This runner is macOS-only; use run_local_dual_process_lan_canary.ps1 on Windows." >&2
  exit 2
fi

CODEC="$(printf '%s' "$CODEC" | tr '[:upper:]' '[:lower:]')"
if [ "$CODEC" != "h264" ] && [ "$CODEC" != "hevc" ] && [ "$CODEC" != "av1" ]; then
  echo "macOS local dual-process canary currently supports h264, hevc, or av1." >&2
  exit 2
fi
if [ "$CODEC" = "hevc" ]; then
  CHAIN_ID="local_dual_process/macos/videotoolbox_hevc/quic_datagram_media_v3_or_v2/videotoolbox_hevc/macos_native_render_proxy"
  RECEIVER_DECODER="videotoolbox_hevc"
elif [ "$CODEC" = "av1" ]; then
  CHAIN_ID="local_dual_process/macos/videotoolbox_av1/quic_datagram_media_v3_or_v2/software_av1/macos_native_render"
  RECEIVER_DECODER="software_av1"
fi

if [ "$DISPLAY_MODE_POLICY" != "none" ] && [ "$DISPLAY_MODE_POLICY" != "temporary" ] && [ "$DISPLAY_MODE_POLICY" != "required" ]; then
  echo "--display-mode-policy must be one of none, temporary, required." >&2
  exit 2
fi
if [ -n "$RENDER_MAX_FPS" ]; then
  case "$RENDER_MAX_FPS" in
    ''|*[!0-9]*)
      echo "--render-max-fps must be a positive integer." >&2
      exit 2
      ;;
    *)
      if [ "$RENDER_MAX_FPS" -le 0 ]; then
        echo "--render-max-fps must be a positive integer." >&2
        exit 2
      fi
      ;;
  esac
fi

if ! command -v python3 >/dev/null 2>&1; then
  echo "python3 is required for Unix-socket IPC health checks and report shaping." >&2
  exit 2
fi

python3 - "$MAX_STEADY_STAGE_P95_MS" "$MAX_REPEAT_LATEST_RATIO" "$MIN_CAPTURE_DIRECT_RATIO" "$MAX_RENDER_PRESENT_SKIP_RATIO" <<'PY'
import math
import sys

names = [
    "MAX_STEADY_STAGE_P95_MS",
    "MAX_REPEAT_LATEST_RATIO",
    "MIN_CAPTURE_DIRECT_RATIO",
    "MAX_RENDER_PRESENT_SKIP_RATIO",
]

try:
    max_p95, max_repeat, min_direct, max_present_skip = [float(value) for value in sys.argv[1:5]]
except ValueError as exc:
    raise SystemExit(f"invalid numeric canary threshold: {exc}")

values = dict(zip(names, [max_p95, max_repeat, min_direct, max_present_skip]))
for name, value in values.items():
    if not math.isfinite(value):
        raise SystemExit(f"{name} must be finite")
if max_p95 <= 0:
    raise SystemExit("MAX_STEADY_STAGE_P95_MS must be > 0")
for name in names[1:]:
    value = values[name]
    if value < 0 or value > 1:
        raise SystemExit(f"{name} must be between 0 and 1")
PY

REPO="$(cd "$REPO_ROOT" && pwd)"
OUTPUT_ROOT="$REPO/$OUTPUT_DIR"
RAW_DIR="$OUTPUT_ROOT/raw"
mkdir -p "$RAW_DIR"

GIT_COMMIT="$(git -C "$REPO" rev-parse --short=12 HEAD)"
PNPM_BIN=""
VITE_BIN=""
if command -v pnpm >/dev/null 2>&1; then
  PNPM_BIN="$(command -v pnpm)"
elif [ -x "$REPO/apps/Rdesk/node_modules/.bin/vite" ]; then
  VITE_BIN="$REPO/apps/Rdesk/node_modules/.bin/vite"
else
  echo "pnpm or apps/Rdesk/node_modules/.bin/vite was not found; the runner will use the static Tauri harness fallback." >&2
fi

if [ "$NO_BUILD" -eq 0 ]; then
  cargo build -p app -p mrd-service
  cargo build --manifest-path "$REPO/apps/Rdesk/src-tauri/Cargo.toml" --bin macos_metal_present_probe
fi

SERVICE_BIN="$REPO/target/debug/mrd-service"
if [ ! -x "$SERVICE_BIN" ]; then
  echo "mrd-service executable was not found at $SERVICE_BIN" >&2
  exit 1
fi
APP_BIN="$REPO/target/debug/app"
if [ ! -x "$APP_BIN" ]; then
  echo "Rdesk app executable was not found at $APP_BIN" >&2
  exit 1
fi
METAL_PRESENT_PROBE_BIN="$REPO/target/debug/macos_metal_present_probe"

profile_spec() {
  local default_bitrate
  case "$1" in
    720p60) default_bitrate=20; echo "1280 720 60 $(profile_bitrate "$default_bitrate")" ;;
    1080p60) default_bitrate=20; echo "1920 1080 60 $(profile_bitrate "$default_bitrate")" ;;
    1080p120) default_bitrate=20; echo "1920 1080 120 $(profile_bitrate "$default_bitrate")" ;;
    1080p144) default_bitrate=20; echo "1920 1080 144 $(profile_bitrate "$default_bitrate")" ;;
    2k60) default_bitrate=20; echo "2560 1440 60 $(profile_bitrate "$default_bitrate")" ;;
    2k144)
      if [ "$CODEC" = "hevc" ]; then
        default_bitrate=40
      elif [ "$CODEC" = "av1" ]; then
        default_bitrate=32
      else
        default_bitrate=80
      fi
      echo "2560 1440 144 $(profile_bitrate "$default_bitrate")"
      ;;
    1600p120) default_bitrate=80; echo "2560 1600 120 $(profile_bitrate "$default_bitrate")" ;;
    1600p165) default_bitrate=80; echo "2560 1600 165 $(profile_bitrate "$default_bitrate")" ;;
    native|native120|source-native)
      if [ "$CODEC" = "hevc" ]; then
        default_bitrate=40
      elif [ "$CODEC" = "av1" ]; then
        default_bitrate=32
      else
        default_bitrate=80
      fi
      echo "2560 1600 120 $(profile_bitrate "$default_bitrate")"
      ;;
    *) return 1 ;;
  esac
}

profile_uses_source_fit() {
  case "$1" in
    native|native120|source-native) return 0 ;;
    *) return 1 ;;
  esac
}

profile_bitrate() {
  local default_bitrate="$1"
  if [ "$BITRATE_MBPS_SET" -eq 1 ]; then
    echo "$BITRATE_MBPS"
  else
    echo "$default_bitrate"
  fi
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

prepare_ad_hoc_vite_node() {
  local output_path="$1"
  local log_path="$2"
  local vite_js_path="$3"
  local node_source
  node_source="$(command -v node 2>/dev/null || true)"
  if [ -z "$node_source" ] || [ ! -x "$node_source" ] || [ ! -f "$vite_js_path" ]; then
    return 1
  fi

  {
    echo "Preparing ad-hoc Node for Vite"
    echo "source=$node_source"
    echo "output=$output_path"
    echo "vite=$vite_js_path"
  } >>"$log_path"

  cp "$node_source" "$output_path" >>"$log_path" 2>&1 || return 1
  chmod 755 "$output_path" >>"$log_path" 2>&1 || true
  codesign -s - --force "$output_path" >>"$log_path" 2>&1 || return 1
  "$output_path" "$vite_js_path" --version >>"$log_path" 2>&1 || return 1
  echo "$output_path"
}

run_macos_metal_present_probe_diagnostic() {
  local logs_dir="$1"
  local width="$2"
  local height="$3"
  local fps="$4"
  local bitrate="$5"
  local codec="$6"
  local report_path="$logs_dir/macos_metal_present_probe_${codec}.json"
  local stderr_path="$logs_dir/macos_metal_present_probe_${codec}.stderr.log"

  if [ ! -x "$METAL_PRESENT_PROBE_BIN" ]; then
    echo "Metal present probe skipped: executable not found at $METAL_PRESENT_PROBE_BIN"
    return 0
  fi

  echo "Running macOS Metal present diagnostic for ${codec} ${width}x${height}@${fps}" >&2
  if "$METAL_PRESENT_PROBE_BIN" \
    --frames 60 \
    --warmup 10 \
    --width "$width" \
    --height "$height" \
    --codec "$codec" \
    --fps "$fps" \
    --bitrate-mbps "$bitrate" \
    --show \
    --no-activate-app \
    --content-view \
    --pump-events >"$report_path" 2>"$stderr_path"; then
    echo "Metal present probe completed: $report_path"
  else
    echo "Metal present probe failed: $report_path stderr: $stderr_path"
  fi
}

wait_json_file_ready() {
  local path="$1"
  local timeout_secs="$2"
  python3 - "$path" "$timeout_secs" <<'PY'
import json
import os
import sys
import time

path = sys.argv[1]
deadline = time.time() + float(sys.argv[2])
while time.time() <= deadline:
    if os.path.exists(path) and os.path.getsize(path) > 0:
        try:
            with open(path, encoding="utf-8") as file:
                json.load(file)
            raise SystemExit(0)
        except Exception:
            pass
    time.sleep(0.05)
raise SystemExit(f"timed out waiting for JSON file: {path}")
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

wait_http_ready() {
  local url="$1"
  local timeout_secs="$2"
  python3 - "$url" "$timeout_secs" <<'PY'
import sys
import time
import urllib.request

url = sys.argv[1]
deadline = time.time() + float(sys.argv[2])
last_error = None
opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))

while time.time() < deadline:
    try:
        with opener.open(url, timeout=1.0) as response:
            if response.status < 500:
                raise SystemExit(0)
    except Exception as exc:
        last_error = str(exc)
        time.sleep(0.25)

raise SystemExit(f"HTTP readiness timed out for {url}. Last error: {last_error}")
PY
}

write_static_tauri_lan_e2e_harness() {
  local harness_dir="$1"
  mkdir -p "$harness_dir"
  cat >"$harness_dir/index.html" <<'HTML'
<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>Rdesk LAN E2E Harness</title>
    <style>
      html,
      body {
        width: 100%;
        height: 100%;
        margin: 0;
        overflow: hidden;
        background: #0e141b;
        color: #d7e0ea;
        font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
      }
      #display-surface {
        position: fixed;
        inset: 0;
        background: #111922;
      }
      #panel {
        position: fixed;
        left: 18px;
        top: 18px;
        width: min(720px, calc(100vw - 36px));
        max-height: calc(100vh - 36px);
        padding: 16px;
        box-sizing: border-box;
        border: 1px solid rgba(255, 255, 255, 0.16);
        background: rgba(10, 16, 24, 0.92);
        overflow: auto;
      }
      #status {
        font-size: 14px;
        font-weight: 600;
        margin-bottom: 12px;
      }
      #log {
        white-space: pre-wrap;
        font-size: 12px;
        line-height: 1.4;
        margin: 0;
      }
      body.display #panel {
        display: none;
      }
    </style>
  </head>
  <body>
    <div id="display-surface"></div>
    <section id="panel">
      <div id="status">waiting</div>
      <pre id="log"></pre>
    </section>
    <script>
      (() => {
        "use strict";

        const statusEl = document.getElementById("status");
        const logEl = document.getElementById("log");
        const surfaceEl = document.getElementById("display-surface");

        const sleep = (ms) => new Promise((resolve) => window.setTimeout(resolve, ms));
        const nowMs = () => Date.now();

        function setStatus(value) {
          statusEl.textContent = value;
        }

        function log(message) {
          const line = `[${new Date().toISOString()}] ${message}`;
          logEl.textContent += `${line}\n`;
          console.log(line);
        }

        class LanHarnessError extends Error {
          constructor(reason, message) {
            super(message);
            this.reason = reason;
          }
        }

        async function waitForTauriInvoke() {
          const deadline = nowMs() + 10000;
          while (nowMs() <= deadline) {
            const invoke = window.__TAURI__?.core?.invoke;
            if (typeof invoke === "function") return invoke;
            await sleep(50);
          }
          throw new Error("Tauri global invoke API was not available");
        }

        async function invoke(command, args) {
          const invokeFn = await waitForTauriInvoke();
          return invokeFn(command, args);
        }

        async function invokeRequired(command, args, reason) {
          try {
            return await invoke(command, args);
          } catch (error) {
            const message = error instanceof Error ? error.message : String(error);
            throw new LanHarnessError(reason, `${command}: ${message}`);
          }
        }

        function numberParam(params, name, fallback) {
          const raw = params.get(name);
          if (raw == null || raw.trim() === "") return fallback;
          const value = Number(raw);
          return Number.isFinite(value) ? value : fallback;
        }

        function boolParam(params, name, fallback) {
          const raw = params.get(name);
          if (raw == null || raw.trim() === "") return fallback;
          return ["1", "true", "yes", "on"].includes(raw.trim().toLowerCase());
        }

        function readConfig() {
          const params = new URLSearchParams(window.location.search);
          const codec = (params.get("codec") || "h264").trim().toLowerCase();
          const requestedProfile = {
            width: numberParam(params, "width", 1280),
            height: numberParam(params, "height", 720),
            fps: numberParam(params, "fps", 60),
            bitrate_mbps: numberParam(params, "bitrateMbps", 20),
            codec,
            hdr_enabled: boolParam(params, "hdrEnabled", false),
          };
          if (codec === "hevc") {
            Object.assign(requestedProfile, {
              codec_profile: params.get("codecProfile") || "main",
              bit_depth: numberParam(params, "bitDepth", 8),
              chroma_subsampling: params.get("chromaSubsampling") || "4:2:0",
              pixel_format: params.get("pixelFormat") || "nv12",
            });
          }
          return {
            autorun: params.get("autorun") === "lan-e2e",
            targetDeviceId: params.get("targetDeviceId") || "",
            transport: params.get("transport") || "quic",
            timeoutMs: numberParam(params, "timeoutMs", 32500),
            minSampleDurationMs: numberParam(params, "minSampleDurationMs", 30000),
            minDecodedFrames: numberParam(params, "minDecodedFrames", 20),
            minFps: numberParam(params, "minFps", 30),
            stopOnComplete: boolParam(params, "stopOnComplete", true),
            captureSourceId: params.get("captureSourceId") || "",
            captureSourceKind: params.get("captureSourceKind") || "display",
            renderDisplaySourceId: params.get("renderDisplaySourceId") || "",
            expectedPeerBuildId: params.get("expectedPeerBuildId") || "",
            renderDisplay: boolParam(params, "renderDisplay", true),
            requestedProfile,
          };
        }

        function stage(stages, name, status, error) {
          const item = { stage: name, status, timestamp: nowMs() };
          if (error) item.error = error;
          stages.push(item);
          log(`${name}: ${status}${error ? `: ${error}` : ""}`);
        }

        function getValue(object, snakeName, camelName, fallback) {
          if (!object || typeof object !== "object") return fallback;
          if (Object.prototype.hasOwnProperty.call(object, snakeName)) {
            return object[snakeName];
          }
          if (Object.prototype.hasOwnProperty.call(object, camelName)) {
            return object[camelName];
          }
          return fallback;
        }

        function pipelineCounter(snapshot, key) {
          const value = getValue(snapshot, key, key, 0);
          return typeof value === "number" && Number.isFinite(value) ? value : 0;
        }

        function sourceKind(source) {
          return getValue(source, "source_kind", "sourceKind", "");
        }

        function chooseSource(sources, config) {
          const normalizedId = config.captureSourceId.trim().toLowerCase();
          if (normalizedId) {
            const source = sources.find((candidate) => String(candidate.id || "").toLowerCase() === normalizedId);
            if (!source) {
              throw new LanHarnessError(
                "capture_source_failed",
                `requested remote capture source is unavailable: ${config.captureSourceId}`
              );
            }
            return source;
          }

          const preferredKind = config.captureSourceKind.trim();
          if (preferredKind) {
            const source = sources.find((candidate) => sourceKind(candidate) === preferredKind);
            if (source) return source;
          }

          for (const kind of ["display_shared", "display", "window"]) {
            const source = sources.find((candidate) => sourceKind(candidate) === kind);
            if (source) return source;
          }
          if (!sources.length) {
            throw new LanHarnessError("capture_source_failed", "no remote capture source available");
          }
          return sources[0];
        }

        function captureSourceDisplayPlacementRef(source) {
          const kind = sourceKind(source).toLowerCase();
          if (!kind.includes("display")) return source.id;
          for (const candidate of [source.class_name, source.className, source.title]) {
            const value = typeof candidate === "string" ? candidate.trim() : "";
            if (value && /DISPLAY\d+/i.test(value)) return value;
          }
          return source.id;
        }

        function mediaProbeValid(probeSnapshot) {
          return (
            getValue(probeSnapshot, "media_probe_valid", "mediaProbeValid", false) === true
          );
        }

        function sessionReceiverActive(sessionSnapshot) {
          return (
            getValue(sessionSnapshot, "receiver_active", "receiverActive", false) === true
          );
        }

        function sessionFailed(sessionSnapshot) {
          return (
            getValue(sessionSnapshot, "state", "state", "") === "failed" ||
            Boolean(getValue(sessionSnapshot, "last_error", "lastError", ""))
          );
        }

        function buildReport(state, status, failureReason, errorMessage, extra = {}) {
          const completed = status === "completed";
          return {
            status,
            scenarioId: "lan.e2e.remote_display",
            sessionId: state.sessionId,
            controllerDeviceId: state.controllerDeviceId,
            peer: state.peer,
            displayWindow: state.displayWindow,
            captureSource: state.captureSource,
            captureSourceSelection: state.captureSourceSelection,
            sessionSnapshot: state.sessionSnapshot,
            probeSnapshot: state.probeSnapshot,
            mediaPipelineSnapshot: state.mediaPipelineSnapshot,
            requestedProfile: state.config.requestedProfile,
            faultEvents: [],
            validationMode: "quic_datagram",
            dataPlaneVerified: completed,
            mediaVerified: completed,
            sampleDurationMs: extra.sampleDurationMs || 0,
            sampleFramesDecoded: extra.sampleFramesDecoded || 0,
            sampleFramesDropped: extra.sampleFramesDropped || 0,
            sampleSequenceGapDrops: extra.sampleSequenceGapDrops || 0,
            sampleDecodeErrorDrops: extra.sampleDecodeErrorDrops || 0,
            sampleTransientDrops: extra.sampleTransientDrops || 0,
            sampleFpsElapsedMs: extra.sampleFpsElapsedMs,
            sampleFpsTargetDurationMs: extra.sampleFpsTargetDurationMs,
            sampleObservedFps: extra.sampleObservedFps,
            sampleObservedFpsAtTargetDuration: extra.sampleObservedFpsAtTargetDuration,
            sampleRenderFramesPresented: extra.sampleRenderFramesPresented || 0,
            sampleObservedRenderFps: extra.sampleObservedRenderFps,
            sampleObservedRenderFpsAtTargetDuration: extra.sampleObservedRenderFpsAtTargetDuration,
            sampleRenderQueueReplacements: extra.sampleRenderQueueReplacements || 0,
            sampleRenderPresentSkips: extra.sampleRenderPresentSkips || 0,
            thresholds: {
              minSampleDurationMs: state.config.minSampleDurationMs,
              minDecodedFrames: state.config.minDecodedFrames,
              minFps: state.config.minFps,
            },
            failureReason,
            errorMessage,
            startedAt: state.startedAt,
            finishedAt: nowMs(),
            stages: state.stages,
          };
        }

        function isUnsupportedAv1CapabilityError(error) {
          const message = error instanceof Error ? error.message : String(error || "");
          return (
            /encode\.videotoolbox_av1/i.test(message) ||
            /VTCompressionSessionCreate\(AV1\)/i.test(message)
          );
        }

        async function writeReport(report) {
          window.__MRD_LAN_E2E_REPORT__ = report;
          await invoke("automation_write_report", { report });
        }

        async function attachNativeSurface(displayContext) {
          document.body.classList.add("display");
          const displaySurfaceId = getValue(displayContext, "surface_id", "surfaceId", "");
          setStatus(`attaching native surface ${displaySurfaceId}`);
          let rect = surfaceEl.getBoundingClientRect();
          for (let index = 0; index < 60 && (rect.width <= 0 || rect.height <= 0); index += 1) {
            await new Promise((resolve) => window.requestAnimationFrame(resolve));
            rect = surfaceEl.getBoundingClientRect();
          }

          const payload = {
            rect: {
              x: Math.round(rect.left),
              y: Math.round(rect.top),
              width: Math.max(1, Math.round(rect.width)),
              height: Math.max(1, Math.round(rect.height)),
            },
            enabled: true,
            visible: true,
          };
          const snapshot = await invokeRequired(
            "configure_remote_display_native_surface",
            payload,
            "display_window_failed"
          );
          try {
            await invoke("present_test_harness_frame_on_native_surface");
          } catch (_) {
            // Optional warmup command; old builds may not expose it.
          }
          window.__MRD_STATIC_DISPLAY_READY__ = { context: displayContext, snapshot };
          setStatus(`native surface attached ${displaySurfaceId}`);

          let pending = false;
          window.addEventListener("resize", () => {
            if (pending) return;
            pending = true;
            window.setTimeout(async () => {
              pending = false;
              try {
                const nextRect = surfaceEl.getBoundingClientRect();
                await invoke("configure_remote_display_native_surface", {
                  rect: {
                    x: Math.round(nextRect.left),
                    y: Math.round(nextRect.top),
                    width: Math.max(1, Math.round(nextRect.width)),
                    height: Math.max(1, Math.round(nextRect.height)),
                  },
                  enabled: true,
                  visible: true,
                });
              } catch (error) {
                log(`resize surface sync failed: ${String(error)}`);
              }
            }, 100);
          });
        }

        async function waitForDisplaySurface(state) {
          const deadline = nowMs() + Math.min(state.config.timeoutMs, 15000);
          while (nowMs() <= deadline) {
            state.mediaPipelineSnapshot = await invokeRequired(
              "ipc_media_pipeline_snapshot",
              { sessionId: state.sessionId },
              "runtime_error"
            );
            const surfaces = getValue(state.mediaPipelineSnapshot, "attached_surfaces", "attachedSurfaces", []) || [];
            const displaySurfaceId = getValue(state.displayWindow, "surface_id", "surfaceId", "");
            const attached = surfaces.some(
              (surface) =>
                getValue(surface, "surface_id", "surfaceId", "") === displaySurfaceId
            );
            if (attached) {
              state.displayWindow = {
                ...state.displayWindow,
                renderer_attached: true,
                native_surface_attached: true,
                render_mode: "macos_native",
              };
              return true;
            }
            await sleep(200);
          }
          return false;
        }

        async function runMainHarness(config) {
          const state = {
            config,
            startedAt: nowMs(),
            stages: [],
            controllerDeviceId: null,
            peer: undefined,
            sessionId: `lan-static-${nowMs().toString(36)}-${Math.random().toString(16).slice(2, 8)}`,
            displayWindow: undefined,
            captureSource: undefined,
            captureSourceSelection: undefined,
            sessionSnapshot: undefined,
            probeSnapshot: undefined,
            mediaPipelineSnapshot: undefined,
          };
          let sessionStarted = false;
          const sample = {};

          try {
            setStatus("running LAN E2E harness");
            stage(state.stages, "preflight", "started");
            await invokeRequired("ipc_service_health", undefined, "service_unhealthy");
            try {
              const runtime = await invoke("ipc_runtime_snapshot");
              state.controllerDeviceId =
                getValue(runtime, "device_id", "deviceId", null) ||
                getValue(runtime?.device, "device_id", "deviceId", null);
            } catch (error) {
              log(`runtime snapshot skipped: ${String(error)}`);
            }

            const discoveryDeadline = nowMs() + config.timeoutMs;
            while (nowMs() <= discoveryDeadline) {
              const discovery = await invokeRequired(
                "ipc_refresh_lan_discovery",
                undefined,
                "peer_not_found"
              );
              const peers = Array.isArray(discovery.peers) ? discovery.peers : [];
              state.peer = peers.find((candidate) => candidate.device_id === config.targetDeviceId);
              if (state.peer) break;
              await sleep(500);
            }
            if (!state.peer) {
              const message = `LAN peer not found: ${config.targetDeviceId}`;
              stage(state.stages, "preflight", "failed", message);
              await writeReport(buildReport(state, "failed", "peer_not_found", message, sample));
              return;
            }
            const transports = Array.isArray(state.peer.transports) ? state.peer.transports : [];
            if (!transports.includes(config.transport)) {
              const message = `LAN peer is not ready for ${config.transport}: transports=${transports.join(",")}`;
              stage(state.stages, "preflight", "failed", message);
              await writeReport(buildReport(state, "failed", "peer_not_ready", message, sample));
              return;
            }
            if (
              config.expectedPeerBuildId &&
              String(state.peer.service_build_id || "").trim() !== config.expectedPeerBuildId
            ) {
              const message = `LAN peer build mismatch: expected ${config.expectedPeerBuildId}, got ${state.peer.service_build_id || "unknown"}`;
              stage(state.stages, "preflight", "skipped", message);
              await writeReport(buildReport(state, "skipped", "peer_version_mismatch", message, sample));
              return;
            }
            stage(state.stages, "preflight", "completed");

            stage(state.stages, "pairing", "started");
            try {
              await invokeRequired(
                "ipc_start_lan_remote_session",
                {
                  sessionId: state.sessionId,
                  targetDeviceId: state.peer.device_id,
                  transportKind: config.transport,
                  requestedProfile: config.requestedProfile,
                },
                "session_start_failed"
              );
            } catch (error) {
              if (
                config.requestedProfile.codec === "av1" &&
                error instanceof LanHarnessError &&
                error.reason === "session_start_failed" &&
                isUnsupportedAv1CapabilityError(error)
              ) {
                stage(state.stages, "pairing", "skipped", error.message);
                await writeReport(buildReport(state, "skipped", "unsupported", error.message, sample));
                return;
              }
              throw error;
            }
            sessionStarted = true;
            stage(state.stages, "pairing", "completed");

            stage(state.stages, "capture_source", "started");
            const sources = await invokeRequired(
              "ipc_list_remote_capture_sources",
              { sessionId: state.sessionId, includePreviews: false, limit: 24 },
              "capture_source_failed"
            );
            state.captureSource = chooseSource(Array.isArray(sources) ? sources : [], config);
            state.captureSourceSelection = await invokeRequired(
              "ipc_select_remote_capture_source",
              { sessionId: state.sessionId, sourceId: state.captureSource.id },
              "capture_source_failed"
            );
            if (String(state.captureSourceSelection.status || "").toLowerCase() !== "selected") {
              throw new LanHarnessError(
                "capture_source_failed",
                state.captureSourceSelection.reason || `remote capture source rejected: ${state.captureSource.id}`
              );
            }
            state.captureSource = state.captureSourceSelection.source || state.captureSource;
            stage(state.stages, "capture_source", "completed");

            stage(state.stages, "receiver", "started");
            await invokeRequired(
              "ipc_start_receiver",
              { sessionId: state.sessionId },
              "receiver_start_failed"
            );
            stage(state.stages, "receiver", "completed");

            if (config.renderDisplay) {
              stage(state.stages, "display", "started");
              state.displayWindow = await invokeRequired(
                "open_remote_display_window",
                {
                  sessionId: state.sessionId,
                  surfaceId: null,
                  preferredDisplaySourceId: config.renderDisplaySourceId || null,
                  avoidCaptureSourceId: state.captureSource
                    ? captureSourceDisplayPlacementRef(state.captureSource)
                    : null,
                },
                "display_window_failed"
              );
              if (!(await waitForDisplaySurface(state))) {
                const surfaces = getValue(
                  state.mediaPipelineSnapshot,
                  "attached_surfaces",
                  "attachedSurfaces",
                  []
                ) || [];
                const message = `Remote display native surface did not attach for ${state.displayWindow.label}/${getValue(state.displayWindow, "surface_id", "surfaceId", "")}; attached surfaces: ${surfaces.map((surface) => getValue(surface, "surface_id", "surfaceId", "")).join(", ") || "none"}`;
                stage(state.stages, "display", "failed", message);
                await writeReport(buildReport(state, "failed", "display_window_failed", message, sample));
                return;
              }
              stage(state.stages, "display", "completed");
            } else {
              stage(state.stages, "display", "skipped", "Render display disabled for diagnostics");
            }

            stage(state.stages, "sample", "started");
            const sampleDeadline = nowMs() + Math.max(config.timeoutMs, config.minSampleDurationMs + 1000);
            const sampleStarted = nowMs();
            let baseline = null;
            while (nowMs() <= sampleDeadline) {
              state.sessionSnapshot = await invokeRequired(
                "ipc_session_snapshot",
                { sessionId: state.sessionId },
                "runtime_error"
              );
              state.probeSnapshot = await invokeRequired(
                "ipc_probe_snapshot",
                { sessionId: state.sessionId },
                "runtime_error"
              );
              state.mediaPipelineSnapshot = await invokeRequired(
                "ipc_media_pipeline_snapshot",
                { sessionId: state.sessionId },
                "runtime_error"
              );

              if (sessionFailed(state.sessionSnapshot)) {
                const message =
                  getValue(state.sessionSnapshot, "last_error", "lastError", "") ||
                  "LAN session entered failed state";
                stage(state.stages, "sample", "failed", message);
                await writeReport(buildReport(state, "failed", "runtime_error", message, sample));
                return;
              }
              const probeError = getValue(state.probeSnapshot, "last_error", "lastError", "");
              if (probeError) {
                stage(state.stages, "sample", "failed", probeError);
                await writeReport(buildReport(state, "failed", "runtime_error", probeError, sample));
                return;
              }

              sample.sampleDurationMs = nowMs() - sampleStarted;
              const currentPresented = pipelineCounter(state.mediaPipelineSnapshot, "render_presented_frames");
              const baselineReady = state.probeSnapshot && (!state.displayWindow || currentPresented > 0);
              if (!baseline && baselineReady) {
                baseline = {
                  framesDecoded: getValue(state.probeSnapshot, "frames_decoded", "framesDecoded", 0) || 0,
                  framesDropped: getValue(state.probeSnapshot, "frames_dropped", "framesDropped", 0) || 0,
                  sequenceGapDrops: getValue(state.probeSnapshot, "sequence_gap_drops", "sequenceGapDrops", 0) || 0,
                  decodeErrorDrops: getValue(state.probeSnapshot, "decode_error_drops", "decodeErrorDrops", 0) || 0,
                  transientDrops: getValue(state.probeSnapshot, "transient_drops", "transientDrops", 0) || 0,
                  renderPresentedFrames: currentPresented,
                  renderQueueReplacements: pipelineCounter(state.mediaPipelineSnapshot, "render_queue_replacements"),
                  renderPresentSkips: pipelineCounter(state.mediaPipelineSnapshot, "render_present_skips"),
                  durationMs: sample.sampleDurationMs,
                };
              } else if (baseline && state.probeSnapshot) {
                sample.sampleFramesDecoded = Math.max(
                  0,
                  (getValue(state.probeSnapshot, "frames_decoded", "framesDecoded", 0) || 0) - baseline.framesDecoded
                );
                sample.sampleFramesDropped = Math.max(
                  0,
                  (getValue(state.probeSnapshot, "frames_dropped", "framesDropped", 0) || 0) - baseline.framesDropped
                );
                sample.sampleSequenceGapDrops = Math.max(
                  0,
                  (getValue(state.probeSnapshot, "sequence_gap_drops", "sequenceGapDrops", 0) || 0) - baseline.sequenceGapDrops
                );
                sample.sampleDecodeErrorDrops = Math.max(
                  0,
                  (getValue(state.probeSnapshot, "decode_error_drops", "decodeErrorDrops", 0) || 0) - baseline.decodeErrorDrops
                );
                sample.sampleTransientDrops = Math.max(
                  0,
                  (getValue(state.probeSnapshot, "transient_drops", "transientDrops", 0) || 0) - baseline.transientDrops
                );
                sample.sampleRenderFramesPresented = Math.max(
                  0,
                  currentPresented - baseline.renderPresentedFrames
                );
                sample.sampleRenderQueueReplacements = Math.max(
                  0,
                  pipelineCounter(state.mediaPipelineSnapshot, "render_queue_replacements") - baseline.renderQueueReplacements
                );
                sample.sampleRenderPresentSkips = Math.max(
                  0,
                  pipelineCounter(state.mediaPipelineSnapshot, "render_present_skips") - baseline.renderPresentSkips
                );
                sample.sampleFpsElapsedMs = Math.max(0, sample.sampleDurationMs - baseline.durationMs);
                if (sample.sampleFpsElapsedMs > 0) {
                  sample.sampleObservedFps = sample.sampleFramesDecoded * 1000 / sample.sampleFpsElapsedMs;
                  sample.sampleObservedRenderFps =
                    sample.sampleRenderFramesPresented * 1000 / sample.sampleFpsElapsedMs;
                }
                if (config.minSampleDurationMs > 0 && sample.sampleFpsElapsedMs) {
                  sample.sampleFpsTargetDurationMs = config.minSampleDurationMs;
                  sample.sampleObservedFpsAtTargetDuration =
                    sample.sampleFramesDecoded * 1000 / config.minSampleDurationMs;
                  sample.sampleObservedRenderFpsAtTargetDuration =
                    sample.sampleRenderFramesPresented * 1000 / config.minSampleDurationMs;
                }
              }

              const sampleReady =
                sample.sampleFpsElapsedMs != null &&
                sample.sampleFpsElapsedMs + Math.min(250, Math.floor(config.minSampleDurationMs * 0.01)) >=
                  config.minSampleDurationMs;
              const fpsForThreshold =
                sample.sampleObservedFps ||
                getValue(state.probeSnapshot, "current_fps", "currentFps", 0) ||
                0;
              const renderReady =
                !state.displayWindow ||
                (sample.sampleRenderFramesPresented || 0) >= config.minDecodedFrames;

              if (
                sessionReceiverActive(state.sessionSnapshot) &&
                (sample.sampleFramesDecoded || 0) >= config.minDecodedFrames &&
                mediaProbeValid(state.probeSnapshot) &&
                fpsForThreshold >= config.minFps &&
                renderReady &&
                sampleReady
              ) {
                stage(state.stages, "sample", "completed");
                stage(state.stages, "assert", "completed");
                await writeReport(buildReport(state, "completed", undefined, undefined, sample));
                return;
              }
              await sleep(500);
            }

            const finalFps =
              sample.sampleObservedFps ||
              getValue(state.probeSnapshot, "current_fps", "currentFps", 0) ||
              0;
            const message = `LAN static Tauri quic_datagram did not reach threshold: decoded ${sample.sampleFramesDecoded || 0}/${config.minDecodedFrames}, rendered ${sample.sampleRenderFramesPresented || 0}/${state.displayWindow ? config.minDecodedFrames : 0}, fps ${finalFps}/${config.minFps}, sample ${sample.sampleDurationMs || 0}/${config.minSampleDurationMs} ms`;
            stage(state.stages, "assert", "failed", message);
            await writeReport(buildReport(state, "failed", "no_remote_frames", message, sample));
          } catch (error) {
            const reason = error instanceof LanHarnessError ? error.reason : "runtime_error";
            const message = error instanceof Error ? error.message : String(error);
            stage(state.stages, "assert", "failed", message);
            await writeReport(buildReport(state, "failed", reason, message, sample));
          } finally {
            if (sessionStarted && config.stopOnComplete) {
              try {
                await invoke("ipc_stop_session", { sessionId: state.sessionId });
              } catch (error) {
                log(`stop session failed: ${String(error)}`);
              }
            }
          }
        }

        async function main() {
          try {
            const displayContext = await invoke("current_remote_display_window_context").catch(() => null);
            if (displayContext) {
              await attachNativeSurface(displayContext);
              return;
            }

            const config = readConfig();
            if (!config.autorun) {
              setStatus("idle");
              log(`loaded ${window.location.pathname}${window.location.search}`);
              return;
            }
            await runMainHarness(config);
          } catch (error) {
            const message = error instanceof Error ? error.stack || error.message : String(error);
            setStatus("failed");
            log(message);
          }
        }

        main();
      })();
    </script>
  </body>
</html>
HTML
}

start_static_tauri_lan_e2e_server() {
  local harness_dir="$1"
  local stdout_path="$2"
  local stderr_path="$3"
  python3 - "$harness_dir" >"$stdout_path" 2>"$stderr_path" <<'PY' &
import functools
import http.server
import os
import socketserver
import sys
import urllib.parse

root = os.path.abspath(sys.argv[1])

class Handler(http.server.SimpleHTTPRequestHandler):
    def __init__(self, *args, **kwargs):
        super().__init__(*args, directory=root, **kwargs)

    def do_GET(self):
        path = urllib.parse.urlparse(self.path).path
        rel = urllib.parse.unquote(path).lstrip("/")
        if not rel or not os.path.isfile(os.path.join(root, rel)):
            self.path = "/index.html"
        return super().do_GET()

    def log_message(self, fmt, *args):
        sys.stderr.write("%s\n" % (fmt % args))

class Server(socketserver.ThreadingMixIn, http.server.HTTPServer):
    daemon_threads = True
    allow_reuse_address = True

server = Server(("127.0.0.1", 9531), Handler)
print("static harness ready on http://127.0.0.1:9531", flush=True)
server.serve_forever()
PY
  echo "$!"
}

start_tauri_lan_e2e_app() {
  local stdout_path="$1"
  local stderr_path="$2"
  env \
    MRD_SERVICE_IPC_ENDPOINT="$controller_socket" \
    MRD_SERVICE_PREBUILT_EXE="$run_service_bin" \
    MRD_SERVICE_BOOTSTRAP_DISABLED="1" \
    MRD_RDESK_SINGLE_INSTANCE_ADDR="127.0.0.1:${single_instance_port}" \
    MRD_LAN_E2E_AUTORUN="1" \
    MRD_LAN_E2E_TARGET_DEVICE_ID="$peer_device" \
    MRD_LAN_E2E_TRANSPORT="quic" \
    MRD_LAN_E2E_TIMEOUT_MS="$timeout_ms" \
    MRD_LAN_E2E_MIN_SAMPLE_DURATION_MS="$((DURATION_SECS * 1000))" \
    MRD_LAN_E2E_MIN_DECODED_FRAMES="20" \
    MRD_LAN_E2E_MIN_FPS="$min_fps" \
    MRD_LAN_E2E_STOP_ON_COMPLETE="$lan_e2e_stop_on_complete" \
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
    MRD_LAN_E2E_RENDER_DISPLAY="$RENDER_DISPLAY" \
    MRD_LAN_E2E_SOURCE_FIT_PROFILE="$source_fit_profile" \
    MRD_LAN_RENDER_MAX_FPS="$RENDER_MAX_FPS" \
    MRD_MACOS_RENDER_PROXY_ASYNC_PRESENT="$RENDER_PROXY_ASYNC_PRESENT" \
    MRD_VIDEOTOOLBOX_HEVC_RAW_DECODE_ASYNC="$HEVC_RAW_DECODE_ASYNC" \
    MRD_VIDEOTOOLBOX_HEVC_RAW_DECODE_MAX_PENDING_INPUTS="$HEVC_RAW_DECODE_MAX_PENDING_INPUTS" \
    "$APP_BIN" >"$stdout_path" 2>"$stderr_path" &
  tauri_pid="$!"
}

run_static_tauri_lan_e2e_fallback() {
  local vite_failure_message="$1"
  local harness_dir="$run_dir/static-tauri-harness"
  local static_stdout="$logs_dir/static-tauri-harness.stdout.log"
  local static_stderr="$logs_dir/static-tauri-harness.stderr.log"
  local static_ready_error=""

  write_static_tauri_lan_e2e_harness "$harness_dir"
  vite_pid="$(start_static_tauri_lan_e2e_server "$harness_dir" "$static_stdout" "$static_stderr")"
  if ! static_ready_error="$(wait_http_ready "http://127.0.0.1:9531/" 10 2>&1)"; then
    write_failure_report "$report_path" "${vite_failure_message} Static harness server was not reachable. ${static_ready_error}. Logs: $logs_dir" "-" "-" "-"
    append_local_dual_run_metadata "$report_path" "$run_id" "$run_dir" "$logs_dir" "$controller_pid" "$peer_pid" "$tauri_pid" "$vite_pid" "$render_proxy_pid" "$controller_socket" "$peer_socket" "$controller_device" "$peer_device" "$controller_port" "$peer_port" "static_tauri_harness"
    kill_tree "$vite_pid"
    sleep 1
    kill_tree_force "$vite_pid"
    return 0
  fi

  echo "Vite unavailable; running static Tauri LAN E2E harness for ${profile_id}" >&2
  start_tauri_lan_e2e_app "$logs_dir/tauri-static.stdout.log" "$logs_dir/tauri-static.stderr.log"

  local deadline status controller_exit peer_exit tauri_exit
  deadline=$((SECONDS + DURATION_SECS + 600))
  status=""
  while [ "$SECONDS" -lt "$deadline" ]; do
    if [ -s "$report_path" ]; then
      status="$(status_from_report "$report_path")"
      if [ "$status" = "completed" ] || [ "$status" = "failed" ] || [ "$status" = "skipped" ]; then
        break
      fi
    fi
    if ! kill -0 "$controller_pid" >/dev/null 2>&1 || ! kill -0 "$peer_pid" >/dev/null 2>&1 || ! kill -0 "$tauri_pid" >/dev/null 2>&1 || ! kill -0 "$vite_pid" >/dev/null 2>&1; then
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
    write_failure_report "$report_path" "macOS static Tauri LAN E2E harness did not produce a completed report before timeout or process exit. ${vite_failure_message} Logs: $logs_dir" "$controller_exit" "$peer_exit" "$tauri_exit"
  fi

  if [ -s "$report_path" ] && kill -0 "$peer_pid" >/dev/null 2>&1; then
    enrich_report_with_peer_pipeline_snapshot "$peer_socket" "$report_path" || true
  fi
  if [ -s "$report_path" ]; then
    validate_report_performance_thresholds "$report_path"
    append_local_dual_run_metadata "$report_path" "$run_id" "$run_dir" "$logs_dir" "$controller_pid" "$peer_pid" "$tauri_pid" "$vite_pid" "$render_proxy_pid" "$controller_socket" "$peer_socket" "$controller_device" "$peer_device" "$controller_port" "$peer_port" "static_tauri_harness"
  fi

  if [ "$KEEP_TAURI_OPEN" -eq 0 ]; then
    kill_tree "$tauri_pid"
  fi
  kill_tree "$vite_pid"
  sleep 1
  if [ "$KEEP_TAURI_OPEN" -eq 0 ]; then
    kill_tree_force "$tauri_pid"
  fi
  kill_tree_force "$vite_pid"
  return 0
}

enrich_report_with_peer_pipeline_snapshot() {
  local socket_path="$1"
  local report_path="$2"
  python3 - "$socket_path" "$report_path" <<'PY'
import json
import socket
import struct
import sys

socket_path, report_path = sys.argv[1:3]

try:
    with open(report_path, encoding="utf-8") as file:
        raw_report = json.load(file)
except Exception:
    raise SystemExit(0)

report = (
    raw_report.get("report")
    if isinstance(raw_report.get("report"), dict)
    else raw_report
)
session_id = report.get("sessionId") or raw_report.get("session_id")
if not session_id:
    raise SystemExit(0)

payload = json.dumps({
    "type": "MediaPipelineSnapshot",
    "session_id": session_id,
}).encode("utf-8")

try:
    client = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    client.settimeout(2.0)
    client.connect(socket_path)
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
finally:
    try:
        client.close()
    except Exception:
        pass

if response.get("type") != "MediaPipelineSnapshot":
    report["senderMediaPipelineSnapshotError"] = response
    raw_report["senderMediaPipelineSnapshotError"] = response
else:
    peer_snapshot = response.get("snapshot") or {}
    report["senderMediaPipelineSnapshot"] = peer_snapshot
    raw_report["senderMediaPipelineSnapshot"] = peer_snapshot
    pipeline = report.get("mediaPipelineSnapshot")
    if isinstance(pipeline, dict):
        existing_stages = {
            item.get("stage")
            for item in pipeline.get("stage_metrics", [])
            if isinstance(item, dict)
        }
        merged_stages = list(pipeline.get("stage_metrics", []))
        for item in peer_snapshot.get("stage_metrics", []):
            if not isinstance(item, dict):
                continue
            stage = item.get("stage")
            if not isinstance(stage, str) or not stage.startswith("sender."):
                continue
            if stage in existing_stages:
                continue
            merged_stages.append(item)
            existing_stages.add(stage)
        pipeline["stage_metrics"] = merged_stages
        if pipeline.get("sender_transport") is None and peer_snapshot.get("sender_transport") is not None:
            pipeline["sender_transport"] = peer_snapshot.get("sender_transport")

with open(report_path, "w", encoding="utf-8") as file:
    json.dump(raw_report, file, indent=2)
PY
}

validate_report_performance_thresholds() {
  local report_path="$1"
  python3 - "$report_path" "$MAX_STEADY_STAGE_P95_MS" "$MAX_REPEAT_LATEST_RATIO" "$MIN_CAPTURE_DIRECT_RATIO" "$MAX_RENDER_PRESENT_SKIP_RATIO" "$RENDER_DISPLAY" "$CODEC" <<'PY'
import json
import sys

(
    path,
    max_p95_raw,
    max_repeat_ratio_raw,
    min_capture_direct_ratio_raw,
    max_render_present_skip_ratio_raw,
    render_display_raw,
    expected_codec,
) = sys.argv[1:8]
max_p95 = float(max_p95_raw)
max_repeat_ratio = float(max_repeat_ratio_raw)
min_capture_direct_ratio = float(min_capture_direct_ratio_raw)
max_render_present_skip_ratio = float(max_render_present_skip_ratio_raw)
render_display = render_display_raw not in {"0", "false", "False", "off", "OFF"}
expected_codec = expected_codec.lower()
expected_encoder = f"videotoolbox_{expected_codec}"
expected_receiver_decoders = {
    "h264": {"rdesk_videotoolbox", "videotoolbox", "videotoolbox_h264"},
    "hevc": {"rdesk_videotoolbox_hevc", "videotoolbox_hevc"},
    "av1": {"software_av1", "dav1d"},
}.get(expected_codec, set())

try:
    with open(path, encoding="utf-8") as file:
        raw_report = json.load(file)
except Exception:
    raise SystemExit(0)

report = raw_report.get("report") if isinstance(raw_report.get("report"), dict) else raw_report
if not isinstance(report, dict):
    raise SystemExit(0)

if report.get("status") != "completed":
    raise SystemExit(0)

pipeline = report.get("mediaPipelineSnapshot") or {}
sender_pipeline = report.get("senderMediaPipelineSnapshot") or {}
requested_profile = report.get("requestedProfile") or {}
profile_fps = requested_profile.get("fps")
render_target_fps = pipeline.get("render_pacing_target_fps")
display_limited_reason = None
if (
    render_display
    and isinstance(profile_fps, (int, float))
    and isinstance(render_target_fps, (int, float))
    and profile_fps > 0
    and render_target_fps > 0
    and render_target_fps < profile_fps
):
    display_limited_reason = (
        f"local render target {render_target_fps:.0f}fps is below requested "
        f"{profile_fps:.0f}fps; row is display_refresh_limited"
    )
stage_metrics = pipeline.get("stage_metrics") or []
steady_prefixes = ("sender.", "receiver.")
ignored_stages = {
    "sender.encoder_create",
    # sender.loop is an aggregate cycle timer and includes pacing/idle wait.
    # Validate the active sender work with the capture/encode/send stages below.
    "sender.loop",
    "sender.pacing_wait",
    "receiver.message_wait",
    "receiver.read",
}
active_pixel_format = pipeline.get("active_pixel_format")
if render_display and isinstance(active_pixel_format, str) and active_pixel_format.startswith("proxy_"):
    ignored_stages.add("receiver.decode")
    ignored_stages.add("receiver.record")
steady_metrics = [
    item for item in stage_metrics
    if isinstance(item, dict)
    and isinstance(item.get("stage"), str)
    and item["stage"].startswith(steady_prefixes)
    and item["stage"] not in ignored_stages
]
sender_metrics = [item for item in steady_metrics if item["stage"].startswith("sender.")]
receiver_metrics = [item for item in steady_metrics if item["stage"].startswith("receiver.")]
failures = []
if not sender_metrics:
    failures.append("missing sender steady-state stage metrics")
if not receiver_metrics:
    failures.append("missing receiver steady-state stage metrics")

for item in steady_metrics:
    p95 = item.get("p95_ms")
    if isinstance(p95, (int, float)) and p95 > max_p95:
        failures.append(f"{item['stage']} p95 {p95:.3f}ms exceeds {max_p95:.3f}ms")

sender_transport = (
    sender_pipeline.get("sender_transport")
    or pipeline.get("sender_transport")
    or {}
)
active_encoder = sender_pipeline.get("active_encoder") or pipeline.get("active_encoder")
if not isinstance(active_encoder, str) or active_encoder.lower() != expected_encoder:
    failures.append(
        f"unexpected sender encoder for macOS {expected_codec} chain: "
        f"{active_encoder or 'missing'}; expected {expected_encoder}"
    )
sender_active_codec = sender_pipeline.get("active_codec") or pipeline.get("active_codec")
if not isinstance(sender_active_codec, str) or sender_active_codec.lower() != expected_codec:
    failures.append(
        f"unexpected sender codec for macOS {expected_codec} chain: "
        f"{sender_active_codec or 'missing'}"
    )
codec_fallback_reason = sender_pipeline.get("codec_fallback_reason") or pipeline.get("codec_fallback_reason")
if codec_fallback_reason:
    failures.append(f"unexpected codec fallback for macOS chain: {codec_fallback_reason}")
frames_completed = sender_transport.get("frames_completed") or 0
repeated_latest = sender_transport.get("repeated_latest_frames") or 0
if frames_completed <= 0:
    failures.append("missing sender completed frame count for repeat-latest validation")
else:
    fresh_sender_frames = max(0, frames_completed - repeated_latest)
    repeat_ratio = repeated_latest / frames_completed
    fresh_ratio = fresh_sender_frames / frames_completed
    report["repeatLatestFrameRatio"] = repeat_ratio
    report["freshSenderFrames"] = fresh_sender_frames
    report["freshSenderFrameRatio"] = fresh_ratio
    if repeat_ratio > max_repeat_ratio:
        warnings = report.setdefault("performanceWarnings", [])
        warnings.append(
            f"repeat-latest ratio {repeat_ratio:.3f} exceeds {max_repeat_ratio:.3f} "
            f"({repeated_latest}/{frames_completed}, fresh {fresh_sender_frames}/{frames_completed})"
        )

capture_frame_samples = sender_transport.get("capture_frame_samples") or 0
capture_direct_frames = sender_transport.get("capture_macos_cv_pixel_buffer_frames") or 0
capture_cpu_frames = sender_transport.get("capture_cpu_frames") or 0
if capture_frame_samples <= 0:
    failures.append("missing sender capture frame samples for direct-path validation")
else:
    direct_ratio = capture_direct_frames / capture_frame_samples
    cpu_ratio = capture_cpu_frames / capture_frame_samples
    report["captureDirectFrameRatio"] = direct_ratio
    report["captureCpuFrameRatio"] = cpu_ratio
    if direct_ratio < min_capture_direct_ratio:
        failures.append(
            f"capture direct CVPixelBuffer ratio {direct_ratio:.3f} below "
            f"{min_capture_direct_ratio:.3f} "
            f"({capture_direct_frames}/{capture_frame_samples}); "
            f"cpu ratio {cpu_ratio:.3f}"
        )

sample_render_presented = report.get("sampleRenderFramesPresented")
pipeline_render_presented = pipeline.get("render_presented_frames")
render_presented = (
    sample_render_presented
    if isinstance(sample_render_presented, (int, float))
    else pipeline_render_presented
)
if render_display and (not isinstance(render_presented, (int, float)) or render_presented <= 0):
    failures.append("missing native render-presented frames")
if render_display:
    active_renderer = pipeline.get("active_renderer")
    if not isinstance(active_renderer, str) or active_renderer.lower() not in {"macos", "metal"}:
        failures.append(f"unexpected native renderer for macOS chain: {active_renderer or 'missing'}")
    active_decoder = pipeline.get("active_decoder")
    active_decoder_normalized = active_decoder.lower() if isinstance(active_decoder, str) else None
    if active_decoder_normalized not in expected_receiver_decoders:
        expected_decoder_text = "/".join(sorted(expected_receiver_decoders)) or "videotoolbox"
        failures.append(
            f"unexpected receiver decoder for macOS {expected_codec} chain: "
            f"{active_decoder or 'missing'}; expected {expected_decoder_text}"
        )
    receiver_active_codec = pipeline.get("active_codec")
    if not isinstance(receiver_active_codec, str) or receiver_active_codec.lower() != expected_codec:
        failures.append(
            f"unexpected receiver codec for macOS {expected_codec} chain: "
            f"{receiver_active_codec or 'missing'}"
        )
    thresholds = report.get("thresholds") or {}
    min_decoded_frames = thresholds.get("minDecodedFrames")
    min_fps = thresholds.get("minFps")
    min_render_fps = thresholds.get("minRenderFps")
    if not isinstance(min_render_fps, (int, float)):
        min_render_fps = min_fps
    if isinstance(profile_fps, (int, float)) and profile_fps >= 120:
        high_refresh_min_render_fps = profile_fps * 0.8
        if not isinstance(min_render_fps, (int, float)):
            min_render_fps = high_refresh_min_render_fps
        else:
            min_render_fps = max(min_render_fps, high_refresh_min_render_fps)
    if isinstance(min_render_fps, (int, float)) and min_render_fps > 0:
        thresholds["minRenderFps"] = min_render_fps
        report["thresholds"] = thresholds
    render_fps = report.get("sampleObservedRenderFpsAtTargetDuration")
    if not isinstance(render_fps, (int, float)):
        render_fps = report.get("sampleObservedRenderFps")
    if isinstance(min_decoded_frames, (int, float)) and min_decoded_frames > 0:
        if not isinstance(render_presented, (int, float)) or render_presented < min_decoded_frames:
            presented_value = render_presented if isinstance(render_presented, (int, float)) else 0
            failures.append(
                f"native render-presented sample {presented_value:.0f}/"
                f"{min_decoded_frames:.0f} frames below threshold"
            )
    if isinstance(min_render_fps, (int, float)) and min_render_fps > 0:
        if not isinstance(render_fps, (int, float)) or render_fps < min_render_fps:
            render_fps_value = render_fps if isinstance(render_fps, (int, float)) else 0
            failures.append(
                f"native render FPS {render_fps_value:.1f}/{min_render_fps:.1f} below threshold"
            )
    render_present_skips = report.get("sampleRenderPresentSkips")
    if not isinstance(render_present_skips, (int, float)):
        render_present_skips = pipeline.get("render_present_skips")
    if (
        isinstance(render_present_skips, (int, float))
        and isinstance(render_presented, (int, float))
        and render_presented > 0
    ):
        present_skip_ratio = render_present_skips / render_presented
        report["renderPresentSkipRatio"] = present_skip_ratio
        if present_skip_ratio > max_render_present_skip_ratio:
            failures.append(
                f"native render present skip ratio {present_skip_ratio:.3f} exceeds "
                f"{max_render_present_skip_ratio:.3f} "
                f"({render_present_skips:.0f}/{render_presented:.0f})"
            )

if display_limited_reason:
    if failures:
        warnings = report.setdefault("performanceWarnings", [])
        warnings.append("display-limited threshold observations: " + "; ".join(failures))
    report["status"] = "skipped"
    report["failureReason"] = "display_refresh_limited"
    report["errorMessage"] = display_limited_reason
    stages = report.setdefault("stages", [])
    stages.append({
        "stage": "assert",
        "status": "skipped",
        "timestamp": report.get("finishedAt"),
        "error": display_limited_reason,
    })
elif failures:
    report["status"] = "failed"
    report["failureReason"] = "performance_threshold"
    report["errorMessage"] = "; ".join(failures)
    stages = report.setdefault("stages", [])
    stages.append({
        "stage": "assert",
        "status": "failed",
        "timestamp": report.get("finishedAt"),
        "error": report["errorMessage"],
    })

with open(path, "w", encoding="utf-8") as file:
    json.dump(raw_report, file, indent=2)
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

append_local_dual_run_metadata() {
  local report_path="$1"
  local run_id="$2"
  local run_dir="$3"
  local logs_dir="$4"
  local controller_pid="$5"
  local peer_pid="$6"
  local tauri_pid="$7"
  local vite_pid="$8"
  local render_proxy_pid="$9"
  local controller_socket="${10}"
  local peer_socket="${11}"
  local controller_device="${12}"
  local peer_device="${13}"
  local controller_port="${14}"
  local peer_port="${15}"
  local harness_mode="${16}"
  python3 - "$report_path" "$run_id" "$run_dir" "$logs_dir" "$controller_pid" "$peer_pid" "$tauri_pid" "$vite_pid" "$render_proxy_pid" "$controller_socket" "$peer_socket" "$controller_device" "$peer_device" "$controller_port" "$peer_port" "$harness_mode" <<'PY'
import json
import sys

(
    report_path,
    run_id,
    run_dir,
    logs_dir,
    controller_pid,
    peer_pid,
    tauri_pid,
    vite_pid,
    render_proxy_pid,
    controller_socket,
    peer_socket,
    controller_device,
    peer_device,
    controller_port,
    peer_port,
    harness_mode,
) = sys.argv[1:17]

def maybe_int(value):
    try:
        parsed = int(str(value).strip())
    except (TypeError, ValueError):
        return None
    return parsed if parsed > 0 else None

try:
    with open(report_path, encoding="utf-8") as file:
        report = json.load(file)
except Exception:
    raise SystemExit(0)

controller_pid_int = maybe_int(controller_pid)
peer_pid_int = maybe_int(peer_pid)
controller_port_int = maybe_int(controller_port)
peer_port_int = maybe_int(peer_port)
controller_endpoint = f"127.0.0.1:{controller_port_int}" if controller_port_int else None
peer_endpoint = f"127.0.0.1:{peer_port_int}" if peer_port_int else None

report["localDualProcess"] = {
    "run_id": run_id,
    "run_dir": run_dir,
    "logs_dir": logs_dir,
    "harness_mode": harness_mode,
    "service_process_count": int(controller_pid_int is not None) + int(peer_pid_int is not None),
    "distinct_service_processes": (
        controller_pid_int is not None
        and peer_pid_int is not None
        and controller_pid_int != peer_pid_int
    ),
    "distinct_ipc_endpoints": bool(controller_socket and peer_socket and controller_socket != peer_socket),
    "discovery_path": "udp_loopback_lan_discovery",
    "controller": {
        "role": "controller",
        "pid": controller_pid_int,
        "device_id": controller_device,
        "ipc_endpoint": controller_socket,
        "discovery_port": controller_port_int,
        "probe_endpoint": peer_endpoint,
    },
    "peer": {
        "role": "peer",
        "pid": peer_pid_int,
        "device_id": peer_device,
        "ipc_endpoint": peer_socket,
        "discovery_port": peer_port_int,
        "probe_endpoint": controller_endpoint,
    },
    "tauri": {
        "pid": maybe_int(tauri_pid),
    },
    "vite": {
        "pid": maybe_int(vite_pid),
    },
    "render_proxy": {
        "pid": maybe_int(render_proxy_pid),
    },
}

with open(report_path, "w", encoding="utf-8") as file:
    json.dump(report, file, indent=2)
PY
}

record_local_dual_cleanup() {
  local report_path="$1"
  local controller_pid="$2"
  local peer_pid="$3"
  local tauri_pid="$4"
  local vite_pid="$5"
  local render_proxy_pid="$6"
  local controller_socket="$7"
  local peer_socket="$8"
  local keep_tauri_open="$9"
  python3 - "$report_path" "$controller_pid" "$peer_pid" "$tauri_pid" "$vite_pid" "$render_proxy_pid" "$controller_socket" "$peer_socket" "$keep_tauri_open" <<'PY'
import json
import os
import sys
from datetime import datetime, timezone

(
    report_path,
    controller_pid,
    peer_pid,
    tauri_pid,
    vite_pid,
    render_proxy_pid,
    controller_socket,
    peer_socket,
    keep_tauri_open,
) = sys.argv[1:10]

def maybe_int(value):
    try:
        parsed = int(str(value).strip())
    except (TypeError, ValueError):
        return None
    return parsed if parsed > 0 else None

def pid_alive(value):
    pid = maybe_int(value)
    if pid is None:
        return False
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
    return True

try:
    with open(report_path, encoding="utf-8") as file:
        report = json.load(file)
except Exception:
    raise SystemExit(0)

controller_alive = pid_alive(controller_pid)
peer_alive = pid_alive(peer_pid)
tauri_alive = pid_alive(tauri_pid)
vite_alive = pid_alive(vite_pid)
render_proxy_alive = pid_alive(render_proxy_pid)
ipc_endpoints_removed = not os.path.exists(controller_socket) and not os.path.exists(peer_socket)
tauri_expected_alive = keep_tauri_open == "1"
all_clean = (
    not controller_alive
    and not peer_alive
    and (tauri_expected_alive or not tauri_alive)
    and not vite_alive
    and not render_proxy_alive
    and ipc_endpoints_removed
)

report["processCleanup"] = {
    "recorded_at": datetime.now(timezone.utc).isoformat(),
    "status": "completed" if all_clean else "leaked",
    "controller_alive_after_cleanup": controller_alive,
    "peer_alive_after_cleanup": peer_alive,
    "tauri_alive_after_cleanup": tauri_alive,
    "vite_alive_after_cleanup": vite_alive,
    "render_proxy_alive_after_cleanup": render_proxy_alive,
    "tauri_expected_alive": tauri_expected_alive,
    "ipc_endpoints_removed": ipc_endpoints_removed,
}

with open(report_path, "w", encoding="utf-8") as file:
    json.dump(report, file, indent=2)
PY
}

run_service_only_lan_e2e() {
  local controller_socket="$1"
  local peer_socket="$2"
  local report_path="$3"
  local session_id="$4"
  local target_device_id="$5"
  local width="$6"
  local height="$7"
  local fps="$8"
  local bitrate="$9"
  local timeout_ms="${10}"
  local min_sample_duration_ms="${11}"
  local min_decoded_frames="${12}"
  local min_fps="${13}"
  local capture_source_id="${14}"
  local capture_source_kind="${15}"
  local profile_source_fit="${16:-0}"
  local render_surface_ready_path="${17:-}"
  python3 - "$controller_socket" "$peer_socket" "$report_path" "$session_id" "$target_device_id" "$width" "$height" "$fps" "$bitrate" "$CODEC" "$timeout_ms" "$min_sample_duration_ms" "$min_decoded_frames" "$min_fps" "$capture_source_id" "$capture_source_kind" "$profile_source_fit" "$render_surface_ready_path" <<'PY'
import json
import socket
import struct
import sys
import time

(
    controller_socket,
    peer_socket,
    report_path,
    session_id,
    target_device_id,
    width,
    height,
    fps,
    bitrate,
    codec,
    timeout_ms,
    min_sample_duration_ms,
    min_decoded_frames,
    min_fps,
    capture_source_id,
    capture_source_kind,
    profile_source_fit_raw,
    render_surface_ready_path,
) = sys.argv[1:19]

width = int(width)
height = int(height)
fps = int(fps)
bitrate = int(bitrate)
timeout_ms = int(timeout_ms)
min_sample_duration_ms = int(min_sample_duration_ms)
min_decoded_frames = int(min_decoded_frames)
min_fps = float(min_fps)
profile_source_fit = profile_source_fit_raw.strip().lower() in ("1", "true", "yes", "on")
started_at = int(time.time() * 1000)
stages = []
sample_interval_s = 0.5


def now_ms():
    return int(time.time() * 1000)


def stage(name, status, error=None):
    item = {"stage": name, "status": status, "timestamp": now_ms()}
    if error:
        item["error"] = error
    stages.append(item)


def ipc(socket_path, payload):
    body = json.dumps(payload).encode("utf-8")
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as client:
        client.settimeout(5.0)
        client.connect(socket_path)
        client.sendall(struct.pack("<I", len(body)) + body)
        header = client.recv(4)
        if len(header) != 4:
            raise RuntimeError("short IPC response header")
        length = struct.unpack("<I", header)[0]
        chunks = []
        remaining = length
        while remaining:
            chunk = client.recv(remaining)
            if not chunk:
                raise RuntimeError("IPC stream closed while reading response")
            chunks.append(chunk)
            remaining -= len(chunk)
    response = json.loads(b"".join(chunks).decode("utf-8"))
    if response.get("type") == "Error":
        raise RuntimeError(f"{response.get('code')}: {response.get('message')}")
    return response


def report(status, failure_reason=None, error_message=None, **extra):
    requested_profile = extra.get("requested_profile") or globals().get("current_requested_profile")
    if not requested_profile:
        requested_profile = {
            "width": width,
            "height": height,
            "fps": fps,
            "bitrate_mbps": bitrate,
            "codec": codec,
            **({"codec_profile": "main", "bit_depth": 8, "chroma_subsampling": "4:2:0", "pixel_format": "nv12", "hdr_enabled": False} if codec == "hevc" else {}),
        }
    data = {
        "status": status,
        "scenarioId": "lan.e2e.remote_display",
        "sessionId": session_id,
        "controllerDeviceId": None,
        "peer": extra.get("peer"),
        "captureSource": extra.get("capture_source"),
        "captureSourceSelection": extra.get("capture_selection"),
        "displayWindow": extra.get("display_window", globals().get("display_window")),
        "sessionSnapshot": extra.get("session_snapshot"),
        "probeSnapshot": extra.get("probe_snapshot"),
        "mediaPipelineSnapshot": extra.get("pipeline_snapshot"),
        "requestedProfile": requested_profile,
        "faultEvents": [],
        "validationMode": "quic_datagram",
        "dataPlaneVerified": status == "completed",
        "mediaVerified": status == "completed",
        "sampleDurationMs": extra.get("sample_duration_ms", 0),
        "sampleFramesDecoded": extra.get("sample_frames_decoded", 0),
        "sampleFramesDropped": extra.get("sample_frames_dropped", 0),
        "sampleSequenceGapDrops": extra.get("sample_sequence_gap_drops", 0),
        "sampleDecodeErrorDrops": extra.get("sample_decode_error_drops", 0),
        "sampleTransientDrops": extra.get("sample_transient_drops", 0),
        "sampleFpsElapsedMs": extra.get("sample_fps_elapsed_ms"),
        "sampleFpsTargetDurationMs": extra.get("sample_fps_target_duration_ms"),
        "sampleObservedFps": extra.get("sample_observed_fps"),
        "sampleObservedFpsAtTargetDuration": extra.get("sample_observed_fps_at_target_duration"),
        "sampleRenderFramesPresented": extra.get("sample_render_frames_presented", 0),
        "sampleRenderQueueReplacements": extra.get("sample_render_queue_replacements", 0),
        "sampleRenderPresentSkips": extra.get("sample_render_present_skips", 0),
        "sampleObservedRenderFps": extra.get("sample_observed_render_fps"),
        "sampleObservedRenderFpsAtTargetDuration": extra.get("sample_observed_render_fps_at_target_duration"),
        "thresholds": {
            "minSampleDurationMs": min_sample_duration_ms,
            "minDecodedFrames": min_decoded_frames,
            "minFps": min_fps,
        },
        "failureReason": failure_reason,
        "errorMessage": error_message,
        "startedAt": started_at,
        "finishedAt": now_ms(),
        "stages": stages,
    }
    with open(report_path, "w", encoding="utf-8") as file:
        json.dump(data, file, indent=2)
    return data


def is_unsupported_av1_capability_error(message):
    text = str(message or "").lower()
    return (
        "encode.videotoolbox_av1" in text
        or "vtcompressionsessioncreate(av1)" in text
    )


UNAVAILABLE_AUTO_SELECT_CLASS_NAMES = {
    "screencapturekitdisplayunavailable",
}


def source_available_for_auto_select(source):
    class_name = str(source.get("class_name") or source.get("className") or "").strip().lower()
    return class_name not in UNAVAILABLE_AUTO_SELECT_CLASS_NAMES


def even_dimension(value):
    return max(2, int(value) & ~1)


def source_fitted_profile(source, profile):
    source_width = source.get("width", 0)
    source_height = source.get("height", 0)
    if source_width <= 0 or source_height <= 0:
        return None
    scale = min(profile["width"] / source_width, profile["height"] / source_height, 1.0)
    fitted_width = even_dimension(max(2, round(source_width * scale)))
    fitted_height = even_dimension(max(2, round(source_height * scale)))
    if fitted_width == profile["width"] and fitted_height == profile["height"]:
        return None
    fitted = dict(profile)
    fitted["width"] = fitted_width
    fitted["height"] = fitted_height
    return fitted


def media_profiles_match(left, right):
    return (
        left.get("width") == right.get("width")
        and left.get("height") == right.get("height")
        and left.get("fps") == right.get("fps")
        and left.get("bitrate_mbps") == right.get("bitrate_mbps")
        and str(left.get("codec", "")).lower() == str(right.get("codec", "")).lower()
        and left.get("codec_profile") == right.get("codec_profile")
        and left.get("bit_depth") == right.get("bit_depth")
        and left.get("chroma_subsampling") == right.get("chroma_subsampling")
        and left.get("pixel_format") == right.get("pixel_format")
        and left.get("hdr_enabled") == right.get("hdr_enabled")
    )


def choose_source(sources):
    normalized_id = capture_source_id.strip().lower()
    if normalized_id:
        for source in sources:
            if source.get("id", "").lower() == normalized_id:
                return source
        raise RuntimeError(f"requested remote capture source is unavailable: {capture_source_id}")
    auto_sources = [source for source in sources if source_available_for_auto_select(source)]
    normalized_kind = capture_source_kind.strip()
    if normalized_kind:
        for source in auto_sources:
            if source.get("source_kind") == normalized_kind:
                return source
    for kind in ("display_shared", "display", "window"):
        for source in auto_sources:
            if source.get("source_kind") == kind:
                return source
    if auto_sources:
        return auto_sources[0]
    if not sources:
        raise RuntimeError("no remote capture source available")
    return sources[0]


def pipeline_counter(snapshot, key):
    value = (snapshot or {}).get(key, 0)
    if isinstance(value, (int, float)):
        return value
    return 0


peer = None
capture_source = None
capture_selection = None
display_window = None
session_snapshot = None
probe_snapshot = None
pipeline_snapshot = None
session_started = False
current_requested_profile = None

try:
    stage("preflight", "started")
    ipc(controller_socket, {"type": "ServiceHealth"})
    ipc(peer_socket, {"type": "ServiceHealth"})
    discovery_deadline = time.time() + timeout_ms / 1000
    while time.time() <= discovery_deadline:
        discovery = ipc(controller_socket, {"type": "RefreshLanDiscovery"}).get("snapshot", {})
        for candidate in discovery.get("peers", []):
            if candidate.get("device_id") == target_device_id:
                peer = candidate
                break
        if peer:
            break
        time.sleep(sample_interval_s)
    if not peer:
        message = f"LAN peer not found: {target_device_id}"
        stage("preflight", "failed", message)
        report("failed", "peer_not_found", message)
        raise SystemExit(0)
    if "quic" not in peer.get("transports", []):
        message = f"LAN peer is not ready for quic: transports={peer.get('transports')}"
        stage("preflight", "failed", message)
        report("failed", "peer_not_ready", message, peer=peer)
        raise SystemExit(0)
    stage("preflight", "completed")

    requested_profile = {
        "width": width,
        "height": height,
        "fps": fps,
        "bitrate_mbps": bitrate,
        "codec": codec,
    }
    if codec == "hevc":
        requested_profile.update({
            "codec_profile": "main",
            "bit_depth": 8,
            "chroma_subsampling": "4:2:0",
            "pixel_format": "nv12",
            "hdr_enabled": False,
        })
    current_requested_profile = requested_profile

    stage("pairing", "started")
    try:
        ipc(controller_socket, {
            "type": "StartLanRemoteSession",
            "session_id": session_id,
            "target_device_id": target_device_id,
            "transport_kind": "quic",
            "requested_profile": requested_profile,
        })
    except Exception as exc:
        message = str(exc)
        if codec == "av1" and is_unsupported_av1_capability_error(message):
            stage("pairing", "skipped", message)
            report("skipped", "unsupported", message, peer=peer)
            raise SystemExit(0)
        raise
    session_started = True
    stage("pairing", "completed")

    stage("capture_source", "started")
    sources = ipc(controller_socket, {
        "type": "ListRemoteCaptureSources",
        "session_id": session_id,
        "include_previews": False,
        "limit": 24,
    }).get("sources", [])
    capture_source = choose_source(sources)
    capture_selection = ipc(controller_socket, {
        "type": "SelectRemoteCaptureSource",
        "session_id": session_id,
        "source_id": capture_source["id"],
    }).get("selection")
    if not capture_selection or str(capture_selection.get("status", "")).lower() != "selected":
        raise RuntimeError(f"remote capture source rejected: {capture_source.get('id')}")
    capture_source = capture_selection.get("source") or capture_source
    stage("capture_source", "completed")
    if profile_source_fit:
        next_profile = source_fitted_profile(capture_source, requested_profile)
        if next_profile:
            for _ in range(3):
                update_response = ipc(controller_socket, {
                    "type": "UpdateMediaProfile",
                    "session_id": session_id,
                    "requested_profile": next_profile,
                })
                negotiation = update_response.get("negotiation", {})
                selected_profile = negotiation.get("selected") if isinstance(negotiation, dict) else None
                if not isinstance(selected_profile, dict):
                    requested_profile = next_profile
                    current_requested_profile = requested_profile
                    break
                if media_profiles_match(selected_profile, next_profile):
                    requested_profile = selected_profile
                    current_requested_profile = requested_profile
                    break
                next_profile = selected_profile
            else:
                requested_profile = next_profile
                current_requested_profile = requested_profile

    if render_surface_ready_path:
        stage("display", "started")
        with open(render_surface_ready_path, encoding="utf-8") as file:
            render_surface = json.load(file)
        attach_response = ipc(controller_socket, {
            "type": "AttachRenderSurface",
            "session_id": session_id,
            "surface_id": render_surface["surface_id"],
            "backend": render_surface.get("backend") or "macos",
            "window_handle": render_surface.get("window_handle"),
            "render_proxy_endpoint": render_surface.get("render_proxy_endpoint"),
        })
        display_window = {
            "mode": "native_render_proxy",
            "surface_id": render_surface["surface_id"],
            "backend": render_surface.get("backend") or "macos",
            "window_handle": render_surface.get("window_handle"),
            "render_proxy_endpoint": render_surface.get("render_proxy_endpoint"),
            "attach_response": attach_response,
        }
        stage("display", "completed")
    else:
        stage("display", "skipped", "service-only canary fallback does not attach a render surface")

    stage("receiver", "started")
    ipc(controller_socket, {"type": "StartReceiver", "session_id": session_id})
    stage("receiver", "completed")

    stage("sample", "started")
    sample_deadline = time.time() + max(timeout_ms, min_sample_duration_ms + 1000) / 1000
    sample_started = now_ms()
    baseline = None
    sample_frames_decoded = 0
    sample_frames_dropped = 0
    sample_sequence_gap_drops = 0
    sample_decode_error_drops = 0
    sample_transient_drops = 0
    sample_render_frames_presented = 0
    sample_render_queue_replacements = 0
    sample_render_present_skips = 0
    sample_observed_render_fps = None
    sample_observed_render_fps_at_target = None
    sample_fps_elapsed_ms = None
    sample_observed_fps = None
    sample_observed_fps_at_target = None
    sample_duration_ms = 0
    while time.time() <= sample_deadline:
        session_snapshot = ipc(controller_socket, {"type": "SessionRuntimeSnapshot", "session_id": session_id}).get("snapshot")
        probe_snapshot = ipc(controller_socket, {"type": "ProbeSnapshot", "session_id": session_id}).get("snapshot")
        pipeline_snapshot = ipc(controller_socket, {"type": "MediaPipelineSnapshot", "session_id": session_id}).get("snapshot")
        if session_snapshot and (session_snapshot.get("state") == "failed" or session_snapshot.get("last_error")):
            message = session_snapshot.get("last_error") or "LAN session entered failed state"
            stage("sample", "failed", message)
            report("failed", "runtime_error", message, peer=peer, capture_source=capture_source, capture_selection=capture_selection, session_snapshot=session_snapshot, probe_snapshot=probe_snapshot, pipeline_snapshot=pipeline_snapshot)
            raise SystemExit(0)
        if probe_snapshot and probe_snapshot.get("last_error"):
            message = probe_snapshot["last_error"]
            stage("sample", "failed", message)
            report("failed", "runtime_error", message, peer=peer, capture_source=capture_source, capture_selection=capture_selection, session_snapshot=session_snapshot, probe_snapshot=probe_snapshot, pipeline_snapshot=pipeline_snapshot)
            raise SystemExit(0)
        sample_duration_ms = now_ms() - sample_started
        current_render_presented = pipeline_counter(pipeline_snapshot, "render_presented_frames")
        baseline_ready = probe_snapshot and (
            not display_window
            or current_render_presented > 0
        )
        if baseline is None and baseline_ready:
            baseline = {
                "frames_decoded": probe_snapshot.get("frames_decoded", 0),
                "frames_dropped": probe_snapshot.get("frames_dropped", 0),
                "sequence_gap_drops": probe_snapshot.get("sequence_gap_drops", 0),
                "decode_error_drops": probe_snapshot.get("decode_error_drops", 0),
                "transient_drops": probe_snapshot.get("transient_drops", 0),
                "render_presented_frames": current_render_presented,
                "render_queue_replacements": pipeline_counter(pipeline_snapshot, "render_queue_replacements"),
                "render_present_skips": pipeline_counter(pipeline_snapshot, "render_present_skips"),
                "duration_ms": sample_duration_ms,
            }
        elif baseline and probe_snapshot:
            sample_frames_decoded = max(0, probe_snapshot.get("frames_decoded", 0) - baseline["frames_decoded"])
            sample_frames_dropped = max(0, probe_snapshot.get("frames_dropped", 0) - baseline["frames_dropped"])
            sample_sequence_gap_drops = max(0, probe_snapshot.get("sequence_gap_drops", 0) - baseline["sequence_gap_drops"])
            sample_decode_error_drops = max(0, probe_snapshot.get("decode_error_drops", 0) - baseline["decode_error_drops"])
            sample_transient_drops = max(0, probe_snapshot.get("transient_drops", 0) - baseline["transient_drops"])
            sample_render_frames_presented = max(0, pipeline_counter(pipeline_snapshot, "render_presented_frames") - baseline["render_presented_frames"])
            sample_render_queue_replacements = max(0, pipeline_counter(pipeline_snapshot, "render_queue_replacements") - baseline["render_queue_replacements"])
            sample_render_present_skips = max(0, pipeline_counter(pipeline_snapshot, "render_present_skips") - baseline["render_present_skips"])
            sample_fps_elapsed_ms = max(0, sample_duration_ms - baseline["duration_ms"])
            if sample_fps_elapsed_ms > 0:
                sample_observed_fps = sample_frames_decoded * 1000 / sample_fps_elapsed_ms
                sample_observed_render_fps = sample_render_frames_presented * 1000 / sample_fps_elapsed_ms
            if min_sample_duration_ms > 0 and sample_fps_elapsed_ms:
                sample_observed_fps_at_target = sample_frames_decoded * 1000 / min_sample_duration_ms
                sample_observed_render_fps_at_target = sample_render_frames_presented * 1000 / min_sample_duration_ms
        sample_ready = (
            sample_fps_elapsed_ms is not None
            and sample_fps_elapsed_ms + min(250, int(min_sample_duration_ms * 0.01)) >= min_sample_duration_ms
        )
        fps_for_threshold = sample_observed_fps or (probe_snapshot or {}).get("current_fps") or 0
        render_ready = (
            not display_window
            or sample_render_frames_presented >= min_decoded_frames
        )
        if (
            session_snapshot
            and session_snapshot.get("receiver_active")
            and sample_frames_decoded >= min_decoded_frames
            and (probe_snapshot or {}).get("media_probe_valid") is True
            and fps_for_threshold >= min_fps
            and render_ready
            and sample_ready
        ):
            stage("sample", "completed")
            stage("assert", "completed")
            report("completed", peer=peer, capture_source=capture_source, capture_selection=capture_selection, session_snapshot=session_snapshot, probe_snapshot=probe_snapshot, pipeline_snapshot=pipeline_snapshot, sample_duration_ms=sample_duration_ms, sample_frames_decoded=sample_frames_decoded, sample_frames_dropped=sample_frames_dropped, sample_sequence_gap_drops=sample_sequence_gap_drops, sample_decode_error_drops=sample_decode_error_drops, sample_transient_drops=sample_transient_drops, sample_render_frames_presented=sample_render_frames_presented, sample_render_queue_replacements=sample_render_queue_replacements, sample_render_present_skips=sample_render_present_skips, sample_observed_render_fps=sample_observed_render_fps, sample_observed_render_fps_at_target_duration=sample_observed_render_fps_at_target, sample_fps_elapsed_ms=sample_fps_elapsed_ms, sample_fps_target_duration_ms=min_sample_duration_ms if min_sample_duration_ms > 0 else None, sample_observed_fps=sample_observed_fps, sample_observed_fps_at_target_duration=sample_observed_fps_at_target)
            raise SystemExit(0)
        time.sleep(sample_interval_s)

    final_fps = sample_observed_fps or (probe_snapshot or {}).get("current_fps") or 0
    message = f"LAN service-only quic_datagram did not reach threshold: decoded {sample_frames_decoded}/{min_decoded_frames}, rendered {sample_render_frames_presented}/{min_decoded_frames if display_window else 0}, fps {final_fps}/{min_fps}, sample {sample_duration_ms}/{min_sample_duration_ms} ms"
    stage("assert", "failed", message)
    report("failed", "no_remote_frames", message, peer=peer, capture_source=capture_source, capture_selection=capture_selection, session_snapshot=session_snapshot, probe_snapshot=probe_snapshot, pipeline_snapshot=pipeline_snapshot, sample_duration_ms=sample_duration_ms, sample_frames_decoded=sample_frames_decoded, sample_frames_dropped=sample_frames_dropped, sample_sequence_gap_drops=sample_sequence_gap_drops, sample_decode_error_drops=sample_decode_error_drops, sample_transient_drops=sample_transient_drops, sample_render_frames_presented=sample_render_frames_presented, sample_render_queue_replacements=sample_render_queue_replacements, sample_render_present_skips=sample_render_present_skips, sample_observed_render_fps=sample_observed_render_fps, sample_observed_render_fps_at_target_duration=sample_observed_render_fps_at_target, sample_fps_elapsed_ms=sample_fps_elapsed_ms, sample_fps_target_duration_ms=min_sample_duration_ms if min_sample_duration_ms > 0 else None, sample_observed_fps=sample_observed_fps, sample_observed_fps_at_target_duration=sample_observed_fps_at_target)
except SystemExit:
    raise
except Exception as exc:
    message = str(exc)
    stage("assert", "failed", message)
    report("failed", "runtime_error", message, peer=peer, capture_source=capture_source, capture_selection=capture_selection, session_snapshot=session_snapshot, probe_snapshot=probe_snapshot, pipeline_snapshot=pipeline_snapshot)
finally:
    if session_started:
        try:
            ipc(controller_socket, {"type": "StopSession", "session_id": session_id})
        except Exception:
            pass
PY
}

status_from_report() {
  local report_path="$1"
  python3 - "$report_path" <<'PY'
import json
import sys
try:
    with open(sys.argv[1], encoding="utf-8") as file:
        report = json.load(file)
        print(
            report.get("status")
            or report.get("final_status")
            or (report.get("report") or {}).get("status")
            or ""
        )
except Exception:
    print("")
PY
}

write_summary_report() {
  local output_root="$1"
  python3 - "$output_root" "$GIT_COMMIT" "$CHAIN_ID" "$RENDER_MAX_FPS" <<'PY'
import glob
import json
import os
import sys

output_root, git_commit, chain_id, render_max_fps_raw = sys.argv[1:5]
try:
    render_max_fps_override = int(render_max_fps_raw) if render_max_fps_raw else None
except ValueError:
    render_max_fps_override = None
rows = []
DEFAULT_MACOS_RENDER_TARGET_FPS = 120
def stage_p95(pipeline, stage_name):
    for item in pipeline.get("stage_metrics") or []:
        if isinstance(item, dict) and item.get("stage") == stage_name:
            value = item.get("p95_ms")
            if isinstance(value, (int, float)):
                return value
    return None

def ratio(numerator, denominator):
    if isinstance(numerator, (int, float)) and isinstance(denominator, (int, float)) and denominator > 0:
        return numerator / denominator
    return None

def sender_stage_p95(sender_pipeline, receiver_pipeline, stage_name):
    value = stage_p95(sender_pipeline, stage_name)
    return value if value is not None else stage_p95(receiver_pipeline, stage_name)

def is_unsupported_av1_report(report):
    profile = report.get("requestedProfile") or {}
    if str(profile.get("codec", "")).lower() != "av1":
        return False
    message = " ".join(
        str(value or "")
        for value in (report.get("failureReason"), report.get("errorMessage"))
    ).lower()
    return (
        "encode.videotoolbox_av1" in message
        or "vtcompressionsessioncreate(av1)" in message
    )

def display_refresh_limit_reason(requested_fps, render_target_fps):
    if not isinstance(requested_fps, (int, float)):
        return None
    if not isinstance(render_target_fps, (int, float)):
        return None
    if requested_fps <= 0 or render_target_fps <= 0:
        return None
    if render_target_fps >= requested_fps:
        return None
    return (
        f"local render target {render_target_fps:.0f}fps is below requested "
        f"{requested_fps:.0f}fps; row is display_refresh_limited"
    )

def inferred_render_target_fps(requested_fps, render_target_fps):
    if isinstance(render_target_fps, (int, float)):
        return render_target_fps
    if not isinstance(requested_fps, (int, float)) or requested_fps <= 0:
        return None
    if isinstance(render_max_fps_override, int) and render_max_fps_override > 0:
        return min(requested_fps, render_max_fps_override)
    if requested_fps >= DEFAULT_MACOS_RENDER_TARGET_FPS:
        return min(requested_fps, DEFAULT_MACOS_RENDER_TARGET_FPS)
    return None

for raw_path in sorted(glob.glob(os.path.join(output_root, "raw", "local-dual-*.json"))):
    with open(raw_path, encoding="utf-8") as file:
        raw_report = json.load(file)
    report = (
        raw_report.get("report")
        if isinstance(raw_report.get("report"), dict)
        else raw_report
    )
    raw_name = os.path.basename(raw_path)
    profile_id = raw_name[len("local-dual-"):-len(".json")]
    probe = report.get("probeSnapshot") or {}
    pipeline = report.get("mediaPipelineSnapshot") or {}
    capture = report.get("captureSource") or raw_report.get("capture_source") or {}
    local_dual = raw_report.get("localDualProcess") or report.get("localDualProcess") or {}
    cleanup = raw_report.get("processCleanup") or report.get("processCleanup") or {}
    display_window = report.get("displayWindow") or {}
    controller_process = local_dual.get("controller") or {}
    peer_process = local_dual.get("peer") or {}
    requested_profile = report.get("requestedProfile") or {}
    fps_elapsed = report.get("sampleObservedFps")
    fps_target = report.get("sampleObservedFpsAtTargetDuration")
    fps = fps_target
    if fps is None:
        fps = fps_elapsed
    if fps is None:
        fps = probe.get("current_fps", 0)
    render_fps_elapsed = report.get("sampleObservedRenderFps")
    render_fps_target = report.get("sampleObservedRenderFpsAtTargetDuration")
    render_fps = render_fps_target
    if render_fps is None:
        render_fps = render_fps_elapsed
    thresholds = report.get("thresholds") or {}
    sender_pipeline = report.get("senderMediaPipelineSnapshot") or {}
    sender_transport = sender_pipeline.get("sender_transport") or pipeline.get("sender_transport") or {}
    sender_last_frame_error = sender_transport.get("last_frame_error")
    sample_render_presented = report.get("sampleRenderFramesPresented")
    sample_queue_replacements = report.get("sampleRenderQueueReplacements")
    queue_replacements = sample_queue_replacements if isinstance(sample_queue_replacements, (int, float)) else pipeline.get("render_queue_replacements")
    sample_present_skips = report.get("sampleRenderPresentSkips")
    present_skips = sample_present_skips if isinstance(sample_present_skips, (int, float)) else pipeline.get("render_present_skips")
    repeated_latest_frames = sender_transport.get("repeated_latest_frames")
    frames_completed = sender_transport.get("frames_completed")
    fresh_sender_frames = None
    if isinstance(frames_completed, (int, float)) and isinstance(repeated_latest_frames, (int, float)):
        fresh_sender_frames = max(0, frames_completed - repeated_latest_frames)
    capture_frame_samples = sender_transport.get("capture_frame_samples")
    capture_direct_frames = sender_transport.get("capture_macos_cv_pixel_buffer_frames")
    capture_cpu_frames = sender_transport.get("capture_cpu_frames")
    unsupported_av1 = is_unsupported_av1_report(report)
    requested_fps = requested_profile.get("fps")
    render_target_fps = inferred_render_target_fps(
        requested_fps,
        pipeline.get("render_pacing_target_fps"),
    )
    display_limited_reason = display_refresh_limit_reason(requested_fps, render_target_fps)
    report_status = report.get("status")
    report_failure_reason = report.get("failureReason")
    sender_error_text = (
        sender_last_frame_error.lower()
        if isinstance(sender_last_frame_error, str)
        else ""
    )
    capture_permission_required = (
        report_status == "failed"
        and report_failure_reason == "no_remote_frames"
        and "screen recording permission is not granted" in sender_error_text
    )
    capture_display_unavailable = (
        report_status == "failed"
        and report_failure_reason == "no_remote_frames"
        and not capture_permission_required
        and (
            "screencapturekit found no capture display" in sender_error_text
            or "display capture is unavailable" in sender_error_text
        )
    )
    capture_window_unavailable = (
        report_status == "failed"
        and report_failure_reason == "no_remote_frames"
        and not capture_permission_required
        and not capture_display_unavailable
        and capture.get("source_kind") == "window"
        and (
            "window_capture_source_not_found" in sender_error_text
            or "macos lan capture pump timed out waiting for a captured frame" in sender_error_text
            or "screencapturekit timed out waiting" in sender_error_text
        )
    )
    display_limited = (
        display_limited_reason is not None
        and not unsupported_av1
        and not capture_permission_required
        and not capture_display_unavailable
        and not capture_window_unavailable
        and (
            report_status in {"completed", "skipped"}
            or (
                report_status == "failed"
                and report_failure_reason
                in {"display_refresh_limited", "performance_threshold", "no_remote_frames"}
            )
        )
    )
    row_status = "skipped" if unsupported_av1 or display_limited else report_status or "failed"
    row_classification = (
        "completed"
        if row_status == "completed"
        else "unsupported"
        if unsupported_av1
        else "capture_permission_required"
        if capture_permission_required
        else "capture_display_unavailable"
        if capture_display_unavailable
        else "capture_window_unavailable"
        if capture_window_unavailable
        else "display_refresh_limited"
        if display_limited
        else report.get("failureReason", "failed")
    )
    row = {
        "id": profile_id,
        "mode": "local-dual-process",
        "chain": chain_id,
        "session_id": report.get("sessionId"),
        "status": row_status,
        "classification": row_classification,
        "media_verified": report.get("mediaVerified"),
        "data_plane_verified": report.get("dataPlaneVerified"),
        "display_native_surface_attached": display_window.get("native_surface_attached"),
        "display_render_mode": display_window.get("render_mode"),
        "display_renderer_attached": display_window.get("renderer_attached"),
        "display_surface_id": display_window.get("surface_id"),
        "fps_observed": fps or 0,
        "fps_observed_elapsed": fps_elapsed,
        "fps_observed_target_duration": fps_target,
        "render_fps_observed": render_fps,
        "render_fps_observed_elapsed": render_fps_elapsed,
        "render_fps_observed_target_duration": render_fps_target,
        "sample_duration_ms": report.get("sampleDurationMs"),
        "sample_fps_elapsed_ms": report.get("sampleFpsElapsedMs"),
        "sample_fps_target_duration_ms": report.get("sampleFpsTargetDurationMs"),
        "sample_target_duration_ms": thresholds.get("minSampleDurationMs"),
        "requested_fps": requested_fps,
        "render_pacing_target_fps": render_target_fps,
        "display_refresh_limited": display_limited,
        "display_refresh_limit_reason": display_limited_reason if display_limited else None,
        "capture_display_unavailable": capture_display_unavailable,
        "capture_window_unavailable": capture_window_unavailable,
        "sample_render_frames_presented": report.get("sampleRenderFramesPresented"),
        "sample_frames_decoded": report.get("sampleFramesDecoded"),
        "render_queue_policy": pipeline.get("render_queue_policy"),
        "render_queue_replacements": queue_replacements,
        "render_queue_replacement_ratio": ratio(queue_replacements, sample_render_presented),
        "render_present_skips": present_skips,
        "render_present_skip_ratio": ratio(present_skips, sample_render_presented),
        "render_stale_frame_drops": pipeline.get("render_stale_frame_drops"),
        "repeat_latest_frame_ratio": report.get("repeatLatestFrameRatio") if isinstance(report.get("repeatLatestFrameRatio"), (int, float)) else ratio(repeated_latest_frames, frames_completed),
        "repeated_latest_frames": repeated_latest_frames,
        "fresh_sender_frames": report.get("freshSenderFrames") if isinstance(report.get("freshSenderFrames"), (int, float)) else fresh_sender_frames,
        "fresh_sender_frame_ratio": report.get("freshSenderFrameRatio") if isinstance(report.get("freshSenderFrameRatio"), (int, float)) else ratio(fresh_sender_frames, frames_completed),
        "sender_frames_completed": frames_completed,
        "sender_last_frame_error": sender_last_frame_error,
        "capture_frame_samples": capture_frame_samples,
        "capture_macos_cv_pixel_buffer_frames": capture_direct_frames,
        "capture_cpu_frames": capture_cpu_frames,
        "capture_bgra32_frames": sender_transport.get("capture_bgra32_frames"),
        "capture_rgba32_frames": sender_transport.get("capture_rgba32_frames"),
        "capture_rgb24_frames": sender_transport.get("capture_rgb24_frames"),
        "capture_nv12_frames": sender_transport.get("capture_nv12_frames"),
        "capture_direct_frame_ratio": report.get("captureDirectFrameRatio") if isinstance(report.get("captureDirectFrameRatio"), (int, float)) else ratio(capture_direct_frames, capture_frame_samples),
        "capture_cpu_frame_ratio": report.get("captureCpuFrameRatio") if isinstance(report.get("captureCpuFrameRatio"), (int, float)) else ratio(capture_cpu_frames, capture_frame_samples),
        "capture_memory_path": sender_transport.get("capture_memory_path"),
        "sender_datagram_fragments_sent": sender_transport.get("datagram_fragments_sent"),
        "receiver_proxy_forward_direct_v3_p95_ms": stage_p95(pipeline, "receiver.proxy_forward_direct_v3"),
        "render_present_p95_ms": stage_p95(pipeline, "render_present"),
        "render_proxy_next_drawable_p95_ms": stage_p95(pipeline, "render_proxy_next_drawable"),
        "render_proxy_draw_present_p95_ms": stage_p95(pipeline, "render_proxy_draw_present"),
        "render_present_gap_p95_ms": stage_p95(pipeline, "render_present_gap"),
        "render_enqueue_gap_p95_ms": stage_p95(pipeline, "render_enqueue_gap"),
        "sender_capture_p95_ms": sender_stage_p95(sender_pipeline, pipeline, "sender.capture"),
        "sender_encode_p95_ms": sender_stage_p95(sender_pipeline, pipeline, "sender.encode"),
        "sender_send_datagram_p95_ms": sender_stage_p95(sender_pipeline, pipeline, "sender.send_datagram"),
        "sender_fragment_p95_ms": sender_stage_p95(sender_pipeline, pipeline, "sender.fragment"),
        "queue_depth": pipeline.get("queue_depth"),
        "swap_chain_max_frame_latency": pipeline.get("swap_chain_max_frame_latency"),
        "swap_chain_present_mode": pipeline.get("swap_chain_present_mode"),
        "decoded_frames": probe.get("frames_decoded", 0),
        "dropped_frames": probe.get("frames_dropped", 0),
        "active_encoder": sender_pipeline.get("active_encoder") or pipeline.get("active_encoder"),
        "active_codec": sender_pipeline.get("active_codec") or pipeline.get("active_codec"),
        "sender_active_codec": sender_pipeline.get("active_codec"),
        "receiver_active_codec": pipeline.get("active_codec"),
        "codec_fallback_reason": sender_pipeline.get("codec_fallback_reason") or pipeline.get("codec_fallback_reason"),
        "active_decoder": pipeline.get("active_decoder"),
        "active_renderer": pipeline.get("active_renderer"),
        "active_width": pipeline.get("active_width"),
        "active_height": pipeline.get("active_height"),
        "active_fps": pipeline.get("active_fps"),
        "active_bitrate_mbps": pipeline.get("active_bitrate_mbps"),
        "local_dual_run_id": local_dual.get("run_id"),
        "local_dual_harness_mode": local_dual.get("harness_mode"),
        "local_dual_service_process_count": local_dual.get("service_process_count"),
        "local_dual_distinct_service_processes": local_dual.get("distinct_service_processes"),
        "local_dual_distinct_ipc_endpoints": local_dual.get("distinct_ipc_endpoints"),
        "local_dual_discovery_path": local_dual.get("discovery_path"),
        "controller_pid": controller_process.get("pid"),
        "peer_pid": peer_process.get("pid"),
        "controller_ipc_endpoint": controller_process.get("ipc_endpoint"),
        "peer_ipc_endpoint": peer_process.get("ipc_endpoint"),
        "controller_discovery_port": controller_process.get("discovery_port"),
        "peer_discovery_port": peer_process.get("discovery_port"),
        "process_cleanup_status": cleanup.get("status"),
        "process_cleanup_ipc_endpoints_removed": cleanup.get("ipc_endpoints_removed"),
        "controller_alive_after_cleanup": cleanup.get("controller_alive_after_cleanup"),
        "peer_alive_after_cleanup": cleanup.get("peer_alive_after_cleanup"),
        "tauri_alive_after_cleanup": cleanup.get("tauri_alive_after_cleanup"),
        "vite_alive_after_cleanup": cleanup.get("vite_alive_after_cleanup"),
        "render_proxy_alive_after_cleanup": cleanup.get("render_proxy_alive_after_cleanup"),
        "capture_source_id": capture.get("id"),
        "capture_source_kind": capture.get("source_kind"),
        "capture_source_width": capture.get("width"),
        "capture_source_height": capture.get("height"),
        "raw_report_path": raw_path,
        "error_message": sender_last_frame_error if capture_permission_required or capture_display_unavailable or capture_window_unavailable else display_limited_reason if display_limited else report.get("errorMessage"),
        "performance_warnings": report.get("performanceWarnings"),
    }
    rows.append(row)

summary = {
    "mode": "local-dual-process",
    "platform": "macos",
    "git_commit": git_commit,
    "chain": chain_id,
    "render_max_fps_override": render_max_fps_override,
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
    file.write("| Profile | Status | FPS | Render FPS | Target | Repl% | Skip% | Repeat% | Fresh% | Capture p95 | Send p95 | nextDrawable p95 | DrawPresent p95 | Queue | Swap | Decoded | Presented | Codec | Encoder | Decoder | Renderer | Capture | Notes |\n")
    file.write("| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- | ---: | ---: | --- | --- | --- | --- | --- | --- |\n")
    for row in rows:
        error = (row.get("error_message") or "").replace("|", "\\|").replace("\n", " ")
        warnings = row.get("performance_warnings")
        warnings_text = ""
        if isinstance(warnings, list):
            warning_parts = [str(item).replace("|", "\\|").replace("\n", " ") for item in warnings]
            warnings_text = "; ".join(part for part in warning_parts if part)
        elif warnings:
            warnings_text = str(warnings).replace("|", "\\|").replace("\n", " ")
        codec_fallback = (row.get("codec_fallback_reason") or "").replace("|", "\\|").replace("\n", " ")
        sender_codec = row.get("sender_active_codec")
        receiver_codec = row.get("receiver_active_codec")
        codec_text = row.get("active_codec") or "-"
        if sender_codec and receiver_codec and sender_codec != receiver_codec:
            codec_text = f"{sender_codec}/{receiver_codec}"
        render_fps = row.get("render_fps_observed")
        render_fps_text = f"{render_fps:.1f}" if isinstance(render_fps, (int, float)) else "-"
        presented = row.get("sample_render_frames_presented")
        presented_text = str(presented) if isinstance(presented, (int, float)) else "-"
        fps_target = row.get("fps_observed_target_duration")
        fps_elapsed = row.get("fps_observed_elapsed")
        fps_note = ""
        if isinstance(fps_target, (int, float)) and isinstance(fps_elapsed, (int, float)):
            target_ms = row.get("sample_fps_target_duration_ms")
            elapsed_ms = row.get("sample_fps_elapsed_ms")
            duration_note = ""
            if isinstance(target_ms, (int, float)) and isinstance(elapsed_ms, (int, float)):
                duration_note = f" over {target_ms:.0f}/{elapsed_ms:.0f}ms"
            fps_note = f"target {fps_target:.1f}, elapsed {fps_elapsed:.1f}{duration_note}"
        render_target = row.get("render_fps_observed_target_duration")
        render_elapsed = row.get("render_fps_observed_elapsed")
        if isinstance(render_target, (int, float)) and isinstance(render_elapsed, (int, float)):
            render_fps_text = f"{render_fps_text} ({render_target:.1f}/{render_elapsed:.1f})"
        replacement_ratio = row.get("render_queue_replacement_ratio")
        replacement_text = f"{replacement_ratio * 100:.2f}" if isinstance(replacement_ratio, (int, float)) else "-"
        present_skip_ratio = row.get("render_present_skip_ratio")
        present_skip_text = f"{present_skip_ratio * 100:.2f}" if isinstance(present_skip_ratio, (int, float)) else "-"
        repeat_ratio = row.get("repeat_latest_frame_ratio")
        repeat_text = f"{repeat_ratio * 100:.2f}" if isinstance(repeat_ratio, (int, float)) else "-"
        fresh_ratio = row.get("fresh_sender_frame_ratio")
        fresh_text = f"{fresh_ratio * 100:.2f}" if isinstance(fresh_ratio, (int, float)) else "-"
        sender_capture_p95 = row.get("sender_capture_p95_ms")
        sender_capture_text = f"{sender_capture_p95:.2f}" if isinstance(sender_capture_p95, (int, float)) else "-"
        sender_send_p95 = row.get("sender_send_datagram_p95_ms")
        sender_send_text = f"{sender_send_p95:.2f}" if isinstance(sender_send_p95, (int, float)) else "-"
        next_drawable_p95 = row.get("render_proxy_next_drawable_p95_ms")
        next_drawable_text = f"{next_drawable_p95:.2f}" if isinstance(next_drawable_p95, (int, float)) else "-"
        draw_present_p95 = row.get("render_proxy_draw_present_p95_ms")
        draw_present_text = f"{draw_present_p95:.2f}" if isinstance(draw_present_p95, (int, float)) else "-"
        queue_depth = row.get("queue_depth")
        queue_depth_text = f"{queue_depth:.0f}" if isinstance(queue_depth, (int, float)) else "-"
        swap_latency = row.get("swap_chain_max_frame_latency")
        swap_present_mode = row.get("swap_chain_present_mode")
        swap_text = "-"
        if isinstance(swap_latency, (int, float)):
            swap_text = f"{swap_latency:.0f}"
            if swap_present_mode:
                swap_text = f"{swap_text}/{swap_present_mode}"
        elif swap_present_mode:
            swap_text = str(swap_present_mode)
        swap_text = swap_text.replace("|", "\\|")
        direct_ratio = row.get("capture_direct_frame_ratio")
        cpu_ratio = row.get("capture_cpu_frame_ratio")
        capture_samples = row.get("capture_frame_samples")
        capture_path = row.get("capture_memory_path") or "-"
        capture_format = "-"
        format_counts = [
            ("NV12", row.get("capture_nv12_frames")),
            ("BGRA", row.get("capture_bgra32_frames")),
            ("RGBA", row.get("capture_rgba32_frames")),
            ("RGB24", row.get("capture_rgb24_frames")),
        ]
        numeric_format_counts = [(name, count) for name, count in format_counts if isinstance(count, (int, float))]
        if numeric_format_counts:
            capture_format = max(numeric_format_counts, key=lambda item: item[1])[0]
        if isinstance(direct_ratio, (int, float)) and isinstance(cpu_ratio, (int, float)):
            capture_text = f"{capture_path} direct {direct_ratio * 100:.1f}% cpu {cpu_ratio * 100:.1f}% {capture_format}"
        elif isinstance(capture_samples, (int, float)) and capture_samples > 0:
            capture_text = f"{capture_path} samples {capture_samples} {capture_format}"
        else:
            capture_text = "-"
        source_width = row.get("capture_source_width")
        source_height = row.get("capture_source_height")
        if isinstance(source_width, (int, float)) and isinstance(source_height, (int, float)):
            source_text = f"source {source_width:.0f}x{source_height:.0f}"
            capture_text = source_text if capture_text == "-" else f"{source_text} -> {capture_text}"
        capture_text = capture_text.replace("|", "\\|")
        requested_fps = row.get("requested_fps")
        render_pacing_target_fps = row.get("render_pacing_target_fps")
        target_text = (
            f"{requested_fps:.0f}/{render_pacing_target_fps:.0f}"
            if isinstance(requested_fps, (int, float))
            and isinstance(render_pacing_target_fps, (int, float))
            and render_pacing_target_fps > 0
            and render_pacing_target_fps != requested_fps
            else f"{requested_fps:.0f}"
            if isinstance(requested_fps, (int, float))
            else "-"
        )
        sender_encode_p95 = row.get("sender_encode_p95_ms")
        render_path_expected = (
            bool(row.get("active_renderer"))
            or (isinstance(presented, (int, float)) and presented > 0)
        )
        fps_validation_target = requested_fps
        fps_target_label = "requested"
        if (
            render_path_expected
            and isinstance(render_pacing_target_fps, (int, float))
            and render_pacing_target_fps > 0
        ):
            fps_validation_target = render_pacing_target_fps
            fps_target_label = "local render target"
        fps_headroom_note = ""
        if (
            row.get("status") == "completed"
            and isinstance(fps_validation_target, (int, float))
            and fps_validation_target > 0
            and (
                (
                    isinstance(fps_target, (int, float))
                    and fps_target < fps_validation_target * 0.9
                )
                or (
                    render_path_expected
                    and isinstance(render_target, (int, float))
                    and render_target < fps_validation_target * 0.9
                )
            )
        ):
            encode_note = (
                f"; sender.encode p95 {sender_encode_p95:.2f}ms"
                if isinstance(sender_encode_p95, (int, float))
                else ""
            )
            target_fps_text = f"{fps_target:.1f}" if isinstance(fps_target, (int, float)) else "-"
            target_render_text = (
                f"{render_target:.1f}" if isinstance(render_target, (int, float)) else "-"
            )
            fps_headroom_note = (
                f"below {fps_target_label} {fps_validation_target:.0f}fps"
                f" (target fps {target_fps_text}, render {target_render_text}{encode_note})"
            )
        notes_parts = []
        if error:
            notes_parts.append(error)
        elif codec_fallback:
            notes_parts.append(codec_fallback)
        elif warnings_text:
            notes_parts.append(warnings_text)
        if fps_headroom_note:
            notes_parts.append(fps_headroom_note.replace("|", "\\|"))
        if fps_note:
            notes_parts.append(fps_note)
        notes_text = "; ".join(notes_parts) if notes_parts else "-"
        file.write(
            f"| {row['id']} | {row['status']} | {row['fps_observed']:.1f} | {render_fps_text} | "
            f"{target_text} | {replacement_text} | {present_skip_text} | {repeat_text} | {fresh_text} | "
            f"{sender_capture_text} | {sender_send_text} | {next_drawable_text} | {draw_present_text} | "
            f"{queue_depth_text} | {swap_text} | "
            f"{row['decoded_frames']} | {presented_text} | {codec_text} | "
            f"{row.get('active_encoder') or '-'} | "
            f"{row.get('active_decoder') or '-'} | {row.get('active_renderer') or '-'} | "
            f"{capture_text} | {notes_text} |\n"
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
  local source_fit_profile="$SOURCE_FIT_PROFILE"
  if profile_uses_source_fit "$profile_id"; then
    source_fit_profile=1
  fi

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

  local controller_pid="" peer_pid="" tauri_pid="" vite_pid="" render_proxy_pid=""
  trap 'kill_tree "$tauri_pid"; kill_tree "$vite_pid"; kill_tree "$render_proxy_pid"; kill_tree "$controller_pid"; kill_tree "$peer_pid"; sleep 1; kill_tree_force "$tauri_pid"; kill_tree_force "$vite_pid"; kill_tree_force "$render_proxy_pid"; kill_tree_force "$controller_pid"; kill_tree_force "$peer_pid"; rm -f "$controller_socket" "$peer_socket"' RETURN

  echo "Running macOS local dual-process LAN canary ${profile_id}"

  env \
    MRD_SERVICE_IPC_ENDPOINT="$controller_socket" \
    MRD_LAN_DEVICE_ID="$controller_device" \
    MRD_LAN_DEVICE_NAME="Local Dual Controller" \
    MRD_LAN_DISCOVERY_PORT="$controller_port" \
    MRD_LAN_DISCOVERY_PROBE_ENDPOINTS="127.0.0.1:${peer_port}" \
    MRD_SERVICE_BUILD_ID="$GIT_COMMIT" \
    MRD_LAN_RECEIVER_DECODER="$RECEIVER_DECODER" \
    MRD_LAN_RENDER_MAX_FPS="$RENDER_MAX_FPS" \
    MRD_WEB_BRIDGE_ENABLED="false" \
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
    MRD_LAN_RENDER_MAX_FPS="$RENDER_MAX_FPS" \
    MRD_WEB_BRIDGE_ENABLED="false" \
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
  local lan_e2e_stop_on_complete
  lan_e2e_stop_on_complete="true"
  if [ "$DISPLAY_MODE_POLICY" = "none" ]; then
    lan_e2e_stop_on_complete="false"
  fi

  local vite_js_path vite_node_bin vite_node_log
  vite_js_path="$REPO/apps/Rdesk/node_modules/vite/bin/vite.js"
  vite_node_bin=""
  vite_node_log="$logs_dir/vite-node-launcher.log"
  if [ -f "$vite_js_path" ]; then
    if ! vite_node_bin="$(prepare_ad_hoc_vite_node "$run_bin_dir/node-vite" "$vite_node_log" "$vite_js_path" 2>/dev/null)"; then
      vite_node_bin=""
    fi
  fi

  local vite_ready_error=""
  local vite_stderr_excerpt=""
  if [ -n "$vite_node_bin" ] || [ -n "$PNPM_BIN" ] || [ -n "$VITE_BIN" ]; then
    (
      cd "$REPO/apps/Rdesk"
      if [ -n "$vite_node_bin" ]; then
        "$vite_node_bin" "$vite_js_path" --host 127.0.0.1 --port 9531 --strictPort
      elif [ -n "$PNPM_BIN" ]; then
        "$PNPM_BIN" exec vite --host 127.0.0.1 --port 9531 --strictPort
      else
        "$VITE_BIN" --host 127.0.0.1 --port 9531 --strictPort
      fi
    ) >"$logs_dir/vite.stdout.log" 2>"$logs_dir/vite.stderr.log" &
    vite_pid="$!"
    if ! vite_ready_error="$(wait_http_ready "http://127.0.0.1:9531/" 30 2>&1)"; then
      vite_stderr_excerpt="$(python3 - "$logs_dir/vite.stderr.log" <<'PY'
import sys

path = sys.argv[1]
try:
    with open(path, encoding="utf-8", errors="replace") as file:
        text = file.read(2000)
except Exception:
    text = ""
text = " ".join(text.split())
print(text)
PY
)"
    else
      vite_ready_error=""
    fi
  else
    vite_ready_error="Vite launcher unavailable; pnpm and apps/Rdesk/node_modules/.bin/vite were not found."
  fi

  if [ -n "$vite_ready_error" ]; then
    local vite_failure_message
    vite_failure_message="macOS local dual-process LAN E2E could not start Vite dev server. ${vite_ready_error}."
    if [ -n "$vite_stderr_excerpt" ]; then
      vite_failure_message="${vite_failure_message} Vite stderr: ${vite_stderr_excerpt}"
    fi
    vite_failure_message="${vite_failure_message} Logs: $logs_dir"
    echo "$vite_ready_error" >&2
    if [ -n "$vite_pid" ]; then
      kill_tree "$vite_pid"
      sleep 1
      kill_tree_force "$vite_pid"
      vite_pid=""
    fi
    run_static_tauri_lan_e2e_fallback "$vite_failure_message"
    kill_tree "$render_proxy_pid"
    kill_tree "$controller_pid"
    kill_tree "$peer_pid"
    sleep 1
    kill_tree_force "$render_proxy_pid"
    kill_tree_force "$controller_pid"
    kill_tree_force "$peer_pid"
    rm -f "$controller_socket" "$peer_socket"
    if [ -s "$report_path" ]; then
      record_local_dual_cleanup "$report_path" "$controller_pid" "$peer_pid" "$tauri_pid" "$vite_pid" "$render_proxy_pid" "$controller_socket" "$peer_socket" "$KEEP_TAURI_OPEN"
    fi
    trap - RETURN
    return 0
  fi

  env \
    MRD_SERVICE_IPC_ENDPOINT="$controller_socket" \
    MRD_SERVICE_PREBUILT_EXE="$run_service_bin" \
    MRD_SERVICE_BOOTSTRAP_DISABLED="1" \
    MRD_RDESK_SINGLE_INSTANCE_ADDR="127.0.0.1:${single_instance_port}" \
    MRD_LAN_E2E_AUTORUN="1" \
    MRD_LAN_E2E_TARGET_DEVICE_ID="$peer_device" \
    MRD_LAN_E2E_TRANSPORT="quic" \
    MRD_LAN_E2E_TIMEOUT_MS="$timeout_ms" \
    MRD_LAN_E2E_MIN_SAMPLE_DURATION_MS="$((DURATION_SECS * 1000))" \
    MRD_LAN_E2E_MIN_DECODED_FRAMES="20" \
    MRD_LAN_E2E_MIN_FPS="$min_fps" \
    MRD_LAN_E2E_STOP_ON_COMPLETE="$lan_e2e_stop_on_complete" \
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
    MRD_LAN_E2E_RENDER_DISPLAY="$RENDER_DISPLAY" \
    MRD_LAN_E2E_SOURCE_FIT_PROFILE="$source_fit_profile" \
    MRD_LAN_RENDER_MAX_FPS="$RENDER_MAX_FPS" \
    MRD_MACOS_RENDER_PROXY_ASYNC_PRESENT="$RENDER_PROXY_ASYNC_PRESENT" \
    MRD_VIDEOTOOLBOX_HEVC_RAW_DECODE_ASYNC="$HEVC_RAW_DECODE_ASYNC" \
    MRD_VIDEOTOOLBOX_HEVC_RAW_DECODE_MAX_PENDING_INPUTS="$HEVC_RAW_DECODE_MAX_PENDING_INPUTS" \
    "$APP_BIN" >"$logs_dir/tauri.stdout.log" 2>"$logs_dir/tauri.stderr.log" &
  tauri_pid="$!"

  local deadline status controller_exit peer_exit tauri_exit
  deadline=$((SECONDS + DURATION_SECS + 600))
  status=""
  while [ "$SECONDS" -lt "$deadline" ]; do
    if [ -s "$report_path" ]; then
      status="$(status_from_report "$report_path")"
      if [ "$status" = "completed" ] || [ "$status" = "failed" ] || [ "$status" = "skipped" ]; then
        break
      fi
    fi
    if ! kill -0 "$controller_pid" >/dev/null 2>&1 || ! kill -0 "$peer_pid" >/dev/null 2>&1 || ! kill -0 "$tauri_pid" >/dev/null 2>&1 || ! kill -0 "$vite_pid" >/dev/null 2>&1; then
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

  if [ -s "$report_path" ] && kill -0 "$peer_pid" >/dev/null 2>&1; then
    enrich_report_with_peer_pipeline_snapshot "$peer_socket" "$report_path" || true
  fi
  if [ -s "$report_path" ]; then
    validate_report_performance_thresholds "$report_path"
    append_local_dual_run_metadata "$report_path" "$run_id" "$run_dir" "$logs_dir" "$controller_pid" "$peer_pid" "$tauri_pid" "$vite_pid" "$render_proxy_pid" "$controller_socket" "$peer_socket" "$controller_device" "$peer_device" "$controller_port" "$peer_port" "vite_tauri_harness"
  fi

  if [ "$KEEP_TAURI_OPEN" -eq 0 ]; then
    kill_tree "$tauri_pid"
  fi
  kill_tree "$vite_pid"
  kill_tree "$controller_pid"
  kill_tree "$peer_pid"
  sleep 1
  if [ "$KEEP_TAURI_OPEN" -eq 0 ]; then
    kill_tree_force "$tauri_pid"
  fi
  kill_tree_force "$vite_pid"
  kill_tree_force "$controller_pid"
  kill_tree_force "$peer_pid"
  rm -f "$controller_socket" "$peer_socket"
  if [ -s "$report_path" ]; then
    record_local_dual_cleanup "$report_path" "$controller_pid" "$peer_pid" "$tauri_pid" "$vite_pid" "$render_proxy_pid" "$controller_socket" "$peer_socket" "$KEEP_TAURI_OPEN"
  fi
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
