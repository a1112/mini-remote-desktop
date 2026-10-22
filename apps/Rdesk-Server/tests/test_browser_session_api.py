"""Browser credentials cross the real HTTP/auth/database boundary in these tests."""
from datetime import UTC, datetime, timedelta
import hashlib
import base64

import jwt
import pytest
from sqlalchemy import select

from app.core.config import settings
from app.core.security import create_device_access_token
from app.core.security import create_access_token
from app.models.device import Device
from app.models.session_request import SessionRequest
from app.models.user import User
from app.models.device_network_group import DeviceNetworkGroup
from test_device_session_api import (
    JWT_SECRET, DeviceSessionsAPI, device_sessions_api, _headers,
)
from test_wan_relay_access import wan_relay_api
from types import SimpleNamespace
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from cryptography.hazmat.primitives.serialization import Encoding, PublicFormat
from pydantic import SecretStr


PUBLIC_KEY = list(range(1, 33))
TARGET_KEY_ID = "cd" * 32
DIRECTORY_SEED = bytes.fromhex("43" * 32)
DIRECTORY_PUBLIC = Ed25519PrivateKey.from_private_bytes(DIRECTORY_SEED).public_key().public_bytes(Encoding.Raw, PublicFormat.Raw)
DIRECTORY_KEY_ID = hashlib.sha256(DIRECTORY_PUBLIC).hexdigest()


@pytest.fixture(autouse=True)
def trusted_realtime_identity(monkeypatch):
    from app.api.v1 import browser_sessions
    monkeypatch.setattr(settings, "public_signal_server_key_id", "ab" * 32)
    monkeypatch.setattr(settings, "relay_directory_signing_key_id", DIRECTORY_KEY_ID)
    monkeypatch.setattr(settings, "relay_directory_signing_private_key", SecretStr(base64.b64encode(DIRECTORY_SEED).decode()))
    async def identity(target_id):
        return TARGET_KEY_ID
    monkeypatch.setattr(browser_sessions, "query_realtime_target_key", identity, raising=False)


def _browser_headers(api):
    return {"Authorization": f"Bearer {api.user_token}"}


def _browser_request(**changes):
    return {
        "session_id": "browser-session-1",
        "idempotency_key": [8] * 16,
        "controller_public_key": PUBLIC_KEY,
        "target_device_id": "same-owner-target",
        "requested_scopes": ["input.keyboard", "input.pointer", "screen.view"],
        "route_policy": "direct_first",
        **changes,
    }


def _create_browser(api, **changes):
    return api.client.post("/api/v1/browser-sessions", headers=_browser_headers(api),
                           json=_browser_request(**changes))


def test_browser_bootstrap_is_bound_to_one_session_key_user_and_target(device_sessions_api):
    api = device_sessions_api
    response = _create_browser(api)
    assert response.status_code == 200, response.text
    body = response.json()
    assert set(body) == {"controller_device_id", "controller_key_id", "expires_at_ms",
        "session", "credential", "signaling_url", "signaling_server_device_id",
        "signaling_server_key_id", "target_key_id", "relay_directory_key_id", "relay_directory_public_key"}
    assert body["target_key_id"] == TARGET_KEY_ID
    assert body["signaling_server_key_id"] == "ab" * 32
    assert body["relay_directory_public_key"] == list(DIRECTORY_PUBLIC)
    assert hashlib.sha256(bytes(body["relay_directory_public_key"])).hexdigest() == body["relay_directory_key_id"]
    assert body["controller_device_id"].startswith("browser_")
    assert body["controller_key_id"] == hashlib.sha256(bytes(PUBLIC_KEY)).hexdigest()
    assert body["session"]["status"] == "requested"
    assert body["session"]["request"]["controller_device_id"] == body["controller_device_id"]
    assert body["session"]["request"]["requested_profile"]["codec"] == "h264"
    credential = body["credential"]
    claims = jwt.decode(credential["token"], JWT_SECRET, algorithms=["HS256"],
                        issuer=settings.jwt_issuer, audience=settings.signaling_jwt_audience)
    assert claims["token_type"] == "browser_signaling"
    assert claims["role"] == "Controller"
    assert claims["sub"] == claims["device_id"] == body["controller_device_id"]
    assert claims["device_key_id"] == body["controller_key_id"]
    assert claims["user_id"] == "controller-user"
    assert claims["tenant_id"] == "tenant-a"
    assert claims["session_id"] == "browser-session-1"
    assert claims["target_device_id"] == "same-owner-target"
    assert claims["allowed_scopes"] == ["input.keyboard", "input.pointer", "screen.view"]
    assert claims["exp"] * 1000 <= body["expires_at_ms"]
    assert "refresh_token" not in response.text and "access_token" not in response.text
    assert response.headers["cache-control"] == "no-store, private"
    row = api.session.get(SessionRequest, "browser-session-1")
    shadow = api.session.get(Device, row.requester_device_id)
    assert shadow.principal_kind == "browser_controller"
    assert shadow.motherboard_serial_digest is None
    retried = _create_browser(api)
    assert retried.status_code == 200
    assert retried.json()["controller_device_id"] == body["controller_device_id"]
    assert api.session.query(SessionRequest).count() == 1


