"""Self-service device codes require proof of the resident's durable machine key.

This proves a key, not a hardware serial or account ownership. Ordinary device
HTTP credentials remain bearer credentials; WSS registration separately proves
the pinned machine key using the existing signed challenge protocol.
"""
from __future__ import annotations

import hashlib
import hmac
import ipaddress
import json
import re
import secrets
import struct
from datetime import UTC, datetime, timedelta
from typing import Callable
from urllib.parse import urlsplit

from cryptography.exceptions import InvalidSignature
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey
from pydantic import ValidationError
from sqlalchemy import delete, func, select
from sqlalchemy.exc import IntegrityError

from app.models.device import Device, generate_device_id_from_digest
from app.models.device_machine_identity import DeviceMachineIdentity, DeviceSelfEnrollmentChallenge
from app.models.relay_audit_event import RelayAuditEvent
from app.schemas.device import DeviceRegisterRequest
from app.schemas.device_self_enrollment import (
    DeviceSelfEnrollmentChallengeRequest,
    DeviceSelfEnrollmentChallengeResponse,
    DeviceSelfRegisterRequest,
)
from app.services.device_enrollment import (
    DeviceEnrollmentError,
    _device_identity_constraint,
    _serial_lock,
    _utc,
    device_serial_digest,
)

SIGNATURE_DOMAIN = b"MRD_DEVICE_SELF_ENROLLMENT_V1"
CONTEXT_PREFIX = b"MRD_CONTEXT_SIGNATURE_V1"
_LOCK_ID = hashlib.sha256(b"MRD_DEVICE_SELF_ENROLLMENT_TRANSACTION_V1").hexdigest()
_MAX_CHALLENGES = 10_000


def canonical_self_registration(api_url: str, request: DeviceSelfRegisterRequest) -> bytes:
    """Hash the exact transported UTF-8 string, never reconstructed JSON."""
    body_digest = hashlib.sha256(request.registration_json.encode("utf-8")).hexdigest()
    return "\n".join((
        "POST", api_url + "/devices/self-register", request.challenge_id,
        request.nonce, request.key_id, body_digest,
    )).encode("utf-8")


def contextual_self_registration(canonical: bytes) -> bytes:
    return (CONTEXT_PREFIX + struct.pack(">H", len(SIGNATURE_DOMAIN)) + SIGNATURE_DOMAIN
            + struct.pack(">Q", len(canonical)) + canonical)


def enrollment_peer_ip(request: object, trusted_proxies: str) -> str:
    """Ignore forwarded headers unless the actual socket peer is configured."""
    client = getattr(request, "client", None)
    try:
        peer = ipaddress.ip_address(client.host)
        trusted = [ipaddress.ip_network(value.strip()) for value in trusted_proxies.split(",") if value.strip()]
    except (AttributeError, ValueError):
        raise DeviceEnrollmentError("device_self_enrollment_unavailable", 503, "self enrollment unavailable") from None
    if any(peer in network for network in trusted):
        values = request.headers.getlist("x-real-ip")
        if len(values) == 1:
            try:
                return str(ipaddress.ip_address(values[0]))
            except ValueError:
                _invalid()
        elif values:
            _invalid()
    return str(peer)


