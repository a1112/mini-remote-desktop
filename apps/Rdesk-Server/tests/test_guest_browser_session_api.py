"""Guest consent authority crosses physical signature, HTTP and persistent DB."""

from datetime import UTC, datetime, timedelta
import base64, hashlib, json, struct
import jwt
import pytest
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from cryptography.hazmat.primitives.serialization import Encoding, PublicFormat
from pydantic import SecretStr
from sqlalchemy import select
from app.core.config import settings
from app.core.security import create_device_access_token
from app.models.device import Device
from app.models.device_machine_identity import DeviceMachineIdentity
from app.models.session_request import SessionRequest
from app.models.user import User
from test_device_session_api import device_sessions_api, _headers, JWT_SECRET
from test_browser_session_api import (
    trusted_realtime_identity,
    _browser_request,
    TARGET_KEY_ID,
)

PASSWORD = "ABCDEFG2"
SALT = bytes(range(16))
KEY = Ed25519PrivateKey.from_private_bytes(bytes([91]) * 32)
PUBLIC = KEY.public_key().public_bytes(Encoding.Raw, PublicFormat.Raw)
KEY_ID = hashlib.sha256(PUBLIC).hexdigest()


@pytest.fixture(autouse=True)
def guest_config(monkeypatch, trusted_realtime_identity):
    from app.api.v1 import guest_browser_sessions, browser_sessions

    async def target_identity(_):
        return KEY_ID

    monkeypatch.setattr(browser_sessions, "query_realtime_target_key", target_identity)
    monkeypatch.setattr(
        guest_browser_sessions,
        "enrollment_peer_ip",
        lambda request, proxies: "127.0.0.1",
    )
    monkeypatch.setattr(settings, "public_api_url", "https://guest.test/api/v1")
    monkeypatch.setitem(settings.__dict__, "guest_browser_enabled", True)
    monkeypatch.setattr(settings, "device_serial_pepper", SecretStr("a1" * 32))


def _publish(api, *, target="unbound-1", generation=1, enabled=True, **changes):
    device = api.devices[target]
    if (
        api.session.scalar(
            select(DeviceMachineIdentity).where(
                DeviceMachineIdentity.device_row_id == device.id
            )
        )
        is None
    ):
        api.session.add(
            DeviceMachineIdentity(
                key_id=KEY_ID,
                public_key=PUBLIC.hex(),
                device_row_id=device.id,
                created_at=datetime.now(UTC),
            )
        )
        api.session.commit()
    body = dict(
        device_id=device.device_id,
        auth_version=device.auth_version,
        generation=generation,
        enabled=enabled,
        expires_at_ms=(
            int((datetime.now(UTC) + timedelta(seconds=590)).timestamp() * 1000)
            if enabled
            else None
        ),
        salt=SALT.hex() if enabled else None,
        verifier=(
            hashlib.pbkdf2_hmac("sha256", PASSWORD.encode(), SALT, 600000).hex()
            if enabled
            else None
        ),
        allowed_scopes=(
            ["input.keyboard", "input.pointer", "screen.view"] if enabled else []
        ),
    )
    body.update(changes)
    access = json.dumps(body, separators=(",", ":"))
    canonical = "\n".join(
        (
            "POST",
            settings.public_api_url + "/devices/temporary-access",
            KEY_ID,
            hashlib.sha256(access.encode()).hexdigest(),
        )
    ).encode()
    domain = b"MRD_DEVICE_TEMPORARY_ACCESS_V1"
    signed = (
        b"MRD_CONTEXT_SIGNATURE_V1"
        + struct.pack(">H", len(domain))
        + domain
        + struct.pack(">Q", len(canonical))
        + canonical
    )
    return api.client.post(
        "/api/v1/devices/temporary-access",
        headers=_headers(api, target),
        json=dict(
            key_id=KEY_ID,
            public_key=PUBLIC.hex(),
            access_json=access,
            signature=KEY.sign(signed).hex(),
        ),
    )


def _guest(api, **changes):
    return api.client.post(
        "/api/v1/guest-browser-sessions",
        json={
            **_browser_request(
                target_device_id="unbound-1", session_id="guest-session-1"
            ),
            "temporary_password": PASSWORD,
            **changes,
        },
    )


