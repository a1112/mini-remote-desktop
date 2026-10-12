"""Paired routing keeps backend identity distinct from target-local trust policy."""
from datetime import UTC, datetime
import hashlib

import jwt
import pytest
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from sqlalchemy import select

from app.core.security import create_access_token, create_device_access_token
from app.models.browser_controller import BrowserController
from app.models.device_machine_identity import DeviceMachineIdentity
from app.models.session_request import SessionRequest
from app.models.user import User
from app.schemas.session import DeviceSessionCanonicalRequest
from app.services.device_sessions import wan_request_commitment
from test_device_session_api import device_sessions_api, _create, _request, _headers, JWT_SECRET
from test_browser_session_api import trusted_realtime_identity, _create_browser, _browser_headers
from test_wan_relay_access import wan_relay_api, _access


def _pin(api, public_id, seed):
    public = Ed25519PrivateKey.from_private_bytes(bytes([seed]) * 32).public_key().public_bytes_raw()
    mapping = DeviceMachineIdentity(key_id=hashlib.sha256(public).hexdigest(),
        public_key=public.hex(), device_row_id=api.devices[public_id].id, created_at=datetime.now(UTC))
    api.session.add(mapping)
    api.session.commit()
    return mapping


def _paired_request(**changes):
    return {**_request(target_device_id="same-owner-target"), "access_mode": "paired", **changes}


def test_paired_and_attended_cross_language_vectors():
    request = {**_request(), "controller_device_id": "controller-1"}
    assert wan_request_commitment(DeviceSessionCanonicalRequest.model_validate(request)) == (
        "d4942aab9c4cd956ba314d4d4b6c19b744cd20132de7a69b2fb18b045de41608")
    request["access_mode"] = "paired"
    assert wan_request_commitment(DeviceSessionCanonicalRequest.model_validate(request)) == (
        "ea69079ebeba95592015cbd595dc1e8add335405bc36641745ec2f16793272a2")


def test_physical_paired_same_owner_formal_keys_and_exact_retry(device_sessions_api):
    api = device_sessions_api
    controller = _pin(api, "controller-1", 21)
    target = _pin(api, "same-owner-target", 22)
    response = _create(api, payload=_paired_request())
    assert response.status_code == 200, response.text
    assert response.json()["status"] == "requested"  # Backend never establishes local trust.
    row = api.session.get(SessionRequest, "session-1")
    assert row.paired_controller_key_id == controller.key_id
    assert row.paired_target_key_id == target.key_id
    assert _create(api, payload=_paired_request()).json() == response.json()
    assert api.client.get("/api/v1/device-sessions/session-1",
        headers=_headers(api, "same-owner-target")).status_code == 200
    changed = _paired_request(access_mode="attended")
    assert _create(api, payload=changed).status_code == 409


@pytest.mark.parametrize("invalid", ["missing_controller", "missing_target", "bad_hash", "cross_owner"])
def test_physical_paired_rejects_legacy_missing_or_bad_pins_and_other_owner(device_sessions_api, invalid):
    api = device_sessions_api
    if invalid != "missing_controller":
        _pin(api, "controller-1", 21)
    target = _pin(api, "same-owner-target", 22) if invalid != "missing_target" else None
    if invalid == "bad_hash":
        target.public_key = "01" * 32
        api.session.commit()
    if invalid == "cross_owner":
        api.devices["same-owner-target"].bound_user_id = "target-user"
        api.session.commit()
    response = _create(api, payload=_paired_request())
    assert response.status_code == 403, response.text
    assert api.session.query(SessionRequest).count() == 0


@pytest.mark.parametrize("mutation", ["owner", "key", "missing_mapping"])
def test_paired_physical_authority_is_rechecked_on_inspection_and_retry(device_sessions_api, mutation):
    api = device_sessions_api
    _pin(api, "controller-1", 21)
    target = _pin(api, "same-owner-target", 22)
    assert _create(api, payload=_paired_request()).status_code == 200
    if mutation == "owner":
        api.devices["same-owner-target"].bound_user_id = "target-user"
    elif mutation == "key":
        public = Ed25519PrivateKey.from_private_bytes(bytes([23]) * 32).public_key().public_bytes_raw()
        target.key_id, target.public_key = hashlib.sha256(public).hexdigest(), public.hex()
    else:
        api.session.delete(target)
    api.session.commit()
    assert api.client.get("/api/v1/device-sessions/session-1",
        headers=_headers(api, "same-owner-target")).status_code == 404
    assert _create(api, payload=_paired_request()).status_code == 409


def _browser_pin(api, monkeypatch):
    target = _pin(api, "same-owner-target", 22)
    async def target_key(_):
        return target.key_id
    monkeypatch.setattr("app.api.v1.browser_sessions.query_realtime_target_key", target_key)
    return target