class DeviceSelfEnrollmentService:
    def __init__(
        self, session: object, *, api_url: str, serial_pepper: bytes,
        ttl_seconds: int = 60, global_per_minute: int = 1000,
        ip_per_minute: int = 30, key_per_minute: int = 5,
        now: Callable[[], datetime] | None = None,
    ) -> None:
        parts = urlsplit(api_url)
        if (parts.scheme != "https" or not parts.hostname or parts.username or parts.password
                or parts.query or parts.fragment or len(api_url) > 2048
                or any(char.isspace() for char in api_url)):
            raise ValueError("self enrollment origin is unavailable")
        for value, maximum in ((ttl_seconds, 120), (global_per_minute, 5000),
                               (ip_per_minute, 500), (key_per_minute, 30)):
            if type(value) is not int or not 1 <= value <= maximum:
                raise ValueError("self enrollment limits are unavailable")
        if ttl_seconds < 10 or len(serial_pepper) < 32:
            raise ValueError("self enrollment configuration is unavailable")
        self._session = session
        self._api_url = api_url.rstrip("/")
        self._serial_pepper = serial_pepper
        self._ttl = ttl_seconds
        self._limits = global_per_minute, ip_per_minute, key_per_minute
        self._now = now or (lambda: datetime.now(UTC))

    async def issue(
        self, request: DeviceSelfEnrollmentChallengeRequest, *, peer_ip: str,
    ) -> DeviceSelfEnrollmentChallengeResponse:
        _validate_key(request.key_id, request.public_key)
        try:
            peer = str(ipaddress.ip_address(peer_ip))
        except ValueError:
            _invalid()
        peer_digest = hashlib.sha256(b"MRD_SELF_ENROLLMENT_PEER_V1\0" + peer.encode("ascii")).hexdigest()
        # This transaction-scoped PostgreSQL advisory lock serializes counting,
        # deletion and insertion across API processes, not just this worker.
        async with _serial_lock(self._session, _LOCK_ID):
            now = _utc(self._now())
            await self._session.execute(delete(DeviceSelfEnrollmentChallenge).where(
                DeviceSelfEnrollmentChallenge.expires_at < now - timedelta(seconds=60)
            ))
            cutoff = now - timedelta(seconds=60)
            recent = select(func.count()).select_from(DeviceSelfEnrollmentChallenge).where(
                DeviceSelfEnrollmentChallenge.issued_at > cutoff
            )
            counts = (
                await self._session.scalar(recent),
                await self._session.scalar(recent.where(DeviceSelfEnrollmentChallenge.peer_digest == peer_digest)),
                await self._session.scalar(recent.where(DeviceSelfEnrollmentChallenge.key_id == request.key_id)),
            )
            total = await self._session.scalar(select(func.count()).select_from(DeviceSelfEnrollmentChallenge))
            if int(total or 0) >= _MAX_CHALLENGES or any(int(count or 0) >= limit for count, limit in zip(counts, self._limits)):
                raise DeviceEnrollmentError("device_self_enrollment_rate_limited", 429, "self enrollment rate limited")
            challenge_id = secrets.token_hex(16)
            nonce = secrets.token_hex(32)
            expires = now + timedelta(seconds=self._ttl)
            self._session.add(DeviceSelfEnrollmentChallenge(
                challenge_id=challenge_id, nonce_digest=_nonce_digest(nonce),
                key_id=request.key_id, public_key=request.public_key, api_url=self._api_url,
                peer_digest=peer_digest, issued_at=now, expires_at=expires,
            ))
            await self._session.flush()
        return DeviceSelfEnrollmentChallengeResponse(
            challenge_id=challenge_id, nonce=nonce, api_url=self._api_url,
            expires_at_ms=int(expires.timestamp() * 1000),
        )

    async def register(self, request: DeviceSelfRegisterRequest) -> Device:
        public_key = _validate_key(request.key_id, request.public_key)
        async with _serial_lock(self._session, _LOCK_ID):
            now = _utc(self._now())
            challenge = await self._session.scalar(select(DeviceSelfEnrollmentChallenge).where(
                DeviceSelfEnrollmentChallenge.challenge_id == request.challenge_id
            ).with_for_update().execution_options(populate_existing=True))
            if (challenge is None or challenge.consumed_at is not None
                    or _utc(challenge.expires_at) <= now or _utc(challenge.issued_at) > now
                    or challenge.api_url != self._api_url
                    or not hmac.compare_digest(challenge.key_id, request.key_id)
                    or not hmac.compare_digest(challenge.public_key, request.public_key)
                    or not hmac.compare_digest(challenge.nonce_digest, _nonce_digest(request.nonce))):
                _invalid()
            try:
                Ed25519PublicKey.from_public_bytes(public_key).verify(
                    bytes.fromhex(request.signature),
                    contextual_self_registration(canonical_self_registration(self._api_url, request)),
                )
                registration = DeviceRegisterRequest.model_validate(json.loads(
                    request.registration_json, object_pairs_hook=_unique_json_object,
                    parse_constant=_reject_json_constant,
                ))
                if not registration.hostname.strip() or not registration.os_version.strip():
                    raise ValueError("machine description is empty")
            except (InvalidSignature, ValueError, ValidationError):
                _invalid()
            serial_digest = device_serial_digest(registration.motherboard_serial, self._serial_pepper)
            # Lock in the same order as administrator OTP enrollment, so a serial
            # cannot race either route into duplicate or hijacked identities.
            async with _serial_lock(self._session, serial_digest):
                mapping = await self._session.scalar(select(DeviceMachineIdentity).where(
                    DeviceMachineIdentity.key_id == request.key_id
                ).with_for_update().execution_options(populate_existing=True))
                if mapping is not None:
                    if not hmac.compare_digest(mapping.public_key, request.public_key):
                        _invalid()
                    device = await self._session.scalar(select(Device).where(
                        Device.id == mapping.device_row_id
                    ).with_for_update().execution_options(populate_existing=True))
                    if device is None or device.auth_revoked_at is not None:
                        _invalid()
                    if not hmac.compare_digest(device.motherboard_serial_digest or "", serial_digest):
                        _conflict()
                    # A new valid challenge may replace lost local credentials,
                    # but never changes code, account owner, auth version or key.
                    device.hostname = registration.hostname
                    device.os_version = registration.os_version
                    device.os = registration.os_version.split()[0]
                else:
                    existing = await self._session.scalar(select(Device).where(
                        Device.motherboard_serial_digest == serial_digest
                    ).with_for_update().execution_options(populate_existing=True))
                    if existing is not None:
                        # Serial numbers are public metadata, not recovery proof.
                        _conflict()
                    device = await self._allocate_device(registration, serial_digest)
                    self._session.add(DeviceMachineIdentity(
                        key_id=request.key_id, public_key=request.public_key,
                        device_row_id=device.id, created_at=now,
                    ))
                    await self._session.flush()
                # Serial or row locks may have blocked after the first check.
                # Expiry is evaluated at consumption, never at request arrival.
                now = _utc(self._now())
                if _utc(challenge.expires_at) <= now or _utc(challenge.issued_at) > now:
                    _invalid()
                challenge.consumed_at = now
                self._session.add(RelayAuditEvent(
                    action="device_self_enrolled", node_id=None, actor_id=None,
                    details={"device_id": device.device_id, "key_id": request.key_id}, created_at=now,
                ))
                await self._session.flush()
                return device

    async def _allocate_device(self, registration: DeviceRegisterRequest, serial_digest: str) -> Device:
        device_id = generate_device_id_from_digest(serial_digest)
        for attempt in range(32):
            device = Device(
                name=registration.device_name or registration.hostname, device_id=device_id,
                os=registration.os_version.split()[0], os_version=registration.os_version,
                hostname=registration.hostname, motherboard_serial=None,
                motherboard_serial_digest=serial_digest, cpu_info=registration.cpu_info,
                total_memory_mb=registration.total_memory_mb, gpu_info=registration.gpu_info,
                tenant_id="default", is_bound=False, bound_user_id=None,
            )
            savepoint = await self._session.begin_nested()
            try:
                self._session.add(device)
                await self._session.flush()
            except IntegrityError as error:
                await savepoint.rollback()
                constraint = _device_identity_constraint(error)
                if constraint == "serial":
                    _conflict()
                if constraint != "code":
                    raise
                if attempt + 1 < 32:
                    device_id = str(secrets.randbelow(10**10)).zfill(10)
            else:
                await savepoint.commit()
                return device
        raise DeviceEnrollmentError("device_code_unavailable", 503, "device code allocation unavailable")