@pytest.mark.parametrize("target", ["target-1", "foreign-1", "unbound-1", "missing"])
def test_browser_can_only_request_accessible_physical_targets(device_sessions_api, target):
    response = _create_browser(device_sessions_api, target_device_id=target)
    assert response.status_code == 404, response.text
    assert device_sessions_api.session.query(SessionRequest).count() == 0


@pytest.mark.parametrize("changes", [
    {"requested_scopes": ["terminal.open", "screen.view"]},
    {"requested_scopes": ["input.keyboard"]},
    {"requested_scopes": ["screen.view", "input.keyboard"]},
    {"controller_public_key": [0] * 32},
    {"controller_public_key": [1] * 31},
    {"role": "Agent"}, {"access_mode": "unattended"},
    {"requested_profile": {"width": 1920, "height": 1080, "fps": 30,
                           "bitrate_mbps": 10, "codec": "hevc"}},
])
def test_browser_requests_fail_closed_for_unsupported_privileges(device_sessions_api, changes):
    response = _create_browser(device_sessions_api, **changes)
    assert response.status_code == 400, response.text
    assert response.headers["cache-control"] == "no-store, private"


def test_machine_token_is_not_browser_authorization(device_sessions_api):
    api = device_sessions_api
    response = api.client.post("/api/v1/browser-sessions",
                              headers={"Authorization": f"Bearer {api.tokens['controller-1']}"},
                              json=_browser_request())
    assert response.status_code == 401


def test_browser_shadow_is_invisible_and_cannot_mint_machine_credentials(device_sessions_api):
    api = device_sessions_api
    body = _create_browser(api).json()
    row = api.session.get(SessionRequest, "browser-session-1")
    shadow = api.session.get(Device, row.requester_device_id)
    listed = api.client.get("/api/v1/devices", headers=_browser_headers(api))
    assert listed.status_code == 200
    assert all(device["device_id"] != body["controller_device_id"] for device in listed.json())
    assert api.client.get(f"/api/v1/devices/{shadow.id}", headers=_browser_headers(api)).status_code == 404
    machine_token = create_device_access_token(shadow)
    machine_headers = {"X-Rdesk-Device-Authorization": f"Bearer {machine_token}"}
    minted = api.client.post("/api/v1/realtime/device-credentials", headers=machine_headers,
                            json={"device_key_id": body["controller_key_id"], "role": "Agent"})
    assert minted.status_code == 401
    assert api.client.get(f"/api/v1/devices/{shadow.device_id}/binding-status",
                          headers=_browser_headers(api)).status_code == 404


def test_native_target_inspection_keeps_exact_native_shape(device_sessions_api):
    api = device_sessions_api
    body = _create_browser(api).json()
    target = api.client.get("/api/v1/device-sessions/browser-session-1",
                           headers=_headers(api, "same-owner-target"))
    assert target.status_code == 200, target.text
    assert target.json() == body["session"]


