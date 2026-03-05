import argparse
import asyncio
import base64
from collections import Counter
import json
import os
import re
import signal
import socket
import ssl
import statistics
import struct
import subprocess
import sys
import time
from dataclasses import dataclass
from datetime import datetime
from pathlib import Path
from typing import Any, Dict, List, Optional, Tuple
from urllib.parse import urlparse

import websockets
from aiortc import (
    RTCPeerConnection,
    RTCConfiguration,
    RTCIceServer,
    RTCSessionDescription,
)
from aioquic.asyncio import connect
from aioquic.asyncio.protocol import QuicConnectionProtocol
from aioquic.h3.connection import H3Connection
from aioquic.h3.events import HeadersReceived, WebTransportStreamDataReceived
from aioquic.quic.configuration import QuicConfiguration
from aioquic.quic.events import StreamDataReceived
import psutil


def percentile(values: List[float], p: float) -> float:
    if not values:
        return 0.0
    s = sorted(values)
    if len(s) == 1:
        return float(s[0])
    k = (len(s) - 1) * (p / 100.0)
    f = int(k)
    c = min(f + 1, len(s) - 1)
    if f == c:
        return float(s[f])
    return float(s[f] + (s[c] - s[f]) * (k - f))


def summarize_intervals_ms(intervals_ms: List[float]) -> Dict[str, Any]:
    total = len(intervals_ms)
    gt_100 = sum(1 for x in intervals_ms if x > 100.0)
    gt_200 = sum(1 for x in intervals_ms if x > 200.0)
    gt_500 = sum(1 for x in intervals_ms if x > 500.0)
    return {
        "samples": total,
        "mean": round(statistics.mean(intervals_ms), 3) if intervals_ms else 0.0,
        "std": round(statistics.pstdev(intervals_ms), 3) if len(intervals_ms) > 1 else 0.0,
        "p50": round(percentile(intervals_ms, 50), 3) if intervals_ms else 0.0,
        "p95": round(percentile(intervals_ms, 95), 3) if intervals_ms else 0.0,
        "p99": round(percentile(intervals_ms, 99), 3) if intervals_ms else 0.0,
        "max": round(max(intervals_ms), 3) if intervals_ms else 0.0,
        "gt_100ms": gt_100,
        "gt_200ms": gt_200,
        "gt_500ms": gt_500,
        "gt_100ms_ratio": round((gt_100 / total), 6) if total > 0 else 0.0,
        "gt_200ms_ratio": round((gt_200 / total), 6) if total > 0 else 0.0,
    }


class SignalingSession:
    def __init__(self, ws_url: str):
        self.ws_url = ws_url
        self.ws = None
        self.queue: asyncio.Queue[dict] = asyncio.Queue()
        self.reader_task = None
        self.controller_name = f"core-transport-suite-{int(time.time())}"

    async def connect(self) -> None:
        self.ws = await websockets.connect(self.ws_url, ping_interval=30)
        self.reader_task = asyncio.create_task(self._reader())

    async def close(self) -> None:
        if self.reader_task:
            self.reader_task.cancel()
            try:
                await self.reader_task
            except BaseException:
                pass
            self.reader_task = None
        if self.ws:
            await self.ws.close()
            self.ws = None

    async def _reader(self) -> None:
        assert self.ws is not None
        try:
            async for raw in self.ws:
                try:
                    msg = json.loads(raw)
                except Exception:
                    continue
                await self.queue.put(msg)
        except Exception:
            return

    async def send(self, payload: dict) -> None:
        assert self.ws is not None
        await self.ws.send(json.dumps(payload))

    async def register(self, codecs: Optional[List[str]] = None, features: Optional[List[str]] = None) -> None:
        codecs = [str(c).lower() for c in (codecs or ["h264"]) if str(c).strip()]
        if not codecs:
            codecs = ["h264"]
        features = [str(f) for f in (features or ["core-transport-suite"]) if str(f).strip()]
        await self.send({
            "type": "device",
            "action": "register",
            "payload": {
                "type": "controller",
                "name": self.controller_name,
                "protocolVersion": 2,
                "transports": ["webrtc", "quic", "webtransport"],
                "capabilities": {
                    "protocols": ["webrtc", "quic", "webtransport"],
                    "platforms": ["python"],
                    "codecs": codecs,
                    "features": features,
                },
            },
        })

    async def get_device_list(self) -> None:
        await self.send({"type": "device", "action": "getDeviceList", "payload": {}})

    async def discover_agent(self, timeout_sec: float) -> str:
        deadline = time.time() + timeout_sec
        while time.time() < deadline:
            await self.get_device_list()
            end = time.time() + 1.0
            while time.time() < end:
                left = max(0.1, deadline - time.time())
                try:
                    msg = await asyncio.wait_for(self.queue.get(), timeout=min(0.5, left))
                except asyncio.TimeoutError:
                    continue
                if msg.get("type") != "device":
                    continue
                payload = msg.get("payload") or {}
                devs = payload.get("deviceList") or []
                for d in devs:
                    if not d.get("online", True):
                        continue
                    name = d.get("name", "")
                    did = d.get("id", "")
                    if did and name and self.controller_name not in name and "Rust Agent" in name:
                        return did
            await asyncio.sleep(0.2)
        raise TimeoutError("agent discovery timeout")

    async def wait_for(self, *, typ: str, action: str, timeout_sec: float) -> dict:
        deadline = time.time() + timeout_sec
        while time.time() < deadline:
            left = max(0.1, deadline - time.time())
            msg = await asyncio.wait_for(self.queue.get(), timeout=left)
            if msg.get("type") == typ and msg.get("action") == action:
                return msg
        raise TimeoutError(f"wait_for timeout: {typ}/{action}")


class QuicFrameProtocol(QuicConnectionProtocol):
    def __init__(self, *args, **kwargs):
        super().__init__(*args, **kwargs)
        self.stream_buffers: Dict[int, bytearray] = {}
        self.frames: List[Tuple[float, int, int, int]] = []  # (rx_ts, seq, tx_us, payload_len)
        self.connected_event = asyncio.Event()

    def connection_made(self, transport):
        super().connection_made(transport)
        self.connected_event.set()

    def quic_event_received(self, event):
        if isinstance(event, StreamDataReceived):
            buf = self.stream_buffers.setdefault(event.stream_id, bytearray())
            buf.extend(event.data)
            self._drain_frames(buf)

    def _drain_frames(self, buf: bytearray) -> None:
        while True:
            if len(buf) < 20:
                return
            length = struct.unpack(">I", bytes(buf[0:4]))[0]
            frame_len = 20 + length
            if len(buf) < frame_len:
                return
            seq = struct.unpack(">Q", bytes(buf[4:12]))[0]
            tx_us = struct.unpack(">Q", bytes(buf[12:20]))[0]
            rx_ts = time.perf_counter()
            self.frames.append((rx_ts, seq, tx_us, length))
            del buf[:frame_len]


