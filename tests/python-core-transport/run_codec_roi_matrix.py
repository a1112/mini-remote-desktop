import argparse
import asyncio
import copy
import json
import time
from datetime import datetime
from pathlib import Path
from typing import Any, Dict, List, Optional

from run_core_transport_suite import CoreTransportSuite, load_config


def _make_capture_patch(
    codec: str,
    roi_enable: bool,
    roi_rect: str,
    roi_qoffset: float,
    roi_require_native: Optional[bool] = None,
) -> Dict[str, Any]:
    patch: Dict[str, Any] = {"codecPolicy": {"force": codec}}
    if roi_enable or roi_require_native is not None:
        patch["qualityPolicy"] = {"roi": {}}
    if roi_enable:
        vals = [float(x.strip()) for x in roi_rect.split(",")]
        if len(vals) == 4:
            patch["qualityPolicy"]["roi"].update(
                {
                    "enable": True,
                    "rect": {"x": vals[0], "y": vals[1], "w": vals[2], "h": vals[3]},
                    "qoffset": float(roi_qoffset),
                }
            )
    if roi_require_native is not None:
        patch["qualityPolicy"]["roi"]["requireNative"] = bool(roi_require_native)
    return patch


def _mk_case(
    base_cfg: Dict[str, Any],
    *,
    transport: str,
    codec: str,
    roi_enable: bool,
    native_roi_probe: bool,
    roi_rect: str,
    roi_qoffset: float,
    roi_require_native: Optional[bool] = None,
) -> Dict[str, Any]:
    cfg = copy.deepcopy(base_cfg)
    analysis = cfg.setdefault("analysis", {})
    analysis["controller_codecs"] = [codec, "h264"]
    analysis["controller_features"] = ["core-transport-suite", "codec-roi-matrix"]
    analysis["capture_patch"] = _make_capture_patch(
        codec,
        roi_enable,
        roi_rect,
        roi_qoffset,
        roi_require_native=roi_require_native,
    )
    env = dict(analysis.get("agent_env") or {})
    env["AGENT_NVENC_NATIVE_ROI_ENABLE"] = "1" if native_roi_probe else "0"
    analysis["agent_env"] = env
    roi_mode = "performance" if roi_require_native else "quality"
    return {
        "name": f"{transport}-{codec}-roi{'on' if roi_enable else 'off'}-{roi_mode}-native{'on' if native_roi_probe else 'off'}",
        "transport": transport,
        "codec": codec,
        "roi_enable": roi_enable,
        "roi_require_native": bool(roi_require_native) if roi_require_native is not None else None,
        "roi_mode": roi_mode if roi_enable else "off",
        "native_roi_probe": native_roi_probe,
        "cfg": cfg,
    }


async def run_matrix(root: Path, base_cfg: Dict[str, Any], roi_rect: str, roi_qoffset: float) -> Dict[str, Any]:
    cases = [
        _mk_case(base_cfg, transport="webrtc", codec="hevc", roi_enable=False, native_roi_probe=False, roi_rect=roi_rect, roi_qoffset=roi_qoffset),
        _mk_case(base_cfg, transport="webtransport", codec="hevc", roi_enable=True, native_roi_probe=False, roi_rect=roi_rect, roi_qoffset=roi_qoffset, roi_require_native=False),
        _mk_case(base_cfg, transport="webtransport", codec="hevc", roi_enable=True, native_roi_probe=False, roi_rect=roi_rect, roi_qoffset=roi_qoffset, roi_require_native=True),
        _mk_case(base_cfg, transport="webtransport", codec="av1", roi_enable=True, native_roi_probe=False, roi_rect=roi_rect, roi_qoffset=roi_qoffset, roi_require_native=False),
        _mk_case(base_cfg, transport="webtransport", codec="av1", roi_enable=True, native_roi_probe=False, roi_rect=roi_rect, roi_qoffset=roi_qoffset, roi_require_native=True),
        _mk_case(base_cfg, transport="webtransport", codec="av1", roi_enable=True, native_roi_probe=True, roi_rect=roi_rect, roi_qoffset=roi_qoffset, roi_require_native=False),
    ]
    results: List[Dict[str, Any]] = []
    for case in cases:
        suite = CoreTransportSuite(root, case["cfg"])
        payload = await suite.run([case["transport"]])
        one = (payload.get("results") or [{}])[0]
        one["case_name"] = case["name"]
        one["case_transport"] = case["transport"]
        one["case_codec"] = case["codec"]
        one["case_roi_enable"] = case["roi_enable"]
        one["case_roi_require_native"] = case["roi_require_native"]
        one["case_roi_mode"] = case["roi_mode"]
        one["case_native_roi_probe"] = case["native_roi_probe"]
        one["duration_sec"] = int(case["cfg"].get("duration_sec", 0) or 0)
        one["suite_log_dir"] = str(suite.log_dir)
        results.append(one)
    return {
        "suite": "codec-roi-matrix",
        "created_at": datetime.now().isoformat(timespec="seconds"),
        "cases": results,
    }


