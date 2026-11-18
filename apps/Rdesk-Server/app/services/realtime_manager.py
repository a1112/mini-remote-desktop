from __future__ import annotations

from dataclasses import dataclass
from http.client import HTTPException
from pathlib import Path
from typing import Callable, Protocol
from urllib.request import urlopen
import json
import subprocess
import threading


EXPECTED_SERVICE = "realtime-server"
# Registration/migration uses v2; initial session negotiation uses v3.
REQUIRED_PROTOCOL_VERSIONS = frozenset({2, 3})


def _supports_required_protocols(payload: dict[str, object]) -> bool:
    primary = payload.get("protocol_version")
    supported = payload.get("supported_protocol_versions")
    return (
        type(primary) is int
        and isinstance(supported, list)
        and all(type(version) is int for version in supported)
        and primary in supported
        and REQUIRED_PROTOCOL_VERSIONS.issubset(supported)
    )


class ProcessHandle(Protocol):
    def poll(self) -> int | None: ...

    def terminate(self) -> None: ...

    def wait(self, timeout: float | None = None) -> int: ...

    def kill(self) -> None: ...


SpawnProcess = Callable[[list[str], Path], ProcessHandle]


@dataclass
class RealtimeStatus:
    running: bool
    reachable: bool
    status: str
    pid: int | None = None


def _spawn_process(command: list[str], cwd: Path) -> ProcessHandle:
    return subprocess.Popen(command, cwd=str(cwd))


class RealtimeSidecarManager:
    def __init__(
        self,
        *,
        health_url: str,
        command: list[str],
        workdir: str,
        spawner: SpawnProcess | None = None,
    ) -> None:
        self.health_url = health_url
        self.command = command
        self.workdir = Path(workdir)
        self.spawner = spawner or _spawn_process
        self._process: ProcessHandle | None = None
        self._lifecycle_lock = threading.RLock()

    def status(self) -> RealtimeStatus:
        with self._lifecycle_lock:
            process = self._process
            running = bool(process and process.poll() is None)
            pid = getattr(process, "pid", None) if running else None
        health_status = self.probe_health()
        return RealtimeStatus(running, health_status == "ok", health_status, pid)

    def probe_health(self) -> str:
        # Readiness also calls this without holding the lifecycle lock; a slow
        # stop/restart must not prevent an independent health probe.
        try:
            with urlopen(self.health_url, timeout=1.5) as response:
                limit = 16 * 1024
                body = response.read(limit + 1)
                if len(body) > limit:
                    return "unexpected-service"
                payload = json.loads(body)
        except (OSError, HTTPException, ValueError):
            return "unreachable"
        if (
            isinstance(payload, dict)
            and payload.get("status") == "ok"
            and payload.get("service") == EXPECTED_SERVICE
            and _supports_required_protocols(payload)
        ):
            return "ok"
        return "unexpected-service"

    def start(self) -> RealtimeStatus:
        with self._lifecycle_lock:
            if self._process and self._process.poll() is None:
                return self.status()
            if not self.workdir.is_dir():
                raise RuntimeError(
                    f"realtime-server workdir does not exist: {self.workdir}"
                )

            self._process = self.spawner(self.command, self.workdir)
            return self.status()

    def stop(self) -> RealtimeStatus:
        with self._lifecycle_lock:
            if not self._process or self._process.poll() is not None:
                self._process = None
                return self.status()

            self._process.terminate()
            try:
                self._process.wait(timeout=3)
            except subprocess.TimeoutExpired:
                self._process.kill()
                self._process.wait(timeout=3)

            self._process = None
            return self.status()

    def restart(self) -> RealtimeStatus:
        with self._lifecycle_lock:
            self.stop()
            return self.start()
