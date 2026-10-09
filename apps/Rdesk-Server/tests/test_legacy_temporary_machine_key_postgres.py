"""Real two-connection PostgreSQL races; uses only MRD_TEST_DATABASE_URL.

The shared test helper creates and drops one UUID-named isolated schema. These
cases are skipped when no explicit test database is configured; SQLite results
are not evidence for PostgreSQL advisory or row locking.
"""
import asyncio
from datetime import UTC, datetime

import pytest
from sqlalchemy import func, select, text
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey

from app.core.config import settings
from app.core.security import capture_device_auth_snapshot
from app.models.browser_controller import BrowserController
from app.models.device import Device
from app.models.device_machine_identity import DeviceMachineIdentity, DeviceSelfEnrollmentChallenge
from app.models.device_temporary_access import DeviceTemporaryAccess
from app.models.session_request import SessionRequest
from app.models.user import User
from app.schemas.browser_session import BrowserSessionCreateIn
from app.schemas.guest_browser import TemporaryAccessPublishIn
from app.schemas.device_self_enrollment import DeviceSelfEnrollmentChallengeRequest, DeviceSelfRegisterRequest
from app.services.browser_sessions import BrowserSessionService
from app.services.device_enrollment import DeviceEnrollmentError, device_serial_digest
from app.services.device_self_enrollment import DeviceSelfEnrollmentService
from app.services.device_sessions import DeviceSessionError
from app.services.temporary_access import TemporaryAccessService
from app.services.device_principal_keys import principal_key_lock
from test_device_self_enrollment import DATABASE_URL, API_URL, SERIAL_PEPPER, _postgres_sessions, _key, _signed
from test_legacy_temporary_machine_key import _payload, legacy_temporary_config, KEY, KEY_ID, PUBLIC

pytestmark = [pytest.mark.asyncio, pytest.mark.skipif(not DATABASE_URL, reason="MRD_TEST_DATABASE_URL is not configured")]


async def _seed(sessions):
    async with sessions.begin() as db:
        owner = User(id="legacy-owner", username="legacy-owner", email="legacy-owner@example.test", password_hash="unused", role="user", tenant_id="tenant-a")
        db.add(owner)
        await db.flush()
        for index in (1, 2):
            db.add(Device(id="legacy-row-" + str(index), name="Legacy host", device_id="150151577" + str(index), os="Linux", principal_kind="physical", is_bound=True, bound_user_id=owner.id, tenant_id=owner.tenant_id, motherboard_serial_digest=device_serial_digest("legacy-serial-" + str(index), SERIAL_PEPPER), auth_version=7, active_refresh_jti_hash="41" * 32))
        await db.flush()


def _temporary(db):
    return TemporaryAccessService(db, api_url=settings.public_api_url, pepper=bytes.fromhex("a1" * 32))


async def _publish(db, index=1, key=KEY, payload=None):
    device = await db.get(Device, "legacy-row-" + str(index))
    status = await _temporary(db).publish(snapshot=capture_device_auth_snapshot(device), payload=TemporaryAccessPublishIn(**(payload if payload is not None else _payload(device, key=key))))
    assert status.ready


async def _parallel(sessions, operations, *, first_wins_key=None):
    barrier = asyncio.Barrier(2)
    backend_pids = set()
    async def run(index, operation):
        try:
            async with sessions.begin() as db:
                backend_pids.add(await db.scalar(text("SELECT pg_backend_pid()")))
                if index == 0 and first_wins_key is not None:
                    # Hold the actual transaction-scoped principal lock before
                    # both connections cross the start barrier. This forces the
                    # selected winner while the contender uses production locks.
                    async with principal_key_lock(db, first_wins_key):
                        await barrier.wait()
                        await operation(db)
                else:
                    await barrier.wait()
                    await operation(db)
            return "published"
        except (DeviceSessionError, DeviceEnrollmentError) as denied:
            return denied.code
    results = await asyncio.wait_for(asyncio.gather(*(run(index, operation) for index, operation in enumerate(operations))), timeout=15)
    assert len(backend_pids) == 2
    return results