@pytest.mark.parametrize("invalidation", ["expired", "revoked", "user_version"])
def test_invalid_browser_principal_blocks_browser_and_target_use(device_sessions_api, invalidation):
    from app.models.browser_controller import BrowserController
    api = device_sessions_api
    assert _create_browser(api).status_code == 200
    principal = api.session.scalar(select(BrowserController))
    if invalidation == "expired":
        principal.created_at = datetime.now(UTC) - timedelta(minutes=11)
        principal.expires_at = datetime.now(UTC) - timedelta(seconds=1)
    elif invalidation == "revoked":
        principal.revoked_at = datetime.now(UTC)
    else:
        api.session.get(User, "controller-user").session_version += 1
    api.session.commit()
    assert api.client.get("/api/v1/browser-sessions/browser-session-1",
                          headers=_browser_headers(api)).status_code in {401, 404}
    assert api.client.get("/api/v1/device-sessions/browser-session-1",
                          headers=_headers(api, "same-owner-target")).status_code == 404
    assert api.client.post("/api/v1/device-sessions/browser-session-1/approve",
                           headers=_headers(api, "same-owner-target"),
                           json={"approved_scopes": ["screen.view"],
                                 "approved_profile": None}).status_code in {404, 503}


def test_browser_close_revokes_credentials_and_allows_terminal_native_inspection(device_sessions_api):
    api = device_sessions_api
    assert _create_browser(api).status_code == 200
    closed = api.client.post("/api/v1/browser-sessions/browser-session-1/close",
                             headers=_browser_headers(api), json={})
    assert closed.status_code == 200, closed.text
    assert closed.json()["status"] == "closed"
    assert api.client.get("/api/v1/browser-sessions/browser-session-1",
                          headers=_browser_headers(api)).status_code == 404
    assert _create_browser(api).status_code == 409
    assert api.client.post("/api/v1/browser-sessions/browser-session-1/relay-access",
                           headers=_browser_headers(api), json={}).status_code in {404, 503}


def test_server_logout_invalidates_browser_and_target_authority(device_sessions_api):
    api = device_sessions_api
    assert _create_browser(api).status_code == 200
    response = api.client.post("/api/v1/auth/logout", headers=_browser_headers(api), json={})
    assert response.status_code == 200
    assert api.session.get(User, "controller-user").session_version == 2
    assert api.client.get("/api/v1/browser-sessions/browser-session-1",
                          headers=_browser_headers(api)).status_code == 401
    assert api.client.get("/api/v1/device-sessions/browser-session-1",
                          headers=_headers(api, "same-owner-target")).status_code == 404


@pytest.mark.parametrize("missing", ["server_pin", "relay_pin", "target_pin"])
def test_browser_bootstrap_requires_trusted_pins_and_rolls_back(device_sessions_api, monkeypatch, missing):
    from app.api.v1 import browser_sessions
    if missing == "server_pin":
        monkeypatch.setattr(settings, "public_signal_server_key_id", "")
    elif missing == "relay_pin":
        monkeypatch.setattr(settings, "relay_directory_signing_key_id", "")
    else:
        async def unavailable(_):
            return None
        monkeypatch.setattr(browser_sessions, "query_realtime_target_key", unavailable)
    response = _create_browser(device_sessions_api)
    assert response.status_code == 503, response.text
    assert device_sessions_api.session.query(SessionRequest).count() == 0


def _approved_browser(api):
    from app.services.relay_signing import Ed25519RelayDirectorySigner
    api.service._signer = Ed25519RelayDirectorySigner(key_id=DIRECTORY_KEY_ID, private_key_seed=DIRECTORY_SEED)
    user = api.session.get(User, "controller-user")
    browser = SimpleNamespace(client=api.client, session=api.session,
        user_token=create_access_token(user.id, user.username, user.role, user.session_version))
    created = _create_browser(browser, target_device_id="controller-1")
    assert created.status_code == 200, created.text
    response = api.client.post("/api/v1/device-sessions/browser-session-1/approve",
        headers={"X-Rdesk-Device-Authorization": f"Bearer {api.tokens['controller-1']}"},
        json={"approved_scopes": ["screen.view"], "approved_profile": None})
    assert response.status_code == 200, response.text
    return browser


