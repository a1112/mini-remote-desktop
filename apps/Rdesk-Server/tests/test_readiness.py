import asyncio
from contextlib import asynccontextmanager
from types import SimpleNamespace
from unittest.mock import AsyncMock

import pytest
from fastapi.testclient import TestClient
from sqlalchemy.exc import OperationalError

from app import main


@pytest.mark.parametrize(("db_online", "realtime_online"), (
    (True, True),
    (False, True),
    (True, False),
    (False, False),
))
def test_readiness_checks_both_dependencies_and_keeps_liveness(db_online, realtime_online, monkeypatch):
    execute = AsyncMock()
    if not db_online:
        execute.side_effect = OperationalError("SELECT 1", None, OSError("private database detail"))
    connection = SimpleNamespace(execute=execute)

    @asynccontextmanager
    async def connect():
        yield connection

    monkeypatch.setattr(main, "engine", SimpleNamespace(connect=connect))
    monkeypatch.setattr(main.app.state, "realtime_manager", SimpleNamespace(
        probe_health=lambda: "ok" if realtime_online else "unexpected-service",
    ))
    client = TestClient(main.app)
    response = client.get("/readyz")
    ready = db_online and realtime_online
    assert str(execute.await_args.args[0]) == "SELECT 1"
    assert response.status_code == (200 if ready else 503)
    assert response.json() == {
        "status": "ok" if ready else "unavailable",
        "database": db_online,
        "realtime": realtime_online,
    }
    assert response.headers["cache-control"] == "no-store"
    assert client.get("/healthz").status_code == 200


@pytest.mark.asyncio
async def test_database_readiness_does_not_wait_for_the_pool_indefinitely(monkeypatch):
    @asynccontextmanager
    async def connect():
        await asyncio.sleep(10)
        yield

    monkeypatch.setattr(main, "engine", SimpleNamespace(connect=connect))
    assert not await asyncio.wait_for(main._database_is_ready(), 2.5)