def test_paired_browser_stable_key_uses_distinct_short_session_principals(device_sessions_api, trusted_realtime_identity, monkeypatch):
    api = device_sessions_api
    target = _browser_pin(api, monkeypatch)
    first = _create_browser(api, access_mode="paired")
    second = _create_browser(api, access_mode="paired", session_id="browser-session-2")
    assert first.status_code == second.status_code == 200, (first.text, second.text)
    a, b = first.json(), second.json()
    assert a["controller_key_id"] == b["controller_key_id"]
    assert a["controller_device_id"] != b["controller_device_id"]
    assert a["session"]["status"] == b["session"]["status"] == "requested"
    for body in (a, b):
        claims = jwt.decode(body["credential"]["token"], JWT_SECRET,
            algorithms=["HS256"], options={"verify_aud": False})
        assert 0 < claims["exp"] - claims["iat"] <= 600
        assert claims["session_id"] == body["session"]["session_id"]
        row = api.session.get(SessionRequest, claims["session_id"])
        assert row.paired_controller_key_id == body["controller_key_id"]
        assert row.paired_target_key_id == target.key_id
    assert api.session.query(BrowserController).count() == 2


def test_paired_account_browser_admin_cannot_cross_owner(device_sessions_api, trusted_realtime_identity):
    api = device_sessions_api
    owner = api.session.get(User, "controller-user")
    owner.role = "admin"
    api.session.commit()
    api.user_token = create_access_token(owner.id, owner.username, owner.role, owner.session_version)
    _pin(api, "target-1", 22)
    assert _create_browser(api, access_mode="paired", target_device_id="target-1").status_code == 404
    assert api.session.query(SessionRequest).count() == 0


def test_paired_browser_live_target_key_must_match_pin_after_identity_wait(device_sessions_api, trusted_realtime_identity, monkeypatch):
    api = device_sessions_api
    _pin(api, "same-owner-target", 22)
    # Existing fixture's realtime key is intentionally different from the formal mapping.
    assert _create_browser(api, access_mode="paired").status_code == 404
    assert api.session.query(SessionRequest).count() == 0


def test_stable_browser_key_cannot_collide_with_resident_machine(device_sessions_api, trusted_realtime_identity, monkeypatch):
    api = device_sessions_api
    _browser_pin(api, monkeypatch)
    mapping = _pin(api, "controller-1", 21)
    assert _create_browser(api, access_mode="paired",
        controller_public_key=list(bytes.fromhex(mapping.public_key))).status_code == 409


@pytest.mark.parametrize("mutation", ["owner", "mapping", "scope", "mode"])
def test_paired_browser_refresh_rechecks_authority_after_realtime_wait(device_sessions_api, trusted_realtime_identity, monkeypatch, mutation):
    api = device_sessions_api
    target = _browser_pin(api, monkeypatch)
    assert _create_browser(api, access_mode="paired").status_code == 200
    async def change_authority(_):
        if mutation == "owner":
            api.devices["same-owner-target"].bound_user_id = "target-user"
        elif mutation == "mapping":
            target.public_key = "01" * 32
        elif mutation == "scope":
            principal = api.session.scalar(select(BrowserController))
            principal.allowed_scopes = ["screen.view", "terminal.open"]
        else:
            row = api.session.get(SessionRequest, "browser-session-1")
            row.request_payload = {**row.request_payload, "access_mode": "attended"}
        api.session.commit()
        return target.key_id
    monkeypatch.setattr("app.api.v1.browser_sessions.query_realtime_target_key", change_authority)
    response = api.client.get("/api/v1/browser-sessions/browser-session-1", headers=_browser_headers(api))
    assert response.status_code == 404, response.text
    assert "credential" not in response.text


@pytest.mark.parametrize("invalidation", ["owner", "key", "missing_mapping"])
def test_paired_approval_and_relay_reject_changed_owner_or_key(wan_relay_api, invalidation):
    api = wan_relay_api
    target = api.devices["target-1"]
    target.bound_user_id = api.devices["controller-1"].bound_user_id
    api.session.commit()
    api.tokens["target-1"] = create_device_access_token(target)
    _pin(api, "controller-1", 21)
    mapping = _pin(api, "target-1", 22)
    payload = _paired_request(session_id="wan-session-1", target_device_id="target-1")
    assert _create(api, payload=payload).status_code == 200
    approved = api.client.post("/api/v1/device-sessions/wan-session-1/approve",
        headers=_headers(api, "target-1"), json={"approved_scopes": ["screen.view"], "approved_profile": None})
    assert approved.status_code == 200, approved.text
    assert _access(api, caller="controller-1").status_code == 200
    calls_before = len(api.credential_calls)
    if invalidation == "owner":
        target.bound_user_id = "target-user"
    elif invalidation == "key":
        public = Ed25519PrivateKey.from_private_bytes(bytes([23])*32).public_key().public_bytes_raw()
        mapping.key_id, mapping.public_key = hashlib.sha256(public).hexdigest(), public.hex()
    else:
        api.session.delete(mapping)
    api.session.commit()
    api.tokens["target-1"] = create_device_access_token(target)
    response = api.client.post("/api/v1/device-sessions/wan-session-1/approve",
        headers=_headers(api, "target-1"), json={"approved_scopes": ["screen.view"], "approved_profile": None})
    assert response.status_code == 404, response.text
    assert _access(api, caller="controller-1").status_code == 403
    assert len(api.credential_calls) == calls_before