class WebTransportFrameProtocol(QuicConnectionProtocol):
    def __init__(self, *args, authority: str, path: str, **kwargs):
        super().__init__(*args, **kwargs)
        self.authority = authority
        self.path = path
        self.h3 = H3Connection(self._quic, enable_webtransport=True)
        self.session_stream_id: Optional[int] = None
        self.session_ready = asyncio.Event()
        self.session_failed = asyncio.Event()
        self.stream_buffers: Dict[int, bytearray] = {}
        self.frames: List[Tuple[float, int, int, int]] = []

    def start_session(self) -> None:
        sid = self._quic.get_next_available_stream_id(is_unidirectional=False)
        self.session_stream_id = sid
        headers = [
            (b":method", b"CONNECT"),
            (b":scheme", b"https"),
            (b":authority", self.authority.encode()),
            (b":path", self.path.encode()),
            (b":protocol", b"webtransport"),
            (b"sec-webtransport-http3-draft", b"draft02"),
            (b"user-agent", b"core-transport-suite"),
            (b"origin", f"https://{self.authority}".encode()),
        ]
        self.h3.send_headers(stream_id=sid, headers=headers, end_stream=False)
        self.transmit()

    def quic_event_received(self, event):
        for h3_event in self.h3.handle_event(event):
            if isinstance(h3_event, HeadersReceived):
                if h3_event.stream_id == self.session_stream_id:
                    status = None
                    for k, v in h3_event.headers:
                        if k == b":status":
                            status = v.decode(errors="ignore")
                            break
                    if status == "200":
                        self.session_ready.set()
                    else:
                        self.session_failed.set()
            elif isinstance(h3_event, WebTransportStreamDataReceived):
                buf = self.stream_buffers.setdefault(h3_event.stream_id, bytearray())
                buf.extend(h3_event.data)
                self._drain_frames(buf)

    def _drain_frames(self, buf: bytearray) -> None:
        while True:
            if len(buf) < 20:
                return
            length = struct.unpack(">I", bytes(buf[0:4]))[0]
            frame_len = 20 + length
            if len(buf) < frame_len:
                return
            seq = struct.unpack(">Q", bytes(buf[4:12]))[0]
            tx_us = struct.unpack(">Q", bytes(buf[12:20]))[0]
            rx_ts = time.perf_counter()
            self.frames.append((rx_ts, seq, tx_us, length))
            del buf[:frame_len]


def summarize_sizes_bytes(sizes: List[int]) -> Dict[str, Any]:
    if not sizes:
        return {"samples": 0}
    return {
        "samples": len(sizes),
        "min": int(min(sizes)),
        "mean": round(statistics.mean(sizes), 3),
        "p95": round(percentile([float(s) for s in sizes], 95), 3),
        "p99": round(percentile([float(s) for s in sizes], 99), 3),
        "max": int(max(sizes)),
    }