def test_browser_attended_approval_downscopes_credential_and_issues_real_relay_access(wan_relay_api):
    api = wan_relay_api
    browser = _approved_browser(api)
    inspect_response = api.client.get("/api/v1/browser-sessions/browser-session-1",
                                     headers=_browser_headers(browser))
    assert inspect_response.status_code == 200, inspect_response.text
    claims = jwt.decode(inspect_response.json()["credential"]["token"],
        settings.jwt_secret.get_secret_value(), algorithms=["HS256"], issuer=settings.jwt_issuer,
        audience=settings.signaling_jwt_audience)
    assert claims["allowed_scopes"] == ["screen.view"]
    relay = api.client.post("/api/v1/browser-sessions/browser-session-1/relay-access",
        headers=_browser_headers(browser), json={})
    assert relay.status_code == 200, relay.text
    assert relay.json()["generation"] == 0
    assert relay.json()["credentials"]
    assert len(relay.json()["relay_url_digest"]) == 64
    assert api.credential_calls
    from app.services.relay_signing import SignedRelayDirectoryOut, verify_signed_directory
    assert verify_signed_directory(SignedRelayDirectoryOut.model_validate(relay.json()["directory"]),
                                    public_key=bytes(inspect_response.json()["relay_directory_public_key"]))


def test_expired_browser_cannot_use_real_relay_or_native_approval(wan_relay_api):
    from app.models.browser_controller import BrowserController
    api = wan_relay_api
    browser = _approved_browser(api)
    principal = api.session.scalar(select(BrowserController))
    principal.created_at = datetime.now(UTC) - timedelta(minutes=11)
    principal.expires_at = datetime.now(UTC) - timedelta(seconds=1)
    api.session.commit()
    relay = api.client.post("/api/v1/browser-sessions/browser-session-1/relay-access",
                           headers=_browser_headers(browser), json={})
    assert relay.status_code == 404
    physical_relay = api.client.post("/api/v1/relays/access", headers={
        "X-Rdesk-Device-Authorization": f"Bearer {api.tokens['controller-1']}"}, json={
        "session_id": "browser-session-1", "policy_revision": api.service.current_policy.revision,
        "intended_peer_id": "controller-1", "generation": 0, "refresh": False})
    assert physical_relay.status_code == 403, physical_relay.text
    approve = api.client.post("/api/v1/device-sessions/browser-session-1/approve", headers={
        "X-Rdesk-Device-Authorization": f"Bearer {api.tokens['controller-1']}"}, json={
        "approved_scopes": ["screen.view"], "approved_profile": None})
    assert approve.status_code == 404


def test_browser_authority_is_lost_when_target_ownership_changes(device_sessions_api):
    api = device_sessions_api
    assert _create_browser(api).status_code == 200
    api.devices["same-owner-target"].bound_user_id = "target-user"
    api.session.commit()
    assert api.client.get("/api/v1/browser-sessions/browser-session-1",
                          headers=_browser_headers(api)).status_code == 404
    assert api.client.get("/api/v1/device-sessions/browser-session-1",
                          headers=_headers(api, "same-owner-target")).status_code == 404


def test_browser_rechecks_authority_after_waiting_for_target_identity(device_sessions_api, monkeypatch):
    from app.api.v1 import browser_sessions
    api = device_sessions_api
    assert _create_browser(api).status_code == 200
    async def revoked_while_querying(_):
        api.session.get(User, "controller-user").session_version += 1
        api.session.commit()
        return TARGET_KEY_ID
    monkeypatch.setattr(browser_sessions, "query_realtime_target_key", revoked_while_querying)
    inspected = api.client.get("/api/v1/browser-sessions/browser-session-1",
                               headers=_browser_headers(api))
    assert inspected.status_code == 404


def test_browser_credentials_and_authority_cannot_outlive_approved_grant(wan_relay_api):
    api = wan_relay_api
    browser = _approved_browser(api)
    inspected = api.client.get("/api/v1/browser-sessions/browser-session-1",
                               headers=_browser_headers(browser))
    body = inspected.json()
    row = api.session.get(SessionRequest, "browser-session-1")
    expires = row.grant_expires_at.replace(tzinfo=UTC).timestamp()
    assert body["credential"]["expires_at_ms"] <= int(expires * 1000)
    row.grant_expires_at = datetime.now(UTC) - timedelta(seconds=1)
    api.session.commit()
    assert api.client.get("/api/v1/browser-sessions/browser-session-1",
                          headers=_browser_headers(browser)).status_code == 404
    assert api.client.get("/api/v1/device-sessions/browser-session-1", headers={
        "X-Rdesk-Device-Authorization": f"Bearer {api.tokens['controller-1']}"}).status_code == 404