def test_physical_unbound_publishes_without_user_and_never_returns_password(
    device_sessions_api,
):
    api = device_sessions_api
    response = _publish(api)
    assert response.status_code == 200, response.text
    assert response.json()["enabled"] is True
    assert response.json()["generation"] == 1
    assert (
        PASSWORD not in response.text
        and "verifier" not in response.text
        and "salt" not in response.text
    )
    assert response.headers["cache-control"] == "no-store, private"


def test_guest_controller_and_target_need_no_user_account(device_sessions_api):
    api = device_sessions_api
    assert _publish(api).status_code == 200
    response = _guest(api)
    assert response.status_code == 200, response.text
    body = response.json()
    assert body["session"]["status"] == "requested"
    assert body["session"]["authority_kind"] == "temporary_password"
    assert body["session"]["temporary_access_generation"] == 1
    row = api.session.get(SessionRequest, "guest-session-1")
    assert row.requester_user_id is None
    assert api.session.query(User).count() == 4
    assert api.session.get(Device, row.requester_device_id).bound_user_id is None
    claims = jwt.decode(
        body["credential"]["token"],
        JWT_SECRET,
        algorithms=["HS256"],
        issuer=settings.jwt_issuer,
        audience=settings.signaling_jwt_audience,
    )
    assert claims["token_type"] == "guest_browser_signaling"
    assert claims["authority_kind"] == "temporary_password"
    assert claims["user_id"] is None
    assert claims["target_auth_version"] == 1
    assert claims["temporary_access_generation"] == 1
    auth = {"Authorization": "Bearer " + body["http_credential"]["token"]}
    assert (
        api.client.get(
            "/api/v1/guest-browser-sessions/guest-session-1", headers=auth
        ).status_code
        == 200
    )
    assert api.client.get("/api/v1/devices", headers=auth).status_code == 401
    target = api.client.get(
        "/api/v1/device-sessions/guest-session-1", headers=_headers(api, "unbound-1")
    )
    assert target.status_code == 200, target.text
    assert target.json() == body["session"]


@pytest.mark.parametrize(
    "changes",
    [
        {"temporary_password": "WRNG2345"},
        {"target_device_id": "missing"},
        {"target_device_id": "same-owner-target"},
    ],
)
def test_wrong_code_and_password_use_one_error(device_sessions_api, changes):
    api = device_sessions_api
    assert _publish(api).status_code == 200
    response = _guest(api, **changes)
    assert response.status_code == 401, response.text
    assert response.json()["detail"]["code"] == "guest_access_invalid"
    assert api.session.query(SessionRequest).count() == 0


def test_rotation_invalidates_pending_and_disable_revokes_all(device_sessions_api):
    api = device_sessions_api
    assert _publish(api).status_code == 200
    body = _guest(api).json()
    auth = {"Authorization": "Bearer " + body["http_credential"]["token"]}
    assert _publish(api, generation=2).status_code == 200
    assert (
        api.client.get(
            "/api/v1/guest-browser-sessions/guest-session-1", headers=auth
        ).status_code
        == 404
    )
    fresh = _guest(api, session_id="guest-session-2").json()
    assert _publish(api, generation=3, enabled=False).status_code == 200
    assert (
        api.client.get(
            "/api/v1/guest-browser-sessions/guest-session-2",
            headers={"Authorization": "Bearer " + fresh["http_credential"]["token"]},
        ).status_code
        == 404
    )
    assert _guest(api, session_id="guest-session-3").status_code == 401


def _durable_fields(row):
    return {column.key: getattr(row, column.key) for column in row.__table__.columns}