async def require_pinned_signaling_key(session: object, device_row_id: str, key_id: str) -> None:
    mapping = await session.scalar(select(DeviceMachineIdentity).where(
        DeviceMachineIdentity.device_row_id == device_row_id
    ))
    if mapping is not None and not hmac.compare_digest(mapping.key_id, key_id):
        raise DeviceEnrollmentError("device_machine_key_mismatch", 401, "device machine key mismatch")


def _nonce_digest(nonce: str) -> str:
    return hashlib.sha256(b"MRD_SELF_ENROLLMENT_NONCE_V1\0" + bytes.fromhex(nonce)).hexdigest()


def _unique_json_object(pairs: list[tuple[str, object]]) -> dict[str, object]:
    result: dict[str, object] = {}
    for name, value in pairs:
        if name in result:
            raise ValueError("duplicate registration field")
        result[name] = value
    return result


def _reject_json_constant(_: str) -> object:
    raise ValueError("nonfinite registration number")


def _validate_key(key_id: str, public_key: str) -> bytes:
    if (re.fullmatch(r"[0-9a-f]{64}", key_id) is None
            or re.fullmatch(r"[0-9a-f]{64}", public_key) is None):
        _invalid()
    raw = bytes.fromhex(public_key)
    if raw == bytes(32) or not hmac.compare_digest(hashlib.sha256(raw).hexdigest(), key_id):
        _invalid()
    return raw


def _invalid() -> None:
    raise DeviceEnrollmentError("device_self_enrollment_invalid", 401, "self enrollment proof invalid")


def _conflict() -> None:
    raise DeviceEnrollmentError("device_self_enrollment_conflict", 409, "self enrollment conflicts")