class ServerPerfMonitor:
    def __init__(self, pid: int, interval_sec: float = 1.0):
        self.pid = pid
        self.interval_sec = interval_sec
        self._start_wall_ts = time.time()
        self._stop_event = asyncio.Event()
        self._task: Optional[asyncio.Task] = None
        self.samples: List[Dict[str, Any]] = []
        self._tracked_pid: Optional[int] = None
        self._last_wall_ts: Optional[float] = None
        self._last_proc_cpu_s: Optional[float] = None

    async def start(self) -> None:
        self._task = asyncio.create_task(self._loop())

    async def stop(self) -> Dict[str, Any]:
        self._stop_event.set()
        if self._task:
            try:
                await self._task
            except Exception:
                pass
        return self.summary()

    async def _loop(self) -> None:
        try:
            parent = psutil.Process(self.pid)
        except Exception:
            return

        while not self._stop_event.is_set():
            proc = self._resolve_target_process(parent)
            sample: Dict[str, Any] = {
                "ts": time.time(),
                "proc_pid": None,
                "proc_name": None,
                "proc_cpu_percent": 0.0,
                "proc_rss_mb": 0.0,
                "gpu_util_percent": None,
                "gpu_mem_used_mb": None,
                "gpu_mem_total_mb": None,
                "gpu_temp_c": None,
            }
            try:
                if self._tracked_pid != proc.pid:
                    self._tracked_pid = proc.pid
                    self._last_wall_ts = None
                    self._last_proc_cpu_s = None
                now_ts = time.time()
                cpu_times = proc.cpu_times()
                proc_cpu_s = float(cpu_times.user + cpu_times.system)
                cpu_pct = 0.0
                if self._last_wall_ts is not None and self._last_proc_cpu_s is not None:
                    d_wall = max(1e-6, now_ts - self._last_wall_ts)
                    d_cpu = max(0.0, proc_cpu_s - self._last_proc_cpu_s)
                    cpu_pct = (d_cpu / d_wall) * 100.0
                self._last_wall_ts = now_ts
                self._last_proc_cpu_s = proc_cpu_s
                sample["proc_pid"] = proc.pid
                sample["proc_name"] = proc.name()
                sample["proc_cpu_percent"] = round(cpu_pct, 3)
                sample["proc_rss_mb"] = round(proc.memory_info().rss / (1024 * 1024), 3)
            except Exception:
                pass

            gpu = self._query_gpu()
            if gpu:
                sample.update(gpu)
            self.samples.append(sample)

            try:
                await asyncio.wait_for(self._stop_event.wait(), timeout=self.interval_sec)
            except asyncio.TimeoutError:
                pass

    def _resolve_target_process(self, parent: psutil.Process) -> psutil.Process:
        try:
            candidates = [parent]
            candidates.extend(parent.children(recursive=True))
            preferred = []
            fallback = []
            for p in candidates:
                try:
                    name = p.name().lower()
                except Exception:
                    continue
                if "agent-rust" in name:
                    preferred.append(p)
                else:
                    fallback.append(p)
            if preferred:
                preferred.sort(key=lambda x: x.pid)
                return preferred[-1]
            # Fallback: cargo parent may hide child tree in some runs.
            global_agent = []
            for p in psutil.process_iter(attrs=["pid", "name", "create_time"]):
                try:
                    name = str((p.info or {}).get("name") or "").lower()
                    cts = float((p.info or {}).get("create_time") or 0.0)
                except Exception:
                    continue
                if "agent-rust" in name and cts >= self._start_wall_ts - 120.0:
                    global_agent.append(p)
            if global_agent:
                global_agent.sort(key=lambda x: x.pid)
                return global_agent[-1]
            fallback.sort(key=lambda x: x.pid)
            return fallback[-1] if fallback else parent
        except Exception:
            return parent

    def _query_gpu(self) -> Optional[Dict[str, Any]]:
        try:
            out = subprocess.check_output(
                [
                    "nvidia-smi",
                    "--query-gpu=utilization.gpu,memory.used,memory.total,temperature.gpu",
                    "--format=csv,noheader,nounits",
                ],
                stderr=subprocess.DEVNULL,
                timeout=1.5,
                text=True,
            ).strip()
            if not out:
                return None
            first = out.splitlines()[0]
            parts = [p.strip() for p in first.split(",")]
            if len(parts) < 4:
                return None
            return {
                "gpu_util_percent": float(parts[0]),
                "gpu_mem_used_mb": float(parts[1]),
                "gpu_mem_total_mb": float(parts[2]),
                "gpu_temp_c": float(parts[3]),
            }
        except Exception:
            return None

    def summary(self) -> Dict[str, Any]:
        if not self.samples:
            return {"samples": 0}
        cpu = [s["proc_cpu_percent"] for s in self.samples]
        rss = [s["proc_rss_mb"] for s in self.samples]
        pids = [int(s["proc_pid"]) for s in self.samples if s.get("proc_pid") is not None]
        names = [str(s["proc_name"]) for s in self.samples if s.get("proc_name")]
        gpu_util = [s["gpu_util_percent"] for s in self.samples if s["gpu_util_percent"] is not None]
        gpu_mem = [s["gpu_mem_used_mb"] for s in self.samples if s["gpu_mem_used_mb"] is not None]
        target_pid = Counter(pids).most_common(1)[0][0] if pids else None
        target_name = Counter(names).most_common(1)[0][0] if names else None
        return {
            "samples": len(self.samples),
            "proc_target": {"pid": target_pid, "name": target_name},
            "proc_cpu_percent": {
                "mean": round(statistics.mean(cpu), 3),
                "p95": round(percentile(cpu, 95), 3),
                "max": round(max(cpu), 3),
            },
            "proc_rss_mb": {
                "mean": round(statistics.mean(rss), 3),
                "p95": round(percentile(rss, 95), 3),
                "max": round(max(rss), 3),
            },
            "gpu_util_percent": {
                "samples": len(gpu_util),
                "mean": round(statistics.mean(gpu_util), 3) if gpu_util else None,
                "p95": round(percentile(gpu_util, 95), 3) if gpu_util else None,
                "max": round(max(gpu_util), 3) if gpu_util else None,
            },
            "gpu_mem_used_mb": {
                "samples": len(gpu_mem),
                "mean": round(statistics.mean(gpu_mem), 3) if gpu_mem else None,
                "p95": round(percentile(gpu_mem, 95), 3) if gpu_mem else None,
                "max": round(max(gpu_mem), 3) if gpu_mem else None,
            },
        }


@dataclass
class TransportCaseResult:
    transport: str
    ok: bool
    reason: str
    selected_transport: Optional[str]
    frame_count: int
    jitter_ms: Dict[str, Any]
    tx_gap_ms: Dict[str, Any]
    agent_error_count: int
    agent_quic_drop: int
    server_perf: Dict[str, Any]
    raw: Dict[str, Any]


