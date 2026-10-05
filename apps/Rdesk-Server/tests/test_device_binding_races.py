"""Re-check frozen device authentication after the ownership row lock."""
import asyncio
import os
from datetime import UTC, datetime
from uuid import uuid4

import pytest
from fastapi import HTTPException, Request
from sqlalchemy import select, text
from sqlalchemy.ext.asyncio import async_sessionmaker, create_async_engine

from app.api.v1 import devices as routes
from app.core.security import create_device_access_token, get_current_device
from app.db.session import Base, get_db
from app.models.device import Device
from app.schemas.device import DeviceAutoBindRequest, DeviceBindRequest, DeviceUnbindRequest
from test_device_ownership import (
    EnrollmentSessionShim, _asyncpg_url, _configure_jwt, _device, _dual_headers, _user, device_api,
)

DATABASE_URL = os.getenv("MRD_TEST_DATABASE_URL")


@pytest.fixture
def anyio_backend(): return "asyncio"


@pytest.mark.parametrize("route", ["bind", "auto-bind", "unbind"])
@pytest.mark.parametrize("change", ["version", "revoked", "tenant"])
def test_binding_rechecks_frozen_auth_before_mutation(device_api, route, change):
    if route == "unbind":
        device_api.device.is_bound = True
        device_api.device.bound_user_id = device_api.owner.id
        device_api.device.tenant_id = device_api.owner.tenant_id
        device_api.session.commit()
    before_bound = device_api.device.is_bound
    before_owner = device_api.device.bound_user_id
    class ConcurrentChange(EnrollmentSessionShim):
        changed = False
        async def scalar(self, statement):
            if getattr(statement, "_for_update_arg", None) is not None and not self.changed:
                self.changed = True
                if change == "version": device_api.device.auth_version += 1
                elif change == "revoked": device_api.device.auth_revoked_at = datetime.now(UTC)
                else: device_api.device.tenant_id = "changed-tenant"
                self.session.commit()
            return await super().scalar(statement)
    async def db(): yield ConcurrentChange(device_api.session)
    device_api.app.dependency_overrides[get_db] = db
    result = device_api.client.post(f"/api/v1/devices/{route}",
        json={"device_id": device_api.device.device_id},
        headers=_dual_headers(device_api.user_token, device_api.device_token))
    assert result.status_code == 401, "a refreshed ORM object must not replace the original auth decision"
    device_api.session.refresh(device_api.device)
    assert device_api.device.is_bound == before_bound
    assert device_api.device.bound_user_id == before_owner


@pytest.mark.skipif(not DATABASE_URL, reason="isolated MRD_TEST_DATABASE_URL not configured")
@pytest.mark.anyio
@pytest.mark.parametrize("route", ["bind", "auto-bind", "unbind"])
@pytest.mark.parametrize("change", ["version", "revoked", "tenant", "code", "row_id"])
async def test_postgres_locked_binding_rejects_concurrent_auth_change(monkeypatch, route, change):
    _configure_jwt(monkeypatch)
    schema = "binding_auth_" + uuid4().hex
    admin = create_async_engine(_asyncpg_url(DATABASE_URL))
    async with admin.begin() as connection:
        await connection.execute(text(f'CREATE SCHEMA "{schema}"'))
    engine = create_async_engine(_asyncpg_url(DATABASE_URL),
        connect_args={"server_settings": {"search_path": schema}})
    sessions = async_sessionmaker(engine, expire_on_commit=False)
    try:
        async with engine.begin() as connection:
            await connection.run_sync(Base.metadata.create_all)
        async with sessions.begin() as setup:
            user = _user("binding-owner", "tenant-a")
            device = _device(owner=user if route == "unbind" else None)
            setup.add(user)
            await setup.flush()
            setup.add(device)
            await setup.flush()
            token = create_device_access_token(device)
            row_id, code = device.id, device.device_id
        async with sessions() as waiting, sessions() as changing:
            authenticated = await get_current_device(Request({"type": "http", "headers": [
                (b"x-rdesk-device-authorization", f"Bearer {token}".encode())]}), db=waiting)
            owner = await waiting.scalar(select(type(user)).where(type(user).id == user.id))
            pid = await waiting.scalar(text("SELECT pg_backend_pid()"))
            locked = await changing.scalar(select(Device).where(Device.id == row_id).with_for_update())
            if change == "version": locked.auth_version += 1
            elif change == "revoked": locked.auth_revoked_at = datetime.now(UTC)
            elif change == "tenant": locked.tenant_id = "changed-tenant"
            elif change == "code": locked.device_id = "changed-code"
            else: locked.id = "changed-row"
            await changing.flush()
            function, model = {
                "bind": (routes.bind_device, DeviceBindRequest),
                "auto-bind": (routes.auto_bind_device, DeviceAutoBindRequest),
                "unbind": (routes.unbind_device, DeviceUnbindRequest),
            }[route]
            task = asyncio.create_task(function(model(device_id=code), owner, authenticated, waiting))
            try:
                async with admin.connect() as observer:
                    blocked = False
                    for _ in range(300):
                        blocked = await observer.scalar(text(
                            "SELECT wait_event_type = 'Lock' FROM pg_stat_activity WHERE pid = :pid"), {"pid": pid})
                        if blocked: break
                        await asyncio.sleep(.01)
                    assert blocked, "the tested request must wait on the device row lock"
                await changing.commit()
                with pytest.raises(HTTPException) as rejected:
                    await asyncio.wait_for(task, 5)
                assert rejected.value.status_code == 401
            finally:
                if not task.done():
                    task.cancel()
                    await asyncio.gather(task, return_exceptions=True)
                await waiting.rollback()
                await changing.rollback()
        async with sessions() as checking:
            saved = await checking.scalar(select(Device).where(Device.id == ("changed-row" if change == "row_id" else row_id)))
            assert saved.is_bound == (route == "unbind")
            assert saved.bound_user_id == (user.id if route == "unbind" else None)
    finally:
        await engine.dispose()
        async with admin.begin() as connection:
            await connection.execute(text(f'DROP SCHEMA "{schema}" CASCADE'))
        await admin.dispose()
