"""Real API, Ed25519, database constraints and explicit migration acceptance."""
from __future__ import annotations

import asyncio
import hashlib
import json
import os
from contextlib import asynccontextmanager
from datetime import UTC, datetime, timedelta
from pathlib import Path
from uuid import uuid4

import pytest
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from cryptography.hazmat.primitives.serialization import Encoding, PublicFormat
from fastapi.testclient import TestClient
from pydantic import ValidationError
from sqlalchemy import event, func, select, text
from sqlalchemy.ext.asyncio import async_sessionmaker, create_async_engine
from starlette.requests import Request

from app.api.v1.device_self_enrollment import router as self_router
from app.api.v1.realtime import router as realtime_router
from app.core.config import settings
from app.core.response_security import SensitiveResponseCacheMiddleware
from app.db.migrate_add_device_self_enrollment import (
    DeviceSelfEnrollmentMigrationError, SELF_ENROLLMENT_TABLES, migrate,
)
from app.db.session import Base
from app.models.device import Device
from app.models.device_machine_identity import DeviceMachineIdentity, DeviceSelfEnrollmentChallenge
from app.schemas.device_self_enrollment import (
    DeviceSelfEnrollmentChallengeRequest, DeviceSelfRegistrationPayload, DeviceSelfRegisterRequest,
)
from app.services import device_self_enrollment as enrollment
from app.services.device_enrollment import DeviceEnrollmentError
from app.services.device_self_enrollment import (
    DeviceSelfEnrollmentService, canonical_self_registration,
    contextual_self_registration, enrollment_peer_ip,
)
from test_device_ownership import (
    EnrollmentSessionShim, SERIAL_PEPPER, _asyncpg_url, _register_payload, device_api,
)

API_URL = "https://example.test/rdesk/api/v1"
CHALLENGE_PATH = "/api/v1/devices/self-enrollment-challenge"
REGISTER_PATH = "/api/v1/devices/self-register"
DATABASE_URL = os.getenv("MRD_TEST_DATABASE_URL")


def _key(seed: int = 17):
    private = Ed25519PrivateKey.from_private_bytes(bytes([seed]) * 32)
    public = private.public_key().public_bytes(Encoding.Raw, PublicFormat.Raw)
    return private, {"protocol_version": 1, "key_id": hashlib.sha256(public).hexdigest(), "public_key": public.hex()}


def _signed(private, identity, challenge, *, serial="self-machine", raw=None, api_url=API_URL,
            expected_device_id=None):
    registration = _register_payload(serial)
    if expected_device_id is not None:
        registration["expected_device_id"] = expected_device_id
    payload = {**identity, "challenge_id": challenge["challenge_id"], "nonce": challenge["nonce"],
               "registration_json": raw or json.dumps(registration, separators=(",", ":")),
               "signature": "00" * 64}
    parsed = DeviceSelfRegisterRequest.model_validate(payload)
    payload["signature"] = private.sign(contextual_self_registration(
        canonical_self_registration(api_url, parsed)
    )).hex()
    return payload


@pytest.fixture
def self_api(device_api, monkeypatch):
    monkeypatch.setattr(settings, "device_self_enrollment_enabled", True)
    monkeypatch.setattr(settings, "public_api_url", API_URL)
    device_api.app.include_router(self_router, prefix="/api/v1")
    device_api.app.include_router(realtime_router, prefix="/api/v1")
    device_api.app.add_middleware(SensitiveResponseCacheMiddleware)
    device_api.client.close()
    device_api.client = TestClient(device_api.app, client=("192.0.2.10", 1234))
    engine = device_api.session.get_bind()

    def explicit_sqlite_transaction(connection):
        # SQLite's legacy driver does not begin a transaction for SELECT, so an
        # outermost savepoint may otherwise commit before route-level rollback.
        # Match production PostgreSQL's transaction semantics in API tests.
        connection.exec_driver_sql("BEGIN")
    event.listen(engine, "begin", explicit_sqlite_transaction)
    try:
        yield device_api
    finally:
        event.remove(engine, "begin", explicit_sqlite_transaction)