def test_renewal_preserves_pending_guest_deadline_and_credentials(device_sessions_api):
    from app.models.browser_controller import BrowserController
    from app.models.device_temporary_access import DeviceTemporaryAccess
    api = device_sessions_api
    expires_ms = int((datetime.now(UTC) + timedelta(seconds=120)).timestamp() * 1000)
    assert _publish(api, expires_at_ms=expires_ms).status_code == 200
    response = _guest(api)
    assert response.status_code == 200, response.text
    body = response.json()
    auth = {"Authorization": "Bearer " + body["http_credential"]["token"]}
    row = api.session.get(SessionRequest, "guest-session-1")
    principal = api.session.scalar(select(BrowserController))
    request_before, principal_before = _durable_fields(row), _durable_fields(principal)
    assert row.authority_expires_at.replace(tzinfo=UTC) == datetime.fromtimestamp(expires_ms / 1000, UTC)

    renewed = _publish(api, expires_at_ms=expires_ms + 60_000)
    assert renewed.status_code == 200, renewed.text
    api.session.refresh(row)
    api.session.refresh(principal)
    assert _durable_fields(row) == request_before
    assert _durable_fields(principal) == principal_before
    assert api.session.scalar(select(DeviceTemporaryAccess)).expires_at.replace(tzinfo=UTC) > row.authority_expires_at.replace(tzinfo=UTC)
    inspected = api.client.get("/api/v1/guest-browser-sessions/guest-session-1", headers=auth)
    assert inspected.status_code == 200, inspected.text
    assert inspected.json()["session"]["status"] == "requested"
    # A new request may use the renewed publication; the original request and
    # the credentials returned for it keep their own immutable deadlines.
    claims_before = jwt.decode(body["credential"]["token"], options={"verify_signature": False})
    claims_after = jwt.decode(inspected.json()["credential"]["token"], options={"verify_signature": False})
    assert claims_after["exp"] == claims_before["exp"]
    assert body["http_credential"]["expires_at_ms"] <= expires_ms
    fresh = _guest(api, session_id="guest-session-2", controller_public_key=list(Ed25519PrivateKey.from_private_bytes(bytes([93]) * 32).public_key().public_bytes_raw()))
    assert fresh.status_code == 200, fresh.text
    new_request = api.session.get(SessionRequest, "guest-session-2")
    assert new_request.authority_expires_at.replace(tzinfo=UTC) > row.authority_expires_at.replace(tzinfo=UTC)


from test_wan_relay_access import wan_relay_api


def _unbound_relay_guest(api):
    from app.services.relay_signing import Ed25519RelayDirectorySigner
    from test_browser_session_api import DIRECTORY_KEY_ID, DIRECTORY_SEED

    target = api.devices["controller-1"]
    target.is_bound = False
    target.bound_user_id = None
    api.session.commit()
    api.tokens["controller-1"] = create_device_access_token(target)
    api.service._signer = Ed25519RelayDirectorySigner(
        key_id=DIRECTORY_KEY_ID, private_key_seed=DIRECTORY_SEED
    )
    assert _publish(api, target="controller-1").status_code == 200
    response = _guest(api, target_device_id="controller-1")
    assert response.status_code == 200, response.text
    approved = api.client.post(
        "/api/v1/device-sessions/guest-session-1/approve",
        headers=_headers(api, "controller-1"),
        json={"approved_scopes": ["screen.view"], "approved_profile": None},
    )
    assert approved.status_code == 200, approved.text
    return response.json()


def test_guest_approval_keeps_local_consent_and_issues_real_scoped_relay(wan_relay_api):
    api = wan_relay_api
    body = _unbound_relay_guest(api)
    auth = {"Authorization": "Bearer " + body["http_credential"]["token"]}
    inspected = api.client.get(
        "/api/v1/guest-browser-sessions/guest-session-1", headers=auth
    )
    assert inspected.status_code == 200, inspected.text
    claims = jwt.decode(
        inspected.json()["credential"]["token"],
        settings.jwt_secret.get_secret_value(),
        algorithms=["HS256"],
        issuer=settings.jwt_issuer,
        audience=settings.signaling_jwt_audience,
    )
    assert claims["allowed_scopes"] == ["screen.view"]
    relay = api.client.post(
        "/api/v1/guest-browser-sessions/guest-session-1/relay-access",
        headers=auth,
        json={},
    )
    assert relay.status_code == 200, relay.text
    assert relay.json()["credentials"] and relay.json()["generation"] == 0
    row = api.session.get(SessionRequest, "guest-session-1")
    assert row.grant_expires_at.replace(tzinfo=UTC) <= row.authority_expires_at.replace(
        tzinfo=UTC
    )
    from app.models.relay_reservation import RelayReservation

    assert all(
        r.user_id == "guest-session-guest-session-1"
        for r in api.session.scalars(select(RelayReservation))
    )
    physical = api.client.post(
        "/api/v1/relays/access",
        headers=_headers(api, "controller-1"),
        json={
            "session_id": row.id,
            "policy_revision": row.policy_revision,
            "intended_peer_id": "controller-1",
            "generation": 0,
            "refresh": False,
        },
    )
    assert physical.status_code == 200, physical.text


