#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="."
OUTPUT_DIR="target/codex-local-realistic-lan-e2e-macos"
PROFILE_ID="1080p60"
DURATION_SECS=30
CODEC="h264"
SCENARIOS="clean,lan,stress"
NO_BUILD=0

usage() {
  cat <<'EOF'
Usage: run_local_realistic_lan_e2e_macos.sh [options]

Runs the real macOS capture -> encode -> QUIC -> decode -> native render path
with two isolated mrd-service processes. Scenarios use deterministic sender-side
media impairment and strict decoded/rendered FPS gates.

Options:
  --repo-root PATH
  --output-dir PATH             Default: target/codex-local-realistic-lan-e2e-macos
  --profile-id ID               Default: 1080p60
  --duration-secs SECONDS       Default: 30
  --codec VALUE                 h264|hevc. Default: h264
  --scenarios LIST              clean,lan,stress or a comma-separated subset
  --no-build                    Reuse existing app and mrd-service binaries
  --help

Scenario defaults:
  clean   95% FPS floor, no impairment
  lan     90% FPS floor, production reliable stream, 2ms delay, 1ms jitter
  stress  80% FPS floor, forced datagram media, 0.05% fragment loss, 8ms
          delay, 4ms jitter, 900-byte application MTU
EOF
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --repo-root) REPO_ROOT="$2"; shift 2 ;;
    --output-dir) OUTPUT_DIR="$2"; shift 2 ;;
    --profile-id) PROFILE_ID="$2"; shift 2 ;;
    --duration-secs|--duration) DURATION_SECS="$2"; shift 2 ;;
    --codec) CODEC="$2"; shift 2 ;;
    --scenarios) SCENARIOS="$2"; shift 2 ;;
    --no-build) NO_BUILD=1; shift ;;
    --help|-h) usage; exit 0 ;;
    *) echo "Unknown option: $1" >&2; usage >&2; exit 2 ;;
  esac
done

if [ "$(uname -s)" != "Darwin" ]; then
  echo "This realistic LAN E2E runner is macOS-only." >&2
  exit 2
fi

REPO="$(cd "$REPO_ROOT" && pwd)"
BASE_RUNNER="$REPO/tests/benchmarks/scripts/run_local_dual_process_lan_canary_macos.sh"
if [ ! -f "$BASE_RUNNER" ]; then
  echo "Base macOS LAN canary runner was not found: $BASE_RUNNER" >&2
  exit 2
fi

python3 - "$DURATION_SECS" <<'PY'
import sys

try:
    duration = int(sys.argv[1])
except ValueError as exc:
    raise SystemExit(f"invalid duration: {exc}")
if duration < 5:
    raise SystemExit("duration must be at least 5 seconds")
PY

scenario_args() {
  case "$1" in
    clean)
      printf '%s\n' "0.95 0 0 0 - 55715971976142 0 0"
      ;;
    lan)
      printf '%s\n' "0.90 0 2 1 - 55715971976143 0 0"
      ;;
    stress)
      printf '%s\n' "0.80 0.05 8 4 900 55715971976144 1 0.10"
      ;;
    *)
      echo "Unknown realistic LAN scenario: $1" >&2
      return 2
      ;;
  esac
}

mkdir -p "$REPO/$OUTPUT_DIR"
overall_status=0
build_completed="$NO_BUILD"
scenario_names=()

IFS=',' read -r -a REQUESTED_SCENARIOS <<<"$SCENARIOS"
for scenario in "${REQUESTED_SCENARIOS[@]}"; do
  scenario="$(printf '%s' "$scenario" | xargs)"
  if [ -z "$scenario" ]; then
    continue
  fi
  scenario_names+=("$scenario")

  read -r min_fps_ratio loss_pct base_delay_ms jitter_ms mtu_bytes impairment_seed force_datagram max_drop_ratio <<EOF
$(scenario_args "$scenario")
EOF

  args=(
    --repo-root "$REPO"
    --output-dir "$OUTPUT_DIR/$scenario"
    --profile-id "$PROFILE_ID"
    --duration-secs "$DURATION_SECS"
    --codec "$CODEC"
    --min-fps-ratio "$min_fps_ratio"
    --max-sample-drop-ratio "$max_drop_ratio"
    --loss-pct "$loss_pct"
    --base-delay-ms "$base_delay_ms"
    --jitter-ms "$jitter_ms"
    --impairment-seed "$impairment_seed"
    --allow-aspect-preserving-profile
  )
  if [ "$mtu_bytes" != "-" ]; then
    args+=(--mtu-bytes "$mtu_bytes")
  fi
  if [ "$force_datagram" -eq 1 ]; then
    args+=(--force-datagram-media)
  fi
  if [ "$build_completed" -eq 1 ]; then
    args+=(--no-build)
  fi

  echo "Running realistic macOS LAN E2E scenario: $scenario"
  if bash "$BASE_RUNNER" "${args[@]}"; then
    :
  else
    overall_status=1
  fi
  build_completed=1