def _challenge(api, identity):
    response = api.client.post(CHALLENGE_PATH, json=identity)
    assert response.status_code == 200, response.text
    assert response.headers["cache-control"] == "no-store, private"
    return response.json()


def _enroll(api, *, seed=17, serial="self-machine"):
    private, identity = _key(seed)
    challenge = _challenge(api, identity)
    response = api.client.post(REGISTER_PATH, json=_signed(private, identity, challenge, serial=serial))
    assert response.status_code == 200, response.text
    return private, identity, response.json()


@pytest.mark.parametrize("unicode", [False, True])
def test_shared_cross_language_crypto_vector(unicode):
    fixture = json.loads((Path(__file__).parent / "fixtures/device-self-enrollment-v1.json").read_text(encoding="utf-8"))
    prefix = "unicode_" if unicode else ""
    request = DeviceSelfRegisterRequest.model_validate({
        **{name: fixture[name] for name in ("protocol_version", "key_id", "public_key", "challenge_id", "nonce")},
        "registration_json": fixture[prefix + "registration_json"], "signature": fixture[prefix + "signature"],
    })
    canonical = canonical_self_registration(fixture["api_url"], request)
    assert canonical == fixture[prefix + "canonical"].encode("utf-8")
    assert contextual_self_registration(canonical).hex() == fixture[prefix + "contextual_bytes_hex"]
    private = Ed25519PrivateKey.from_private_bytes(bytes.fromhex(fixture["test_only_seed_hex"]))
    assert private.sign(contextual_self_registration(canonical)).hex() == fixture[prefix + "signature"]
    if unicode:
        assert json.loads(request.registration_json)["device_name"] == "办公电脑"


def test_first_code_unbound_and_fresh_reenrollment_keeps_identity(self_api):
    private, identity, result = _enroll(self_api)
    assert len(result["device_id"]) == 9 and result["device_id"].isascii() and result["device_id"].isdigit()
    assert result["refresh_token"]
    mapping = self_api.session.scalar(select(DeviceMachineIdentity))
    device = self_api.session.get(Device, mapping.device_row_id)
    assert device.is_bound is False and device.bound_user_id is None and device.tenant_id == "default"
    # Simulate loss of the client response/registration file: only the durable
    # machine key remains, and a new one-use challenge recovers the same code.
    self_api.session.expire_all()
    next_challenge = _challenge(self_api, identity)
    again = self_api.client.post(REGISTER_PATH, json=_signed(private, identity, next_challenge))
    assert again.status_code == 200, again.text
    assert again.json()["device_id"] == result["device_id"]
    assert self_api.session.scalar(select(func.count()).select_from(DeviceMachineIdentity)) == 1
    assert self_api.session.scalar(select(func.count()).select_from(Device)) == 2


@pytest.mark.parametrize("legacy_code", ["0123456789", "012345678901"])
def test_fresh_machine_proof_recovers_persisted_legacy_code(self_api, legacy_code):
    private, identity, _ = _enroll(self_api)
    mapping = self_api.session.scalar(select(DeviceMachineIdentity))
    device = self_api.session.get(Device, mapping.device_row_id)
    device.device_id = legacy_code
    self_api.session.commit()
    challenge = _challenge(self_api, identity)
    recovered = self_api.client.post(REGISTER_PATH, json=_signed(private, identity, challenge))
    assert recovered.status_code == 200, recovered.text
    assert recovered.json()["device_id"] == legacy_code
    assert self_api.session.scalar(select(func.count()).select_from(DeviceMachineIdentity)) == 1