def test_browser_shadow_cannot_be_added_to_physical_network_groups(device_sessions_api):
    api = device_sessions_api
    body = _create_browser(api).json()
    groups = api.client.get("/api/v1/network-groups", headers=_browser_headers(api))
    assert groups.status_code == 200, groups.text
    group_id = groups.json()[0]["id"]
    added = api.client.post(f"/api/v1/network-groups/{group_id}/devices", headers=_browser_headers(api),
                            json={"device_ids": [body["controller_device_id"]]})
    assert added.status_code == 201
    assert api.session.query(DeviceNetworkGroup).count() == 0


def test_admin_inventory_also_excludes_browser_shadow(device_sessions_api):
    api = device_sessions_api
    body = _create_browser(api).json()
    owner = api.session.get(User, "controller-user")
    owner.role = "admin"
    api.session.commit()
    api.user_token = create_access_token(owner.id, owner.username, owner.role, owner.session_version)
    listed = api.client.get("/api/v1/devices", headers=_browser_headers(api))
    assert listed.status_code == 200
    assert all(row["device_id"] != body["controller_device_id"] for row in listed.json())


@pytest.mark.parametrize("url", ["https://target.invalid/ws", "ws://[invalid/ws", "wss://user:pass@example.test/ws"])
def test_invalid_browser_signaling_configuration_returns_stable_503(device_sessions_api, monkeypatch, url):
    monkeypatch.setattr(settings, "signaling_ws_url", url)
    response = _create_browser(device_sessions_api)
    assert response.status_code == 503
    assert device_sessions_api.session.query(SessionRequest).count() == 0


def test_browser_creation_locks_devices_before_users_like_native_approval(device_sessions_api):
    from app.db.session import get_db
    from test_wan_relay_access import AsyncSessionShim
    api = device_sessions_api
    shim = AsyncSessionShim(api.session)
    async def database():
        yield shim
    api.app.dependency_overrides[get_db] = database
    assert _create_browser(api).status_code == 200
    assert shim.lock_trace[0] == ("Device",)
    assert ("User",) in shim.lock_trace


def test_browser_target_cannot_approve_input_without_required_screen_scope(wan_relay_api):
    api = wan_relay_api
    user = api.session.get(User, "controller-user")
    browser = SimpleNamespace(client=api.client, session=api.session,
        user_token=create_access_token(user.id, user.username, user.role, user.session_version))
    assert _create_browser(browser, target_device_id="controller-1").status_code == 200
    response = api.client.post("/api/v1/device-sessions/browser-session-1/approve", headers={
        "X-Rdesk-Device-Authorization": f"Bearer {api.tokens['controller-1']}"}, json={
        "approved_scopes": ["input.keyboard"], "approved_profile": None})
    assert response.status_code == 403, response.text
    row = api.session.get(SessionRequest, "browser-session-1")
    assert row.status == "requested" and row.approved_scopes is None
    assert not api.credential_calls


def test_browser_missing_approved_screen_scope_cannot_issue_credentials_or_relay(wan_relay_api):
    api = wan_relay_api
    browser = _approved_browser(api)
    row = api.session.get(SessionRequest, "browser-session-1")
    row.approved_scopes = ["input.keyboard"]
    api.session.commit()
    credentials_before = len(api.credential_calls)
    assert api.client.get("/api/v1/browser-sessions/browser-session-1",
        headers=_browser_headers(browser)).status_code == 404
    assert api.client.get("/api/v1/device-sessions/browser-session-1", headers={
        "X-Rdesk-Device-Authorization": f"Bearer {api.tokens['controller-1']}"}).status_code == 404
    assert api.client.post("/api/v1/browser-sessions/browser-session-1/relay-access",
        headers=_browser_headers(browser), json={}).status_code == 404
    assert len(api.credential_calls) == credentials_before


def test_native_physical_input_only_approval_remains_compatible(wan_relay_api):
    from test_wan_relay_access import _create
    api = wan_relay_api
    assert _create(api).status_code == 200
    response = api.client.post("/api/v1/device-sessions/wan-session-1/approve", headers={
        "X-Rdesk-Device-Authorization": f"Bearer {api.tokens['target-1']}"}, json={
        "approved_scopes": ["input.keyboard"], "approved_profile": None})
    assert response.status_code == 200, response.text
    assert response.json()["approved_scopes"] == ["input.keyboard"]


