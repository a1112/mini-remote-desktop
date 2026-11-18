"""Synthetic provider exercises the real RSA verification and API state machine."""
from __future__ import annotations

import base64
import asyncio
import hashlib
import json
from datetime import UTC, datetime, timedelta
from urllib.parse import parse_qs, urlsplit

import jwt
import httpx
import pytest
from cryptography.fernet import Fernet
from cryptography.hazmat.primitives.asymmetric import rsa
from fastapi import FastAPI
from fastapi.testclient import TestClient
from pydantic import SecretStr, ValidationError
from sqlalchemy import create_engine, select
from sqlalchemy.orm import Session
from sqlalchemy.pool import StaticPool

import app.models  # noqa: F401
from app.api.v1.oidc import router
from app.core.config import Settings, settings
from app.core.response_security import SensitiveResponseCacheMiddleware
from app.core.security import create_access_token
from app.db.session import Base, get_db
from app.models.oidc import OidcIdentity, OidcLoginTransaction
from app.models.user import User
from app.services import oidc


ISSUER = "https://identity.example.test/realms/r-auth"
CLIENT = "rdesk"
REDIRECT = "https://rdesk.example.test/rdesk/api/v1/auth/oidc/callback"
KEY = Fernet.generate_key().decode()


class AsyncSessionShim:
    def __init__(self, session):
        self.session = session
    def add(self, instance):
        self.session.add(instance)
    async def scalar(self, *args, **kwargs):
        return self.session.scalar(*args, **kwargs)
    async def execute(self, *args, **kwargs):
        return self.session.execute(*args, **kwargs)
    async def flush(self):
        self.session.flush()
    async def commit(self):
        self.session.commit()
    async def rollback(self):
        self.session.rollback()


class Provider:
    def __init__(self):
        self.key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
        self.bad_key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
        self.nonce = ""
        self.subject = "stable-subject"
        self.mutation = None
        self.verifier = ""

    async def request(self, method, url, **kwargs):
        if url.endswith("/.well-known/openid-configuration"):
            return {
                "issuer": ISSUER, "authorization_endpoint": ISSUER + "/auth",
                "token_endpoint": ISSUER + "/token", "jwks_uri": ISSUER + "/certs",
                "response_types_supported": ["code"], "code_challenge_methods_supported": ["S256"],
                "id_token_signing_alg_values_supported": ["RS256"],
                "token_endpoint_auth_methods_supported": ["client_secret_basic"],
                "authorization_response_iss_parameter_supported": True,
            }
        if url == ISSUER + "/certs":
            key = json.loads(jwt.algorithms.RSAAlgorithm.to_jwk(self.key.public_key()))
            key.update({"kid": "key-1", "alg": "RS256", "use": "sig"})
            return {"keys": [key]}
        assert method == "POST" and url == ISSUER + "/token"
        assert kwargs["headers"]["Authorization"].startswith("Basic ")
        assert kwargs["data"]["redirect_uri"] == REDIRECT
        self.verifier = kwargs["data"]["code_verifier"]
        now = int(datetime.now(UTC).timestamp())
        claims = {"iss": ISSUER, "sub": self.subject, "aud": CLIENT,
                  "iat": now, "exp": now + 300, "nonce": self.nonce,
                  "email": "legacy@example.test", "preferred_username": "legacy"}
        if self.mutation == "nonce": claims["nonce"] = "incorrect"
        if self.mutation == "issuer": claims["iss"] = "https://other.example.test"
        if self.mutation == "audience": claims["aud"] = "other-client"
        if self.mutation == "azp": claims["aud"] = [CLIENT, "another-client"]
        if self.mutation == "expired": claims["exp"] = now - 1
        signing_key = self.bad_key if self.mutation == "signature" else self.key
        token = jwt.encode(claims, signing_key, algorithm="RS256", headers={"kid": "key-1"})
        return {"id_token": token, "access_token": "provider-access-token-never-returned"}