@pytest.mark.parametrize("stored_code", [None, "0123456789", "012345678901"])
def test_expected_code_recovers_same_assignment_and_owner_without_http_credentials(self_api, stored_code):
    private, identity, result = _enroll(self_api)
    mapping = self_api.session.scalar(select(DeviceMachineIdentity))
    device = self_api.session.get(Device, mapping.device_row_id)
    device.device_id = stored_code or result["device_id"]
    device.is_bound = True
    device.bound_user_id = self_api.owner.id
    device.tenant_id = self_api.owner.tenant_id
    device.auth_version = 3
    self_api.session.commit()
    expected = (device.id, device.device_id, device.is_bound, device.bound_user_id,
                device.tenant_id, device.auth_version, mapping.key_id, mapping.public_key)
    challenge = _challenge(self_api, identity)
    recovered = self_api.client.post(REGISTER_PATH, json=_signed(
        private, identity, challenge, expected_device_id=device.device_id,
    ))
    assert recovered.status_code == 200, recovered.text
    assert recovered.json()["device_id"] == device.device_id
    assert recovered.json()["access_token"] and recovered.json()["refresh_token"]
    self_api.session.refresh(device)
    self_api.session.refresh(mapping)
    assert (device.id, device.device_id, device.is_bound, device.bound_user_id,
            device.tenant_id, device.auth_version, mapping.key_id, mapping.public_key) == expected
    assert self_api.session.scalar(select(func.count()).select_from(Device)) == 2
    assert self_api.session.scalar(select(func.count()).select_from(DeviceMachineIdentity)) == 1


def test_wrong_expected_code_conflicts_before_metadata_updates(self_api):
    private, identity, result = _enroll(self_api)
    mapping = self_api.session.scalar(select(DeviceMachineIdentity))
    device = self_api.session.get(Device, mapping.device_row_id)
    expected = (device.device_id, device.hostname, device.os_version, device.auth_version)
    challenge = _challenge(self_api, identity)
    registration = {**_register_payload("self-machine"), "hostname": "changed-host",
                    "expected_device_id": "different-stored-code"}
    denied = self_api.client.post(REGISTER_PATH, json=_signed(
        private, identity, challenge, raw=json.dumps(registration),
    ))
    assert denied.status_code == 409, denied.text
    assert denied.json()["detail"]["code"] == "device_self_enrollment_conflict"
    assert "access_token" not in denied.text and "refresh_token" not in denied.text
    self_api.session.refresh(device)
    assert (device.device_id, device.hostname, device.os_version, device.auth_version) == expected
    assert device.device_id == result["device_id"]
    assert self_api.session.scalar(select(func.count()).select_from(Device)) == 2
    assert self_api.session.scalar(select(DeviceSelfEnrollmentChallenge).where(
        DeviceSelfEnrollmentChallenge.challenge_id == challenge["challenge_id"]
    )).consumed_at is None


@pytest.mark.parametrize("serial", ["never-registered-machine", "serial-a"])
def test_expected_code_with_missing_machine_mapping_never_allocates(self_api, serial, monkeypatch):
    private, identity = _key()
    challenge = _challenge(self_api, identity)

    async def allocation_forbidden(*args, **kwargs):
        raise AssertionError("recovery cannot allocate a device")
    monkeypatch.setattr(DeviceSelfEnrollmentService, "_allocate_device", allocation_forbidden)
    denied = self_api.client.post(REGISTER_PATH, json=_signed(
        private, identity, challenge, serial=serial, expected_device_id=self_api.device.device_id,
    ))
    assert denied.status_code == 409, denied.text
    assert denied.json()["detail"]["code"] == "device_self_enrollment_conflict"
    assert "access_token" not in denied.text and "refresh_token" not in denied.text
    assert self_api.session.scalar(select(func.count()).select_from(Device)) == 1
    assert self_api.session.scalar(select(func.count()).select_from(DeviceMachineIdentity)) == 0
    assert self_api.session.scalar(select(DeviceSelfEnrollmentChallenge)).consumed_at is None