@pytest.mark.parametrize("invalidation", ["revoked", "closed", "grant_expired", "policy_expired"])
def test_browser_refresh_reads_committed_session_invalidation_after_identity_wait(wan_relay_api, monkeypatch, invalidation):
    from app.api.v1 import browser_sessions
    from sqlalchemy import update
    from sqlalchemy.orm import Session
    api = wan_relay_api
    browser = _approved_browser(api)
    async def invalidate_in_another_session(_):
        changes = {"status": invalidation} if invalidation in {"revoked", "closed"} else {
            "grant_expires_at" if invalidation == "grant_expired" else "policy_expires_at":
                datetime.now(UTC) - timedelta(seconds=1)}
        with Session(api.session.get_bind()) as concurrent:
            concurrent.execute(update(SessionRequest).where(SessionRequest.id == "browser-session-1")
                .values(**changes).execution_options(synchronize_session=False))
            concurrent.commit()
        return TARGET_KEY_ID
    monkeypatch.setattr(browser_sessions, "query_realtime_target_key", invalidate_in_another_session)
    response = api.client.get("/api/v1/browser-sessions/browser-session-1",
                              headers=_browser_headers(browser))
    assert response.status_code == 404, response.text
    assert "credential" not in response.json()


def test_browser_refresh_signs_latest_committed_approval_after_identity_wait(wan_relay_api, monkeypatch):
    from app.api.v1 import browser_sessions
    from sqlalchemy import update
    from sqlalchemy.orm import Session
    api = wan_relay_api
    browser = _approved_browser(api)
    latest_scopes = ["input.keyboard", "screen.view"]
    async def change_approval_in_another_session(_):
        with Session(api.session.get_bind()) as concurrent:
            concurrent.execute(update(SessionRequest).where(SessionRequest.id == "browser-session-1")
                .values(approved_scopes=latest_scopes).execution_options(synchronize_session=False))
            concurrent.commit()
        return TARGET_KEY_ID
    monkeypatch.setattr(browser_sessions, "query_realtime_target_key", change_approval_in_another_session)
    response = api.client.get("/api/v1/browser-sessions/browser-session-1",
                              headers=_browser_headers(browser))
    assert response.status_code == 200, response.text
    claims = jwt.decode(response.json()["credential"]["token"], settings.jwt_secret.get_secret_value(),
        algorithms=["HS256"], issuer=settings.jwt_issuer, audience=settings.signaling_jwt_audience)
    assert response.json()["session"]["approved_scopes"] == latest_scopes
    assert claims["allowed_scopes"] == latest_scopes


def test_browser_refresh_locks_authority_in_native_order_after_identity_wait(wan_relay_api):
    from app.db.session import get_db
    from test_wan_relay_access import AsyncSessionShim
    api = wan_relay_api
    browser = _approved_browser(api)
    shim = AsyncSessionShim(api.session)
    async def database():
        yield shim
    api.client.app.dependency_overrides[get_db] = database
    response = api.client.get("/api/v1/browser-sessions/browser-session-1",
                              headers=_browser_headers(browser))
    assert response.status_code == 200, response.text
    assert shim.lock_trace[:3] == [("Device",), ("User",), ("SessionRequest",)]


@pytest.mark.parametrize("changes", [
    {"principal_kind": "physical"},
    {"is_bound": False, "bound_user_id": None},
])
def test_browser_refresh_rejects_changed_shadow_identity_after_wait(wan_relay_api, monkeypatch, changes):
    from app.api.v1 import browser_sessions
    from sqlalchemy import update
    from sqlalchemy.orm import Session
    api = wan_relay_api
    browser = _approved_browser(api)
    row = api.session.get(SessionRequest, "browser-session-1")
    controller_row_id = row.requester_device_id
    async def change_binding_in_another_session(_):
        with Session(api.session.get_bind()) as concurrent:
            concurrent.execute(update(Device).where(Device.id == controller_row_id)
                .values(**changes).execution_options(synchronize_session=False))
            concurrent.commit()
        return TARGET_KEY_ID
    monkeypatch.setattr(browser_sessions, "query_realtime_target_key", change_binding_in_another_session)
    response = api.client.get("/api/v1/browser-sessions/browser-session-1",
                              headers=_browser_headers(browser))
    assert response.status_code == 404, response.text