@pytest.fixture
def api(monkeypatch):
    for name, value in {
        "oidc_enabled": True, "oidc_issuer": ISSUER, "oidc_client_id": CLIENT,
        "oidc_client_secret": SecretStr("test-client-secret-long-enough"),
        "oidc_redirect_uri": REDIRECT, "oidc_transaction_key": SecretStr(KEY),
        "jwt_secret": "super-secret-JWT-for-tests-12345!@#$", "jwt_issuer": "https://rdesk.example.test",
        "jwt_audience": "rdesk-api", "jwt_expire_minutes": 30, "jwt_max_lifetime_minutes": 60,
    }.items():
        monkeypatch.setattr(settings, name, value)
    oidc_api = __import__("app.api.v1.oidc", fromlist=["router"])
    oidc_api._attempts.clear()
    oidc_api._global_attempts.clear()
    provider = Provider()
    monkeypatch.setattr(oidc, "_json_request", provider.request)
    engine = create_engine("sqlite:///:memory:", connect_args={"check_same_thread": False}, poolclass=StaticPool)
    Base.metadata.create_all(engine)
    session = Session(engine, expire_on_commit=False)
    async def override_db():
        yield AsyncSessionShim(session)
    app = FastAPI()
    app.add_middleware(SensitiveResponseCacheMiddleware)
    app.include_router(router, prefix="/api/v1")
    app.dependency_overrides[get_db] = override_db
    with TestClient(app) as client:
        yield client, session, provider
    session.close()
    engine.dispose()


def start(api, link_token=None):
    client, _, provider = api
    path = "/api/v1/auth/oidc/link/start" if link_token else "/api/v1/auth/oidc/start"
    headers = {"Authorization": "Bearer " + link_token} if link_token else {}
    response = client.post(path, headers=headers)
    assert response.status_code == 200, response.text
    result = response.json()
    query = parse_qs(urlsplit(result["authorization_url"]).query)
    provider.nonce = query["nonce"][0]
    assert query["response_type"] == ["code"]
    assert query["scope"] == ["openid"]
    assert query["code_challenge_method"] == ["S256"]
    assert result["completion_token"] not in result["authorization_url"]
    assert "access_token" not in result["authorization_url"]
    return result, query


def callback(api, query, **params):
    client, _, _ = api
    return client.get("/api/v1/auth/oidc/callback", params={
        "state": query["state"][0], "code": "one-use-provider-code", "iss": ISSUER, **params,
    })


def finish(api, result, token=None):
    headers = {"Authorization": "Bearer " + token} if token else {}
    return api[0].post("/api/v1/auth/oidc/complete", json={"completion_token": result["completion_token"]}, headers=headers)


def test_verified_flow_is_separate_single_use_and_pkce_bound(api):
    client, session, provider = api
    session.add(User(id="legacy", username="legacy", email="legacy@example.test", password_hash="!", role="user"))
    session.commit()
    flow, query = start(api)
    assert finish(api, flow).status_code == 202
    stored = session.scalar(select(OidcLoginTransaction))
    assert stored.completion_digest != flow["completion_token"]
    assert stored.state_digest != query["state"][0]
    assert query["nonce"][0] == stored.nonce
    response = callback(api, query)
    assert response.status_code == 200
    assert response.headers["referrer-policy"] == "no-referrer"
    assert "provider-access-token" not in response.text
    challenge = base64.urlsafe_b64encode(hashlib.sha256(provider.verifier.encode()).digest()).rstrip(b"=").decode()
    assert challenge == query["code_challenge"][0]
    assert callback(api, query).status_code == 400
    complete = finish(api, flow)
    assert complete.status_code == 200 and complete.headers["cache-control"].startswith("no-store")
    assert complete.json()["user_id"] != "legacy"
    assert "provider-access-token" not in complete.text
    assert finish(api, flow).status_code == 400
    mapping = session.scalar(select(OidcIdentity))
    assert (mapping.issuer, mapping.subject, mapping.user_id) == (ISSUER, provider.subject, complete.json()["user_id"])
    second, second_query = start(api)
    assert callback(api, second_query).status_code == 200
    assert finish(api, second).json()["user_id"] == mapping.user_id


@pytest.mark.parametrize("mutation", ["nonce", "issuer", "audience", "azp", "expired", "signature"])
def test_invalid_id_token_never_provisions_or_completes(api, mutation):
    _, session, provider = api
    flow, query = start(api)
    provider.mutation = mutation
    assert callback(api, query).status_code == 400
    assert finish(api, flow).status_code == 400
    assert session.scalar(select(OidcIdentity)) is None
    assert session.scalar(select(User)) is None