def test_renewal_preserves_approved_guest_grants_and_relay_reservations(wan_relay_api):
    from app.models.browser_controller import BrowserController
    from app.models.device_temporary_access import DeviceTemporaryAccess
    from app.models.relay_reservation import RelayReservation
    api = wan_relay_api
    body = _unbound_relay_guest(api)
    auth = {"Authorization": "Bearer " + body["http_credential"]["token"]}
    relay = api.client.post("/api/v1/guest-browser-sessions/guest-session-1/relay-access", headers=auth, json={})
    assert relay.status_code == 200, relay.text
    row = api.session.get(SessionRequest, "guest-session-1")
    principal = api.session.scalar(select(BrowserController))
    reservations = list(api.session.scalars(select(RelayReservation)))
    assert row.status == "approved" and reservations
    request_before, principal_before = _durable_fields(row), _durable_fields(principal)
    reservations_before = [_durable_fields(reservation) for reservation in reservations]
    access = api.session.scalar(select(DeviceTemporaryAccess))
    expires_ms = int(access.expires_at.replace(tzinfo=UTC).timestamp() * 1000) + 1000

    renewal = _publish(api, target="controller-1", expires_at_ms=expires_ms)
    assert renewal.status_code == 200, renewal.text
    api.session.refresh(row)
    api.session.refresh(principal)
    for reservation in reservations:
        api.session.refresh(reservation)
    assert _durable_fields(row) == request_before
    assert _durable_fields(principal) == principal_before
    assert [_durable_fields(reservation) for reservation in reservations] == reservations_before
    inspected = api.client.get("/api/v1/guest-browser-sessions/guest-session-1", headers=auth)
    assert inspected.status_code == 200, inspected.text
    assert inspected.json()["session"]["status"] == "approved"


def test_approved_guest_survives_rotation_only_until_original_ttl_and_disable_revokes(
    wan_relay_api,
):
    api = wan_relay_api
    body = _unbound_relay_guest(api)
    auth = {"Authorization": "Bearer " + body["http_credential"]["token"]}
    assert _publish(api, target="controller-1", generation=2).status_code == 200
    assert (
        api.client.get(
            "/api/v1/guest-browser-sessions/guest-session-1", headers=auth
        ).status_code
        == 200
    )
    assert (
        _publish(api, target="controller-1", generation=3, enabled=False).status_code
        == 200
    )
    assert (
        api.client.get(
            "/api/v1/guest-browser-sessions/guest-session-1", headers=auth
        ).status_code
        == 404
    )
    assert (
        api.client.post(
            "/api/v1/guest-browser-sessions/guest-session-1/relay-access",
            headers=auth,
            json={},
        ).status_code
        == 404
    )


def test_awaited_target_identity_cannot_issue_credential_after_disable(
    device_sessions_api, monkeypatch
):
    from app.api.v1 import browser_sessions

    api = device_sessions_api
    assert _publish(api).status_code == 200
    body = _guest(api).json()

    async def disabled_during_network(_):
        from app.models.device_temporary_access import DeviceTemporaryAccess

        access = api.session.scalar(select(DeviceTemporaryAccess))
        access.enabled = False
        access.salt = None
        access.verifier_hmac = None
        access.expires_at = None
        api.session.commit()
        return KEY_ID

    monkeypatch.setattr(
        browser_sessions, "query_realtime_target_key", disabled_during_network
    )
    response = api.client.get(
        "/api/v1/guest-browser-sessions/guest-session-1",
        headers={"Authorization": "Bearer " + body["http_credential"]["token"]},
    )
    assert response.status_code == 404