class CoreTransportSuite:
    def __init__(self, root: Path, cfg: dict):
        self.root = root
        self.cfg = cfg
        self.duration_sec = int(cfg.get("duration_sec", 30))
        self.thresholds = cfg.get("thresholds", {})
        self.analysis = cfg.get("analysis") or {}
        self.controller_codecs = [
            str(c).lower()
            for c in (self.analysis.get("controller_codecs") or ["h264"])
            if str(c).strip()
        ]
        if not self.controller_codecs:
            self.controller_codecs = ["h264"]
        self.controller_features = [
            str(f)
            for f in (self.analysis.get("controller_features") or ["core-transport-suite"])
            if str(f).strip()
        ]
        if not self.controller_features:
            self.controller_features = ["core-transport-suite"]
        ts = datetime.now().strftime("%Y%m%d-%H%M%S")
        self.log_dir = root / "logs" / f"core-transport-suite-{ts}"
        self.log_dir.mkdir(parents=True, exist_ok=True)

    def _start_stack(self, tag: str) -> Tuple[Any, Any, Path, Path]:
        import subprocess

        signaling_out = self.log_dir / f"signaling.{tag}.out.log"
        signaling_err = self.log_dir / f"signaling.{tag}.err.log"
        agent_out = self.log_dir / f"agent.{tag}.out.log"
        agent_err = self.log_dir / f"agent.{tag}.err.log"

        s_out = open(signaling_out, "w", encoding="utf-8", errors="ignore")
        s_err = open(signaling_err, "w", encoding="utf-8", errors="ignore")
        a_out = open(agent_out, "w", encoding="utf-8", errors="ignore")
        a_err = open(agent_err, "w", encoding="utf-8", errors="ignore")

        signaling = subprocess.Popen(
            ["cargo", "run"],
            cwd=self.root / "signaling-rs",
            stdout=s_out,
            stderr=s_err,
            creationflags=0,
        )
        if not self._wait_tcp_open("127.0.0.1", 9527, timeout_sec=20.0):
            self._stop_proc(signaling)
            raise RuntimeError("signaling server not ready on ws://127.0.0.1:9527")
        agent_env = os.environ.copy()
        # Raise sender queue to reduce burst drops/stalls in QUIC/WebTransport tests.
        agent_env.setdefault("AGENT_QUIC_QUEUE", "512")
        agent_env.setdefault("AGENT_WEBTRANSPORT_QUEUE", "256")
        # QUIC/WebTransport pacer: smooth sender bursts to reduce tail jitter.
        analysis = self.analysis
        pacer = analysis.get("quic_pacer") or {}
        agent_env["AGENT_QUIC_PACE_ENABLE"] = str(
            pacer.get("enable", True)
        ).lower().replace("true", "1").replace("false", "0")
        agent_env["AGENT_QUIC_PACE_MODE"] = str(pacer.get("mode", "manual"))
        agent_env["AGENT_QUIC_PACE_INTERVAL_MS"] = str(int(pacer.get("interval_ms", 1)))
        agent_env["AGENT_QUIC_PACE_BURST"] = str(int(pacer.get("burst", 2)))
        agent_env["AGENT_QUIC_PACE_AUTO_ON_FULL"] = str(int(pacer.get("auto_on_full", 8)))
        agent_env["AGENT_QUIC_PACE_AUTO_OFF_OK"] = str(int(pacer.get("auto_off_ok", 64)))
        qrl = analysis.get("quic_queue_rate_link") or {}
        agent_env["AGENT_QUIC_QUEUE_RATE_LINK_ENABLE"] = str(
            qrl.get("enable", False)
        ).lower().replace("true", "1").replace("false", "0")
        agent_env["AGENT_QUIC_QUEUE_RATE_LINK_MIN_FPS"] = str(int(qrl.get("min_fps", 24)))
        agent_env["AGENT_QUIC_QUEUE_RATE_LINK_MAX_FPS"] = str(int(qrl.get("max_fps", 144)))
        agent_env["AGENT_QUIC_QUEUE_RATE_LINK_DOWN_STEP"] = str(int(qrl.get("down_step", 8)))
        agent_env["AGENT_QUIC_QUEUE_RATE_LINK_UP_STEP"] = str(int(qrl.get("up_step", 2)))
        agent_env["AGENT_QUIC_QUEUE_RATE_LINK_FULL_THRESHOLD"] = str(
            int(qrl.get("full_threshold", 8))
        )
        agent_env["AGENT_QUIC_QUEUE_RATE_LINK_OK_THRESHOLD"] = str(
            int(qrl.get("ok_threshold", 120))
        )
        agent_env["AGENT_QUIC_QUEUE_RATE_LINK_COOLDOWN_MS"] = str(
            int(qrl.get("cooldown_ms", 200))
        )
        for k, v in (analysis.get("agent_env") or {}).items():
            key = str(k).strip()
            if not key:
                continue
            if isinstance(v, bool):
                agent_env[key] = "1" if v else "0"
            else:
                agent_env[key] = str(v)

        agent = subprocess.Popen(
            ["cargo", "run", "--bin", "agent-rust"],
            cwd=self.root / "agent-rust",
            stdout=a_out,
            stderr=a_err,
            env=agent_env,
            creationflags=0,
        )
        time.sleep(1.0)
        if agent.poll() is not None:
            agent = subprocess.Popen(
                ["cargo", "run", "--bin", "agent-rust"],
                cwd=self.root / "agent-rust",
                stdout=a_out,
                stderr=a_err,
                env=agent_env,
                creationflags=0,
            )
            time.sleep(1.0)
            if agent.poll() is not None:
                raise RuntimeError("agent-rust exited during startup")
        return signaling, agent, agent_out, agent_err

    def _wait_tcp_open(self, host: str, port: int, timeout_sec: float) -> bool:
        deadline = time.time() + timeout_sec
        while time.time() < deadline:
            try:
                with socket.create_connection((host, port), timeout=0.5):
                    return True
            except OSError:
                time.sleep(0.2)
        return False

    def _stop_proc(self, proc) -> None:
        if proc is None:
            return
        if proc.poll() is None:
            try:
                proc.terminate()
                proc.wait(timeout=5)
            except Exception:
                try:
                    proc.kill()
                except Exception:
                    pass

    def _read_agent_text(self, agent_out: Path, agent_err: Path) -> str:
        text = ""
        if agent_out.exists():
            text += agent_out.read_text(encoding="utf-8", errors="ignore")
        if agent_err.exists():
            text += "\n" + agent_err.read_text(encoding="utf-8", errors="ignore")
        return text

    def _parse_agent_health(self, agent_out: Path, agent_err: Path) -> Tuple[int, int]:
        text = self._read_agent_text(agent_out, agent_err)
        error_count = 0
        for line in text.splitlines():
            clean = re.sub(r"\x1b\[[0-9;]*m", "", line).strip()
            if not clean:
                continue
            if re.search(r"\bERROR\b", clean) or clean.startswith("Error:") or re.match(r"^error:\s", clean):
                error_count += 1
        quic_drop = 0
        for line in text.splitlines():
            clean = re.sub(r"\x1b\[[0-9;]*m", "", line)
            if "quic_au_dropped" in clean:
                try:
                    part = clean.split("quic_au_dropped")[1]
                    v = int(part.split("=")[1].split()[0])
                    quic_drop = max(quic_drop, v)
                except Exception:
                    pass
        return error_count, quic_drop

    def _parse_agent_encoder_diag(self, agent_out: Path, agent_err: Path) -> Dict[str, Any]:
        text = self._read_agent_text(agent_out, agent_err)
        restart_count = 0
        ffmpeg_pipe_start_count = 0
        roi_requested_count = 0
        roi_applied_count = 0
        for line in text.splitlines():
            clean = re.sub(r"\x1b\[[0-9;]*m", "", line).strip()
            if "ffmpeg_pipe_restart" in clean:
                restart_count += 1
            if "ffmpeg_pipe_start" in clean:
                ffmpeg_pipe_start_count += 1
                m_req = re.search(r"roi_requested[=:](true|false|1|0)", clean, re.IGNORECASE)
                if m_req and m_req.group(1).lower() in ("true", "1"):
                    roi_requested_count += 1
                m_app = re.search(r"roi_applied[=:](true|false|1|0)", clean, re.IGNORECASE)
                if m_app and m_app.group(1).lower() in ("true", "1"):
                    roi_applied_count += 1
        roi_effectiveness = None
        if roi_requested_count > 0:
            roi_effectiveness = round(roi_applied_count / max(1, roi_requested_count), 3)
        return {
            "ffmpeg_pipe_restart_count": restart_count,
            "ffmpeg_pipe_start_count": ffmpeg_pipe_start_count,
            "roi_requested_count": roi_requested_count,
            "roi_applied_count": roi_applied_count,
            "roi_effectiveness": roi_effectiveness,
        }

    @staticmethod
    def _extract_numeric_field(text: str, key: str) -> Optional[float]:
        m = re.search(rf"{re.escape(key)}[=:]\"?(-?\d+(?:\.\d+)?)\"?", text)
        if not m:
            return None
        try:
            return float(m.group(1))
        except Exception:
            return None

    def _parse_agent_pipeline_metrics(self, agent_out: Path, agent_err: Path) -> Dict[str, Any]:
        text = self._read_agent_text(agent_out, agent_err)
        last_line = ""
        for line in text.splitlines():
            clean = re.sub(r"\x1b\[[0-9;]*m", "", line).strip()
            if "[PIPELINE-STATS]" not in clean:
                continue
            if "side=\"agent\"" in clean or "side=agent" in clean:
                last_line = clean
        if not last_line:
            return {}
        keys = [
            "stage_capture_jitter_ms",
            "stage_encode_output_jitter_ms",
            "stage_send_interval_jitter_ms",
            "stage_queue_wait_std_ms",
            "stage_send_std_ms",
            "stage_capture_std_ms",
            "stage_encode_std_ms",
        ]
        out: Dict[str, Any] = {"line": last_line}
        for key in keys:
            out[key] = self._extract_numeric_field(last_line, key)
        return out

    async def _run_control_offer(self, signaling: SignalingSession, target_id: str, transport: str):
        pc = RTCPeerConnection(
            RTCConfiguration(
                iceServers=[
                    RTCIceServer(urls=["stun:stun.l.google.com:19302"]),
                ]
            )
        )
        connected_ev = asyncio.Event()
        frame_times: List[float] = []

        @pc.on("iceconnectionstatechange")
        async def on_ice_state_change():
            if pc.iceConnectionState in ("connected", "completed"):
                connected_ev.set()

        @pc.on("track")
        def on_track(track):
            if track.kind != "video":
                return

            async def recv_loop():
                while True:
                    try:
                        frame = await track.recv()
                    except Exception:
                        break
                    if frame is None:
                        break
                    frame_times.append(time.perf_counter())

            asyncio.create_task(recv_loop())

        @pc.on("icecandidate")
        async def on_ice_candidate(candidate):
            if candidate is None:
                return
            await signaling.send(
                {
                    "type": "webrtc",
                    "action": "iceCandidate",
                    "payload": {
                        "targetDeviceId": target_id,
                        "candidate": {
                            "candidate": candidate.candidate,
                            "sdpMid": candidate.sdpMid,
                            "sdpMLineIndex": candidate.sdpMLineIndex,
                        },
                    },
                }
            )

        pc.addTransceiver("video", direction="recvonly")
        offer = await pc.createOffer()
        await pc.setLocalDescription(offer)

        await signaling.send(
            {
                "type": "webrtc",
                "action": "offer",
                "payload": {
                    "targetDeviceId": target_id,
                    "transport": transport,
                    "offer": {"type": "offer", "sdp": offer.sdp},
                    "capabilities": {
                        "protocols": ["webrtc", "quic", "webtransport"],
                        "platforms": ["python"],
                        "codecs": list(self.controller_codecs),
                        "features": list(self.controller_features),
                    },
                },
            }
        )

        timeout_sec = float(self.thresholds.get("control_connect_timeout_sec", 20))
        answer_msg = await signaling.wait_for(typ="webrtc", action="answer", timeout_sec=timeout_sec)
        payload = answer_msg.get("payload") or {}
        answer = payload.get("answer") or {}
        sdp = answer.get("sdp", "")
        await pc.setRemoteDescription(RTCSessionDescription(sdp=sdp, type="answer"))

        # ICE exchange loop (for a short window)
        ice_end = time.time() + timeout_sec
        while time.time() < ice_end and not connected_ev.is_set():
            try:
                msg = await asyncio.wait_for(signaling.queue.get(), timeout=0.5)
            except asyncio.TimeoutError:
                continue
            if msg.get("type") == "webrtc" and msg.get("action") == "iceCandidate":
                cand = ((msg.get("payload") or {}).get("candidate") or {})
                cstr = cand.get("candidate", "")
                if cstr:
                    from aiortc.sdp import candidate_from_sdp

                    rc = candidate_from_sdp(cstr)
                    rc.sdpMid = cand.get("sdpMid")
                    rc.sdpMLineIndex = cand.get("sdpMLineIndex")
                    await pc.addIceCandidate(rc)

        return pc, frame_times, payload, connected_ev.is_set()

    async def _receive_quic_frames(self, addr: str, duration_sec: int) -> List[Tuple[float, int, int, int]]:
        host, port_str = addr.rsplit(":", 1)
        port = int(port_str)
        cfg = QuicConfiguration(is_client=True)
        cfg.verify_mode = ssl.CERT_NONE
        frames: List[Tuple[float, int, int, int]] = []
        async with connect(host, port, configuration=cfg, create_protocol=QuicFrameProtocol) as client:
            protocol: QuicFrameProtocol = client
            await asyncio.sleep(duration_sec)
            frames = list(protocol.frames)
        return frames

    async def _receive_webtransport_frames(self, url: str, alpn: str, duration_sec: int) -> List[Tuple[float, int, int, int]]:
        parsed = urlparse(url)
        host = parsed.hostname or "127.0.0.1"
        port = parsed.port or 443
        path = parsed.path or "/mrd"
        authority = f"{host}:{port}"

        cfg = QuicConfiguration(is_client=True, alpn_protocols=[alpn or "h3"])
        cfg.verify_mode = ssl.CERT_NONE

        async with connect(
            host,
            port,
            configuration=cfg,
            create_protocol=lambda *args, **kwargs: WebTransportFrameProtocol(
                *args, authority=authority, path=path, **kwargs
            ),
        ) as client:
            protocol: WebTransportFrameProtocol = client
            protocol.start_session()
            ready_task = asyncio.create_task(protocol.session_ready.wait())
            failed_task = asyncio.create_task(protocol.session_failed.wait())
            done, pending = await asyncio.wait(
                [ready_task, failed_task],
                timeout=10.0,
                return_when=asyncio.FIRST_COMPLETED,
            )
            for t in pending:
                t.cancel()
            if not done or protocol.session_failed.is_set() or not protocol.session_ready.is_set():
                return []
            await asyncio.sleep(duration_sec)
            return list(protocol.frames)

    def _evaluate_case(self, transport: str, frame_times: List[float], tx_us_list: List[int], selected: Optional[str], agent_errors: int, quic_drop: int, reason: str = "") -> TransportCaseResult:
        steady_times = self._steady_state_times(frame_times)
        rx_intervals_ms = [
            (steady_times[i] - steady_times[i - 1]) * 1000.0 for i in range(1, len(steady_times))
        ]
        tx_intervals_ms = [
            (tx_us_list[i] - tx_us_list[i - 1]) / 1000.0 for i in range(1, len(tx_us_list)) if tx_us_list[i] >= tx_us_list[i - 1]
        ]

        jitter = summarize_intervals_ms(rx_intervals_ms)
        tx_gap = summarize_intervals_ms(tx_intervals_ms)

        min_frames = int((self.thresholds.get("min_frames") or {}).get(transport, 1))
        jth = self.thresholds.get("jitter") or {}

        ok = True
        if len(frame_times) < min_frames:
            ok = False
            reason = reason or f"insufficient frames: {len(frame_times)} < {min_frames}"
        if jitter["p95"] > float(jth.get("p95_ms_max", 1e9)):
            ok = False
            reason = reason or f"jitter p95 too high: {jitter['p95']}ms"
        if jitter["p99"] > float(jth.get("p99_ms_max", 1e9)):
            ok = False
            reason = reason or f"jitter p99 too high: {jitter['p99']}ms"
        if jitter["gt_100ms"] > int(jth.get("gt_100ms_max", 1_000_000)):
            ok = False
            reason = reason or f"too many >100ms stalls: {jitter['gt_100ms']}"
        if jitter["gt_200ms"] > int(jth.get("gt_200ms_max", 1_000_000)):
            ok = False
            reason = reason or f"too many >200ms stalls: {jitter['gt_200ms']}"
        if jitter["gt_100ms_ratio"] > float(jth.get("gt_100ms_ratio_max", 1.0)):
            ok = False
            reason = reason or f">100ms stall ratio too high: {jitter['gt_100ms_ratio']}"
        if jitter["gt_200ms_ratio"] > float(jth.get("gt_200ms_ratio_max", 1.0)):
            ok = False
            reason = reason or f">200ms stall ratio too high: {jitter['gt_200ms_ratio']}"

        if agent_errors > int(self.thresholds.get("agent_errors_max", 0)):
            ok = False
            reason = reason or f"agent errors: {agent_errors}"
        if quic_drop > int(self.thresholds.get("agent_quic_drop_max", 0)):
            ok = False
            reason = reason or f"agent quic drop: {quic_drop}"

        return TransportCaseResult(
            transport=transport,
            ok=ok,
            reason=reason or "ok",
            selected_transport=selected,
            frame_count=len(frame_times),
            jitter_ms=jitter,
            tx_gap_ms=tx_gap,
            agent_error_count=agent_errors,
            agent_quic_drop=quic_drop,
            server_perf={},
            raw={},
        )

    def _apply_diag_thresholds(self, res: TransportCaseResult) -> None:
        diag_th = (self.thresholds.get("diag") or {})
        raw = res.raw or {}
        diag = raw.get("encoder_diag") or {}
        if "ffmpeg_restart_max" in diag_th:
            max_restart = int(diag_th.get("ffmpeg_restart_max", 1_000_000))
            rs = int(diag.get("ffmpeg_pipe_restart_count", 0))
            if rs > max_restart:
                res.ok = False
                if res.reason == "ok":
                    res.reason = f"ffmpeg restart too high: {rs} > {max_restart}"
        if "roi_effectiveness_min" in diag_th:
            min_eff = float(diag_th.get("roi_effectiveness_min", 0.0))
            eff = diag.get("roi_effectiveness")
            if eff is not None and float(eff) < min_eff:
                res.ok = False
                if res.reason == "ok":
                    res.reason = f"roi effectiveness too low: {eff} < {min_eff}"

    def _apply_pipeline_thresholds(self, res: TransportCaseResult) -> None:
        pj = (self.thresholds.get("agent_pipeline_jitter") or {})
        if not pj:
            return
        raw = res.raw or {}
        metrics = raw.get("agent_pipeline") or {}
        checks = [
            ("capture_jitter_ms_max", "stage_capture_jitter_ms"),
            ("encode_output_jitter_ms_max", "stage_encode_output_jitter_ms"),
            ("send_interval_jitter_ms_max", "stage_send_interval_jitter_ms"),
            ("queue_wait_std_ms_max", "stage_queue_wait_std_ms"),
            ("send_std_ms_max", "stage_send_std_ms"),
        ]
        for th_key, metric_key in checks:
            if th_key not in pj:
                continue
            limit = float(pj.get(th_key, 1e9))
            value = metrics.get(metric_key)
            if value is None:
                res.ok = False
                if res.reason == "ok":
                    res.reason = f"missing agent pipeline metric: {metric_key}"
                continue
            if float(value) > limit:
                res.ok = False
                if res.reason == "ok":
                    res.reason = (
                        f"agent {metric_key} too high: {value:.3f} > {limit:.3f}"
                    )

    def _steady_state_times(self, frame_times: List[float]) -> List[float]:
        if len(frame_times) <= 3:
            return list(frame_times)
        warmup = float((self.cfg.get("analysis") or {}).get("warmup_sec", 1.0))
        cooldown = float((self.cfg.get("analysis") or {}).get("cooldown_sec", 0.5))
        start_t = frame_times[0] + max(0.0, warmup)
        end_t = frame_times[-1] - max(0.0, cooldown)
        if end_t <= start_t:
            return list(frame_times)
        trimmed = [t for t in frame_times if t >= start_t and t <= end_t]
        if len(trimmed) >= 3:
            return trimmed
        return list(frame_times)

    async def _send_capture_patch(self, signaling: SignalingSession, target_id: str) -> None:
        analysis = self.cfg.get("analysis") or {}
        patch = analysis.get("capture_patch")
        if not isinstance(patch, dict) or not patch:
            return
        await signaling.send(
            {
                "type": "control",
                "action": "updateCapture",
                "payload": {
                    "targetDeviceId": target_id,
                    "capture": patch,
                },
            }
        )

    async def run_case(self, transport: str) -> TransportCaseResult:
        signaling_proc = None
        agent_proc = None
        agent_out = None
        agent_err = None
        perf_monitor: Optional[ServerPerfMonitor] = None
        try:
            signaling_proc, agent_proc, agent_out, agent_err = self._start_stack(transport)
            perf_monitor = ServerPerfMonitor(agent_proc.pid)
            await perf_monitor.start()
            await asyncio.sleep(3)

            signaling = SignalingSession("ws://127.0.0.1:9527")
            await signaling.connect()
            await signaling.register(codecs=self.controller_codecs, features=self.controller_features)
            target_id = await signaling.discover_agent(timeout_sec=25)
            await self._send_capture_patch(signaling, target_id)

            pc, webrtc_frame_times, answer_payload, connected = await self._run_control_offer(
                signaling, target_id, transport
            )
            selected = (answer_payload.get("selectedTransport") or "").lower() or None

            if not connected:
                await pc.close()
                await signaling.close()
                agent_errors, quic_drop = self._parse_agent_health(agent_out, agent_err)
                return TransportCaseResult(
                    transport=transport,
                    ok=False,
                    reason="control connection not connected",
                    selected_transport=selected,
                    frame_count=0,
                    jitter_ms={},
                    tx_gap_ms={},
                    agent_error_count=agent_errors,
                    agent_quic_drop=quic_drop,
                    server_perf=await perf_monitor.stop() if perf_monitor else {},
                    raw={"answer_payload": answer_payload},
                )

            frame_records: List[Tuple[float, int, int, int]] = []
            if transport == "webrtc":
                await asyncio.sleep(self.duration_sec)
                frame_times = list(webrtc_frame_times)
                tx_us: List[int] = []
                au_sizes: List[int] = []
            elif transport == "quic":
                quic = answer_payload.get("quic") or {}
                addr = quic.get("addr") or ""
                frame_records = await self._receive_quic_frames(addr, self.duration_sec)
                frame_times = [t[0] for t in frame_records]
                tx_us = [t[2] for t in frame_records]
                au_sizes = [int(t[3]) for t in frame_records]
            elif transport == "webtransport":
                wt = answer_payload.get("webtransport") or {}
                url = wt.get("url") or ""
                alpn = wt.get("alpn") or "h3"
                frame_records = await self._receive_webtransport_frames(url, alpn, self.duration_sec)
                frame_times = [t[0] for t in frame_records]
                tx_us = [t[2] for t in frame_records]
                au_sizes = [int(t[3]) for t in frame_records]
            else:
                frame_times = []
                tx_us = []
                au_sizes = []

            await pc.close()
            await signaling.close()

            agent_errors, quic_drop = self._parse_agent_health(agent_out, agent_err)
            encoder_diag = self._parse_agent_encoder_diag(agent_out, agent_err)
            agent_pipeline = self._parse_agent_pipeline_metrics(agent_out, agent_err)
            res = self._evaluate_case(
                transport=transport,
                frame_times=frame_times,
                tx_us_list=tx_us,
                selected=selected,
                agent_errors=agent_errors,
                quic_drop=quic_drop,
            )
            if perf_monitor:
                res.server_perf = await perf_monitor.stop()
            res.raw = {
                "answer_payload": answer_payload,
                "connected": connected,
                "records": len(frame_records),
                "duration_sec": self.duration_sec,
                "au_size_bytes": summarize_sizes_bytes(au_sizes),
                "encoder_diag": encoder_diag,
                "agent_pipeline": agent_pipeline,
            }
            self._apply_diag_thresholds(res)
            self._apply_pipeline_thresholds(res)
            return res

        except Exception as e:
            agent_errors = 0
            quic_drop = 0
            if agent_out and agent_err:
                agent_errors, quic_drop = self._parse_agent_health(agent_out, agent_err)
            perf = {}
            if perf_monitor:
                perf = await perf_monitor.stop()
            return TransportCaseResult(
                transport=transport,
                ok=False,
                reason=f"exception: {str(e).strip() or repr(e)}",
                selected_transport=None,
                frame_count=0,
                jitter_ms={},
                tx_gap_ms={},
                agent_error_count=agent_errors,
                agent_quic_drop=quic_drop,
                server_perf=perf,
                raw={},
            )
        finally:
            self._stop_proc(agent_proc)
            self._stop_proc(signaling_proc)

    async def run(self, transports: List[str]) -> Dict[str, Any]:
        results: List[TransportCaseResult] = []
        for t in transports:
            r = await self.run_case(t)
            results.append(r)

        payload = {
            "suite": "python-core-transport",
            "created_at": datetime.now().isoformat(timespec="seconds"),
            "log_dir": str(self.log_dir),
            "duration_sec": self.duration_sec,
            "thresholds": self.thresholds,
            "results": [
                {
                    "transport": r.transport,
                    "ok": r.ok,
                    "reason": r.reason,
                    "selected_transport": r.selected_transport,
                    "frame_count": r.frame_count,
                    "jitter_ms": r.jitter_ms,
                    "tx_gap_ms": r.tx_gap_ms,
                    "agent_error_count": r.agent_error_count,
                    "agent_quic_drop": r.agent_quic_drop,
                    "server_perf": r.server_perf,
                    "raw": r.raw,
                }
                for r in results
            ],
        }
        return payload

    def write_reports(self, payload: Dict[str, Any]) -> None:
        json_path = self.log_dir / "suite_result.json"
        md_path = self.log_dir / "suite_report.md"

        json_path.write_text(json.dumps(payload, ensure_ascii=False, indent=2), encoding="utf-8")

        lines: List[str] = []
        lines.append("# Core Transport Suite Report")
        lines.append("")
        lines.append(f"- created_at: `{payload['created_at']}`")
        lines.append(f"- duration_sec: `{payload['duration_sec']}`")
        lines.append(f"- log_dir: `{payload['log_dir']}`")
        lines.append("")
        lines.append("## Summary")
        lines.append("")
        lines.append("| transport | ok | selected | frames | p95(ms) | p99(ms) | >100ms | >100 ratio | >200ms | >200 ratio | agent_errors | quic_drop | ffmpeg_restart | roi_effective | au_mean(B) | au_p95(B) | reason |")
        lines.append("|---|---:|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|")
        for r in payload["results"]:
            j = r.get("jitter_ms") or {}
            raw = r.get("raw") or {}
            diag = raw.get("encoder_diag") or {}
            au = raw.get("au_size_bytes") or {}
            lines.append(
                "| {transport} | {ok} | {selected} | {frames} | {p95} | {p99} | {g100} | {g100r} | {g200} | {g200r} | {ae} | {qd} | {rs} | {roi} | {aumean} | {aup95} | {reason} |".format(
                    transport=r["transport"],
                    ok="PASS" if r["ok"] else "FAIL",
                    selected=r.get("selected_transport") or "-",
                    frames=r.get("frame_count", 0),
                    p95=j.get("p95", 0),
                    p99=j.get("p99", 0),
                    g100=j.get("gt_100ms", 0),
                    g100r=j.get("gt_100ms_ratio", 0),
                    g200=j.get("gt_200ms", 0),
                    g200r=j.get("gt_200ms_ratio", 0),
                    ae=r.get("agent_error_count", 0),
                    qd=r.get("agent_quic_drop", 0),
                    rs=diag.get("ffmpeg_pipe_restart_count", 0),
                    roi=diag.get("roi_effectiveness", "-"),
                    aumean=au.get("mean", "-"),
                    aup95=au.get("p95", "-"),
                    reason=(r.get("reason") or "")[:80],
                )
            )

        lines.append("")
        lines.append("## Server Perf (Sender)")
        lines.append("")
        lines.append("| transport | cpu_mean% | cpu_p95% | rss_mean(MB) | gpu_util_mean% | gpu_mem_mean(MB) |")
        lines.append("|---|---:|---:|---:|---:|---:|")
        for r in payload["results"]:
            p = r.get("server_perf") or {}
            cpu = p.get("proc_cpu_percent") or {}
            rss = p.get("proc_rss_mb") or {}
            gpu = p.get("gpu_util_percent") or {}
            gmem = p.get("gpu_mem_used_mb") or {}
            lines.append(
                f"| {r.get('transport')} | {cpu.get('mean')} | {cpu.get('p95')} | {rss.get('mean')} | {gpu.get('mean')} | {gmem.get('mean')} |"
            )

        lines.append("")
        lines.append("## Agent Pipeline")
        lines.append("")
        lines.append("| transport | capture_jitter(ms) | encode_out_jitter(ms) | send_interval_jitter(ms) | queue_wait_std(ms) | send_std(ms) |")
        lines.append("|---|---:|---:|---:|---:|---:|")
        for r in payload["results"]:
            p = ((r.get("raw") or {}).get("agent_pipeline") or {})
            lines.append(
                "| {t} | {c} | {e} | {s} | {q} | {sd} |".format(
                    t=r.get("transport"),
                    c=p.get("stage_capture_jitter_ms"),
                    e=p.get("stage_encode_output_jitter_ms"),
                    s=p.get("stage_send_interval_jitter_ms"),
                    q=p.get("stage_queue_wait_std_ms"),
                    sd=p.get("stage_send_std_ms"),
                )
            )

        lines.append("")
        lines.append("## Paths")
        lines.append("")
        lines.append(f"- JSON: `{json_path}`")
        lines.append(f"- Logs: `{self.log_dir}`")

        md_path.write_text("\n".join(lines) + "\n", encoding="utf-8")


