import base64
import hashlib
import hmac
from types import SimpleNamespace

from fastapi import FastAPI
from fastapi.testclient import TestClient

from app.api.v1.turn import get_turn_credential_service, router
from app.core.security import get_current_user
from app.services.turn_credentials import TurnCredentialService


NOW = 1_800_000_000
SECRET = "test-turn-secret"


def service() -> TurnCredentialService:
    return TurnCredentialService(
        auth_secret=SECRET,
        urls=[
            "turn:relay.example.test:3478?transport=udp",
            "turns:relay.example.test:5349?transport=tcp",
        ],
        ttl_seconds=600,
        now=lambda: NOW,
    )


def app(*, authenticated: bool) -> FastAPI:
    test_app = FastAPI()
    test_app.include_router(router, prefix="/api/v1")
    test_app.dependency_overrides[get_turn_credential_service] = service
    if authenticated:
        test_app.dependency_overrides[get_current_user] = lambda: SimpleNamespace(
            id="user-42"
        )
    return test_app


def test_authenticated_request_returns_short_lived_session_scoped_credentials():
    client = TestClient(app(authenticated=True))
    response = client.post(
        "/api/v1/turn/credentials",
        json={
            "session_id": "session-7",
            "credential_deadline_unix_seconds": NOW + 300,
        },
    )

    assert response.status_code == 200
    payload = response.json()
    assert payload["expires_at_unix_seconds"] == NOW + 300
    assert payload["ttl_seconds"] == 300
    assert payload["username"] == f"{NOW + 300}:user-42:session-7"
    expected = base64.b64encode(
        # Keep the expected wire-format vector aligned with TURN REST HMAC-SHA1.
        # lgtm[py/weak-sensitive-data-hashing]
        hmac.new(
            SECRET.encode(), payload["username"].encode(), hashlib.sha1
        ).digest()
    ).decode()
    assert hmac.compare_digest(payload["credential"], expected)
    assert payload["urls"] == [
        "turn:relay.example.test:3478?transport=udp",
        "turns:relay.example.test:5349?transport=tcp",
    ]
    assert payload["transport_policy"] == "relay"


def test_anonymous_and_expired_requests_are_rejected():
    anonymous = TestClient(app(authenticated=False)).post(
        "/api/v1/turn/credentials",
        json={
            "session_id": "session-7",
            "credential_deadline_unix_seconds": NOW + 300,
        },
    )
    assert anonymous.status_code in {401, 403}

    expired = TestClient(app(authenticated=True)).post(
        "/api/v1/turn/credentials",
        json={
            "session_id": "session-7",
            "credential_deadline_unix_seconds": NOW,
        },
    )
    assert expired.status_code == 410


def test_service_rejects_invalid_scope_and_never_exceeds_configured_ttl():
    issuer = service()
    credential = issuer.issue(
        user_id="user-42",
        session_id="session-7",
        credential_deadline_unix_seconds=NOW + 3_600,
    )
    assert credential.expires_at_unix_seconds == NOW + 600
    assert issuer.verify(credential.username, credential.credential, NOW + 599)
    assert not issuer.verify(credential.username, credential.credential, NOW + 601)

    try:
        issuer.issue(
            user_id="user-42",
            session_id="bad:scope",
            credential_deadline_unix_seconds=NOW + 300,
        )
    except ValueError as error:
        assert "session_id" in str(error)
    else:
        raise AssertionError("invalid session scope must be rejected")