def test_failed_password_attempts_are_persisted_and_rate_limited(
    device_sessions_api, monkeypatch
):
    from app.models.device_temporary_access import GuestAccessAttempt

    api = device_sessions_api
    assert _publish(api).status_code == 200
    monkeypatch.setattr(settings, "guest_browser_device_per_minute", 2)
    for _ in range(2):
        assert _guest(api, temporary_password="WRNG2345").status_code == 401
    assert api.session.query(GuestAccessAttempt).count() == 2
    assert _guest(api).status_code == 429
    assert api.session.query(GuestAccessAttempt).count() == 2


def test_guest_token_cannot_call_account_or_machine_or_other_session(
    device_sessions_api,
):
    api = device_sessions_api
    assert _publish(api).status_code == 200
    body = _guest(api).json()
    auth = {"Authorization": "Bearer " + body["http_credential"]["token"]}
    assert (
        api.client.get("/api/v1/guest-browser-sessions/other", headers=auth).status_code
        == 404
    )
    assert (
        api.client.post(
            "/api/v1/browser-sessions", headers=auth, json=_browser_request()
        ).status_code
        == 401
    )
    assert (
        api.client.post(
            "/api/v1/realtime/device-credentials",
            headers={"X-Rdesk-Device-Authorization": auth["Authorization"]},
            json={"device_key_id": KEY_ID, "role": "Agent"},
        ).status_code
        == 401
    )
    assert (
        api.client.get(
            "/api/v1/guest-browser-sessions/guest-session-1",
            headers={"Authorization": "Bearer " + body["credential"]["token"]},
        ).status_code
        == 401
    )


def test_guest_target_pin_must_match_published_physical_machine(
    device_sessions_api, monkeypatch
):
    from app.api.v1 import browser_sessions

    api = device_sessions_api
    assert _publish(api).status_code == 200

    async def mismatch(_):
        return "cd" * 32

    monkeypatch.setattr(browser_sessions, "query_realtime_target_key", mismatch)
    response = _guest(api)
    assert response.status_code == 503, response.text
    assert api.session.query(SessionRequest).count() == 0


def test_guest_expiry_auth_version_and_close_fail_closed(device_sessions_api):
    api = device_sessions_api
    assert _publish(api).status_code == 200
    body = _guest(api).json()
    auth = {"Authorization": "Bearer " + body["http_credential"]["token"]}
    assert (
        api.client.post(
            "/api/v1/guest-browser-sessions/guest-session-1/close",
            headers=auth,
            json={},
        ).status_code
        == 200
    )
    assert (
        api.client.get(
            "/api/v1/guest-browser-sessions/guest-session-1", headers=auth
        ).status_code
        == 404
    )
    assert _guest(api).status_code in (401, 409)
    fresh = _guest(api, session_id="guest-session-2").json()
    auth = {"Authorization": "Bearer " + fresh["http_credential"]["token"]}
    api.devices["unbound-1"].auth_version += 1
    api.session.commit()
    assert (
        api.client.get(
            "/api/v1/guest-browser-sessions/guest-session-2", headers=auth
        ).status_code
        == 404
    )


def test_published_verifier_is_peppered_not_stored_password_hash(device_sessions_api):
    from app.models.device_temporary_access import DeviceTemporaryAccess

    api = device_sessions_api
    response = _publish(api)
    assert response.status_code == 200
    access = api.session.scalar(select(DeviceTemporaryAccess))
    verifier = hashlib.pbkdf2_hmac("sha256", PASSWORD.encode(), SALT, 600000)
    assert access.verifier_hmac != verifier
    raw_publication = json.loads(response.request.content)["access_json"]
    assert (
        access.publication_digest
        != hashlib.sha256(raw_publication.encode()).hexdigest()
    )
    assert PASSWORD not in str(access.__dict__)
    assert "verifier" not in response.text


