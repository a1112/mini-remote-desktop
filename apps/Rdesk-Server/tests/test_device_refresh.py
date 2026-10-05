"""Long-lived device credentials are accepted only by the dedicated renewal route."""

import asyncio
import os
from datetime import UTC, datetime, timedelta
from uuid import uuid4

import jwt
import pytest
from fastapi import HTTPException, Request
from sqlalchemy import select, text
from sqlalchemy.ext.asyncio import async_sessionmaker, create_async_engine

from app.api.v1 import devices as device_routes
from app.core.config import settings
from app.core.security import (
    create_device_access_token, create_device_refresh_token,
    get_current_device, get_device_refresh_identity,
)
from app.db.session import Base
from app.models.device import Device
from app.schemas.device import DeviceRegisterRequest, DeviceRegisterResponse
from test_device_ownership import (
    EnrollmentSessionShim, JWT_SECRET, _asyncpg_url, _configure_jwt, _device, _dual_headers,
    _issue_device_enrollment, _register_payload, device_api,
)


REFRESH_PATH = "/api/v1/devices/refresh"
HEADER = "X-Rdesk-Device-Refresh-Authorization"
DATABASE_URL = os.getenv("MRD_TEST_DATABASE_URL")


def _issued_refresh(api):
    result = api.client.post("/api/v1/devices/register", json=_register_payload("serial-a"),
        headers={"X-Rdesk-Device-Authorization": f"Bearer {api.device_token}"})
    assert result.status_code == 200, result.text
    assert result.json().get("refresh_token"), "authorized registration must issue a renewal credential"
    return result.json()["refresh_token"]


def _renew(api, token, serial="serial-a", *, headers=None):
    return api.client.post(REFRESH_PATH, json=_register_payload(serial),
        headers=headers or {HEADER: f"Bearer {token}"})


def test_expired_access_renews_same_device_and_rotates_long_lived_credential(device_api):
    refresh = _issued_refresh(device_api)
    claims = jwt.decode(refresh, options={"verify_signature": False})
    assert claims["aud"] == "rdesk-device-refresh"
    assert claims["token_type"] == claims["role"] == "device_refresh"
    assert claims["sub"] == device_api.device.id
    assert claims["tenant_id"] == device_api.device.tenant_id
    assert claims["serial_digest"] == device_api.device.motherboard_serial_digest
    assert claims["exp"] - claims["iat"] == 365 * 24 * 60 * 60
    access = jwt.decode(device_api.device_token, options={"verify_signature": False})
    now = int(datetime.now(UTC).timestamp())
    access.update(iat=now - 1801, exp=now - 1)
    expired = jwt.encode(access, JWT_SECRET, algorithm="HS256")
    rejected = device_api.client.post("/api/v1/devices/register", json=_register_payload("serial-a"),
        headers={"X-Rdesk-Device-Authorization": f"Bearer {expired}"})
    assert rejected.status_code == 401
    renewed = _renew(device_api, refresh)
    assert renewed.status_code == 200, renewed.text
    assert renewed.json()["device_id"] == device_api.device.device_id
    assert renewed.json()["refresh_token"] != refresh
    assert "no-store" in renewed.headers["cache-control"]
    valid_access = renewed.json()["access_token"]
    assert device_api.client.post("/api/v1/devices/register", json=_register_payload("serial-a"),
        headers={"X-Rdesk-Device-Authorization": f"Bearer {valid_access}"}).status_code == 200
    # These are revocable long-lived credentials, not single-use refresh tokens.
    assert _renew(device_api, refresh).status_code == 200


@pytest.mark.parametrize("mutation", [
    "kind", "role", "audience", "audience_list", "issuer", "expired", "future_iat",
    "long_lived", "device", "tenant", "version", "bool_version", "serial_digest",
    "missing_serial", "missing_jti", "foreign_row", "bad_signature",
])
def test_refresh_rejects_invalid_or_foreign_claims(device_api, mutation):
    token = _issued_refresh(device_api)
    claims = jwt.decode(token, options={"verify_signature": False})
    now = int(datetime.now(UTC).timestamp())
    changes = {
        "kind": {"token_type": "device"}, "role": {"role": "device"},
        "audience": {"aud": "rdesk-device"}, "audience_list": {"aud": [claims["aud"], "rdesk-device"]},
        "issuer": {"iss": "https://foreign.invalid"},
        "expired": {"iat": now - 1801, "exp": now - 1},
        "future_iat": {"iat": now + 301, "exp": now + 3600},
        "long_lived": {"exp": claims["iat"] + 366 * 86400},
        "device": {"device_id": "foreign-device"}, "tenant": {"tenant_id": "foreign-tenant"},
        "version": {"auth_version": 999}, "bool_version": {"auth_version": True},
        "serial_digest": {"serial_digest": "0" * 64}, "foreign_row": {"sub": "foreign-row"},
    }
    claims.update(changes.get(mutation, {}))
    if mutation == "missing_serial": del claims["serial_digest"]
    if mutation == "missing_jti": del claims["jti"]
    token = jwt.encode(claims, "foreign-secret-with-at-least-32-bytes" if mutation == "bad_signature" else JWT_SECRET,
        algorithm="HS256")
    rejected = _renew(device_api, token)
    assert rejected.status_code == 401, rejected.text
    assert token not in rejected.text and "serial-a" not in rejected.text