@pytest.mark.parametrize("scenario", ["same_device_different_keys", "different_devices_same_key", "exact_retry"])
async def test_postgres_first_pin_races_have_one_durable_identity(scenario):
    async with _postgres_sessions() as (sessions, _, _):
        await _seed(sessions)
        other = Ed25519PrivateKey.from_private_bytes(bytes([92]) * 32)
        async with sessions() as db:
            shared_retry = _payload(await db.get(Device, "legacy-row-1")) if scenario == "exact_retry" else None
        async def first(db):
            await _publish(db, payload=shared_retry)
        async def second(db):
            await _publish(db, index=2 if scenario == "different_devices_same_key" else 1, key=other if scenario == "same_device_different_keys" else KEY, payload=shared_retry)
        results = await _parallel(sessions, (first, second))
        assert results.count("published") == (2 if scenario == "exact_retry" else 1)
        assert results.count("guest_access_invalid") == (0 if scenario == "exact_retry" else 1)
        async with sessions() as db:
            mappings = list(await db.scalars(select(DeviceMachineIdentity)))
            publications = list(await db.scalars(select(DeviceTemporaryAccess)))
            assert len(mappings) == len(publications) == 1
            assert mappings[0].device_row_id == publications[0].device_row_id
            assert mappings[0].key_id == publications[0].key_id
            assert publications[0].generation == 1
            assert await db.scalar(select(func.count()).select_from(Device)) == 2
            for device in await db.scalars(select(Device)):
                assert device.is_bound and device.bound_user_id == "legacy-owner"
                assert device.tenant_id == "tenant-a" and device.auth_version == 7
                assert device.active_refresh_jti_hash == "41" * 32
                assert device.device_id in {"1501515771", "1501515772"}


@pytest.mark.parametrize("first_kind", ["temporary", "browser"])
async def test_postgres_browser_registration_and_first_pin_cannot_share_key(first_kind):
    async with _postgres_sessions() as (sessions, _, _):
        await _seed(sessions)
        async def publish(db):
            await _publish(db)
        async def browser(db):
            owner = await db.get(User, "legacy-owner")
            await BrowserSessionService(db).create(user=owner, user_version=owner.session_version, payload=BrowserSessionCreateIn(session_id="legacy-key-browser-race", idempotency_key=[8] * 16, controller_public_key=list(PUBLIC), target_device_id="1501515771", requested_scopes=["screen.view"], route_policy="direct_first"))
        operations = (publish, browser) if first_kind == "temporary" else (browser, publish)
        results = await _parallel(sessions, operations, first_wins_key=KEY_ID)
        assert results[0] == "published"
        assert results.count("published") == 1
        assert any(result in {"guest_access_invalid", "browser_session_conflict"} for result in results)
        async with sessions() as db:
            mappings = list(await db.scalars(select(DeviceMachineIdentity)))
            controllers = list(await db.scalars(select(BrowserController)))
            publications = list(await db.scalars(select(DeviceTemporaryAccess)))
            assert len(mappings) + len(controllers) == 1
            assert len(publications) == len(mappings)
            if mappings:
                assert mappings[0].key_id == publications[0].key_id == KEY_ID
                assert await db.scalar(select(func.count()).select_from(SessionRequest)) == 0
                assert await db.scalar(select(func.count()).select_from(Device)) == 2
            else:
                assert controllers[0].key_id == KEY_ID
                assert await db.scalar(select(func.count()).select_from(SessionRequest)) == 1
                assert await db.scalar(select(func.count()).select_from(Device)) == 3


@pytest.mark.parametrize("first_kind", ["temporary", "self_registration"])
async def test_postgres_self_registration_and_first_pin_cannot_claim_two_devices(first_kind):
    async with _postgres_sessions() as (sessions, _, _):
        await _seed(sessions)
        private, identity = _key(91)
        async with sessions.begin() as db:
            challenge = await DeviceSelfEnrollmentService(db, api_url=API_URL, serial_pepper=SERIAL_PEPPER).issue(DeviceSelfEnrollmentChallengeRequest(**identity), peer_ip="192.0.2.10")
            registration = DeviceSelfRegisterRequest.model_validate(_signed(private, identity, challenge.model_dump()))
        async def publish(db):
            await _publish(db)
        async def self_register(db):
            await DeviceSelfEnrollmentService(db, api_url=API_URL, serial_pepper=SERIAL_PEPPER).register(registration)
        operations = (publish, self_register) if first_kind == "temporary" else (self_register, publish)
        results = await _parallel(sessions, operations, first_wins_key=KEY_ID)
        assert results[0] == "published"
        assert results.count("published") == 1
        assert any(result in {"guest_access_invalid", "device_self_enrollment_conflict"} for result in results)
        async with sessions() as db:
            mapping = (await db.scalars(select(DeviceMachineIdentity))).one()
            assert mapping.key_id == KEY_ID
            publications = list(await db.scalars(select(DeviceTemporaryAccess)))
            challenge_row = await db.get(DeviceSelfEnrollmentChallenge, challenge.challenge_id)
            if publications:
                assert len(publications) == 1
                assert publications[0].device_row_id == mapping.device_row_id == "legacy-row-1"
                assert challenge_row.consumed_at is None
                assert await db.scalar(select(func.count()).select_from(Device)) == 2
            else:
                assert mapping.device_row_id not in {"legacy-row-1", "legacy-row-2"}
                assert challenge_row.consumed_at is not None
                assert await db.scalar(select(func.count()).select_from(Device)) == 3
            for index in (1, 2):
                device = await db.get(Device, "legacy-row-" + str(index))
                assert device.auth_version == 7 and device.bound_user_id == "legacy-owner"
                assert device.active_refresh_jti_hash == "41" * 32
