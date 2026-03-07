from contextlib import asynccontextmanager
from pathlib import Path
from unittest import TestCase

from fastapi import FastAPI
from fastapi.testclient import TestClient

from app.api.v1.realtime import router
from app.services.realtime_manager import RealtimeSidecarManager


class FakeProcess:
    def __init__(self, pid: int = 4242) -> None:
        self.pid = pid
        self.alive = True

    def poll(self):
        return None if self.alive else 0

    def terminate(self):
        self.alive = False

    def wait(self, timeout=None):
        self.alive = False
        return 0

    def kill(self):
        self.alive = False


def build_manager() -> RealtimeSidecarManager:
    def spawn(_command: list[str], _cwd: Path):
        return FakeProcess()

    manager = RealtimeSidecarManager(
        health_url="http://127.0.0.1:9532/health",
        command=["cargo", "run"],
        workdir=".",
        spawner=spawn,
    )

    manager.status = lambda: type(  # type: ignore[method-assign]
        "Status",
        (),
        {"running": bool(manager._process and manager._process.poll() is None), "reachable": True, "status": "ok", "pid": 4242 if manager._process else None},
    )()
    return manager


def build_test_app() -> FastAPI:
    @asynccontextmanager
    async def lifespan(app: FastAPI):
        app.state.realtime_manager = build_manager()
        yield

    app = FastAPI(lifespan=lifespan)
    app.include_router(router, prefix="/api/v1")
    return app


class RealtimeApiTests(TestCase):
    def test_realtime_start_stop_restart_roundtrip(self):
        client = TestClient(build_test_app())

        with client:
            start = client.post("/api/v1/realtime/start")
            self.assertEqual(start.status_code, 200)
            self.assertTrue(start.json()["running"])

            status = client.get("/api/v1/realtime/status")
            self.assertEqual(status.status_code, 200)
            self.assertTrue(status.json()["reachable"])

            restart = client.post("/api/v1/realtime/restart")
            self.assertEqual(restart.status_code, 200)
            self.assertTrue(restart.json()["running"])

            stop = client.post("/api/v1/realtime/stop")
            self.assertEqual(stop.status_code, 200)
            self.assertFalse(stop.json()["running"])
