from datetime import UTC, datetime, timedelta
from dataclasses import dataclass
import hashlib
import secrets

from sqlalchemy import select

from app.models.browser_controller import BrowserController
from app.models.device import Device
from app.models.session_request import SessionRequest
from app.models.user import User
from app.schemas.browser_session import BrowserSessionCreateIn
from app.schemas.session import DeviceSessionCreateIn
from app.services.browser_authority import browser_authority_valid
from app.services.device_sessions import DeviceSessionError, DeviceSessionService
from app.services.session_grants import session_grant_identity_lock
from app.services.device_principal_keys import key_is_physical_machine, principal_key_lock


BROWSER_LIFETIME_SECONDS = 600


@dataclass(frozen=True)
class BrowserCredentialBinding:
    session_id: str
    user_id: str
    user_session_version: int
    tenant_id: str
    controller_row_id: str
    controller_device_id: str
    controller_key_id: str
    target_row_id: str
    target_device_id: str
    request_commitment: str


class BrowserSessionService:
    def __init__(self, db):
        self.db = db

    async def create(self, *, user, user_version, payload: BrowserSessionCreateIn):
        user_role, user_tenant = user.role, user.tenant_id
        key_id = hashlib.sha256(bytes(payload.controller_public_key)).hexdigest()
        async with principal_key_lock(self.db, key_id), session_grant_identity_lock(self.db, "browser:" + payload.session_id):
            if await key_is_physical_machine(self.db, key_id):
                self.conflict()
            owner = await self.db.scalar(select(User).where(User.id == user.id)
                                         .execution_options(populate_existing=True))
            if (owner is None or owner.session_version != user_version
                or owner.role != user_role or owner.tenant_id != user_tenant):
                self.unavailable()
            existing = await self.db.scalar(select(SessionRequest).where(
                SessionRequest.id == payload.session_id).execution_options(populate_existing=True))
            if existing is not None:
                principal = await self.db.scalar(select(BrowserController).where(
                    BrowserController.session_id == existing.id))
                if (principal is None or principal.user_id != owner.id
                    or principal.public_key != bytes(payload.controller_public_key)
                    or not await browser_authority_valid(self.db, existing)
                    or existing.status not in {"requested", "approved"}):
                    self.conflict()
                shadow = await self.db.scalar(select(Device).where(Device.id == principal.device_row_id))
            else:
                target = await self.db.scalar(select(Device).where(
                    Device.device_id == payload.target_device_id,
                    Device.principal_kind == "physical",
                    Device.tenant_id == owner.tenant_id,
                    Device.is_bound.is_(True), Device.auth_revoked_at.is_(None))
                    .with_for_update().execution_options(populate_existing=True))
                if (target is None or target.bound_user_id is None
                    or (owner.role != "admin" and target.bound_user_id != owner.id)):
                    self.unavailable()
                shadow = Device(name="Web browser", device_id="browser_" + secrets.token_hex(16),
                    os="Web browser", principal_kind="browser_controller", tenant_id=owner.tenant_id,
                    is_bound=True, bound_user_id=owner.id, auth_version=1)
                self.db.add(shadow)
                await self.db.flush()
            request = DeviceSessionCreateIn.model_validate(payload.model_dump(
                exclude={"controller_public_key"}))
            row = await DeviceSessionService(self.db).create(current_device=shadow, payload=request)
            # The native service has now locked devices, then users. Recheck the
            # original bearer snapshot after those locks, without reversing their order.
            if owner.session_version != user_version or owner.role != user_role or owner.tenant_id != user_tenant:
                self.unavailable()
            if existing is None:
                now = datetime.now(UTC)
                principal = BrowserController(device_row_id=shadow.id, session_id=row.id,
                    user_id=owner.id, tenant_id=owner.tenant_id, user_session_version=user_version,
                    target_device_row_id=row.target_device_id,
                    public_key=bytes(payload.controller_public_key),
                    key_id=key_id,
                    allowed_scopes=list(payload.requested_scopes), created_at=now,
                    expires_at=now + timedelta(seconds=BROWSER_LIFETIME_SECONDS))
                self.db.add(principal)
                await self.db.flush()
            return row, principal, shadow

    async def inspect(self, *, session_id, user, allow_terminal=False):
        row = await self.db.scalar(select(SessionRequest).where(SessionRequest.id == session_id)
                                   .execution_options(populate_existing=True))
        principal = await self.db.scalar(select(BrowserController).where(
            BrowserController.session_id == session_id).execution_options(populate_existing=True))
        if (row is None or principal is None or row.requester_user_id != user.id
            or row.tenant_id != user.tenant_id or principal.user_id != user.id
            or principal.user_session_version != user.session_version
            or (not allow_terminal and row.status not in {"requested", "approved"})
            or not await browser_authority_valid(self.db, row, allow_terminal=allow_terminal)):
            self.unavailable()
        shadow = await self.db.scalar(select(Device).where(Device.id == principal.device_row_id))
        return row, principal, shadow

    async def revalidate_for_credential(self, binding: BrowserCredentialBinding):
        """Reload authority after network awaits, under the native row-lock order."""
        async with session_grant_identity_lock(self.db, "session:" + binding.session_id):
            devices = list(await self.db.scalars(select(Device)
                .where(Device.id.in_({binding.controller_row_id, binding.target_row_id}))
                .order_by(Device.id).with_for_update().execution_options(populate_existing=True)))
            by_device_id = {device.id: device for device in devices}
            shadow = by_device_id.get(binding.controller_row_id)
            target = by_device_id.get(binding.target_row_id)
            if (shadow is None or target is None
                or shadow.principal_kind != "browser_controller"
                or not shadow.is_bound or shadow.bound_user_id != binding.user_id
                or target.principal_kind != "physical"
                or shadow.device_id != binding.controller_device_id
                or target.device_id != binding.target_device_id):
                self.unavailable()
            user_ids = sorted({user_id for user_id in
                (binding.user_id, shadow.bound_user_id, target.bound_user_id)
                if isinstance(user_id, str)})
            users = list(await self.db.scalars(select(User).where(User.id.in_(user_ids))
                .order_by(User.id).with_for_update().execution_options(populate_existing=True)))
            owner = {user.id: user for user in users}.get(binding.user_id)
            row = await self.db.scalar(select(SessionRequest).where(SessionRequest.id == binding.session_id)
                .with_for_update().execution_options(populate_existing=True))
            principal = await self.db.scalar(select(BrowserController)
                .where(BrowserController.session_id == binding.session_id)
                .with_for_update().execution_options(populate_existing=True))
            if (row is None or principal is None or owner is None
                or owner.session_version != binding.user_session_version
                or owner.tenant_id != binding.tenant_id
                or row.status not in {"requested", "approved"}
                or row.requester_user_id != binding.user_id
                or row.requester_device_id != binding.controller_row_id
                or row.target_device_id != binding.target_row_id
                or row.request_commitment != binding.request_commitment
                or not isinstance(row.request_payload, dict)
                or row.request_payload.get("target_device_id") != binding.target_device_id
                or row.request_payload.get("controller_device_id") != binding.controller_device_id
                or principal.user_id != binding.user_id
                or principal.user_session_version != binding.user_session_version
                or principal.device_row_id != binding.controller_row_id
                or principal.key_id != binding.controller_key_id
                or not await browser_authority_valid(self.db, row)):
                self.unavailable()
            return row, principal, shadow

    async def close(self, *, session_id, user):
        row, principal, shadow = await self.inspect(session_id=session_id, user=user, allow_terminal=True)
        row = await DeviceSessionService(self.db).transition(
            session_id=session_id, current_device=shadow, action="close")
        principal.revoked_at = datetime.now(UTC)
        await self.db.flush()
        return row

    @staticmethod
    def unavailable():
        raise DeviceSessionError("browser_session_not_found", 404, "Browser session is unavailable")

    @staticmethod
    def conflict():
        raise DeviceSessionError("browser_session_conflict", 409, "Browser session state conflicts")