def test_expected_code_cannot_be_changed_without_resigning_payload(self_api):
    private, identity, result = _enroll(self_api)
    challenge = _challenge(self_api, identity)
    payload = _signed(private, identity, challenge, expected_device_id=result["device_id"])
    registration = json.loads(payload["registration_json"])
    registration["expected_device_id"] = "different-stored-code"
    payload["registration_json"] = json.dumps(registration)
    denied = self_api.client.post(REGISTER_PATH, json=payload)
    assert denied.status_code == 401, denied.text
    assert self_api.session.scalar(select(func.count()).select_from(Device)) == 2
    assert self_api.session.scalar(select(func.count()).select_from(DeviceMachineIdentity)) == 1
    assert self_api.session.scalar(select(DeviceSelfEnrollmentChallenge).where(
        DeviceSelfEnrollmentChallenge.challenge_id == challenge["challenge_id"]
    )).consumed_at is None


@pytest.mark.parametrize("expected", ["", "a" * 65, 123456789, True])
def test_expected_code_is_a_bounded_string(expected):
    with pytest.raises(ValidationError):
        DeviceSelfRegistrationPayload.model_validate({
            **_register_payload(), "expected_device_id": expected,
        })


def test_existing_machine_key_cannot_switch_hardware_serial(self_api):
    private, identity, result = _enroll(self_api)
    challenge = _challenge(self_api, identity)
    rejected = self_api.client.post(REGISTER_PATH, json=_signed(
        private, identity, challenge, serial="changed-machine-serial",
    ))
    assert rejected.status_code == 409, rejected.text
    mapping = self_api.session.scalar(select(DeviceMachineIdentity))
    assert self_api.session.get(Device, mapping.device_row_id).device_id == result["device_id"]
    assert self_api.session.scalar(select(func.count()).select_from(Device)) == 2
    stored = self_api.session.scalar(select(DeviceSelfEnrollmentChallenge).where(
        DeviceSelfEnrollmentChallenge.challenge_id == challenge["challenge_id"]
    ))
    assert stored.consumed_at is None


def test_nonce_stored_only_as_digest_and_replay_never_mints_again(self_api):
    private, identity = _key()
    challenge = _challenge(self_api, identity)
    stored = self_api.session.scalar(select(DeviceSelfEnrollmentChallenge))
    assert stored.nonce_digest != challenge["nonce"] and not hasattr(stored, "nonce")
    payload = _signed(private, identity, challenge)
    first = self_api.client.post(REGISTER_PATH, json=payload)
    second = self_api.client.post(REGISTER_PATH, json=payload)
    assert first.status_code == 200 and second.status_code == 401
    assert "access_token" not in second.text and "refresh_token" not in second.text
    assert second.headers["cache-control"] == "no-store, private"


@pytest.mark.parametrize("mutation", ["body", "nonce", "challenge", "public_key", "key_id", "signature", "origin", "expiry"])
def test_signature_binding_and_expiry_reject_tampering(self_api, mutation):
    private, identity = _key()
    challenge = _challenge(self_api, identity)
    payload = _signed(private, identity, challenge)
    if mutation == "body": payload["registration_json"] += " "
    if mutation == "nonce": payload["nonce"] = "ff" * 32
    if mutation == "challenge": payload["challenge_id"] = "ff" * 16
    if mutation in ("public_key", "key_id"): payload[mutation] = "ff" * 32
    if mutation == "signature": payload["signature"] = "ff" * 64
    if mutation == "origin": payload = _signed(private, identity, challenge, api_url="https://attacker.test/api/v1")
    if mutation == "expiry":
        stored = self_api.session.scalar(select(DeviceSelfEnrollmentChallenge))
        stored.issued_at = datetime.now(UTC) - timedelta(minutes=2)
        stored.expires_at = datetime.now(UTC) - timedelta(minutes=1)
        self_api.session.commit()
    rejected = self_api.client.post(REGISTER_PATH, json=payload)
    assert rejected.status_code == 401, rejected.text
    assert self_api.session.scalar(select(func.count()).select_from(DeviceMachineIdentity)) == 0