@pytest.mark.parametrize("expires", [0, 2**63 - 1])
def test_signed_invalid_expiry_is_uniform_not_internal_error(
    device_sessions_api, expires
):
    response = _publish(device_sessions_api, expires_at_ms=expires)
    assert response.status_code == 401
    assert response.json()["detail"]["code"] == "guest_access_invalid"


def test_signed_publication_exact_retry_and_stale_generation(device_sessions_api):
    api = device_sessions_api
    first = _publish(api)
    assert first.status_code == 200
    payload = json.loads(first.request.content)
    retry = api.client.post(
        "/api/v1/devices/temporary-access",
        headers=_headers(api, "unbound-1"),
        json=payload,
    )
    assert retry.status_code == 200
    assert retry.json() == first.json()
    assert _publish(api, generation=2).status_code == 200
    assert (
        api.client.post(
            "/api/v1/devices/temporary-access",
            headers=_headers(api, "unbound-1"),
            json=payload,
        ).status_code
        == 401
    )


def test_rotated_password_rejects_old_password_and_accepts_new_without_account(
    device_sessions_api,
):
    api = device_sessions_api
    assert _publish(api).status_code == 200
    new = "BCDEFGH3"
    verifier = hashlib.pbkdf2_hmac("sha256", new.encode(), SALT, 600000).hex()
    assert _publish(api, generation=2, verifier=verifier).status_code == 200
    assert _guest(api).status_code == 401
    assert _guest(api, temporary_password=new).status_code == 200


def test_shared_native_temporary_publication_signature_vector():
    from pathlib import Path
    from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey
    from app.schemas.guest_browser import TemporaryAccessPublishIn
    from app.services.temporary_access import (
        canonical_temporary_publication,
        contextual_temporary_publication,
    )

    vector = json.loads(
        (
            Path(__file__).parent / "fixtures" / "temporary-access-publication-v1.json"
        ).read_text()
    )
    payload = TemporaryAccessPublishIn(
        **{
            key: vector[key]
            for key in ["key_id", "public_key", "access_json", "signature"]
        }
    )
    canonical = canonical_temporary_publication(vector["api_url"], payload)
    assert canonical.decode() == vector["canonical_utf8"]
    message = contextual_temporary_publication(canonical)
    assert message.hex() == vector["contextual_message_hex"]
    Ed25519PublicKey.from_public_bytes(bytes.fromhex(vector["public_key"])).verify(
        bytes.fromhex(vector["signature"]), message
    )


@pytest.mark.parametrize("invalid_shape", ["duplicate", "multiple_audiences"])
def test_guest_http_credentials_reject_duplicate_and_ambiguous_audience(
    device_sessions_api, invalid_shape
):
    import hmac

    api = device_sessions_api
    assert _publish(api).status_code == 200
    body = _guest(api).json()
    original = body["http_credential"]["token"]
    parts = original.split(".")
    claims = json.loads(base64.urlsafe_b64decode(parts[1] + "=" * (-len(parts[1]) % 4)))
    if invalid_shape == "multiple_audiences":
        claims["aud"] = ["rdesk-guest-browser-http", settings.jwt_audience]
        token = jwt.encode(claims, JWT_SECRET, algorithm="HS256")
    else:
        encoded = json.dumps(claims, separators=(",", ":"))
        duplicate = '{"user_id":null,' + encoded[1:]
        payload = base64.urlsafe_b64encode(duplicate.encode()).rstrip(b"=").decode()
        signed = parts[0] + "." + payload
        signature = (
            base64.urlsafe_b64encode(
                hmac.digest(JWT_SECRET.encode(), signed.encode(), "sha256")
            )
            .rstrip(b"=")
            .decode()
        )
        token = signed + "." + signature
    response = api.client.get(
        "/api/v1/guest-browser-sessions/guest-session-1",
        headers={"Authorization": "Bearer " + token},
    )
    assert response.status_code == 401


