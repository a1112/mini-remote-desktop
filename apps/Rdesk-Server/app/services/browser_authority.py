"""Recheck browser authority at every native session and relay boundary."""
from datetime import UTC, datetime
import hashlib
from sqlalchemy import select

from app.models.browser_controller import BrowserController
from app.models.device import Device
from app.models.user import User


def utc(value):
    return value.replace(tzinfo=UTC) if value.tzinfo is None else value.astimezone(UTC)


async def browser_authority_valid(db, row, *, now=None, allow_terminal=False):
    controller = await db.scalar(select(Device).where(Device.id == row.requester_device_id)
                                 .execution_options(populate_existing=True))
    if controller is None:
        return False
    if getattr(controller, "principal_kind", "physical") != "browser_controller":
        return True
    if allow_terminal and row.status in {"closed", "rejected", "revoked", "expired"}:
        return True
    if row.status == "approved" and "screen.view" not in (row.approved_scopes or []):
        return False
    at = utc(now or datetime.now(UTC))
    if row.status == "approved" and any(not isinstance(expiry, datetime) or utc(expiry) <= at
        for expiry in (row.grant_expires_at, row.policy_expires_at)):
        return False
    principal = await db.scalar(select(BrowserController)
        .where(BrowserController.device_row_id == controller.id)
        .execution_options(populate_existing=True))
    if getattr(row,'authority_kind','account') == 'temporary_password':
        from app.models.device_temporary_access import DeviceTemporaryAccess
        from app.models.device_machine_identity import DeviceMachineIdentity
        target = await db.scalar(select(Device).where(Device.id == row.target_device_id).execution_options(populate_existing=True))
        access = await db.scalar(select(DeviceTemporaryAccess).where(DeviceTemporaryAccess.device_row_id == row.target_device_id).execution_options(populate_existing=True))
        mapping = await db.scalar(select(DeviceMachineIdentity).where(DeviceMachineIdentity.device_row_id == row.target_device_id).execution_options(populate_existing=True))
        return bool(principal is not None and target is not None and access is not None and mapping is not None
            and principal.authority_kind == 'temporary_password' and row.requester_user_id is None
            and principal.user_id is None and principal.user_session_version is None
            and not controller.is_bound and controller.bound_user_id is None
            and target.principal_kind == 'physical' and target.auth_revoked_at is None
            and target.auth_version == row.target_auth_version == principal.target_auth_version == access.target_auth_version
            and access.enabled and mapping.key_id == access.key_id
            and (row.status == 'approved' or (row.temporary_access_generation == access.generation and access.expires_at is not None and utc(access.expires_at) > at))
            and principal.temporary_access_generation == row.temporary_access_generation
            and row.authority_expires_at is not None and utc(row.authority_expires_at) > at
            and principal.revoked_at is None and utc(principal.expires_at) > at
            and utc(principal.expires_at) == utc(row.authority_expires_at)
            and principal.session_id == row.id and principal.target_device_row_id == target.id
            and principal.tenant_id == row.tenant_id == target.tenant_id == controller.tenant_id
            and principal.allowed_scopes == row.requested_scopes
            and hashlib.sha256(principal.public_key).hexdigest() == principal.key_id
            and controller.auth_revoked_at is None)
    if getattr(row,'authority_kind','account') != 'account' or principal is None or principal.authority_kind != 'account':
        return False
    user = await db.scalar(select(User).where(User.id == row.requester_user_id)
                           .execution_options(populate_existing=True))
    target = await db.scalar(select(Device).where(Device.id == row.target_device_id)
                             .execution_options(populate_existing=True))
    return bool(principal is not None and user is not None
        and target is not None and target.principal_kind == "physical"
        and not target.device_id.startswith("browser_") and target.is_bound
        and target.auth_revoked_at is None and target.tenant_id == user.tenant_id
        and target.bound_user_id is not None
        and (user.role == "admin" or target.bound_user_id == user.id)
        and principal.revoked_at is None and utc(principal.expires_at) > at
        and user.role in {"user", "admin"}
        and principal.session_id == row.id and principal.user_id == row.requester_user_id
        and principal.target_device_row_id == row.target_device_id
        and principal.tenant_id == row.tenant_id == user.tenant_id == controller.tenant_id
        and principal.user_session_version == user.session_version
        and principal.allowed_scopes == row.requested_scopes
        and hashlib.sha256(principal.public_key).hexdigest() == principal.key_id
        and controller.auth_revoked_at is None)
