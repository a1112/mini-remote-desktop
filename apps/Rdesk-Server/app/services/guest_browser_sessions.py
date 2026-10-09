"""Temporary password principals are not users and cannot acquire device tokens."""

from datetime import UTC, datetime
import hashlib, secrets
from sqlalchemy import select
from app.models.browser_controller import BrowserController
from app.models.device import Device
from app.models.device_temporary_access import DeviceTemporaryAccess
from app.models.session_request import SessionRequest
from app.models.device_machine_identity import DeviceMachineIdentity
from app.schemas.session import DeviceSessionCreateIn
from app.services.browser_authority import browser_authority_valid, utc
from app.services.browser_sessions import BrowserSessionService
from app.services.device_principal_keys import (
    key_is_physical_machine,
    principal_key_lock,
)
from app.services.device_sessions import (
    DeviceSessionError,
    DeviceSessionService,
    canonical_wan_request,
    wan_request_commitment,
)
from app.services.session_grants import session_grant_identity_lock
from app.services.temporary_access import invalid


class GuestBrowserSessionService:
    def __init__(self, db, temporary):
        self.db = db
        self.temporary = temporary

    async def create(self, payload):
        key_id = hashlib.sha256(bytes(payload.controller_public_key)).hexdigest()
        target = await self.db.scalar(
            select(Device).where(
                Device.device_id == payload.target_device_id,
                Device.principal_kind == "physical",
            )
        )
        access = (
            await self.db.scalar(
                select(DeviceTemporaryAccess).where(
                    DeviceTemporaryAccess.device_row_id == target.id
                )
            )
            if target
            else None
        )
        original = (
            (
                access.generation,
                access.publication_digest,
                access.target_auth_version,
                access.key_id,
            )
            if access
            else None
        )
        valid_password = await self.temporary.verify_password(
            payload.temporary_password.get_secret_value(), access
        )
        if not valid_password or target is None or access is None:
            invalid()
        async with principal_key_lock(self.db, key_id), session_grant_identity_lock(
            self.db, "session:" + payload.session_id
        ):
            if await key_is_physical_machine(self.db, key_id):
                invalid()
            target = await self.db.scalar(
                select(Device)
                .where(Device.id == target.id)
                .with_for_update()
                .execution_options(populate_existing=True)
            )
            access = await self.db.scalar(
                select(DeviceTemporaryAccess)
                .where(DeviceTemporaryAccess.device_row_id == target.id)
                .with_for_update()
                .execution_options(populate_existing=True)
            )
            now = datetime.now(UTC)
            mapping = await self.db.scalar(
                select(DeviceMachineIdentity)
                .where(DeviceMachineIdentity.device_row_id == target.id)
                .execution_options(populate_existing=True)
            )
            if (
                access is None
                or original
                != (
                    access.generation,
                    access.publication_digest,
                    access.target_auth_version,
                    access.key_id,
                )
                or not access.enabled
                or access.expires_at is None
                or utc(access.expires_at) <= now
                or target.auth_revoked_at is not None
                or target.auth_version != access.target_auth_version
                or mapping is None
                or mapping.key_id != access.key_id
                or any(
                    scope not in access.allowed_scopes
                    for scope in payload.requested_scopes
                )
            ):
                invalid()
            row = await self.db.scalar(
                select(SessionRequest)
                .where(SessionRequest.id == payload.session_id)
                .with_for_update()
                .execution_options(populate_existing=True)
            )
            request = DeviceSessionCreateIn.model_validate(
                payload.model_dump(
                    exclude={"controller_public_key", "temporary_password"}
                )
            )
            if row is not None:
                principal = await self.db.scalar(
                    select(BrowserController).where(
                        BrowserController.session_id == row.id
                    )
                )
                shadow = await self.db.scalar(
                    select(Device).where(Device.id == row.requester_device_id)
                )
                if (
                    principal is None
                    or shadow is None
                    or principal.authority_kind != "temporary_password"
                    or principal.public_key != bytes(payload.controller_public_key)
                    or not await browser_authority_valid(self.db, row)
                    or row.status not in {"requested", "approved"}
                    or row.request_payload
                    != canonical_wan_request(request, current_device=shadow).model_dump(
                        mode="json"
                    )
                ):
                    raise DeviceSessionError(
                        "guest_session_conflict", 409, "Guest session state conflicts"
                    )
                return row, principal, shadow
            shadow = Device(
                name="Guest browser",
                device_id="browser_" + secrets.token_hex(16),
                os="Web browser",
                principal_kind="browser_controller",
                tenant_id=target.tenant_id,
                is_bound=False,
                bound_user_id=None,
                auth_version=1,
            )
            self.db.add(shadow)
            await self.db.flush()
            canonical = canonical_wan_request(request, current_device=shadow)
            normalized = canonical.model_dump(mode="json")
            row = SessionRequest(
                id=canonical.session_id,
                requester_user_id=None,
                requester_device_id=shadow.id,
                target_device_id=target.id,
                signaling_room=canonical.session_id,
                tenant_id=target.tenant_id,
                status="requested",
                authority_kind="temporary_password",
                temporary_access_generation=access.generation,
                target_auth_version=target.auth_version,
                authority_expires_at=access.expires_at,
                request_payload=normalized,
                request_commitment=wan_request_commitment(canonical),
                access_mode="attended",
                route_policy=canonical.route_policy,
                requested_scopes=list(canonical.requested_scopes),
                requested_profile=normalized["requested_profile"],
            )
            self.db.add(row)
            await self.db.flush()
            principal = BrowserController(
                device_row_id=shadow.id,
                session_id=row.id,
                user_id=None,
                user_session_version=None,
                tenant_id=target.tenant_id,
                target_device_row_id=target.id,
                public_key=bytes(payload.controller_public_key),
                key_id=key_id,
                allowed_scopes=list(canonical.requested_scopes),
                created_at=now,
                expires_at=access.expires_at,
                authority_kind="temporary_password",
                temporary_access_generation=access.generation,
                target_auth_version=target.auth_version,
            )
            self.db.add(principal)
            await self.db.flush()
            return row, principal, shadow

    async def inspect(self, *, session_id, claims, allow_terminal=False):
        row = await self.db.scalar(
            select(SessionRequest)
            .where(SessionRequest.id == session_id)
            .execution_options(populate_existing=True)
        )
        principal = await self.db.scalar(
            select(BrowserController)
            .where(BrowserController.session_id == session_id)
            .execution_options(populate_existing=True)
        )
        if (
            row is None
            or principal is None
            or claims["session_id"] != session_id
            or row.authority_kind != "temporary_password"
            or principal.authority_kind != "temporary_password"
            or principal.key_id != claims["device_key_id"]
            or principal.temporary_access_generation
            != claims["temporary_access_generation"]
            or principal.target_auth_version != claims["target_auth_version"]
            or principal.tenant_id != claims["tenant_id"]
            or claims["request_commitment"] != row.request_commitment
            or claims["allowed_scopes"] != principal.allowed_scopes
            or claims["target_device_id"] != row.request_payload.get("target_device_id")
            or not await browser_authority_valid(
                self.db, row, allow_terminal=allow_terminal
            )
        ):
            BrowserSessionService.unavailable()
        shadow = await self.db.scalar(
            select(Device).where(Device.id == principal.device_row_id)
        )
        if shadow is None or shadow.device_id != claims["device_id"]:
            BrowserSessionService.unavailable()
        return row, principal, shadow

    async def revalidate_for_credential(self, binding):
        async with session_grant_identity_lock(
            self.db, "session:" + binding.session_id
        ):
            devices = list(
                await self.db.scalars(
                    select(Device)
                    .where(
                        Device.id.in_(
                            {binding.controller_row_id, binding.target_row_id}
                        )
                    )
                    .order_by(Device.id)
                    .with_for_update()
                    .execution_options(populate_existing=True)
                )
            )
            devices = {device.id: device for device in devices}
            shadow = devices.get(binding.controller_row_id)
            target = devices.get(binding.target_row_id)
            mapping = await self.db.scalar(
                select(DeviceMachineIdentity)
                .where(DeviceMachineIdentity.device_row_id == binding.target_row_id)
                .execution_options(populate_existing=True)
            )
            if mapping is None or mapping.key_id != binding.target_key_id:
                from fastapi import HTTPException

                raise HTTPException(
                    503,
                    detail={
                        "code": "browser_identity_unavailable",
                        "message": "Trusted target connection identity is unavailable",
                    },
                )
            row = await self.db.scalar(
                select(SessionRequest)
                .where(SessionRequest.id == binding.session_id)
                .with_for_update()
                .execution_options(populate_existing=True)
            )
            principal = await self.db.scalar(
                select(BrowserController)
                .where(BrowserController.session_id == binding.session_id)
                .with_for_update()
                .execution_options(populate_existing=True)
            )
            if (
                row is None
                or principal is None
                or shadow is None
                or target is None
                or binding.authority_kind != "temporary_password"
                or row.tenant_id != binding.tenant_id
                or principal.tenant_id != binding.tenant_id
                or shadow.tenant_id != binding.tenant_id
                or target.tenant_id != binding.tenant_id
                or row.temporary_access_generation
                != binding.temporary_access_generation
                or principal.temporary_access_generation
                != binding.temporary_access_generation
                or row.target_auth_version != binding.target_auth_version
                or principal.target_auth_version != binding.target_auth_version
                or row.authority_kind != "temporary_password"
                or row.requester_user_id is not None
                or principal.authority_kind != "temporary_password"
                or principal.user_id is not None
                or principal.user_session_version is not None
                or shadow.principal_kind != "browser_controller"
                or shadow.is_bound
                or shadow.bound_user_id is not None
                or target.principal_kind != "physical"
                or row.requester_device_id != shadow.id
                or row.target_device_id != target.id
                or shadow.device_id != binding.controller_device_id
                or target.device_id != binding.target_device_id
                or principal.key_id != binding.controller_key_id
                or row.request_commitment != binding.request_commitment
                or row.status not in {"requested", "approved"}
                or not await browser_authority_valid(self.db, row)
            ):
                BrowserSessionService.unavailable()
            return row, principal, shadow

    async def close(self, *, session_id, claims):
        row, principal, shadow = await self.inspect(
            session_id=session_id, claims=claims, allow_terminal=True
        )
        row = await DeviceSessionService(self.db).transition(
            session_id=session_id, current_device=shadow, action="close"
        )
        principal.revoked_at = datetime.now(UTC)
        await self.db.flush()
        return row