def test_refresh_requires_the_same_machine_and_current_revocation_version(device_api):
    token = _issued_refresh(device_api)
    assert _renew(device_api, token, "different-machine").status_code == 401
    device_api.device.auth_revoked_at = datetime.now(UTC)
    device_api.session.commit()
    assert _renew(device_api, token).status_code == 401
    device_api.device.auth_revoked_at = None
    device_api.device.auth_version += 1
    device_api.session.commit()
    assert _renew(device_api, token).status_code == 401


def test_access_and_refresh_credentials_cannot_cross_endpoint_boundaries(device_api):
    refresh = _issued_refresh(device_api)
    assert _renew(device_api, device_api.device_token).status_code == 401
    for headers in [
        {"X-Rdesk-Device-Authorization": f"Bearer {refresh}"},
        {"Authorization": f"Bearer {refresh}"},
    ]:
        assert device_api.client.post("/api/v1/devices/register", json=_register_payload("serial-a"),
            headers=headers).status_code == 401
    assert device_api.client.post("/api/v1/devices/enrollment-tokens",
        headers={"Authorization": f"Bearer {refresh}"}).status_code in (401, 403)
    assert _renew(device_api, refresh, headers=[(HEADER, f"Bearer {refresh}"), (HEADER, f"Bearer {refresh}")]).status_code == 401


@pytest.mark.parametrize("configured_days", [0, 366, -1, True])
def test_refresh_lifetime_configuration_fails_closed(device_api, monkeypatch, configured_days):
    refresh = _issued_refresh(device_api)
    monkeypatch.setitem(settings.__dict__, "device_refresh_jwt_expire_days", configured_days)
    assert _renew(device_api, refresh).status_code == 401


def test_recover_and_owner_rotation_issue_version_bound_refresh(device_api):
    original = _issued_refresh(device_api)
    result = device_api.client.post(f"/api/v1/devices/{device_api.device.device_id}/credentials/admin-rotate",
        headers={"Authorization": f"Bearer {device_api.admin_token}"})
    assert result.status_code == 200, result.text
    new_refresh = result.json().get("refresh_token")
    assert new_refresh, "controlled same-code recovery must restore long-lived credentials"
    assert _renew(device_api, original).status_code == 401
    assert _renew(device_api, new_refresh).status_code == 200


@pytest.mark.parametrize("audience", ["rdesk-api", "rdesk-device", "rdesk-signaling", ""])
def test_refresh_audience_must_be_independent(device_api, monkeypatch, audience):
    refresh = _issued_refresh(device_api)
    monkeypatch.setitem(settings.__dict__, "device_refresh_jwt_audience", audience)
    assert _renew(device_api, refresh).status_code == 401


def test_first_enrollment_and_owner_rotation_issue_machine_bound_refresh(device_api):
    enrolled = device_api.client.post("/api/v1/devices/register", json=_register_payload("brand-new-serial"),
        headers={"X-Rdesk-Device-Enrollment": _issue_device_enrollment(device_api)})
    assert enrolled.status_code == 200, enrolled.text
    token = enrolled.json()["refresh_token"]
    assert _renew(device_api, token, "brand-new-serial").status_code == 200
    model = DeviceRegisterResponse.model_validate(enrolled.json())
    assert token not in repr(model) and model.access_token not in repr(model)
    device_api.device.is_bound = True
    device_api.device.bound_user_id = device_api.owner.id
    device_api.device.tenant_id = device_api.owner.tenant_id
    device_api.session.commit()
    original = _issued_refresh(device_api)
    rotated = device_api.client.post(f"/api/v1/devices/{device_api.device.device_id}/credentials/rotate",
        headers=_dual_headers(device_api.user_token, device_api.device_token))
    assert rotated.status_code == 200, rotated.text
    assert _renew(device_api, original).status_code == 401
    assert _renew(device_api, rotated.json()["refresh_token"]).status_code == 200


def test_tenant_change_requires_current_authorized_credentials(device_api):
    refresh = _issued_refresh(device_api)
    device_api.device.tenant_id = "changed-tenant"
    device_api.session.commit()
    assert _renew(device_api, refresh).status_code == 401
    upgraded = _issued_refresh(device_api)
    assert _renew(device_api, upgraded).status_code == 200


