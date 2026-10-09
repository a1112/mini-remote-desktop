"""Signed resident publication stores only peppered password verification data."""

import asyncio
from datetime import UTC, datetime, timedelta
import hashlib, hmac, ipaddress, json, struct
from urllib.parse import urlsplit
from cryptography.exceptions import InvalidSignature
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey
from pydantic import ValidationError
from sqlalchemy import delete, func, select, update
from app.models.device import Device
from app.models.browser_controller import BrowserController
from app.models.device_machine_identity import DeviceMachineIdentity
from app.models.device_temporary_access import DeviceTemporaryAccess, GuestAccessAttempt
from app.models.session_request import SessionRequest
from app.models.relay_reservation import RelayReservation
from app.schemas.guest_browser import TemporaryAccessDocument, TemporaryAccessStatus
from app.services.device_enrollment import _serial_lock
from app.services.device_sessions import DeviceSessionError
from app.services.browser_authority import utc
from app.services.device_self_enrollment import (
    _unique_json_object,
    _reject_json_constant,
)

PBKDF2_ITERATIONS = 600_000
DOMAIN = b"MRD_DEVICE_TEMPORARY_ACCESS_V1"
LIMIT_LOCK = hashlib.sha256(b"MRD_GUEST_ACCESS_RATE_LIMIT_V1").hexdigest()


def canonical_temporary_publication(api_url, payload):
    return "\n".join(
        (
            "POST",
            api_url.rstrip("/") + "/devices/temporary-access",
            payload.key_id,
            hashlib.sha256(payload.access_json.encode("utf-8")).hexdigest(),
        )
    ).encode("utf-8")


def contextual_temporary_publication(canonical):
    return (
        b"MRD_CONTEXT_SIGNATURE_V1"
        + struct.pack(">H", len(DOMAIN))
        + DOMAIN
        + struct.pack(">Q", len(canonical))
        + canonical
    )


def snapshot_matches(device, snapshot):
    return bool(
        device.principal_kind == "physical"
        and snapshot.auth_revoked_at is None
        and device.id == snapshot.row_id
        and device.device_id == snapshot.device_id
        and device.auth_version == snapshot.auth_version
        and device.auth_revoked_at is None
        and device.tenant_id == snapshot.tenant_id
        and device.is_bound == snapshot.is_bound
        and device.bound_user_id == snapshot.bound_user_id
    )


def invalid():
    raise DeviceSessionError(
        "guest_access_invalid", 401, "Temporary access is unavailable"
    )