def test_guest_credential_revalidation_preserves_original_tenant_snapshot(
    device_sessions_api, monkeypatch
):
    from app.api.v1 import browser_sessions
    from app.models.browser_controller import BrowserController

    api = device_sessions_api
    assert _publish(api).status_code == 200
    body = _guest(api).json()

    async def changed_while_querying(_):
        row = api.session.get(SessionRequest, "guest-session-1")
        principal = api.session.scalar(select(BrowserController))
        row.tenant_id = principal.tenant_id = "tenant-b"
        api.session.get(Device, row.requester_device_id).tenant_id = "tenant-b"
        api.devices["unbound-1"].tenant_id = "tenant-b"
        api.session.commit()
        return KEY_ID

    monkeypatch.setattr(
        browser_sessions, "query_realtime_target_key", changed_while_querying
    )
    response = api.client.get(
        "/api/v1/guest-browser-sessions/guest-session-1",
        headers={"Authorization": "Bearer " + body["http_credential"]["token"]},
    )
    assert response.status_code == 404


@pytest.mark.parametrize("expiry_stage", ["capacity", "retry", "policy"])
def test_guest_approval_cannot_finish_after_temporary_authority_expiry(
    wan_relay_api, monkeypatch, expiry_stage
):
    from app.api.v1 import device_sessions
    from app.models.browser_controller import BrowserController
    from app.models.device_temporary_access import DeviceTemporaryAccess
    from app.models.relay_access_generation import RelayAccessGeneration
    from app.models.relay_reservation import RelayReservation
    from app.services.device_sessions import DeviceSessionService

    api = wan_relay_api
    target = api.devices["controller-1"]
    target.is_bound = False
    target.bound_user_id = None
    api.session.commit()
    api.tokens["controller-1"] = create_device_access_token(target)
    assert _publish(api, target="controller-1").status_code == 200
    assert _guest(api, target_device_id="controller-1").status_code == 200
    retry = expiry_stage == "retry"
    clock = [datetime.now(UTC)]
    deadline = clock[0] + timedelta(seconds=30 if expiry_stage == "policy" else 5)
    api.session.scalar(select(DeviceTemporaryAccess)).expires_at = deadline
    api.session.scalar(select(BrowserController)).expires_at = deadline
    row = api.session.get(SessionRequest, "guest-session-1")
    row.authority_expires_at = deadline
    api.session.commit()
    monkeypatch.setattr(
        device_sessions,
        "_service",
        lambda db: DeviceSessionService(db, now=lambda: clock[0]),
    )
    api.service._now = lambda: clock[0]
    approval = {"approved_scopes": ["screen.view"], "approved_profile": None}
    path = "/api/v1/device-sessions/guest-session-1/approve"
    if expiry_stage == "policy":
        from dataclasses import replace
        from app.services import device_sessions as session_domain

        api.service._current_policy = replace(
            api.service._current_policy, grant_ttl_seconds=5, policy_ttl_seconds=5
        )
        delegate = session_domain.browser_authority_valid

        async def delayed_policy(*args, **kwargs):
            valid = await delegate(*args, **kwargs)
            if args[1].status == "approved":
                clock[0] = deadline - timedelta(seconds=24)
            return valid

        monkeypatch.setattr(session_domain, "browser_authority_valid", delayed_policy)
    elif retry:
        assert (
            api.client.post(
                path, headers=_headers(api, "controller-1"), json=approval
            ).status_code
            == 200
        )
        delegate = api.service.validate_wan_generation_locked

        async def delayed_retry(**kwargs):
            result = await delegate(**kwargs)
            clock[0] = deadline + timedelta(seconds=1)
            return result

        monkeypatch.setattr(
            api.service, "validate_wan_generation_locked", delayed_retry
        )
    else:
        delegate = api.service._repository.reserve_capacity

        async def delayed_capacity(**kwargs):
            result = await delegate(**kwargs)
            clock[0] = deadline + timedelta(seconds=1)
            return result

        monkeypatch.setattr(
            api.service._repository, "reserve_capacity", delayed_capacity
        )
    response = api.client.post(
        path, headers=_headers(api, "controller-1"), json=approval
    )
    assert response.status_code == 404, response.text
    api.session.expire_all()
    row = api.session.get(SessionRequest, "guest-session-1")
    if not retry:
        assert row.status == "requested"
        assert row.active_relay_generation is None
        assert api.session.query(RelayAccessGeneration).count() == 0
        assert api.session.query(RelayReservation).count() == 0