def load_config(path: Path) -> dict:
    return json.loads(path.read_text(encoding="utf-8-sig"))


async def main_async(args):
    root = Path(args.root).resolve()
    cfg = load_config(Path(args.config).resolve())
    suite = CoreTransportSuite(root, cfg)
    payload = await suite.run(args.transports)
    suite.write_reports(payload)
    print("suite_log_dir", suite.log_dir)
    print("suite_result", json.dumps(payload, ensure_ascii=False))


def parse_args():
    parser = argparse.ArgumentParser(description="Core transport automation suite")
    parser.add_argument("--root", default="J:/ProjectTest/remote-desktop/mini-remote-desktop")
    parser.add_argument(
        "--config",
        default="J:/ProjectTest/remote-desktop/mini-remote-desktop/tests/python-core-transport/thresholds.json",
    )
    parser.add_argument(
        "--transports",
        nargs="+",
        default=["webrtc", "quic", "webtransport"],
    )
    parser.add_argument("--codec", default="", help="Preferred codec for controller capability and capture patch (h264/hevc/av1)")
    parser.add_argument("--roi-enable", action="store_true", help="Enable ROI capture patch")
    parser.add_argument("--roi-rect", default="0.30,0.30,0.40,0.40", help="ROI rect normalized x,y,w,h")
    parser.add_argument("--roi-qoffset", type=float, default=-0.125, help="ROI qoffset, usually negative")
    parser.add_argument("--native-roi-probe", action="store_true", help="Enable native ROI path probe switch in agent")
    return parser.parse_args()


