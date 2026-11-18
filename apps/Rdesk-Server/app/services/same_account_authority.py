"""Account and signing-key bindings for automatic WAN approval."""

import hashlib

from sqlalchemy import func, or_, select, update

from app.core.config import settings
from app.models.browser_controller import BrowserController
from app.models.device import Device
from app.models.device_machine_identity import DeviceMachineIdentity
from app.models.oidc import OidcIdentity
from app.models.relay_audit_event import RelayAuditEvent
from app.models.relay_reservation import RelayReservation
from app.models.session_request import SessionRequest
from app.models.user import User


def _active_device(device: Device) -> bool:
    return bool(device.is_bound and device.bound_user_id is not None
                and device.auth_revoked_at is None)


def _machine_key_matches(pin: DeviceMachineIdentity | None, key_id: str) -> bool:
    if pin is None or pin.key_id != key_id:
        return False
    try:
        public_key = bytes.fromhex(pin.public_key)
    except (TypeError, ValueError):
        return False
    return len(public_key) == 32 and hashlib.sha256(public_key).hexdigest() == key_id


async def same_account_binding_valid(
    db, row, controller, target, owner, *, controller_key_id, target_key_id,
    lock_identity=False,
) -> bool:
    owner_id = row.requester_user_id
    if (
        settings.oidc_enabled is not True
        or not settings.oidc_issuer
        or row.authority_kind != "account"
        or row.status not in {"requested", "approved"}
        or not isinstance(owner_id, str)
        or controller is None or target is None or owner is None
        or controller.id != row.requester_device_id
        or target.id != row.target_device_id
        or controller.id == target.id
        or owner.id != owner_id
        or controller.bound_user_id != owner_id
        or target.bound_user_id != owner_id
        or controller.tenant_id != row.tenant_id
        or target.tenant_id != row.tenant_id
        or owner.tenant_id != row.tenant_id
        or not _active_device(controller)
        or not _active_device(target)
        or target.principal_kind != "physical"
        or controller.principal_kind not in {"physical", "browser_controller"}
        or not isinstance(controller_key_id, str)
        or not isinstance(target_key_id, str)
        or controller_key_id == target_key_id
    ):
        return False
    statement = select(OidcIdentity).where(
        OidcIdentity.user_id == owner_id, OidcIdentity.issuer == settings.oidc_issuer
    ).execution_options(populate_existing=True)
    if lock_identity:
        # Approval has already locked User. Account linking uses the same order.
        statement = statement.with_for_update()
    if await db.scalar(statement) is None:
        return False

    # Machine pins are immutable. Do not acquire their row locks after device
    # locks: self-enrollment locks the pin first, then the physical device.
    target_pin = await db.scalar(select(DeviceMachineIdentity).where(
        DeviceMachineIdentity.device_row_id == target.id
    ).execution_options(populate_existing=True))
    if not _machine_key_matches(target_pin, target_key_id):
        return False
    if controller.principal_kind == "browser_controller":
        browser = await db.scalar(select(BrowserController).where(
            BrowserController.device_row_id == controller.id,
            BrowserController.session_id == row.id,
        ).execution_options(populate_existing=True))
        return bool(browser is not None and browser.key_id == controller_key_id)
    controller_pin = await db.scalar(select(DeviceMachineIdentity).where(
        DeviceMachineIdentity.device_row_id == controller.id
    ).execution_options(populate_existing=True))
    return _machine_key_matches(controller_pin, controller_key_id)


async def same_account_authority_valid(db, row, *, lock_identity=False) -> bool:
    """Revalidate persisted auto-approval; ordinary attended grants are unchanged."""
    authority = getattr(row, "same_account_authority", None)
    if authority is None:
        return True
    if row.status in {"rejected", "expired", "closed", "revoked"}:
        return True  # Terminal sessions must remain inspectable and closable.
    if (
        row.status != "approved"
        or not isinstance(authority, dict)
        or set(authority) != {"owner_user_id", "issuer", "controller_key_id", "target_key_id",
                              "controller_auth_version", "target_auth_version"}
        or authority.get("owner_user_id") != row.requester_user_id
        or authority.get("issuer") != settings.oidc_issuer
        or type(authority.get("controller_auth_version")) is not int
        or type(authority.get("target_auth_version")) is not int
    ):
        return False
    controller = await db.scalar(select(Device).where(Device.id == row.requester_device_id)
                                 .execution_options(populate_existing=True))
    target = await db.scalar(select(Device).where(Device.id == row.target_device_id)
                             .execution_options(populate_existing=True))
    owner = await db.scalar(select(User).where(User.id == row.requester_user_id)
                            .execution_options(populate_existing=True))
    if (controller is None or target is None
        or controller.auth_version != authority["controller_auth_version"]
        or target.auth_version != authority["target_auth_version"]):
        return False
    return await same_account_binding_valid(
        db, row, controller, target, owner,
        controller_key_id=authority.get("controller_key_id"),
        target_key_id=authority.get("target_key_id"), lock_identity=lock_identity,
    )


async def revoke_same_account_on_unbind(db, device, *, now) -> None:
    """Permanently revoke auto grants before an unbound device can be rebound."""
    rows = list(await db.scalars(
        select(SessionRequest).where(
            or_(SessionRequest.requester_device_id == device.id,
                SessionRequest.target_device_id == device.id),
            SessionRequest.status == "approved",
            SessionRequest.same_account_authority.is_not(None),
        ).order_by(SessionRequest.id).with_for_update().execution_options(populate_existing=True)
    ))
    if not rows:
        return
    for row in rows:
        row.status = "revoked"
        row.grant_expires_at = now
        row.policy_expires_at = now
        db.add(RelayAuditEvent(action="wan_session_revoked", node_id=None,
            actor_id=device.id, details={"session_id": row.id, "status": row.status},
            created_at=now))
    await db.execute(update(RelayReservation).where(
        RelayReservation.session_id.in_([row.id for row in rows]),
        RelayReservation.expires_at > now,
    ).values(expires_at=now, superseded_at=func.coalesce(RelayReservation.superseded_at, now))
      .execution_options(synchronize_session=False))
    await db.flush()
