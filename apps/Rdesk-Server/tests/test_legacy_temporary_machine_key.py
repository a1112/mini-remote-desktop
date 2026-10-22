"""Legacy physical publication pins a key through the real device-authenticated API."""

from datetime import UTC, datetime, timedelta
import hashlib
import json
import struct

import pytest
from sqlalchemy import select

from app.core.config import settings
from app.core.security import create_device_access_token
from app.models.device import Device
from app.models.device_machine_identity import DeviceMachineIdentity
from app.models.device_temporary_access import DeviceTemporaryAccess
from test_device_session_api import device_sessions_api, _headers
from test_guest_browser_session_api import KEY, KEY_ID, PUBLIC, PASSWORD, SALT
from test_browser_session_api import trusted_realtime_identity
from pydantic import SecretStr


@pytest.fixture(autouse=True)
def legacy_temporary_config(monkeypatch):
    monkeypatch.setattr(settings, "public_api_url", "https://guest.test/api/v1")
    monkeypatch.setitem(settings.__dict__, "guest_browser_enabled", True)
    monkeypatch.setattr(settings, "device_serial_pepper", SecretStr("a1" * 32))


# This builds only the signed request: unlike _publish, it never seeds a mapping.
def _payload(device, *, key=KEY, enabled=True, **changes):
    public = key.public_key().public_bytes_raw()
    key_id = hashlib.sha256(public).hexdigest()
    document = dict(
        device_id=device.device_id,
        auth_version=device.auth_version,
        generation=1,
        enabled=enabled,
        expires_at_ms=int((datetime.now(UTC) + timedelta(seconds=590)).timestamp() * 1000) if enabled else None,
        salt=SALT.hex() if enabled else None,
        verifier=hashlib.pbkdf2_hmac("sha256", PASSWORD.encode(), SALT, 600000).hex() if enabled else None,
        allowed_scopes=["input.keyboard", "input.pointer", "screen.view"] if enabled else [],
    )
    document.update(changes)
    access = json.dumps(document, separators=(",", ":"))
    canonical = "\n".join(("POST", settings.public_api_url + "/devices/temporary-access", key_id, hashlib.sha256(access.encode()).hexdigest())).encode()
    domain = b"MRD_DEVICE_TEMPORARY_ACCESS_V1"
    signed = b"MRD_CONTEXT_SIGNATURE_V1" + struct.pack(">H", len(domain)) + domain + struct.pack(">Q", len(canonical)) + canonical
    return dict(key_id=key_id, public_key=public.hex(), access_json=access, signature=key.sign(signed).hex())


def _post(api, target="unbound-1", *, payload=None, headers=None):
    return api.client.post(
        "/api/v1/devices/temporary-access",
        headers=_headers(api, target) if headers is None else headers,
        json=_payload(api.devices[target]) if payload is None else payload,
    )


def _device_fields(device):
    return {column.key: getattr(device, column.key) for column in Device.__table__.columns}


@pytest.mark.parametrize("target", ["unbound-1", "target-1"])
def test_legacy_device_first_publication_preserves_identity_without_user_login(device_sessions_api, target):
    api = device_sessions_api
    device = api.devices[target]
    device.motherboard_serial_digest = "29" * 32
    device.active_refresh_jti_hash = "58" * 32
    device.hostname = "legacy-host"
    device.cpu_info = "preserved CPU"
    device.bound_at = datetime.now(UTC) if device.is_bound else None
    api.session.commit()
    api.tokens[target] = create_device_access_token(device)
    api.session.refresh(device)
    before = _device_fields(device)
    assert api.session.query(DeviceMachineIdentity).count() == 0
    assert api.session.query(DeviceTemporaryAccess).count() == 0

    response = _post(api, target)
    assert response.status_code == 200, response.text
    assert response.json()["ready"] is True
    assert response.headers["cache-control"] == "no-store, private"
    api.session.refresh(device)
    assert _device_fields(device) == before
    mapping = api.session.scalar(select(DeviceMachineIdentity).where(DeviceMachineIdentity.device_row_id == device.id))
    assert mapping is not None
    assert (mapping.key_id, mapping.public_key) == (KEY_ID, PUBLIC.hex())
    assert api.session.query(DeviceMachineIdentity).count() == 1
    row = api.session.scalar(select(DeviceTemporaryAccess).where(DeviceTemporaryAccess.device_row_id == device.id))
    assert row is not None and row.generation == 1 and row.key_id == KEY_ID
    assert PASSWORD not in response.text and "verifier" not in response.text


