"""Serialize and keep browser controller keys separate from resident machine keys."""
from sqlalchemy import select

from app.models.browser_controller import BrowserController
from app.models.device_machine_identity import DeviceMachineIdentity
from app.services.session_grants import session_grant_identity_lock


def principal_key_lock(session, key_id):
    return session_grant_identity_lock(session, "principal-key:" + key_id)


async def key_is_browser_controller(session, key_id):
    return await session.scalar(select(BrowserController.device_row_id)
        .where(BrowserController.key_id == key_id)) is not None


async def key_is_physical_machine(session, key_id):
    return await session.scalar(select(DeviceMachineIdentity.device_row_id)
        .where(DeviceMachineIdentity.key_id == key_id)) is not None