class TemporaryAccessService:
    def __init__(self, db, *, api_url, pepper, now=None):
        url = urlsplit(api_url)
        if (
            url.scheme != "https"
            or not url.hostname
            or url.username
            or url.password
            or url.query
            or url.fragment
            or len(pepper) < 32
        ):
            raise ValueError("temporary access configuration unavailable")
        self.db = db
        self.api_url = api_url.rstrip("/")
        self.pepper = pepper
        self.now = now or (lambda: datetime.now(UTC))

    def verifier_hmac(self, verifier):
        return hmac.digest(
            self.pepper, b"MRD_GUEST_PASSWORD_VERIFIER_V1\0" + verifier, "sha256"
        )

    async def publish(self, *, snapshot, payload):
        try:
            raw = bytes.fromhex(payload.public_key)
            if raw == bytes(32) or not hmac.compare_digest(
                hashlib.sha256(raw).hexdigest(), payload.key_id
            ):
                invalid()
            signed = contextual_temporary_publication(
                canonical_temporary_publication(self.api_url, payload)
            )
            Ed25519PublicKey.from_public_bytes(raw).verify(
                bytes.fromhex(payload.signature), signed
            )
            document = TemporaryAccessDocument.model_validate(
                json.loads(
                    payload.access_json,
                    object_pairs_hook=_unique_json_object,
                    parse_constant=_reject_json_constant,
                )
            )
        except (ValueError, InvalidSignature, ValidationError):
            invalid()
        device = await self.db.scalar(
            select(Device)
            .where(Device.id == snapshot.row_id)
            .with_for_update()
            .execution_options(populate_existing=True)
        )
        mapping = await self.db.scalar(
            select(DeviceMachineIdentity)
            .where(DeviceMachineIdentity.device_row_id == snapshot.row_id)
            .execution_options(populate_existing=True)
        )
        if (
            device is None
            or not snapshot_matches(device, snapshot)
            or mapping is None
            or mapping.key_id != payload.key_id
            or mapping.public_key != payload.public_key
            or document.device_id != device.device_id
            or document.auth_version != device.auth_version
        ):
            invalid()
        now = utc(self.now())
        now_ms = int(now.timestamp() * 1000)
        if document.enabled and not now_ms < document.expires_at_ms <= now_ms + 600_000:
            invalid()
        expires = (
            datetime.fromtimestamp(document.expires_at_ms / 1000, UTC)
            if document.enabled
            else None
        )
        row = await self.db.scalar(
            select(DeviceTemporaryAccess)
            .where(DeviceTemporaryAccess.device_row_id == device.id)
            .with_for_update()
            .execution_options(populate_existing=True)
        )
        # An unkeyed digest of verifier-bearing JSON would itself be an offline
        # password oracle, bypassing verifier_hmac after a database-only leak.
        digest = hmac.digest(
            self.pepper,
            b"MRD_TEMPORARY_PUBLICATION_V1\0" + payload.access_json.encode(),
            "sha256",
        ).hex()
        if row is not None and document.generation <= row.generation:
            if (
                document.generation == row.generation
                and digest == row.publication_digest
            ):
                return self.status(row, device)
            invalid()
        values = dict(
            generation=document.generation,
            target_auth_version=device.auth_version,
            key_id=payload.key_id,
            enabled=document.enabled,
            expires_at=expires,
            salt=bytes.fromhex(document.salt) if document.enabled else None,
            verifier_hmac=(
                self.verifier_hmac(bytes.fromhex(document.verifier))
                if document.enabled
                else None
            ),
            publication_digest=digest,
            allowed_scopes=list(document.allowed_scopes),
            updated_at=now,
        )
        if row is None:
            row = DeviceTemporaryAccess(device_row_id=device.id, **values)
            self.db.add(row)
        else:
            for key, value in values.items():
                setattr(row, key, value)
        affected = list(
            await self.db.scalars(
                select(SessionRequest)
                .where(
                    SessionRequest.target_device_id == device.id,
                    SessionRequest.authority_kind == "temporary_password",
                    SessionRequest.status.in_(
                        ["requested", "approved"]
                        if not document.enabled
                        else ["requested"]
                    ),
                )
                .order_by(SessionRequest.id)
                .with_for_update()
                .execution_options(populate_existing=True)
            )
        )
        for grant in affected:
            grant.status = "revoked"
            if grant.grant_expires_at is not None:
                grant.grant_expires_at = now
            if grant.policy_expires_at is not None:
                grant.policy_expires_at = now
            await self.db.execute(
                update(BrowserController)
                .where(BrowserController.session_id == grant.id)
                .values(revoked_at=now)
            )
            await self.db.execute(
                update(RelayReservation)
                .where(
                    RelayReservation.session_id == grant.id,
                    RelayReservation.expires_at > now,
                )
                .values(expires_at=now, superseded_at=now)
            )
        await self.db.flush()
        return self.status(row, device)

    async def inspect(self, device):
        row = await self.db.scalar(
            select(DeviceTemporaryAccess).where(
                DeviceTemporaryAccess.device_row_id == device.id
            )
        )
        return self.status(row, device)

    def status(self, row, device):
        ready = bool(
            row is not None
            and row.enabled
            and row.expires_at is not None
            and utc(row.expires_at) > utc(self.now())
            and row.target_auth_version == device.auth_version
            and device.auth_revoked_at is None
        )
        return TemporaryAccessStatus(
            enabled=bool(row is not None and row.enabled),
            ready=ready,
            expires_at_ms=(
                int(utc(row.expires_at).timestamp() * 1000)
                if row is not None and row.expires_at
                else None
            ),
            generation=row.generation if row is not None else 0,
            reason=None if ready else "temporary_access_unavailable",
        )

    async def record_attempt(self, *, peer_ip, target_device_id, limits):
        try:
            peer = str(ipaddress.ip_address(peer_ip))
        except ValueError:
            invalid()
        peer_digest = hmac.digest(
            self.pepper, b"MRD_GUEST_ACCESS_PEER_V1\0" + peer.encode(), "sha256"
        ).hex()
        target_digest = hmac.digest(
            self.pepper,
            b"MRD_GUEST_ACCESS_TARGET_V1\0" + target_device_id.encode(),
            "sha256",
        ).hex()
        async with _serial_lock(self.db, LIMIT_LOCK):
            now = utc(self.now())
            cutoff = now - timedelta(seconds=60)
            await self.db.execute(
                delete(GuestAccessAttempt).where(GuestAccessAttempt.issued_at <= cutoff)
            )
            recent = select(func.count()).select_from(GuestAccessAttempt)
            counts = (
                await self.db.scalar(recent),
                await self.db.scalar(
                    recent.where(GuestAccessAttempt.peer_digest == peer_digest)
                ),
                await self.db.scalar(
                    recent.where(GuestAccessAttempt.target_digest == target_digest)
                ),
            )
            if any(int(count or 0) >= limit for count, limit in zip(counts, limits)):
                raise DeviceSessionError(
                    "guest_access_rate_limited", 429, "Temporary access is unavailable"
                )
            self.db.add(
                GuestAccessAttempt(
                    peer_digest=peer_digest, target_digest=target_digest, issued_at=now
                )
            )
            await self.db.flush()
            # Commit the attempt before password verification, so failures and
            # later rollback remain durable and multiple workers share limits.
            await self.db.commit()

    async def verify_password(self, password, access):
        salt = (
            access.salt if access is not None and access.salt is not None else bytes(16)
        )
        expected = (
            access.verifier_hmac
            if access is not None and access.verifier_hmac is not None
            else bytes(32)
        )
        derived = await asyncio.to_thread(
            hashlib.pbkdf2_hmac,
            "sha256",
            password.encode("ascii"),
            salt,
            PBKDF2_ITERATIONS,
        )
        return hmac.compare_digest(self.verifier_hmac(derived), expected)
