"""Serialize auto approval and device unbinding on real PostgreSQL rows."""

import asyncio
from datetime import UTC, datetime
import hashlib
import os
from uuid import uuid4

import pytest
from sqlalchemy import text
from sqlalchemy.ext.asyncio import async_sessionmaker, create_async_engine

import app.models  # noqa: F401
from app.api.v1.devices import unbind_device
from app.core.config import settings
from app.core.security import capture_device_auth_snapshot
from app.db.session import Base
from app.models.device import Device
from app.models.device_machine_identity import DeviceMachineIdentity
from app.models.oidc import OidcIdentity
from app.models.session_request import SessionRequest
from app.models.user import User
from app.schemas.device import DeviceUnbindRequest
from app.schemas.session import DeviceSessionApprovalIn, DeviceSessionCreateIn
from app.services.device_sessions import DeviceSessionService
from app.services.session_grants import SessionGrantPolicy


DATABASE_URL = os.getenv("MRD_TEST_DATABASE_URL")
ISSUER = "https://r-auth.example.test/realms/rdesk"


@pytest.mark.asyncio
@pytest.mark.skipif(not DATABASE_URL, reason="requires PostgreSQL concurrency fixture")
async def test_unbind_waits_for_inflight_approval_then_revokes_before_rebind(monkeypatch):
    monkeypatch.setitem(settings.__dict__, "oidc_enabled", True)
    monkeypatch.setitem(settings.__dict__, "oidc_issuer", ISSUER)
    engine = create_async_engine(DATABASE_URL)
    schema = "mrd_same_" + uuid4().hex[:16]
    entered, finish = asyncio.Event(), asyncio.Event()
    try:
        async with engine.begin() as connection:
            await connection.execute(text(f'CREATE SCHEMA "{schema}"'))
            await connection.run_sync(lambda sync: Base.metadata.create_all(
                sync.execution_options(schema_translate_map={None: schema})))
        scoped = engine.execution_options(schema_translate_map={None: schema})
        sessions = async_sessionmaker(scoped, expire_on_commit=False)
        public_keys = [bytes(range(32)), bytes(range(32, 64))]
        key_ids = [hashlib.sha256(public).hexdigest() for public in public_keys]
        async with sessions.begin() as db:
            owner = User(id="owner", username="owner", email="owner@example.test",
                         password_hash="unused", role="user", tenant_id="tenant-a")
            db.add(owner)
            await db.flush()
            controller = Device(id="controller", device_id="controller-1", name="controller",
                os="Linux", tenant_id="tenant-a", is_bound=True, bound_user_id=owner.id)
            target = Device(id="target", device_id="target-1", name="target",
                os="Linux", tenant_id="tenant-a", is_bound=True, bound_user_id=owner.id)
            db.add_all([controller, target])
            await db.flush()
            db.add(OidcIdentity(issuer=ISSUER, subject="subject", user_id=owner.id,
                                created_at=datetime.now(UTC)))
            for device, public, key_id in zip((controller, target), public_keys, key_ids):
                db.add(DeviceMachineIdentity(key_id=key_id, public_key=public.hex(),
                                             device_row_id=device.id, created_at=datetime.now(UTC)))
            await db.flush()
            await DeviceSessionService(db).create(current_device=controller,
                payload=DeviceSessionCreateIn.model_validate({
                    "session_id": "same-race", "idempotency_key": [7] * 16,
                    "target_device_id": "target-1", "access_mode": "attended",
                    "requested_scopes": ["screen.view"], "requested_profile": None,
                    "route_policy": "relay_only",
                }))

        class PausedRelay:
            current_policy = SessionGrantPolicy(
                grant_ttl_seconds=600, policy_ttl_seconds=300, revision=1,
                allowed_regions=("test-region",), preferred_regions=("test-region",),
                accepted_transports=("udp",), max_backups=0,
            )

            async def create_wan_generation_locked(self, *, grant, target_device, generation):
                entered.set()
                await asyncio.wait_for(finish.wait(), timeout=5)
                grant.active_relay_generation = generation
                grant.status = "approved"

        async def approve():
            async with sessions.begin() as db:
                target = await db.get(Device, "target")
                return await DeviceSessionService(db).approve(
                    session_id="same-race", current_device=target,
                    auth_snapshot=capture_device_auth_snapshot(target),
                    payload=DeviceSessionApprovalIn(approved_scopes=["screen.view"], approved_profile=None),
                    relay_access=PausedRelay(), same_account_only=True,
                    controller_key_id=key_ids[0], target_key_id=key_ids[1],
                )

        async def unbind():
            async with sessions() as db:
                target = await db.get(Device, "target")
                owner = await db.get(User, "owner")
                return await unbind_device(payload=DeviceUnbindRequest(device_id="target-1"),
                                           current_user=owner, current_device=target, db=db)

        approval_task = asyncio.create_task(approve())
        await asyncio.wait_for(entered.wait(), timeout=5)
        unbind_task = asyncio.create_task(unbind())
        await asyncio.sleep(0.1)
        assert not unbind_task.done(), "unbind must wait for the approval's device lock"
        finish.set()
        await asyncio.wait_for(approval_task, timeout=5)
        assert (await asyncio.wait_for(unbind_task, timeout=5))["success"]
        async with sessions.begin() as db:
            row = await db.get(SessionRequest, "same-race")
            target = await db.get(Device, "target")
            assert row.status == "revoked" and not target.is_bound
            target.is_bound, target.bound_user_id = True, "owner"
        async with sessions() as db:
            assert (await db.get(SessionRequest, "same-race")).status == "revoked"
    finally:
        finish.set()
        async with engine.begin() as connection:
            await connection.execute(text(f'DROP SCHEMA IF EXISTS "{schema}" CASCADE'))
        await engine.dispose()
