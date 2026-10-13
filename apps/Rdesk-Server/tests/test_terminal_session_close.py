"""Terminal close is cleanup, never a new grant or a new terminal reason."""
from datetime import UTC, datetime, timedelta

import jwt
import pytest
from sqlalchemy import select

from app.models.browser_controller import BrowserController
from app.models.relay_audit_event import RelayAuditEvent
from app.models.relay_reservation import RelayReservation
from app.models.session_request import SessionRequest
from test_device_session_api import (
    JWT_SECRET, device_sessions_api, _create, _headers,
)
from test_browser_session_api import (
    trusted_realtime_identity, _browser_headers, _create_browser,
)
from test_guest_browser_session_api import (
    guest_config, _guest, _publish, _unbound_relay_guest,
)
from test_wan_relay_access import wan_relay_api


TERMINAL_STATES = ["closed", "rejected", "revoked", "expired"]


def _snapshot(row):
    return {column.key: getattr(row, column.key) for column in row.__table__.columns}


def _expire_browser_principal(api, row):
    principal = api.session.scalar(
        select(BrowserController).where(BrowserController.session_id == row.id)
    )
    now = datetime.now(UTC)
    principal.created_at = now - timedelta(minutes=11)
    principal.expires_at = now - timedelta(seconds=1)
    principal.revoked_at = now - timedelta(seconds=1)
    if row.authority_kind == "temporary_password":
        row.authority_expires_at = principal.expires_at
    return principal


def _assert_terminal_close(api, path, auth, row, terminal):
    before = _snapshot(row)
    audit_before = api.session.query(RelayAuditEvent).count()
    for _ in range(2):
        response = api.client.post(path + "/close", headers=auth, json={})
        assert response.status_code == 200, response.text
        assert response.json()["status"] == terminal
        assert response.headers["cache-control"] == "no-store, private"
        api.session.refresh(row)
        assert _snapshot(row) == before
    assert api.session.query(RelayAuditEvent).count() == audit_before


@pytest.mark.parametrize("terminal", TERMINAL_STATES)
@pytest.mark.parametrize("caller", ["controller-1", "target-1"])
def test_exact_native_participants_can_close_terminal_without_rewriting_reason(
    device_sessions_api, terminal, caller
):
    api = device_sessions_api
    assert _create(api).status_code == 200
    row = api.session.get(SessionRequest, "session-1")
    row.status = terminal
    api.session.commit()
    path = "/api/v1/device-sessions/session-1"
    for outsider in ["intruder-1", "same-owner-target", "foreign-1"]:
        assert api.client.post(
            path + "/close", headers=_headers(api, outsider), json={}
        ).status_code == 404
    _assert_terminal_close(api, path, _headers(api, caller), row, terminal)
    if terminal == "expired":
        assert api.client.post(
            path + "/revoke", headers=_headers(api, "target-1"), json={}
        ).status_code == 409
        assert api.client.post(
            path + "/reject", headers=_headers(api, "target-1"), json={}
        ).status_code == 409


@pytest.mark.parametrize("terminal", TERMINAL_STATES)
def test_account_browser_can_close_terminal_after_principal_expiry(
    device_sessions_api, terminal
):
    api = device_sessions_api
    assert _create_browser(api).status_code == 200
    row = api.session.get(SessionRequest, "browser-session-1")
    row.status = terminal
    principal = _expire_browser_principal(api, row)
    api.session.commit()
    path = "/api/v1/browser-sessions/browser-session-1"
    _assert_terminal_close(api, path, _browser_headers(api), row, terminal)
    api.session.refresh(principal)
    assert principal.revoked_at is not None
    assert api.client.get(path, headers=_browser_headers(api)).status_code == 404
    assert api.client.post(
        path + "/relay-access", headers=_browser_headers(api), json={}
    ).status_code in {404, 503}
    assert _create_browser(api).status_code == 409