@pytest.mark.parametrize("revoked", [False, True])
def test_access_upgrade_rechecks_version_after_authentication(device_api, revoked):
    class ConcurrentRotation(EnrollmentSessionShim):
        async def scalar(self, statement):
            if getattr(statement, "_for_update_arg", None) is not None:
                # The auth dependency has already accepted version 1. The
                # locked read returns the admin's committed version 2 instead.
                device_api.device.auth_version += 1
                if revoked:
                    device_api.device.auth_revoked_at = datetime.now(UTC)
                self.session.commit()
            return await super().scalar(statement)

    async def concurrent_db():
        yield ConcurrentRotation(device_api.session)

    from app.db.session import get_db
    device_api.app.dependency_overrides[get_db] = concurrent_db
    result = device_api.client.post("/api/v1/devices/register", json=_register_payload("serial-a"),
        headers={"X-Rdesk-Device-Authorization": f"Bearer {device_api.device_token}"})
    assert result.status_code == 401, "an old access credential cannot acquire the rotated identity"


@pytest.mark.skipif(not DATABASE_URL, reason="MRD_TEST_DATABASE_URL is not configured")
@pytest.mark.anyio
@pytest.mark.parametrize("change", ["revocation", "tenant", "expiry", "access_rotation", "access_revocation"])
async def test_locked_refresh_rechecks_state_after_concurrent_change(monkeypatch, change):
    _configure_jwt(monkeypatch)
    schema = "device_refresh_" + uuid4().hex
    admin_engine = create_async_engine(_asyncpg_url(DATABASE_URL))
    async with admin_engine.begin() as connection:
        await connection.execute(text(f'CREATE SCHEMA "{schema}"'))
    engine = create_async_engine(_asyncpg_url(DATABASE_URL),
        connect_args={"server_settings": {"search_path": schema}})
    sessions = async_sessionmaker(engine, expire_on_commit=False)
    try:
        async with engine.begin() as connection:
            await connection.run_sync(Base.metadata.create_all)
        async with sessions.begin() as setup:
            device = _device()
            setup.add(device)
            await setup.flush()
            token = create_device_refresh_token(device)
            access_token = create_device_access_token(device)
        identity = await get_device_refresh_identity(Request({"type": "http", "headers": [
            (HEADER.lower().encode(), f"Bearer {token}".encode())]}))
        if change == "expiry":
            class ExpiredClock:
                @classmethod
                def now(cls, zone):
                    return datetime.now(zone) + timedelta(days=366)
            monkeypatch.setattr(device_routes, "datetime", ExpiredClock)
        async with sessions() as waiting, sessions() as revoking:
            # Preload the old ORM row as well: populate_existing must replace it.
            await waiting.scalar(select(Device).where(Device.id == identity.row_id))
            current_device = await get_current_device(Request({"type": "http", "headers": [
                (b"x-rdesk-device-authorization", f"Bearer {access_token}".encode())]}), db=waiting)
            pid = await waiting.scalar(text("SELECT pg_backend_pid()"))
            locked = await revoking.scalar(select(Device).where(Device.id == identity.row_id).with_for_update())
            if change in ("revocation", "access_rotation", "access_revocation"):
                locked.auth_version += 1
                if change != "access_rotation":
                    locked.auth_revoked_at = datetime.now(UTC)
            elif change == "tenant":
                locked.tenant_id = "concurrently-changed-tenant"
            await revoking.flush()
            payload = DeviceRegisterRequest.model_validate(_register_payload("serial-a"))
            renewal = (
                device_routes.register_device(payload, current_user=None, current_device=current_device,
                    enrollment_token=None, db=waiting)
                if change.startswith("access_") else
                device_routes.refresh_device_credentials(payload, identity, waiting)
            )
            task = asyncio.create_task(renewal)
            try:
                async with admin_engine.connect() as observer:
                    for _ in range(300):
                        is_waiting = await observer.scalar(text(
                            "SELECT wait_event_type = 'Lock' FROM pg_stat_activity WHERE pid = :pid"), {"pid": pid})
                        if is_waiting:
                            break
                        await asyncio.sleep(0.01)
                    assert is_waiting, "renewal must wait for the device-row lock"
                await revoking.commit()
                with pytest.raises(HTTPException) as rejected:
                    await asyncio.wait_for(task, timeout=5)
                assert rejected.value.status_code == 401
            finally:
                if not task.done():
                    task.cancel()
                    await asyncio.gather(task, return_exceptions=True)
                await waiting.rollback()
                await revoking.rollback()
        async with sessions() as verify:
            unchanged = await verify.scalar(select(Device).where(Device.id == identity.row_id))
            assert unchanged.name == "workstation-a"
    finally:
        await engine.dispose()
        async with admin_engine.begin() as connection:
            await connection.execute(text(f'DROP SCHEMA "{schema}" CASCADE'))
        await admin_engine.dispose()