def _assert_rejected_without_pin(api, response):
    assert response.status_code == 401, response.text
    assert response.json()["detail"] == {"code": "guest_access_invalid", "message": "Temporary access is unavailable"}
    assert api.session.query(DeviceMachineIdentity).count() == 0
    assert api.session.query(DeviceTemporaryAccess).count() == 0


@pytest.mark.parametrize("changes", [
    {"expires_at_ms": 1},
    {"expires_at_ms": 2**63 - 1},
    {"auth_version": 2},
    {"device_id": "another-device"},
    {"generation": 0},
    {"generation": True},
    {"allowed_scopes": ["screen.view", "input.pointer"]},
    {"allowed_scopes": ["screen.view", "terminal.open"]},
    {"unknown_field": "not allowed"},
])
def test_invalid_signed_first_publication_never_pins(device_sessions_api, changes):
    api = device_sessions_api
    _assert_rejected_without_pin(api, _post(api, payload=_payload(api.devices["unbound-1"], **changes)))


def test_wrong_signature_first_publication_never_pins(device_sessions_api):
    api = device_sessions_api
    payload = _payload(api.devices["unbound-1"])
    payload["signature"] = "00" * 64
    _assert_rejected_without_pin(api, _post(api, payload=payload))


def test_disabled_first_publication_has_no_freshness_and_cannot_pin(device_sessions_api):
    api = device_sessions_api
    _assert_rejected_without_pin(api, _post(api, payload=_payload(api.devices["unbound-1"], enabled=False)))


@pytest.mark.parametrize("header", ["Authorization", "X-Rdesk-Device-Authorization"])
def test_account_token_cannot_pin_a_physical_machine(device_sessions_api, header):
    api = device_sessions_api
    response = _post(api, headers={header: "Bearer " + api.user_token})
    assert response.status_code == 401
    assert api.session.query(DeviceMachineIdentity).count() == 0


def _pin(api, device, *, key=KEY):
    public = key.public_key().public_bytes_raw()
    mapping = DeviceMachineIdentity(key_id=hashlib.sha256(public).hexdigest(), public_key=public.hex(), device_row_id=device.id, created_at=datetime.now(UTC))
    api.session.add(mapping)
    api.session.commit()
    api.session.refresh(mapping)
    return mapping


def test_existing_physical_key_cannot_be_replaced(device_sessions_api):
    from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
    api = device_sessions_api
    old = _pin(api, api.devices["unbound-1"], key=Ed25519PrivateKey.from_private_bytes(bytes([92]) * 32))
    original = (old.key_id, old.public_key, old.device_row_id, old.created_at)
    response = _post(api)
    assert response.status_code == 401
    api.session.refresh(old)
    assert (old.key_id, old.public_key, old.device_row_id, old.created_at) == original
    assert api.session.query(DeviceMachineIdentity).count() == 1
    assert api.session.query(DeviceTemporaryAccess).count() == 0


def test_key_pinned_to_another_physical_device_cannot_be_claimed(device_sessions_api):
    api = device_sessions_api
    old = _pin(api, api.devices["target-1"])
    response = _post(api)
    assert response.status_code == 401
    assert api.session.get(DeviceMachineIdentity, KEY_ID).device_row_id == old.device_row_id
    assert api.session.query(DeviceMachineIdentity).count() == 1
    assert api.session.query(DeviceTemporaryAccess).count() == 0


@pytest.mark.parametrize("state", ["active", "expired", "revoked"])
def test_browser_key_is_never_repurposed_as_a_machine_key(device_sessions_api, trusted_realtime_identity, state):
    from app.models.browser_controller import BrowserController
    from test_browser_session_api import _create_browser
    api = device_sessions_api
    browser = _create_browser(api, controller_public_key=list(PUBLIC))
    assert browser.status_code == 200, browser.text
    principal = api.session.scalar(select(BrowserController))
    assert principal.key_id == KEY_ID
    if state == "expired":
        principal.created_at = datetime.now(UTC) - timedelta(minutes=11)
        principal.expires_at = datetime.now(UTC) - timedelta(seconds=1)
    elif state == "revoked":
        principal.revoked_at = datetime.now(UTC)
    api.session.commit()
    _assert_rejected_without_pin(api, _post(api))


