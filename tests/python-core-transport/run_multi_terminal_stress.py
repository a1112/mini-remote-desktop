import argparse
import asyncio
import json
import os
import statistics
import time
from datetime import datetime
from pathlib import Path
from typing import Any, Dict, List, Optional, Tuple

from run_core_transport_suite import (
    CoreTransportSuite,
    ServerPerfMonitor,
    SignalingSession,
    summarize_intervals_ms,
)


class MultiTerminalStressSuite:
    def __init__(self, root: Path, cfg: dict):
        self.root = root
        self.cfg = cfg
        self.duration_sec = int(cfg.get("duration_sec", 20))
        self.transport = str(cfg.get("transport", "webrtc")).lower()
        self.clients = int(cfg.get("clients", 5))
        ts = datetime.now().strftime("%Y%m%d-%H%M%S")
        self.log_dir = root / "logs" / f"multi-terminal-stress-{self.transport}-{ts}"
        self.log_dir.mkdir(parents=True, exist_ok=True)
        thresholds = dict(cfg.get("thresholds", {}))
        thresholds.setdefault("control_connect_timeout_sec", 35)
        self._core = CoreTransportSuite(
            root,
            {
                "duration_sec": self.duration_sec,
                "thresholds": thresholds,
                "analysis": cfg.get("analysis", {}),
            },
        )

    async def _discover_target(self) -> str:
        signaling = SignalingSession("ws://127.0.0.1:9527")
        signaling.controller_name = f"mt-discover-{time.time_ns()}"
        await signaling.connect()
        try:
            await signaling.register()
            target_id = await signaling.discover_agent(timeout_sec=25)
            await self._core._send_capture_patch(signaling, target_id)
            return target_id
        finally:
            await signaling.close()

    async def _run_one_client(self, idx: int, target_id: str, start_delay_sec: float = 0.0) -> Dict[str, Any]:
        if start_delay_sec > 0:
            await asyncio.sleep(start_delay_sec)
        signaling = SignalingSession("ws://127.0.0.1:9527")
        signaling.controller_name = f"mt-client-{idx}-{time.time_ns()}"
        await signaling.connect()
        await signaling.register()
        try:
            pc, webrtc_frame_times, answer_payload, connected = await self._core._run_control_offer(
                signaling, target_id, self.transport
            )
            selected = (answer_payload.get("selectedTransport") or "").lower() or None
            if not connected:
                await pc.close()
                return {
                    "client_index": idx,
                    "ok": False,
                    "reason": "control not connected",
                    "selected_transport": selected,
                    "frame_count": 0,
                    "jitter_ms": {},
                    "tx_gap_ms": {},
                }

            frame_times: List[float] = []
            tx_us: List[int] = []
            if self.transport == "webrtc":
                await asyncio.sleep(self.duration_sec)
                frame_times = list(webrtc_frame_times)
            elif self.transport == "quic":
                quic = answer_payload.get("quic") or {}
                addr = quic.get("addr") or ""
                frame_records = await self._core._receive_quic_frames(addr, self.duration_sec)
                frame_times = [t[0] for t in frame_records]
                tx_us = [t[2] for t in frame_records]
            elif self.transport == "webtransport":
                wt = answer_payload.get("webtransport") or {}
                url = wt.get("url") or ""
                alpn = wt.get("alpn") or "h3"
                frame_records = await self._core._receive_webtransport_frames(url, alpn, self.duration_sec)
                frame_times = [t[0] for t in frame_records]
                tx_us = [t[2] for t in frame_records]
            else:
                await pc.close()
                return {
                    "client_index": idx,
                    "ok": False,
                    "reason": f"unsupported transport: {self.transport}",
                    "selected_transport": selected,
                    "frame_count": 0,
                    "jitter_ms": {},
                    "tx_gap_ms": {},
                }

            await pc.close()
            steady = self._core._steady_state_times(frame_times)
            rx_intervals_ms = [(steady[i] - steady[i - 1]) * 1000.0 for i in range(1, len(steady))]
            tx_intervals_ms = [
                (tx_us[i] - tx_us[i - 1]) / 1000.0
                for i in range(1, len(tx_us))
                if tx_us[i] >= tx_us[i - 1]
            ]
            return {
                "client_index": idx,
                "ok": True,
                "reason": "ok",
                "selected_transport": selected,
                "frame_count": len(frame_times),
                "jitter_ms": summarize_intervals_ms(rx_intervals_ms),
                "tx_gap_ms": summarize_intervals_ms(tx_intervals_ms),
            }
        except Exception as e:
            return {
                "client_index": idx,
                "ok": False,
                "reason": f"exception: {str(e).strip() or repr(e)}",
                "selected_transport": None,
                "frame_count": 0,
                "jitter_ms": {},
                "tx_gap_ms": {},
            }
        finally:
            await signaling.close()

    def _aggregate(self, client_results: List[Dict[str, Any]]) -> Dict[str, Any]:
        ok_count = sum(1 for r in client_results if r.get("ok"))
        frame_counts = [int(r.get("frame_count", 0)) for r in client_results]
        p95_list = [float((r.get("jitter_ms") or {}).get("p95", 0.0)) for r in client_results]
        p99_list = [float((r.get("jitter_ms") or {}).get("p99", 0.0)) for r in client_results]
        gt100_list = [int((r.get("jitter_ms") or {}).get("gt_100ms", 0)) for r in client_results]
        gt200_list = [int((r.get("jitter_ms") or {}).get("gt_200ms", 0)) for r in client_results]
        return {
            "clients_total": len(client_results),
            "clients_ok": ok_count,
            "success_rate": round(ok_count / max(1, len(client_results)), 3),
            "frame_count": {
                "min": min(frame_counts) if frame_counts else 0,
                "mean": round(statistics.mean(frame_counts), 3) if frame_counts else 0.0,
                "max": max(frame_counts) if frame_counts else 0,
            },
            "jitter": {
                "p95_mean": round(statistics.mean(p95_list), 3) if p95_list else 0.0,
                "p99_mean": round(statistics.mean(p99_list), 3) if p99_list else 0.0,
                "gt_100ms_total": sum(gt100_list),
                "gt_200ms_total": sum(gt200_list),
            },
        }

    async def run(self) -> Dict[str, Any]:
        signaling_proc = None
        agent_proc = None
        agent_out = None
        agent_err = None
        perf_monitor: Optional[ServerPerfMonitor] = None
        start = time.time()
        prev_max_clients = os.environ.get("AGENT_MAX_CLIENTS")
        try:
            os.environ["AGENT_MAX_CLIENTS"] = str(max(self.clients, 1))
            signaling_proc, agent_proc, agent_out, agent_err = self._core._start_stack(f"{self.transport}-mt")
            perf_monitor = ServerPerfMonitor(agent_proc.pid)
            await perf_monitor.start()
            await asyncio.sleep(2.0)
            target_id = await self._discover_target()
            tasks = [
                asyncio.create_task(self._run_one_client(i + 1, target_id, start_delay_sec=i * 0.35))
                for i in range(self.clients)
            ]
            client_results = await asyncio.gather(*tasks)
            agent_errors, quic_drop = self._core._parse_agent_health(agent_out, agent_err)
            payload = {
                "suite": "multi-terminal-stress",
                "created_at": datetime.now().isoformat(timespec="seconds"),
                "transport": self.transport,
                "clients": self.clients,
                "duration_sec": self.duration_sec,
                "elapsed_sec": round(time.time() - start, 3),
                "log_dir": str(self.log_dir),
                "agent_error_count": agent_errors,
                "agent_quic_drop": quic_drop,
                "server_perf": await perf_monitor.stop() if perf_monitor else {},
                "aggregate": self._aggregate(client_results),
                "clients_result": client_results,
            }
            self.write_reports(payload)
            return payload
        finally:
            if prev_max_clients is None:
                os.environ.pop("AGENT_MAX_CLIENTS", None)
            else:
                os.environ["AGENT_MAX_CLIENTS"] = prev_max_clients
            self._core._stop_proc(agent_proc)
            self._core._stop_proc(signaling_proc)

    def write_reports(self, payload: Dict[str, Any]) -> None:
        json_path = self.log_dir / "suite_result.json"
        md_path = self.log_dir / "suite_report.md"
        json_path.write_text(json.dumps(payload, ensure_ascii=False, indent=2), encoding="utf-8")

        lines: List[str] = []
        lines.append("# Multi Terminal Stress Report")
        lines.append("")
        lines.append(f"- created_at: `{payload['created_at']}`")
        lines.append(f"- transport: `{payload['transport']}`")
        lines.append(f"- clients: `{payload['clients']}`")
        lines.append(f"- duration_sec: `{payload['duration_sec']}`")
        lines.append(f"- elapsed_sec: `{payload['elapsed_sec']}`")
        lines.append(f"- log_dir: `{payload['log_dir']}`")
        lines.append("")
        agg = payload.get("aggregate") or {}
        jitter = agg.get("jitter") or {}
        frames = agg.get("frame_count") or {}
        lines.append("## Aggregate")
        lines.append("")
        lines.append(f"- success_rate: `{agg.get('success_rate')}`")
        lines.append(f"- clients_ok: `{agg.get('clients_ok')}` / `{agg.get('clients_total')}`")
        lines.append(f"- frame_count(min/mean/max): `{frames.get('min')}/{frames.get('mean')}/{frames.get('max')}`")
        lines.append(f"- jitter p95 mean: `{jitter.get('p95_mean')} ms`")
        lines.append(f"- jitter p99 mean: `{jitter.get('p99_mean')} ms`")
        lines.append(f"- gt_100ms total: `{jitter.get('gt_100ms_total')}`")
        lines.append(f"- gt_200ms total: `{jitter.get('gt_200ms_total')}`")
        lines.append(f"- agent_error_count: `{payload.get('agent_error_count')}`")
        lines.append(f"- agent_quic_drop: `{payload.get('agent_quic_drop')}`")
        lines.append("")
        lines.append("## Per Client")
        lines.append("")
        lines.append("| client | ok | selected | frames | p95(ms) | p99(ms) | >100ms | >200ms | reason |")
        lines.append("|---:|---:|---|---:|---:|---:|---:|---:|---|")
        for r in payload.get("clients_result", []):
            j = r.get("jitter_ms") or {}
            lines.append(
                "| {idx} | {ok} | {sel} | {frames} | {p95} | {p99} | {g100} | {g200} | {reason} |".format(
                    idx=r.get("client_index"),
                    ok="PASS" if r.get("ok") else "FAIL",
                    sel=r.get("selected_transport") or "-",
                    frames=r.get("frame_count", 0),
                    p95=j.get("p95", 0),
                    p99=j.get("p99", 0),
                    g100=j.get("gt_100ms", 0),
                    g200=j.get("gt_200ms", 0),
                    reason=(r.get("reason") or "")[:80],
                )
            )
        lines.append("")
        lines.append("## Paths")
        lines.append("")
        lines.append(f"- JSON: `{json_path}`")
        lines.append(f"- Logs: `{self.log_dir}`")
        md_path.write_text("\n".join(lines) + "\n", encoding="utf-8")


def parse_args():
    parser = argparse.ArgumentParser(description="5-terminal concurrent stress suite")
    parser.add_argument("--root", default="J:/ProjectTest/remote-desktop/mini-remote-desktop")
    parser.add_argument("--transport", default="webrtc", choices=["webrtc", "quic", "webtransport"])
    parser.add_argument("--clients", type=int, default=5)
    parser.add_argument("--duration", type=int, default=20)
    parser.add_argument("--analysis-json", default="")
    return parser.parse_args()


async def main_async(args):
    cfg = {
        "transport": args.transport,
        "clients": max(1, args.clients),
        "duration_sec": max(3, args.duration),
        "analysis": {},
        "thresholds": {},
    }
    if args.analysis_json:
        cfg["analysis"] = json.loads(Path(args.analysis_json).read_text(encoding="utf-8-sig"))
    root = Path(args.root).resolve()
    suite = MultiTerminalStressSuite(root, cfg)
    payload = await suite.run()
    print("suite_log_dir", suite.log_dir)
    print("suite_result", json.dumps(payload, ensure_ascii=False))


if __name__ == "__main__":
    args = parse_args()
    asyncio.run(main_async(args))