def write_report(root: Path, payload: Dict[str, Any]) -> Path:
    ts = datetime.now().strftime("%Y%m%d-%H%M%S")
    out_dir = root / "logs" / f"codec-roi-matrix-{ts}"
    out_dir.mkdir(parents=True, exist_ok=True)
    json_path = out_dir / "suite_result.json"
    md_path = out_dir / "suite_report.md"
    json_path.write_text(json.dumps(payload, ensure_ascii=False, indent=2), encoding="utf-8")

    baseline_map: Dict[str, Dict[str, Any]] = {}
    for r in payload.get("cases", []):
        key = (
            f"{r.get('case_transport')}|{r.get('case_codec')}|{1 if r.get('case_roi_enable') else 0}"
            f"|{1 if r.get('case_native_roi_probe') else 0}"
        )
        if (not r.get("case_native_roi_probe")) and (r.get("case_roi_mode") in ("quality", "off")):
            baseline_map[key] = r

    def _delta_vs_off(r: Dict[str, Any], field: str) -> str:
        key = (
            f"{r.get('case_transport')}|{r.get('case_codec')}|{1 if r.get('case_roi_enable') else 0}"
            f"|{1 if r.get('case_native_roi_probe') else 0}"
        )
        base = baseline_map.get(key)
        if not base:
            return "-"
        if field == "p95":
            v = float((r.get("jitter_ms") or {}).get("p95", 0.0))
            b = float((base.get("jitter_ms") or {}).get("p95", 0.0))
            return f"{(v - b):+.3f}"
        if field == "cpu":
            v = float((((r.get("server_perf") or {}).get("proc_cpu_percent") or {}).get("mean") or 0.0))
            b = float((((base.get("server_perf") or {}).get("proc_cpu_percent") or {}).get("mean") or 0.0))
            return f"{(v - b):+.3f}"
        if field == "gpu":
            v = float((((r.get("server_perf") or {}).get("gpu_util_percent") or {}).get("mean") or 0.0))
            b = float((((base.get("server_perf") or {}).get("gpu_util_percent") or {}).get("mean") or 0.0))
            return f"{(v - b):+.3f}"
        if field == "fps":
            v = float(r.get("fps_avg") or 0.0)
            b = float(base.get("fps_avg") or 0.0)
            return f"{(v - b):+.3f}"
        return "-"

    lines: List[str] = []
    lines.append("# Codec + ROI Matrix Report")
    lines.append("")
    lines.append(f"- created_at: `{payload.get('created_at')}`")
    lines.append(f"- cases: `{len(payload.get('cases') or [])}`")
    lines.append("")
    lines.append("| case | roi_mode | native_roi_probe | ok | selected_transport | selected_codec | frames | fps_avg | delta_fps_vs_off | p95(ms) | delta_p95_vs_off(ms) | delta_cpu_mean_vs_off(%) | delta_gpu_mean_vs_off(%) | >100ms | agent_errors | quic_drop |")
    lines.append("|---|---|---:|---:|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|")
    for r in payload.get("cases", []):
        j = r.get("jitter_ms") or {}
        duration = max(1, int(r.get("duration_sec") or 8))
        fps_avg = round(float(r.get("frame_count", 0)) / float(duration), 3)
        r["fps_avg"] = fps_avg
        lines.append(
            "| {case} | {mode} | {native} | {ok} | {transport} | {codec} | {frames} | {fps} | {dfps} | {p95} | {dp95} | {dcpu} | {dgpu} | {g100} | {ae} | {qd} |".format(
                case=r.get("case_name"),
                mode=r.get("case_roi_mode", "-"),
                native="on" if r.get("case_native_roi_probe") else "off",
                ok="PASS" if r.get("ok") else "FAIL",
                transport=r.get("selected_transport") or "-",
                codec=((r.get("raw") or {}).get("answer_payload") or {}).get("selectedCodec", "-"),
                frames=r.get("frame_count", 0),
                fps=fps_avg,
                dfps=_delta_vs_off(r, "fps"),
                p95=j.get("p95", 0),
                dp95=_delta_vs_off(r, "p95"),
                dcpu=_delta_vs_off(r, "cpu"),
                dgpu=_delta_vs_off(r, "gpu"),
                g100=j.get("gt_100ms", 0),
                ae=r.get("agent_error_count", 0),
                qd=r.get("agent_quic_drop", 0),
            )
        )
    lines.append("")
    lines.append("## Paths")
    lines.append("")
    lines.append(f"- JSON: `{json_path}`")
    lines.append(f"- Logs: `{out_dir}`")
    md_path.write_text("\n".join(lines) + "\n", encoding="utf-8")
    return out_dir


def parse_args():
    parser = argparse.ArgumentParser(description="Codec/ROI matrix suite on core transports")
    parser.add_argument("--root", default="J:/ProjectTest/remote-desktop/mini-remote-desktop")
    parser.add_argument(
        "--config",
        default="J:/ProjectTest/remote-desktop/mini-remote-desktop/tests/python-core-transport/thresholds.quick.json",
    )
    parser.add_argument("--roi-rect", default="0.30,0.30,0.40,0.40")
    parser.add_argument("--roi-qoffset", type=float, default=-0.125)
    return parser.parse_args()


async def main_async(args):
    root = Path(args.root).resolve()
    cfg = load_config(Path(args.config).resolve())
    payload = await run_matrix(root, cfg, args.roi_rect, args.roi_qoffset)
    out_dir = write_report(root, payload)
    print("suite_log_dir", out_dir)
    print("suite_result", json.dumps(payload, ensure_ascii=False))


if __name__ == "__main__":
    args = parse_args()
    asyncio.run(main_async(args))