def test_exact_retry_keeps_one_pin_and_later_disable_keeps_that_pin(device_sessions_api):
    api = device_sessions_api
    payload = _payload(api.devices["unbound-1"])
    first = _post(api, payload=payload)
    assert first.status_code == 200, first.text
    mapping = api.session.get(DeviceMachineIdentity, KEY_ID)
    api.session.refresh(mapping)
    original = (mapping.key_id, mapping.public_key, mapping.device_row_id, mapping.created_at)
    retry = _post(api, payload=payload)
    assert retry.status_code == 200 and retry.json() == first.json()
    assert api.session.query(DeviceMachineIdentity).count() == 1
    disabled = _post(api, payload=_payload(api.devices["unbound-1"], generation=2, enabled=False))
    assert disabled.status_code == 200 and disabled.json()["ready"] is False
    api.session.refresh(mapping)
    assert (mapping.key_id, mapping.public_key, mapping.device_row_id, mapping.created_at) == original
    assert _post(api, payload=payload).status_code == 401


def test_signed_same_generation_renewal_only_updates_publication_lease(device_sessions_api):
    api = device_sessions_api
    device = api.devices["unbound-1"]
    expires_ms = int((datetime.now(UTC) + timedelta(seconds=120)).timestamp() * 1000)
    first_payload = _payload(device, expires_at_ms=expires_ms)
    assert _post(api, payload=first_payload).status_code == 200
    row = api.session.get(DeviceTemporaryAccess, device.id)
    before = {column.key: getattr(row, column.key) for column in DeviceTemporaryAccess.__table__.columns}
    mapping = api.session.get(DeviceMachineIdentity, KEY_ID)
    pin_before = (mapping.key_id, mapping.public_key, mapping.device_row_id, mapping.created_at)

    renewal = _payload(device, expires_at_ms=expires_ms + 60_000)
    response = _post(api, payload=renewal)
    assert response.status_code == 200, response.text
    assert response.json()["ready"] is True
    assert response.json()["generation"] == 1
    assert response.json()["expires_at_ms"] == expires_ms + 60_000
    api.session.refresh(row)
    after = {column.key: getattr(row, column.key) for column in DeviceTemporaryAccess.__table__.columns}
    mutable = {"expires_at", "publication_digest", "updated_at"}
    assert {key: value for key, value in after.items() if key not in mutable} == {
        key: value for key, value in before.items() if key not in mutable
    }
    assert after["publication_digest"] != before["publication_digest"]
    assert after["updated_at"] >= before["updated_at"]
    assert after["publication_digest"] != hashlib.sha256(renewal["access_json"].encode()).hexdigest()
    api.session.refresh(mapping)
    assert (mapping.key_id, mapping.public_key, mapping.device_row_id, mapping.created_at) == pin_before
    assert _post(api, payload=renewal).json() == response.json()
    # An old publication/late acknowledgement cannot replace the newer lease.
    assert _post(api, payload=first_payload).status_code == 401
    api.session.refresh(row)
    assert row.expires_at == after["expires_at"]