done

if [ "${#scenario_names[@]}" -eq 0 ]; then
  echo "No realistic LAN scenarios were selected." >&2
  exit 2
fi

python3 - "$REPO/$OUTPUT_DIR" "$PROFILE_ID" "$CODEC" "${scenario_names[@]}" <<'PY'
import json
import os
import sys

output_root, profile_id, codec, *scenario_names = sys.argv[1:]
scenario_defaults = {
    "clean": {
        "min_fps_ratio": 0.95,
        "max_sample_drop_ratio": 0.0,
        "transport_mode": "production_reliable",
        "loss_pct": 0.0,
        "base_delay_ms": 0,
        "jitter_ms": 0,
        "mtu_bytes": None,
    },
    "lan": {
        "min_fps_ratio": 0.90,
        "max_sample_drop_ratio": 0.0,
        "transport_mode": "production_reliable",
        "loss_pct": 0.0,
        "base_delay_ms": 2,
        "jitter_ms": 1,
        "mtu_bytes": None,
    },
    "stress": {
        "min_fps_ratio": 0.80,
        "max_sample_drop_ratio": 0.10,
        "transport_mode": "forced_datagram",
        "loss_pct": 0.05,
        "base_delay_ms": 8,
        "jitter_ms": 4,
        "mtu_bytes": 900,
    },
}

rows = []
git_commit = None
for scenario in scenario_names:
    summary_path = os.path.join(
        output_root, scenario, "local-dual-process-canary-report.json"
    )
    row = {
        "scenario": scenario,
        "profile": profile_id,
        "codec": codec,
        "status": "failed",
        "classification": "missing_report",
        "network": scenario_defaults.get(scenario),
        "summary_path": summary_path,
    }
    try:
        with open(summary_path, encoding="utf-8") as file:
            summary = json.load(file)
        git_commit = git_commit or summary.get("git_commit")
        source_rows = summary.get("rows") or []
        if source_rows:
            source = source_rows[0]
            row.update(source)
            row["scenario"] = scenario
            row["profile"] = profile_id
            row["codec"] = codec
            row["network"] = scenario_defaults.get(scenario)
            row["summary_path"] = summary_path
    except Exception as exc:
        row["error_message"] = str(exc)
    rows.append(row)

status = "completed" if rows and all(row.get("status") == "completed" for row in rows) else "failed"
report = {
    "schema_version": 1,
    "kind": "local_realistic_lan_e2e_macos",
    "status": status,
    "git_commit": git_commit,
    "profile": profile_id,
    "codec": codec,
    "rows": rows,
}

json_path = os.path.join(output_root, "realistic-lan-e2e-report.json")
md_path = os.path.join(output_root, "realistic-lan-e2e-report.md")
with open(json_path, "w", encoding="utf-8") as file:
    json.dump(report, file, indent=2)

with open(md_path, "w", encoding="utf-8") as file:
    file.write("# macOS Local Realistic LAN E2E Report\n\n")
    file.write(f"- Status: {status}\n")
    file.write(f"- GitCommit: {git_commit or '-'}\n")
    file.write(f"- Profile: {profile_id}\n")
    file.write(f"- Codec: {codec}\n\n")
    file.write("| Scenario | Status | Decode FPS | Render FPS | Loss | Delay | Jitter | MTU | Notes |\n")
    file.write("| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | --- |\n")
    for row in rows:
        network = row.get("network") or {}
        decode_fps = row.get("fps_observed")
        render_fps = row.get("render_fps_observed")
        decode_text = f"{decode_fps:.1f}" if isinstance(decode_fps, (int, float)) else "-"
        render_text = f"{render_fps:.1f}" if isinstance(render_fps, (int, float)) else "-"
        mtu = network.get("mtu_bytes")
        note = str(row.get("error_message") or "").replace("|", "\\|").replace("\n", " ")
        file.write(
            f"| {row['scenario']} | {row.get('status', 'failed')} | {decode_text} | "
            f"{render_text} | {network.get('loss_pct', 0)}% | "
            f"{network.get('base_delay_ms', 0)}ms | {network.get('jitter_ms', 0)}ms | "
            f"{mtu or '-'} | {note or '-'} |\n"
        )

print(json_path)
print(md_path)
PY

echo "macOS realistic LAN E2E report written to $REPO/$OUTPUT_DIR"
exit "$overall_status"