def test_raw_unicode_and_whitespace_are_signed_without_json_reconstruction(self_api):
    private, identity = _key()
    raw = json.dumps({**_register_payload("unicode-machine"), "device_name": "办公电脑"}, ensure_ascii=False, indent=2)
    challenge = _challenge(self_api, identity)
    response = self_api.client.post(REGISTER_PATH, json=_signed(private, identity, challenge, raw=raw))
    assert response.status_code == 200 and response.json()["device_name"] == "办公电脑"


@pytest.mark.parametrize("raw", [
    '{"motherboard_serial":"s","hostname":"h","os_version":"Linux","hostname":"other"}',
    '{"motherboard_serial":"s","hostname":"h","os_version":"Linux","bound_user_id":"attacker"}',
    '{"motherboard_serial":"s","hostname":"h","os_version":" "}',
    '{"motherboard_serial":"s","hostname":"h","os_version":"Linux","total_memory_mb":NaN}',
])
def test_signed_registration_rejects_duplicate_unknown_and_invalid_fields(self_api, raw):
    private, identity = _key()
    challenge = _challenge(self_api, identity)
    response = self_api.client.post(REGISTER_PATH, json=_signed(private, identity, challenge, raw=raw))
    assert response.status_code == 401
    assert self_api.session.scalar(select(func.count()).select_from(DeviceMachineIdentity)) == 0


@pytest.mark.parametrize("with_expected_code", [False, True])
def test_revocation_never_restored_by_fresh_machine_proof(self_api, with_expected_code):
    private, identity, result = _enroll(self_api)
    mapping = self_api.session.scalar(select(DeviceMachineIdentity))
    device = self_api.session.get(Device, mapping.device_row_id)
    device.auth_revoked_at = datetime.now(UTC)
    device.auth_version += 1
    self_api.session.commit()
    challenge = _challenge(self_api, identity)
    rejected = self_api.client.post(REGISTER_PATH, json=_signed(
        private, identity, challenge,
        expected_device_id=result["device_id"] if with_expected_code else None,
    ))
    assert rejected.status_code == 401
    self_api.session.refresh(device)
    assert device.auth_revoked_at is not None and device.device_id == result["device_id"]
    assert self_api.session.scalar(select(func.count()).select_from(DeviceMachineIdentity)) == 1


def test_serial_cannot_claim_legacy_device_or_another_machine(self_api):
    private, identity = _key()
    challenge = _challenge(self_api, identity)
    legacy = self_api.client.post(REGISTER_PATH, json=_signed(private, identity, challenge, serial="serial-a"))
    assert legacy.status_code == 409
    assert self_api.session.scalar(select(func.count()).select_from(DeviceMachineIdentity)) == 0
    assert self_api.session.get(Device, self_api.device.id).device_id == "100000000001"
    _enroll(self_api, seed=19, serial="other-machine")
    different = _challenge(self_api, identity)
    takeover = self_api.client.post(REGISTER_PATH, json=_signed(private, identity, different, serial="other-machine"))
    assert takeover.status_code == 409
    assert self_api.session.scalar(select(func.count()).select_from(DeviceMachineIdentity)) == 1


def test_new_device_signaling_key_must_match_pinned_machine(self_api):
    _, identity, result = _enroll(self_api)
    headers = {"X-Rdesk-Device-Authorization": "Bearer " + result["access_token"]}
    good = self_api.client.post("/api/v1/realtime/device-credentials", headers=headers,
        json={"device_key_id": identity["key_id"], "role": "Peer"})
    bad = self_api.client.post("/api/v1/realtime/device-credentials", headers=headers,
        json={"device_key_id": "ab" * 32, "role": "Peer"})
    assert good.status_code == 200, good.text
    assert bad.status_code == 401 and "token" not in bad.json()