@pytest.mark.parametrize("change", [
    "salt", "verifier", "scopes", "auth_version", "device_id", "key",
    "disabled", "shorter", "equal_different_json", "expired", "overlong",
    "signature", "stored_auth", "stored_key",
])
def test_same_generation_renewal_rejects_material_identity_and_lease_changes(device_sessions_api, change):
    from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
    api = device_sessions_api
    device = api.devices["unbound-1"]
    expires_ms = int((datetime.now(UTC) + timedelta(seconds=120)).timestamp() * 1000)
    assert _post(api, payload=_payload(device, expires_at_ms=expires_ms)).status_code == 200
    row = api.session.get(DeviceTemporaryAccess, device.id)
    if change == "stored_auth":
        row.target_auth_version += 1
    elif change == "stored_key":
        row.key_id = "93" * 32
    api.session.commit()
    before = {column.key: getattr(row, column.key) for column in DeviceTemporaryAccess.__table__.columns}
    changes = {"expires_at_ms": expires_ms + 60_000}
    if change == "salt":
        changes["salt"] = "74" * 16
    elif change == "verifier":
        changes["verifier"] = "75" * 32
    elif change == "scopes":
        changes["allowed_scopes"] = ["screen.view"]
    elif change == "auth_version":
        changes["auth_version"] = device.auth_version + 1
    elif change == "device_id":
        changes["device_id"] = "another-device"
    elif change == "shorter":
        changes["expires_at_ms"] = expires_ms - 1
    elif change == "equal_different_json":
        changes["expires_at_ms"] = expires_ms
    elif change == "expired":
        changes["expires_at_ms"] = 1
    elif change == "overlong":
        changes["expires_at_ms"] = int((datetime.now(UTC) + timedelta(seconds=601)).timestamp() * 1000)
    if change == "disabled":
        payload = _payload(device, enabled=False)
    else:
        key = Ed25519PrivateKey.from_private_bytes(bytes([92]) * 32) if change == "key" else KEY
        payload = _payload(device, key=key, **changes)
    if change == "equal_different_json":
        # The same expiry accepts only its exact signed publication retry.
        payload["access_json"] += " "
        canonical = "\n".join(("POST", settings.public_api_url + "/devices/temporary-access", KEY_ID, hashlib.sha256(payload["access_json"].encode()).hexdigest())).encode()
        domain = b"MRD_DEVICE_TEMPORARY_ACCESS_V1"
        message = b"MRD_CONTEXT_SIGNATURE_V1" + struct.pack(">H", len(domain)) + domain + struct.pack(">Q", len(canonical)) + canonical
        payload["signature"] = KEY.sign(message).hex()
    elif change == "signature":
        payload["signature"] = "00" * 64
    response = _post(api, payload=payload)
    assert response.status_code == 401, response.text
    assert response.json()["detail"]["code"] == "guest_access_invalid"
    api.session.refresh(row)
    assert {column.key: getattr(row, column.key) for column in DeviceTemporaryAccess.__table__.columns} == before
    assert api.session.get(DeviceMachineIdentity, KEY_ID).public_key == PUBLIC.hex()


def test_same_generation_renewal_recovers_after_offline_gap_but_expired_retry_fails(device_sessions_api):
    import asyncio
    from app.core.security import capture_device_auth_snapshot
    from app.schemas.guest_browser import TemporaryAccessPublishIn
    from app.services.temporary_access import TemporaryAccessService
    from app.services.device_sessions import DeviceSessionError
    from test_relay_node_api import AsyncSessionShim
    api = device_sessions_api
    device = api.devices["unbound-1"]
    clock = [datetime.now(UTC)]
    service = TemporaryAccessService(AsyncSessionShim(api.session), api_url=settings.public_api_url, pepper=bytes.fromhex("a1" * 32), now=lambda: clock[0])
    snapshot = capture_device_auth_snapshot(device)
    deadline = clock[0] + timedelta(seconds=5)
    first = TemporaryAccessPublishIn(**_payload(device, expires_at_ms=int(deadline.timestamp() * 1000)))
    assert asyncio.run(service.publish(snapshot=snapshot, payload=first)).ready
    api.session.commit()
    row = api.session.get(DeviceTemporaryAccess, device.id)
    secret_before = (row.salt, row.verifier_hmac, row.generation)
    clock[0] = deadline + timedelta(seconds=1)
    assert service.status(row, device).ready is False
    with pytest.raises(DeviceSessionError) as rejected:
        asyncio.run(service.publish(snapshot=snapshot, payload=first))
    assert rejected.value.code == "guest_access_invalid"
    expires_ms = int((clock[0] + timedelta(seconds=600)).timestamp() * 1000)
    renewal = TemporaryAccessPublishIn(**_payload(device, expires_at_ms=expires_ms))
    status = asyncio.run(service.publish(snapshot=snapshot, payload=renewal))
    assert status.ready and status.expires_at_ms == expires_ms
    assert (row.salt, row.verifier_hmac, row.generation) == secret_before


