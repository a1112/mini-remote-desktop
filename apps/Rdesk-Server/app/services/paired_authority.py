"""Backend key/owner eligibility; only the target-local policy establishes trust."""
import hashlib
import re

from sqlalchemy import select

from app.models.browser_controller import BrowserController
from app.models.device import Device
from app.models.device_machine_identity import DeviceMachineIdentity


async def machine_key_id(db, device):
    if device is None or device.principal_kind != "physical" or device.auth_revoked_at is not None:
        return None
    mapping = await db.scalar(select(DeviceMachineIdentity)
        .where(DeviceMachineIdentity.device_row_id == device.id)
        .execution_options(populate_existing=True))
    if mapping is None or any(not isinstance(value, str) or re.fullmatch(r"[0-9a-f]{64}", value) is None
        for value in (mapping.key_id, mapping.public_key)):
        return None
    public = bytes.fromhex(mapping.public_key)
    return mapping.key_id if any(public) and hashlib.sha256(public).hexdigest() == mapping.key_id else None


async def paired_session_authority_valid(db, row, *, controller=None):
    if row.access_mode != "paired":
        return row.access_mode == "attended"
    if not isinstance(row.request_payload, dict) or row.request_payload.get("access_mode") != "paired":
        return False
    if controller is None:
        controller = await db.scalar(select(Device).where(Device.id == row.requester_device_id)
            .execution_options(populate_existing=True))
    target = await db.scalar(select(Device).where(Device.id == row.target_device_id)
        .execution_options(populate_existing=True))
    target_key = await machine_key_id(db, target)
    if controller is None or target_key is None or target_key != row.paired_target_key_id:
        return False
    if controller.tenant_id != row.tenant_id or target.tenant_id != row.tenant_id:
        return False
    if controller.principal_kind == "physical":
        return bool(controller.is_bound and target.is_bound
            and controller.bound_user_id is not None
            and controller.bound_user_id == target.bound_user_id == row.requester_user_id
            and await machine_key_id(db, controller) == row.paired_controller_key_id)
    if controller.principal_kind != "browser_controller":
        return False
    principal = await db.scalar(select(BrowserController)
        .where(BrowserController.device_row_id == controller.id)
        .execution_options(populate_existing=True))
    if principal is None or principal.key_id != row.paired_controller_key_id:
        return False
    if row.authority_kind == "temporary_password":
        return principal.authority_kind == "temporary_password"
    return bool(controller.is_bound and target.is_bound
        and controller.bound_user_id is not None
        and controller.bound_user_id == target.bound_user_id == row.requester_user_id)
