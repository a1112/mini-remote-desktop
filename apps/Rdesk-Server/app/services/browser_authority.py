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
