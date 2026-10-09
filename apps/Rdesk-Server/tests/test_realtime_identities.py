import base64
import hashlib
import hmac
import json
import httpx
import pytest
from pydantic import SecretStr
from app.core.config import settings
from app.services import realtime_identities as identities

NOW = 1_790_000_000_000
KEY = bytes(range(32))
CLIENT = httpx.AsyncClient


@pytest.fixture
def anyio_backend():
    return "asyncio"


@pytest.fixture(autouse=True)
def configured(monkeypatch):
    monkeypatch.setattr(settings, "realtime_presence_secret", SecretStr(base64.urlsafe_b64encode(KEY).rstrip(b"=").decode()))
    monkeypatch.setattr(settings, "realtime_presence_url", "http://127.0.0.1:9542/internal/presence")
    monkeypatch.setattr(identities, "_now_ms", lambda: NOW)


def _sidecar(monkeypatch, body, status=200):
    def handler(request):
        assert str(request.url) == "http://127.0.0.1:9542/internal/identities"
        assert json.loads(request.content) == {"device_ids": ["target-1"]}
        token = base64.urlsafe_b64encode(hmac.new(KEY, b"MRD_REALTIME_PRESENCE_QUERY_V1\0", hashlib.sha256).digest()).rstrip(b"=").decode()
        assert request.headers["authorization"] == f"Bearer {token}"
        return httpx.Response(status, json=body)
    monkeypatch.setattr(identities.httpx, "AsyncClient", lambda **opts: CLIENT(transport=httpx.MockTransport(handler), **opts))


@pytest.mark.anyio
async def test_authenticated_identity_lookup_returns_verified_target_pin(monkeypatch):
    _sidecar(monkeypatch, {"version": 1, "sampled_at_ms": NOW, "identities": [
        {"device_id": "target-1", "device_key_id": "ab" * 32, "role": "Agent"}]})
    assert await identities.query_realtime_target_key("target-1") == "ab" * 32


@pytest.mark.anyio
@pytest.mark.parametrize("mutation", ["controller", "missing", "foreign", "duplicate", "key", "stale", "extra"])
async def test_invalid_identity_cannot_be_used_as_a_target_pin(monkeypatch, mutation):
    body = {"version": 1, "sampled_at_ms": NOW, "identities": [
        {"device_id": "target-1", "device_key_id": "ab" * 32, "role": "Agent"}]}
    if mutation == "controller": body["identities"][0]["role"] = "Controller"
    elif mutation == "missing": body["identities"] = []
    elif mutation == "foreign": body["identities"][0]["device_id"] = "other"
    elif mutation == "duplicate": body["identities"] *= 2
    elif mutation == "key": body["identities"][0]["device_key_id"] = "invalid"
    elif mutation == "stale": body["sampled_at_ms"] -= 5001
    elif mutation == "extra": body["token"] = "do-not-accept"
    _sidecar(monkeypatch, body)
    assert await identities.query_realtime_target_key("target-1") is None


@pytest.mark.anyio
async def test_identity_lookup_never_sends_credentials_to_non_loopback(monkeypatch):
    monkeypatch.setattr(settings, "realtime_presence_url", "http://evil.test/internal/presence")
    monkeypatch.setattr(identities.httpx, "AsyncClient", lambda **_: pytest.fail("unsafe endpoint"))
    assert await identities.query_realtime_target_key("target-1") is None
