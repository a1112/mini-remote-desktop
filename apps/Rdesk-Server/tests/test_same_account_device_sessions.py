"""The same-account shortcut must retain the normal session security gates."""

from datetime import UTC, datetime
import hashlib

import pytest
from sqlalchemy import func, select

from app.core.config import settings
from app.core.security import create_access_token
from app.models.device_machine_identity import DeviceMachineIdentity
from app.models.oidc import OidcIdentity
from app.models.relay_access_generation import RelayAccessGeneration
from app.models.relay_reservation import RelayReservation
from app.models.session_request import SessionRequest
from app.models.user import User
from test_wan_relay_access import (
    _access,
    _approve,
    _create,
    _headers,
    wan_relay_api,
)


ISSUER = "https://r-auth.example.test/realms/rdesk"
CONTROLLER_PUBLIC = bytes(range(32))
TARGET_PUBLIC = bytes(range(32, 64))
CONTROLLER_KEY = hashlib.sha256(CONTROLLER_PUBLIC).hexdigest()
TARGET_KEY = hashlib.sha256(TARGET_PUBLIC).hexdigest()
KEYS = {"controller_key_id": CONTROLLER_KEY, "target_key_id": TARGET_KEY}
APPROVAL = {
    "approved_scopes": ["input.keyboard", "screen.view"],
    "approved_profile": None,
}


@pytest.fixture
def same_account_api(wan_relay_api, monkeypatch):
    api = wan_relay_api
    monkeypatch.setitem(settings.__dict__, "oidc_enabled", True)
    monkeypatch.setitem(settings.__dict__, "oidc_issuer", ISSUER)
    controller = api.devices["controller-1"]
    target = api.devices["target-1"]
    target.bound_user_id = controller.bound_user_id
    api.session.add(
        OidcIdentity(
            issuer=ISSUER,
            subject="verified-subject",
            user_id=controller.bound_user_id,
            created_at=datetime.now(UTC),
        )
    )
    for device, public in ((controller, CONTROLLER_PUBLIC), (target, TARGET_PUBLIC)):
        api.session.add(
            DeviceMachineIdentity(
                key_id=hashlib.sha256(public).hexdigest(),
                public_key=public.hex(),
                device_row_id=device.id,
                created_at=datetime.now(UTC),
            )
        )
    api.session.commit()
    assert _create(api).status_code == 200
    return api


def _trust(api, *, caller="target-1", keys=KEYS):
    return api.client.get(
        "/api/v1/device-sessions/wan-session-1/same-account",
        headers=_headers(api, caller),
        params=keys,
    )


def _auto_approve(api, *, caller="target-1", keys=KEYS, payload=APPROVAL):
    return api.client.post(
        "/api/v1/device-sessions/wan-session-1/approve-same-account",
        headers=_headers(api, caller),
        params=keys,
        json=payload,
    )


def test_verified_owner_and_exact_registered_keys_only(same_account_api):
    api = same_account_api
    response = _trust(api)
    assert response.status_code == 200 and response.json() == {"trusted": True}
    assert response.headers["cache-control"] == "no-store, private"
    assert _trust(api, caller="controller-1").status_code == 403
    assert _trust(api, caller="unrelated-1").status_code == 404
    assert _trust(api, keys={}).status_code == 400
    assert _trust(api, keys={**KEYS, "controller_key_id": "a" * 64}).json() == {
        "trusted": False
    }
    assert _trust(api, keys={**KEYS, "target_key_id": "b" * 64}).json() == {
        "trusted": False
    }
    assert _auto_approve(api, keys={**KEYS, "controller_key_id": "a" * 64}).status_code == 403
    assert _auto_approve(api, caller="unrelated-1").status_code == 404
    assert _auto_approve(api, caller="controller-1").status_code == 403


@pytest.mark.parametrize("change", ["disabled", "issuer", "identity", "target_key"])
def test_current_verified_identity_and_keys_are_required(same_account_api, monkeypatch, change):
    api = same_account_api
    if change == "disabled":
        monkeypatch.setitem(settings.__dict__, "oidc_enabled", False)
    elif change == "issuer":
        monkeypatch.setitem(settings.__dict__, "oidc_issuer", "https://other.example.test")
    elif change == "identity":
        api.session.delete(api.session.get(OidcIdentity, (ISSUER, "verified-subject")))
        api.session.commit()
    else:
        api.session.delete(api.session.get(DeviceMachineIdentity, TARGET_KEY))
        api.session.commit()
    assert _trust(api).json() == {"trusted": False}
    assert _auto_approve(api).status_code == 403
    assert api.session.get(SessionRequest, "wan-session-1").status == "requested"
    assert api.session.scalar(select(func.count()).select_from(RelayReservation)) == 0