def test_same_generation_renewal_cannot_resurrect_explicitly_disabled_state(device_sessions_api):
    api = device_sessions_api
    device = api.devices["unbound-1"]
    assert _post(api).status_code == 200
    assert _post(api, payload=_payload(device, generation=2, enabled=False)).status_code == 200
    row = api.session.get(DeviceTemporaryAccess, device.id)
    before = {column.key: getattr(row, column.key) for column in DeviceTemporaryAccess.__table__.columns}
    response = _post(api, payload=_payload(device, generation=2))
    assert response.status_code == 401, response.text
    api.session.refresh(row)
    assert {column.key: getattr(row, column.key) for column in DeviceTemporaryAccess.__table__.columns} == before


@pytest.mark.parametrize("stage", ["principal", "device", "temporary"])
def test_renewal_rechecks_expiry_after_database_lock_waits(device_sessions_api, monkeypatch, stage):
    import asyncio
    from contextlib import asynccontextmanager
    from app.core.security import capture_device_auth_snapshot
    from app.schemas.guest_browser import TemporaryAccessPublishIn
    from app.services import temporary_access as domain
    from test_relay_node_api import AsyncSessionShim
    api = device_sessions_api
    device = api.devices["unbound-1"]
    clock = [datetime.now(UTC)]
    original_ms = int((clock[0] + timedelta(seconds=2)).timestamp() * 1000)
    assert _post(api, payload=_payload(device, expires_at_ms=original_ms)).status_code == 200
    row = api.session.get(DeviceTemporaryAccess, device.id)
    before = {column.key: getattr(row, column.key) for column in DeviceTemporaryAccess.__table__.columns}
    deadline = clock[0] + timedelta(seconds=5)
    payload = TemporaryAccessPublishIn(**_payload(device, expires_at_ms=int(deadline.timestamp() * 1000)))
    if stage == "principal":
        delegate = domain.principal_key_lock
        @asynccontextmanager
        async def delayed_lock(db, key_id):
            async with delegate(db, key_id):
                clock[0] = deadline
                yield
        monkeypatch.setattr(domain, "principal_key_lock", delayed_lock)
    class WaitingSession(AsyncSessionShim):
        async def scalar(self, statement, *args, **kwargs):
            result = await super().scalar(statement, *args, **kwargs)
            selected = statement.column_descriptions[0].get("entity")
            if (stage == "device" and selected is Device) or (stage == "temporary" and selected is DeviceTemporaryAccess):
                clock[0] = deadline
            return result
    service = domain.TemporaryAccessService(WaitingSession(api.session), api_url=settings.public_api_url, pepper=bytes.fromhex("a1" * 32), now=lambda: clock[0])
    with pytest.raises(domain.DeviceSessionError) as rejected:
        asyncio.run(service.publish(snapshot=capture_device_auth_snapshot(device), payload=payload))
    assert rejected.value.code == "guest_access_invalid"
    api.session.refresh(row)
    assert {column.key: getattr(row, column.key) for column in DeviceTemporaryAccess.__table__.columns} == before


def test_missing_pin_with_existing_publication_is_storage_inconsistency(device_sessions_api):
    api = device_sessions_api
    _pin(api, api.devices["unbound-1"])
    assert _post(api).status_code == 200
    api.session.delete(api.session.get(DeviceMachineIdentity, KEY_ID))
    api.session.commit()
    response = _post(api, payload=_payload(api.devices["unbound-1"], generation=2))
    assert response.status_code == 401
    assert api.session.query(DeviceMachineIdentity).count() == 0
    assert api.session.query(DeviceTemporaryAccess).one().generation == 1