@pytest.mark.parametrize("occupied", ["000000041", "100000000001"])
def test_code_collision_retries_and_never_replaces_existing_row(self_api, monkeypatch, occupied):
    self_api.device.device_id = occupied
    self_api.session.commit()
    monkeypatch.setattr(enrollment, "generate_device_id_from_digest", lambda _: self_api.device.device_id)
    bounds = []
    monkeypatch.setattr(enrollment.secrets, "randbelow", lambda bound: bounds.append(bound) or 42)
    _, _, result = _enroll(self_api)
    assert result["device_id"] == "000000042"
    assert bounds == [10**9]
    self_api.session.refresh(self_api.device)
    assert self_api.device.device_id == occupied


def test_exhausted_code_collisions_roll_back_mapping_and_nonce(self_api, monkeypatch):
    self_api.device.device_id = "000000042"
    self_api.session.commit()
    monkeypatch.setattr(enrollment, "generate_device_id_from_digest", lambda _: "000000042")
    bounds = []
    monkeypatch.setattr(enrollment.secrets, "randbelow", lambda bound: bounds.append(bound) or 42)
    private, identity = _key()
    challenge = _challenge(self_api, identity)
    denied = self_api.client.post(REGISTER_PATH, json=_signed(private, identity, challenge))
    assert denied.status_code == 503
    assert len(bounds) == 31 and set(bounds) == {10**9}
    assert self_api.session.scalar(select(func.count()).select_from(Device)) == 1
    assert self_api.session.scalar(select(func.count()).select_from(DeviceMachineIdentity)) == 0
    assert self_api.session.scalar(select(DeviceSelfEnrollmentChallenge)).consumed_at is None


def test_credential_configuration_failure_rolls_back_claim_and_allocation(self_api, monkeypatch):
    private, identity = _key()
    challenge = _challenge(self_api, identity)
    payload = _signed(private, identity, challenge)
    with monkeypatch.context() as patch:
        patch.setattr(settings, "jwt_secret", "")
        denied = self_api.client.post(REGISTER_PATH, json=payload)
    assert denied.status_code == 503
    assert self_api.session.scalar(select(func.count()).select_from(Device)) == 1
    assert self_api.session.scalar(select(func.count()).select_from(DeviceMachineIdentity)) == 0
    assert self_api.session.scalar(select(DeviceSelfEnrollmentChallenge)).consumed_at is None
    assert self_api.client.post(REGISTER_PATH, json=payload).status_code == 200


@pytest.mark.parametrize("limit", ["global", "ip", "key"])
def test_database_shared_challenge_rate_limits(self_api, monkeypatch, limit):
    monkeypatch.setattr(settings, "device_self_enrollment_" + limit + "_per_minute", 1)
    _, identity = _key()
    _challenge(self_api, identity)
    other = identity if limit == "key" else _key(19)[1]
    denied = self_api.client.post(CHALLENGE_PATH, json=other)
    assert denied.status_code == 429 and denied.headers["retry-after"] == "60"
    assert self_api.session.scalar(select(func.count()).select_from(DeviceSelfEnrollmentChallenge)) == 1


def test_old_challenges_are_cleaned_and_budget_reopens(self_api):
    _, identity = _key()
    _challenge(self_api, identity)
    stored = self_api.session.scalar(select(DeviceSelfEnrollmentChallenge))
    stored.issued_at = datetime.now(UTC) - timedelta(minutes=4)
    stored.expires_at = datetime.now(UTC) - timedelta(minutes=3)
    self_api.session.commit()
    _challenge(self_api, identity)
    assert self_api.session.scalar(select(func.count()).select_from(DeviceSelfEnrollmentChallenge)) == 1