@pytest.mark.parametrize("terminal", TERMINAL_STATES)
@pytest.mark.parametrize("access_mode", ["attended", "paired"])
def test_guest_can_close_terminal_with_bound_live_token_but_expired_principal(
    device_sessions_api, terminal, access_mode
):
    api = device_sessions_api
    assert _publish(api).status_code == 200
    created = _guest(api, access_mode=access_mode)
    assert created.status_code == 200, created.text
    auth = {"Authorization": "Bearer " + created.json()["http_credential"]["token"]}
    row = api.session.get(SessionRequest, "guest-session-1")
    row.status = terminal
    principal = _expire_browser_principal(api, row)
    api.session.commit()
    path = "/api/v1/guest-browser-sessions/guest-session-1"
    assert api.client.post(
        "/api/v1/guest-browser-sessions/other-session/close", headers=auth, json={}
    ).status_code == 404
    _assert_terminal_close(api, path, auth, row, terminal)
    api.session.refresh(principal)
    assert principal.revoked_at is not None
    assert api.client.get(path, headers=auth).status_code == 404
    assert api.client.post(path + "/relay-access", headers=auth, json={}).status_code in {404, 503}
    claims = jwt.decode(created.json()["http_credential"]["token"], options={"verify_signature": False})
    claims["iat"] = int(datetime.now(UTC).timestamp()) - 600
    claims["exp"] = claims["iat"] + 599
    expired_auth = {"Authorization": "Bearer " + jwt.encode(claims, JWT_SECRET, algorithm="HS256")}
    assert api.client.post(path + "/close", headers=expired_auth, json={}).status_code == 401


@pytest.mark.parametrize("terminal", TERMINAL_STATES)
def test_terminal_close_expires_existing_reservations_without_new_authority(
    wan_relay_api, terminal
):
    api = wan_relay_api
    created = _unbound_relay_guest(api)
    auth = {"Authorization": "Bearer " + created["http_credential"]["token"]}
    path = "/api/v1/guest-browser-sessions/guest-session-1"
    assert api.client.post(path + "/relay-access", headers=auth, json={}).status_code == 200
    row = api.session.get(SessionRequest, "guest-session-1")
    row.status = terminal
    api.session.commit()
    reservations = list(api.session.scalars(select(RelayReservation)))
    assert reservations
    before = {r.id: (r.expires_at, r.superseded_at) for r in reservations}
    started = datetime.now(UTC)
    _assert_terminal_close(api, path, auth, row, terminal)
    stopped = datetime.now(UTC)
    for reservation in reservations:
        api.session.refresh(reservation)
        expiry = reservation.expires_at.replace(tzinfo=UTC)
        assert started <= expiry <= stopped
        assert reservation.expires_at <= before[reservation.id][0]
        assert reservation.superseded_at is not None
    assert api.session.query(RelayReservation).count() == len(reservations)
    assert api.client.post(path + "/relay-access", headers=auth, json={}).status_code == 404


@pytest.mark.parametrize("channel", ["account", "guest"])
def test_terminal_cleanup_does_not_bypass_expired_active_browser_authority(
    device_sessions_api, channel
):
    api = device_sessions_api
    if channel == "account":
        assert _create_browser(api).status_code == 200
        path = "/api/v1/browser-sessions/browser-session-1"
        auth = _browser_headers(api)
        row = api.session.get(SessionRequest, "browser-session-1")
    else:
        assert _publish(api).status_code == 200
        created = _guest(api)
        assert created.status_code == 200
        path = "/api/v1/guest-browser-sessions/guest-session-1"
        auth = {"Authorization": "Bearer " + created.json()["http_credential"]["token"]}
        row = api.session.get(SessionRequest, "guest-session-1")
    _expire_browser_principal(api, row)
    api.session.commit()
    assert row.status == "requested"
    assert api.client.post(path + "/close", headers=auth, json={}).status_code == 404
    api.session.refresh(row)
    assert row.status == "requested"