def test_pin_and_publication_are_not_committed_by_service(device_sessions_api):
    import asyncio
    from app.core.security import capture_device_auth_snapshot
    from app.schemas.guest_browser import TemporaryAccessPublishIn
    from app.services.temporary_access import TemporaryAccessService
    from test_relay_node_api import AsyncSessionShim
    api = device_sessions_api
    device = api.devices["unbound-1"]
    service = TemporaryAccessService(AsyncSessionShim(api.session), api_url=settings.public_api_url, pepper=bytes.fromhex("a1" * 32))
    status = asyncio.run(service.publish(snapshot=capture_device_auth_snapshot(device), payload=TemporaryAccessPublishIn(**_payload(device))))
    assert status.ready is True
    assert api.session.query(DeviceMachineIdentity).count() == 1
    assert api.session.query(DeviceTemporaryAccess).count() == 1
    api.session.rollback()
    assert api.session.query(DeviceMachineIdentity).count() == 0
    assert api.session.query(DeviceTemporaryAccess).count() == 0


@pytest.mark.parametrize("stage", ["principal", "device", "temporary", "sessions"])
def test_first_pin_rechecks_freshness_after_database_waits(device_sessions_api, monkeypatch, stage):
    import asyncio
    from contextlib import asynccontextmanager
    from app.core.security import capture_device_auth_snapshot
    from app.schemas.guest_browser import TemporaryAccessPublishIn
    from app.services import temporary_access as domain
    from test_relay_node_api import AsyncSessionShim
    api = device_sessions_api
    device = api.devices["unbound-1"]
    clock = [datetime.now(UTC)]
    deadline = clock[0] + timedelta(seconds=5)
    payload = TemporaryAccessPublishIn(**_payload(device, expires_at_ms=int(deadline.timestamp() * 1000)))
    if stage == "principal":
        delegate = domain.principal_key_lock
        @asynccontextmanager
        async def delayed_lock(db, key_id):
            async with delegate(db, key_id):
                clock[0] = deadline + timedelta(seconds=1)
                yield
        monkeypatch.setattr(domain, "principal_key_lock", delayed_lock)
    class WaitingSession(AsyncSessionShim):
        async def scalar(self, statement, *args, **kwargs):
            result = await super().scalar(statement, *args, **kwargs)
            selected = statement.column_descriptions[0].get("entity")
            if (stage == "device" and selected is Device) or (stage == "temporary" and selected is DeviceTemporaryAccess):
                clock[0] = deadline + timedelta(seconds=1)
            return result
        async def scalars(self, *args, **kwargs):
            result = await super().scalars(*args, **kwargs)
            if stage == "sessions":
                clock[0] = deadline + timedelta(seconds=1)
            return result
    service = domain.TemporaryAccessService(WaitingSession(api.session), api_url=settings.public_api_url, pepper=bytes.fromhex("a1" * 32), now=lambda: clock[0])
    with pytest.raises(domain.DeviceSessionError) as rejected:
        asyncio.run(service.publish(snapshot=capture_device_auth_snapshot(device), payload=payload))
    assert rejected.value.code == "guest_access_invalid"
    assert api.session.query(DeviceMachineIdentity).count() == 0
    assert api.session.query(DeviceTemporaryAccess).count() == 0


@pytest.mark.parametrize("changes", [
    {"auth_version": 2},
    {"tenant_id": "tenant-b"},
    {"is_bound": True, "bound_user_id": "target-user"},
    {"auth_revoked_at": datetime(2025, 1, 1, tzinfo=UTC)},
    {"device_id": "changed-device"},
    {"principal_kind": "browser_controller"},
])
def test_first_pin_revalidates_authenticated_snapshot_after_lock(device_sessions_api, monkeypatch, changes):
    from app.services import temporary_access as domain
    from sqlalchemy import update
    api = device_sessions_api
    delegate = domain.key_is_browser_controller
    async def identity_changes_after_authentication(db, key_id):
        result = await delegate(db, key_id)
        await db.execute(update(Device).where(Device.id == api.devices["unbound-1"].id).values(**changes))
        return result
    monkeypatch.setattr(domain, "key_is_browser_controller", identity_changes_after_authentication)
    _assert_rejected_without_pin(api, _post(api))


