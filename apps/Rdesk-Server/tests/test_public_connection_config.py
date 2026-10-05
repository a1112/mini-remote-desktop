import base64

from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from cryptography.hazmat.primitives.serialization import Encoding, PublicFormat
from fastapi import FastAPI
from fastapi.testclient import TestClient
from pydantic import SecretStr


def configured_client(monkeypatch):
    from app.api.v1 import public
    monkeypatch.setattr(public.settings, "public_api_url", "https://175.178.16.90/rdesk/api/v1")
    monkeypatch.setattr(public.settings, "signaling_ws_url", "wss://175.178.16.90/rdesk-realtime/ws")
    monkeypatch.setattr(public.settings, "relay_directory_signing_key_id", "test-public")
    monkeypatch.setattr(public.settings, "relay_directory_signing_private_key", SecretStr(base64.b64encode(bytes(range(32))).decode()))
    app = FastAPI()
    app.include_router(public.router)
    return TestClient(app), public


def test_bootstrap_returns_only_verified_public_metadata(monkeypatch):
    client, public = configured_client(monkeypatch)
    response = client.get("/public/connection-config")
    assert response.status_code == 200
    body = response.json()
    assert body["signaling_url"].startswith("wss://175.178.16.90/")
    assert body["relay_directory_url"] == "https://175.178.16.90/rdesk/api/v1/relays/access"
    expected = Ed25519PrivateKey.from_private_bytes(bytes(range(32))).public_key().public_bytes(Encoding.Raw, PublicFormat.Raw)
    assert body["relay_directory_keys"] == {"test-public": base64.b64encode(expected).decode()}
    for secret in ["private_key", "turn_auth_secret", "access_token", public.settings.relay_directory_signing_private_key.get_secret_value()]:
        assert secret not in response.text


def test_missing_keys_and_changed_origin_fail_closed(monkeypatch):
    client, public = configured_client(monkeypatch)
    monkeypatch.setattr(public.settings, "signaling_ws_url", "wss://untrusted.example/ws")
    assert client.get("/public/connection-config").status_code == 503
    monkeypatch.setattr(public.settings, "signaling_ws_url", "wss://175.178.16.90/rdesk-realtime/ws")
    monkeypatch.setattr(public.settings, "relay_directory_signing_private_key", SecretStr("invalid-secret"))
    response = client.get("/public/connection-config")
    assert response.status_code == 503
    assert "invalid-secret" not in response.text