def test_state_issuer_and_duplicate_callback_parameters_reject(api):
    client, session, _ = api
    flow, query = start(api)
    assert callback(api, query, iss="https://wrong.example.test").status_code == 400
    assert finish(api, flow).status_code == 400
    second, second_query = start(api)
    duplicate = client.get("/api/v1/auth/oidc/callback", params=[
        ("state", second_query["state"][0]), ("state", "bad"), ("code", "code"), ("iss", ISSUER)])
    assert duplicate.status_code == 400
    assert session.scalar(select(OidcIdentity)) is None
    assert finish(api, second).status_code == 202


def test_claimed_response_issuer_and_flow_configuration_must_match(api, monkeypatch):
    client, _, _ = api
    flow, query = start(api)
    no_issuer = client.get("/api/v1/auth/oidc/callback", params={"state": query["state"][0], "code": "code"})
    assert no_issuer.status_code == 400 and finish(api, flow).status_code == 400
    second, second_query = start(api)
    monkeypatch.setattr(settings, "oidc_client_id", "changed-client")
    assert callback(api, second_query).status_code == 400
    assert finish(api, second).status_code == 400


def test_expired_state_and_completion_are_rejected(api):
    _, session, _ = api
    flow, query = start(api)
    stored = session.scalar(select(OidcLoginTransaction))
    stored.expires_at = datetime.now(UTC) - timedelta(seconds=1)
    session.commit()
    assert callback(api, query).status_code == 400
    assert finish(api, flow).status_code == 400


def test_link_requires_local_bearer_and_never_moves_existing_mapping(api):
    client, session, provider = api
    a = User(id="local-a", username="local-a", email="a@example.test", password_hash="!", role="user")
    b = User(id="local-b", username="local-b", email="b@example.test", password_hash="!", role="user")
    session.add_all([a, b])
    session.commit()
    assert client.post("/api/v1/auth/oidc/link/start").status_code in (401, 403)
    token_a = create_access_token(a.id, a.username, a.role, a.session_version)
    token_b = create_access_token(b.id, b.username, b.role, b.session_version)
    flow, query = start(api, token_a)
    assert callback(api, query).status_code == 200
    assert session.scalar(select(OidcIdentity)) is None
    assert finish(api, flow).status_code == 400
    assert finish(api, flow, token_b).status_code == 400
    assert session.scalar(select(OidcIdentity)) is None
    assert finish(api, flow, token_a).json()["user_id"] == a.id
    other, other_query = start(api, token_b)
    assert callback(api, other_query).status_code == 200
    assert finish(api, other, token_b).status_code == 400
    assert session.scalar(select(OidcIdentity)).user_id == a.id
    assert provider.subject == "stable-subject"


def test_link_rechecks_local_session_before_callback_and_completion(api):
    _, session, _ = api
    user = User(id="local", username="local", email="local@example.test", password_hash="!", role="user")
    session.add(user)
    session.commit()
    token = create_access_token(user.id, user.username, user.role, user.session_version)
    flow, query = start(api, token)
    user.session_version += 1
    session.commit()
    assert callback(api, query).status_code == 400
    assert session.scalar(select(OidcIdentity)) is None
    token = create_access_token(user.id, user.username, user.role, user.session_version)
    second, second_query = start(api, token)
    assert callback(api, second_query).status_code == 200
    user.session_version += 1
    session.commit()
    assert session.scalar(select(OidcIdentity)) is None
    assert finish(api, second, token).status_code == 400
    assert session.scalar(select(OidcIdentity)) is None


def test_disabled_and_strict_settings(api, monkeypatch):
    client, _, _ = api
    monkeypatch.setattr(settings, "oidc_enabled", False)
    assert client.get("/api/v1/auth/oidc/capabilities").json() == {"enabled": False}
    assert client.post("/api/v1/auth/oidc/start").status_code == 404
    config = dict(oidc_enabled=True, oidc_issuer=ISSUER, oidc_client_id=CLIENT,
                  oidc_client_secret="test-client-secret-long-enough", oidc_redirect_uri=REDIRECT,
                  oidc_transaction_key=KEY, _env_file=None)
    assert Settings(**config).oidc_enabled
    for field, bad in [("oidc_issuer", "http://identity.example.test"),
                       ("oidc_issuer", ISSUER + "?evil=yes"),
                       ("oidc_redirect_uri", "https://evil.test/callback"),
                       ("oidc_transaction_key", "bad"), ("oidc_client_secret", "short")]:
        with pytest.raises(ValidationError):
            Settings(**{**config, field: bad})