@pytest.mark.parametrize("conflict", ["key", "device"])
def test_known_machine_unique_conflict_rolls_back_first_pin_and_publication(device_sessions_api, conflict):
    from sqlalchemy import event, insert
    api = device_sessions_api
    def conflicting_insert(session, flush_context, instances):
        if any(isinstance(item, DeviceMachineIdentity) for item in session.new):
            session.connection().execute(insert(DeviceMachineIdentity).values(key_id=KEY_ID if conflict == "key" else "93" * 32, public_key=PUBLIC.hex(), device_row_id=api.devices["target-1" if conflict == "key" else "unbound-1"].id, created_at=datetime.now(UTC)))
    event.listen(api.session, "before_flush", conflicting_insert)
    try:
        _assert_rejected_without_pin(api, _post(api))
    finally:
        event.remove(api.session, "before_flush", conflicting_insert)


def test_unknown_integrity_failure_is_propagated_and_transaction_can_roll_back(device_sessions_api):
    import asyncio
    from sqlalchemy import event, text
    from sqlalchemy.exc import IntegrityError
    from app.core.security import capture_device_auth_snapshot
    from app.schemas.guest_browser import TemporaryAccessPublishIn
    from app.services.temporary_access import TemporaryAccessService
    from test_relay_node_api import AsyncSessionShim
    api = device_sessions_api
    def unrelated_constraint(session, flush_context, instances):
        if any(isinstance(item, DeviceMachineIdentity) for item in session.new):
            session.connection().execute(text("INSERT INTO device_machine_identities (key_id, public_key, device_row_id, created_at) VALUES ('invalid', 'invalid', 'unbound-row', CURRENT_TIMESTAMP)"))
    event.listen(api.session, "before_flush", unrelated_constraint)
    try:
        service = TemporaryAccessService(AsyncSessionShim(api.session), api_url=settings.public_api_url, pepper=bytes.fromhex("a1" * 32))
        with pytest.raises(IntegrityError):
            asyncio.run(service.publish(snapshot=capture_device_auth_snapshot(api.devices["unbound-1"]), payload=TemporaryAccessPublishIn(**_payload(api.devices["unbound-1"]))))
    finally:
        event.remove(api.session, "before_flush", unrelated_constraint)
        api.session.rollback()
    assert api.session.query(DeviceMachineIdentity).count() == 0
    assert api.session.query(DeviceTemporaryAccess).count() == 0


@pytest.mark.parametrize("kind", ["absent", "ordinary_device_header", "browser_signaling", "browser_shadow_device", "guest_http"])
def test_nonphysical_or_wrong_header_credentials_cannot_first_pin(device_sessions_api, monkeypatch, kind):
    from app.api.v1 import browser_sessions
    from app.models.session_request import SessionRequest
    from test_browser_session_api import _create_browser
    from test_guest_browser_session_api import _guest
    api = device_sessions_api
    target = "target-1"
    payload = _payload(api.devices[target])
    if kind == "absent":
        headers = {}
    elif kind == "ordinary_device_header":
        headers = {"Authorization": "Bearer " + api.tokens[target]}
    elif kind in {"browser_signaling", "browser_shadow_device"}:
        browser = _create_browser(api)
        assert browser.status_code == 200, browser.text
        if kind == "browser_signaling":
            token = browser.json()["credential"]["token"]
        else:
            row = api.session.get(SessionRequest, "browser-session-1")
            token = create_device_access_token(api.session.get(Device, row.requester_device_id))
        headers = {"X-Rdesk-Device-Authorization": "Bearer " + token}
    else:
        assert _post(api).status_code == 200
        async def target_key(_):
            return KEY_ID
        monkeypatch.setattr(browser_sessions, "query_realtime_target_key", target_key)
        from app.api.v1 import guest_browser_sessions
        monkeypatch.setattr(guest_browser_sessions, "enrollment_peer_ip", lambda request, proxies: "127.0.0.1")
        guest = _guest(api)
        assert guest.status_code == 200, guest.text
        headers = {"X-Rdesk-Device-Authorization": "Bearer " + guest.json()["http_credential"]["token"]}
    mapping_count = api.session.query(DeviceMachineIdentity).count()
    response = _post(api, target, payload=payload, headers=headers)
    assert response.status_code == 401, response.text
    assert api.session.query(DeviceMachineIdentity).count() == mapping_count
    assert api.session.scalar(select(DeviceMachineIdentity).where(DeviceMachineIdentity.device_row_id == api.devices[target].id)) is None