def test_disabled_server_and_invalid_key_fail_closed(self_api, monkeypatch):
    _, identity = _key()
    monkeypatch.setattr(settings, "device_self_enrollment_enabled", False)
    assert self_api.client.post(CHALLENGE_PATH, json=identity).status_code == 503
    monkeypatch.setattr(settings, "device_self_enrollment_enabled", True)
    assert self_api.client.post(CHALLENGE_PATH, json={**identity, "key_id": "00" * 32}).status_code == 401
    assert self_api.client.post(CHALLENGE_PATH, json={**identity, "protocol_version": True}).status_code == 422


def test_registration_utf8_limit_is_bytes_and_forbids_envelope_extras():
    private, identity = _key()
    challenge = {"challenge_id": "11" * 16, "nonce": "22" * 32}
    payload = _signed(private, identity, challenge)
    for changes in ({"registration_json": "中" * 3000}, {"access_token": "not-accepted"},
                    {"expected_device_id": "unsigned-outer-field"}):
        with pytest.raises(ValidationError):
            DeviceSelfRegisterRequest.model_validate({**payload, **changes})


def test_forwarded_ip_used_only_for_explicit_socket_proxy():
    def request(peer, values):
        return Request({"type": "http", "client": (peer, 1234), "headers": values})
    spoof = request("192.0.2.10", [(b"x-real-ip", b"198.51.100.20")])
    assert enrollment_peer_ip(spoof, "") == "192.0.2.10"
    assert enrollment_peer_ip(spoof, "127.0.0.1") == "192.0.2.10"
    trusted = request("127.0.0.1", [(b"x-real-ip", b"198.51.100.20")])
    assert enrollment_peer_ip(trusted, "127.0.0.1/32") == "198.51.100.20"
    for headers in ([(b"x-real-ip", b"1.2.3.4, 5.6.7.8")], [(b"x-real-ip", b"1.2.3.4"), (b"x-real-ip", b"5.6.7.8")]):
        with pytest.raises(DeviceEnrollmentError):
            enrollment_peer_ip(request("127.0.0.1", headers), "127.0.0.1")


@pytest.mark.parametrize("blocked_lock", ["global", "serial"])
def test_expiry_rechecked_after_lock_wait_before_consumption(self_api, monkeypatch, blocked_lock):
    private, identity = _key()
    clock = [datetime.now(UTC)]
    session = EnrollmentSessionShim(self_api.session)
    service = DeviceSelfEnrollmentService(session, api_url=API_URL,
        serial_pepper=SERIAL_PEPPER, now=lambda: clock[0])

    async def setup():
        challenge = await service.issue(DeviceSelfEnrollmentChallengeRequest(**identity), peer_ip="192.0.2.10")
        await session.commit()
        return challenge.model_dump()
    challenge = asyncio.run(setup())
    original_lock = enrollment._serial_lock

    @asynccontextmanager
    async def blocked(session, digest):
        async with original_lock(session, digest):
            if (digest == enrollment._LOCK_ID) == (blocked_lock == "global"):
                clock[0] += timedelta(seconds=90)
            yield
    monkeypatch.setattr(enrollment, "_serial_lock", blocked)

    async def consume():
        try:
            with pytest.raises(DeviceEnrollmentError):
                await service.register(DeviceSelfRegisterRequest.model_validate(_signed(private, identity, challenge)))
        finally:
            await session.rollback()
    asyncio.run(consume())
    assert self_api.session.scalar(select(func.count()).select_from(DeviceMachineIdentity)) == 0
    stored = self_api.session.scalar(select(DeviceSelfEnrollmentChallenge))
    assert stored.consumed_at is None
    assert self_api.session.scalar(select(func.count()).select_from(Device)) == 1