if __name__ == "__main__":
    args = parse_args()
    if args.codec:
        cfg_path = Path(args.config).resolve()
        cfg = load_config(cfg_path)
        analysis = cfg.setdefault("analysis", {})
        codec = str(args.codec).strip().lower()
        analysis["controller_codecs"] = [codec, "h264"]
        cap = dict(analysis.get("capture_patch") or {})
        cap["codecPolicy"] = {"force": codec}
        if args.roi_enable:
            rect_vals = [float(x.strip()) for x in str(args.roi_rect).split(",")]
            if len(rect_vals) == 4:
                cap["qualityPolicy"] = cap.get("qualityPolicy") or {}
                cap["qualityPolicy"]["roi"] = {
                    "enable": True,
                    "rect": {"x": rect_vals[0], "y": rect_vals[1], "w": rect_vals[2], "h": rect_vals[3]},
                    "qoffset": float(args.roi_qoffset),
                }
        analysis["capture_patch"] = cap
        env_overrides = dict(analysis.get("agent_env") or {})
        env_overrides["AGENT_NVENC_NATIVE_ROI_ENABLE"] = "1" if args.native_roi_probe else "0"
        analysis["agent_env"] = env_overrides
        tmp_cfg = cfg_path.parent / f".tmp.codec-roi.{int(time.time())}.json"
        tmp_cfg.write_text(json.dumps(cfg, ensure_ascii=False, indent=2), encoding="utf-8")
        args.config = str(tmp_cfg)
    asyncio.run(main_async(args))