def test_ownership_change_between_check_and_approval_does_not_inherit_trust(same_account_api):
    api = same_account_api
    assert _trust(api).json() == {"trusted": True}
    api.devices["target-1"].bound_user_id = "target-user"
    api.session.commit()
    assert _trust(api).json() == {"trusted": False}
    assert _auto_approve(api).status_code == 403
    assert api.session.get(SessionRequest, "wan-session-1").status == "requested"
    # The existing explicit target approval remains available cross-account.
    assert _approve(api).status_code == 200


def test_auto_approval_respects_requested_scope_and_existing_relay_revocation(same_account_api):
    api = same_account_api
    expanded = _auto_approve(
        api, payload={"approved_scopes": ["input.pointer"], "approved_profile": None}
    )
    assert expanded.status_code == 403
    first = _auto_approve(api)
    second = _auto_approve(api)
    assert first.status_code == second.status_code == 200
    assert first.json()["approved_scopes"] == APPROVAL["approved_scopes"]
    assert first.json()["active_relay_generation"] == 0
    assert api.session.scalar(select(func.count()).select_from(RelayAccessGeneration)) == 1
    assert _access(api, caller="controller-1").status_code == 200
    assert _access(api, caller="target-1").status_code == 200
    revoked = api.client.post(
        "/api/v1/device-sessions/wan-session-1/revoke",
        headers=_headers(api, "target-1"),
        json={},
    )
    assert revoked.status_code == 200
    assert _trust(api).json() == {"trusted": False}
    assert _auto_approve(api).status_code == 403
    assert _access(api, caller="controller-1").status_code == 403


@pytest.mark.parametrize("change", ["target_owner", "disabled", "issuer", "identity", "target_key",
                                    "controller_version", "target_version"])
def test_approved_auto_grant_rechecks_snapshot_on_inspection_and_relay(same_account_api, monkeypatch, change):
    api = same_account_api
    assert _auto_approve(api).status_code == 200
    row = api.session.get(SessionRequest, "wan-session-1")
    assert row.same_account_authority == {
        "owner_user_id": "controller-user", "issuer": ISSUER,
        "controller_key_id": CONTROLLER_KEY, "target_key_id": TARGET_KEY,
        "controller_auth_version": 1, "target_auth_version": 1,
    }
    if change == "target_owner":
        api.devices["target-1"].bound_user_id = "target-user"
    elif change == "disabled":
        monkeypatch.setitem(settings.__dict__, "oidc_enabled", False)
    elif change == "issuer":
        monkeypatch.setitem(settings.__dict__, "oidc_issuer", "https://other.example.test")
    elif change == "identity":
        api.session.delete(api.session.get(OidcIdentity, (ISSUER, "verified-subject")))
    elif change == "target_key":
        api.session.delete(api.session.get(DeviceMachineIdentity, TARGET_KEY))
    elif change == "controller_version":
        api.devices["controller-1"].auth_version += 1
    else:
        api.devices["target-1"].auth_version += 1
        # The device may refresh its credential, but an older automatic grant
        # must not inherit the replacement credential's authority.
        from app.core.security import create_device_access_token
        api.tokens["target-1"] = create_device_access_token(api.devices["target-1"])
    api.session.commit()
    inspected = api.client.get("/api/v1/device-sessions/wan-session-1",
                               headers=_headers(api, "target-1"))
    assert inspected.status_code == 404
    assert _access(api, caller="target-1").status_code == 403
    assert api.credential_calls == []


@pytest.mark.parametrize("device_id", ["controller-1", "target-1"])
def test_unbinding_either_device_permanently_revokes_auto_grant(same_account_api, device_id):
    api = same_account_api
    assert _auto_approve(api).status_code == 200
    owner = api.session.get(User, "controller-user")
    headers = {**_headers(api, device_id), "Authorization": "Bearer " + create_access_token(
        owner.id, owner.username, owner.role, owner.session_version)}
    response = api.client.post("/api/v1/devices/unbind", headers=headers, json={"device_id": device_id})
    assert response.status_code == 200, response.text
    row = api.session.get(SessionRequest, "wan-session-1")
    assert row.status == "revoked"
    reservations = list(api.session.scalars(select(RelayReservation)))
    assert reservations and all(reservation.superseded_at is not None for reservation in reservations)
    rebound = api.client.post("/api/v1/devices/auto-bind", headers=headers, json={"device_id": device_id})
    assert rebound.status_code == 200, rebound.text
    assert api.session.get(SessionRequest, "wan-session-1").status == "revoked"
    assert _access(api, caller="target-1").status_code == 403