def test_issue_expiry_starts_after_global_lock_wait(self_api, monkeypatch):
    _, identity = _key()
    clock = [datetime.now(UTC)]
    before = clock[0]
    original_lock = enrollment._serial_lock

    @asynccontextmanager
    async def blocked(session, digest):
        async with original_lock(session, digest):
            clock[0] += timedelta(seconds=90)
            yield
    monkeypatch.setattr(enrollment, "_serial_lock", blocked)
    service = DeviceSelfEnrollmentService(EnrollmentSessionShim(self_api.session), api_url=API_URL,
        serial_pepper=SERIAL_PEPPER, now=lambda: clock[0])
    result = asyncio.run(service.issue(DeviceSelfEnrollmentChallengeRequest(**identity), peer_ip="192.0.2.10"))
    assert result.expires_at_ms == int((before + timedelta(seconds=150)).timestamp() * 1000)


@asynccontextmanager
async def _postgres_sessions():
    assert DATABASE_URL
    schema = "self_enrollment_" + uuid4().hex
    admin = create_async_engine(_asyncpg_url(DATABASE_URL))
    async with admin.begin() as connection:
        await connection.execute(text(f'CREATE SCHEMA "{schema}"'))
    engine = create_async_engine(_asyncpg_url(DATABASE_URL),
        connect_args={"server_settings": {"search_path": schema}})
    sessions = async_sessionmaker(engine, expire_on_commit=False)
    try:
        async with engine.begin() as connection:
            await connection.run_sync(lambda sync: Base.metadata.create_all(sync,
                tables=[table for table in Base.metadata.sorted_tables if table.name not in SELF_ENROLLMENT_TABLES]))
            await migrate(connection, schema=schema)
            await migrate(connection, schema=schema)  # Repeated startup is read-only/idempotent.
        yield sessions, engine, schema
    finally:
        await engine.dispose()
        async with admin.begin() as connection:
            await connection.execute(text(f'DROP SCHEMA "{schema}" CASCADE'))
        await admin.dispose()


def _service(session):
    return DeviceSelfEnrollmentService(session, api_url=API_URL, serial_pepper=SERIAL_PEPPER)


@pytest.mark.skipif(not DATABASE_URL, reason="MRD_TEST_DATABASE_URL is not configured")
@pytest.mark.asyncio
@pytest.mark.parametrize("same_challenge", [True, False])
async def test_postgres_concurrent_claims_once_and_same_key_one_code(same_challenge):
    private, identity = _key()
    async with _postgres_sessions() as (sessions, _, _):
        challenges = []
        for _ in range(1 if same_challenge else 2):
            async with sessions.begin() as session:
                challenge = await _service(session).issue(DeviceSelfEnrollmentChallengeRequest(**identity), peer_ip="192.0.2.10")
                challenges.append(challenge.model_dump())
        async def consume(index):
            try:
                async with sessions.begin() as session:
                    device = await _service(session).register(DeviceSelfRegisterRequest.model_validate(
                        _signed(private, identity, challenges[0 if same_challenge else index])))
                    return device.device_id
            except DeviceEnrollmentError as error:
                return error.code
        results = await asyncio.wait_for(asyncio.gather(consume(0), consume(1)), timeout=15)
        if same_challenge:
            assert sum(result == "device_self_enrollment_invalid" for result in results) == 1
        else:
            assert results[0] == results[1] and results[0].isdigit()
        async with sessions() as session:
            assert await session.scalar(select(func.count()).select_from(DeviceMachineIdentity)) == 1
            assert await session.scalar(select(func.count()).select_from(Device)) == 1


@pytest.mark.skipif(not DATABASE_URL, reason="MRD_TEST_DATABASE_URL is not configured")
@pytest.mark.asyncio
async def test_postgres_migration_verifies_schema_drift():
    async with _postgres_sessions() as (_, engine, schema):
        async with engine.begin() as connection:
            await connection.execute(text(f'ALTER TABLE "{schema}".device_self_enrollment_challenges DROP CONSTRAINT ck_device_self_expiry'))
        with pytest.raises(DeviceSelfEnrollmentMigrationError):
            async with engine.begin() as connection:
                await migrate(connection, schema=schema)