def test_endpoint_must_share_issuer_https_origin(api):
    for value in ("http://identity.example.test/token", "https://evil.test/token",
                  "https://identity.example.test:444/token", "https://identity.example.test/token?x=1"):
        with pytest.raises(oidc.OidcFailure):
            oidc._endpoint(value)


def test_trusted_proxy_separates_clients_and_rejects_ambiguous_addresses(api, monkeypatch):
    monkeypatch.setattr(settings, "oidc_trusted_proxies", "127.0.0.1/32")
    path = "/api/v1/auth/oidc/start"
    with TestClient(api[0].app, client=("127.0.0.1", 5000)) as proxied:
        for _ in range(10):
            assert proxied.post(path, headers={"X-Real-IP": "192.0.2.1"}).status_code == 200
        assert proxied.post(path, headers={"X-Real-IP": "::ffff:192.0.2.1"}).status_code == 429
        assert proxied.post(path, headers={"X-Real-IP": "192.0.2.2"}).status_code == 200
        for headers in (
            [("X-Real-IP", "192.0.2.3"), ("X-Real-IP", "192.0.2.4")],
            [("X-Real-IP", "192.0.2.3, 192.0.2.4")],
            [("X-Real-IP", "fe80::1%en0")],
        ):
            assert proxied.post(path, headers=headers).status_code == 400


def test_direct_requests_cannot_select_their_limit_bucket(api, monkeypatch):
    monkeypatch.setattr(settings, "oidc_trusted_proxies", "127.0.0.1/32")
    path = "/api/v1/auth/oidc/start"
    with TestClient(api[0].app, client=("192.0.2.100", 5000)) as direct:
        for index in range(10):
            assert direct.post(path, headers={"X-Real-IP": f"198.51.100.{index}"}).status_code == 200
        assert direct.post(path, headers={"X-Real-IP": "198.51.100.11"}).status_code == 429
    for unsafe in ("*", "0.0.0.0/0", "::/0"):
        with pytest.raises(ValidationError, match="RDESK_OIDC_TRUSTED_PROXIES"):
            Settings(oidc_trusted_proxies=unsafe, _env_file=None)


@pytest.mark.asyncio
async def test_provider_http_disables_redirects_and_rejects_ambiguous_or_oversized_json(monkeypatch):
    actual_client = httpx.AsyncClient
    status, body = 200, b'{"ok":true}'
    def handle(_request):
        return httpx.Response(status, content=body)
    transport = httpx.MockTransport(handle)
    def client_factory(**kwargs):
        assert kwargs["follow_redirects"] is False and kwargs["trust_env"] is False
        return actual_client(transport=transport, **kwargs)
    monkeypatch.setattr(oidc.httpx, "AsyncClient", client_factory)
    assert await oidc._json_request("GET", "https://identity.example.test/test") == {"ok": True}
    status = 302
    with pytest.raises(oidc.OidcFailure):
        await oidc._json_request("GET", "https://identity.example.test/test")
    status, body = 200, b'{"value":1,"value":2}'
    with pytest.raises(oidc.OidcFailure):
        await oidc._json_request("GET", "https://identity.example.test/test")
    body = b"a" * 131_073
    with pytest.raises(oidc.OidcFailure):
        await oidc._json_request("GET", "https://identity.example.test/test")


@pytest.mark.asyncio
async def test_provider_http_has_total_deadline(monkeypatch):
    actual_client = httpx.AsyncClient
    async def slow(_request):
        await asyncio.sleep(0.1)
        return httpx.Response(200, json={})
    transport = httpx.MockTransport(slow)
    monkeypatch.setattr(oidc, "PROVIDER_DEADLINE_SECONDS", 0.01)
    monkeypatch.setattr(oidc.httpx, "AsyncClient", lambda **kwargs: actual_client(transport=transport, **kwargs))
    with pytest.raises(oidc.OidcFailure):
        await oidc._json_request("GET", "https://identity.example.test/test")
