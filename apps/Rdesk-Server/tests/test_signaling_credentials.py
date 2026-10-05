from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import socket
import struct
import subprocess
import time
from urllib.request import urlopen

from fastapi import FastAPI
from fastapi.testclient import TestClient
import jwt
import pytest

from app.api.v1.realtime import router
from app.core.config import settings
from app.core.response_security import SensitiveResponseCacheMiddleware
from app.core.security import create_device_access_token, create_access_token
from app.db.session import get_db
from app.models.device import Device


SECRET = "signaling-test-secret-0123456789-ABCDEFGHIJKLMNOPQRSTUVWXYZ"
KEY_ID = "ab" * 32


@pytest.fixture
def client(monkeypatch):
    monkeypatch.setattr(settings, "jwt_secret", SECRET)
    monkeypatch.setattr(settings, "jwt_issuer", "rdesk-tests")
    monkeypatch.setattr(settings, "jwt_audience", "rdesk-users")
    monkeypatch.setattr(settings, "device_jwt_audience", "rdesk-devices")
    device = Device(id="row-a", device_id="012345678901", name="test",
                    os="Windows", tenant_id="tenant-a", auth_version=1,
                    is_bound=False, auth_revoked_at=None)

    class Database:
        async def scalar(self, statement):
            return device

    async def database():
        yield Database()

    app = FastAPI()
    app.include_router(router, prefix="/api/v1")
    app.add_middleware(SensitiveResponseCacheMiddleware)
    app.dependency_overrides[get_db] = database
    with TestClient(app) as http:
        yield http, device


def request(http, token=None, **body):
    headers = {"X-Rdesk-Device-Authorization": f"Bearer {token}"} if token else {}
    return http.post("/api/v1/realtime/device-credentials", headers=headers,
                     json={"device_key_id": KEY_ID, "role": "Controller", **body})


@pytest.mark.parametrize("role", ["Controller", "Agent", "Peer"])
def test_device_token_mints_key_and_role_bound_signaling_credential(client, role):
    http, device = client
    before = int(datetime.now(timezone.utc).timestamp())
    response = request(http, create_device_access_token(device), role=role)
    assert response.status_code == 200
    result = response.json()
    claims = jwt.decode(result["token"], SECRET, algorithms=["HS256"],
                        issuer="rdesk-tests", audience="rdesk-signaling")
    assert claims["sub"] == claims["device_id"] == device.device_id
    assert claims["device_key_id"] == result["device_key_id"] == KEY_ID
    assert claims["role"] == result["role"] == role
    assert claims["token_type"] == "signaling"
    assert before <= claims["iat"] <= claims["exp"]
    assert 0 < claims["exp"] - claims["iat"] <= 3600
    assert result["expires_at_ms"] == claims["exp"] * 1000
    assert result["device_id"] == device.device_id
    assert response.headers["cache-control"] == "no-store, private"


def test_anonymous_and_user_credentials_cannot_mint_signaling_token(client):
    http, _ = client
    for token in (None, create_access_token("user-a", "test", "user")):
        response = request(http, token)
        assert response.status_code == 401
        assert response.headers["cache-control"] == "no-store, private"


def test_revoked_device_token_cannot_mint_signaling_token(client):
    http, device = client
    token = create_device_access_token(device)
    device.auth_version = 2
    assert request(http, token).status_code == 401


@pytest.mark.parametrize("body", [
    {"role": "admin"}, {"device_key_id": "not-a-key"},
    {"device_key_id": "AB" * 32}, {"device_id": "other"},
])
def test_signaling_request_rejects_invalid_identity_and_role(client, body):
    http, device = client
    response = request(http, create_device_access_token(device), **body)
    assert response.status_code == 422
    assert response.headers["cache-control"] == "no-store, private"


def test_invalid_signaling_configuration_fails_closed(client, monkeypatch):
    http, device = client
    token = create_device_access_token(device)
    monkeypatch.setattr(settings, "signaling_jwt_audience", "rdesk-devices", raising=False)
    response = request(http, token)
    assert response.status_code == 503
    assert SECRET not in response.text


def test_overlong_signaling_lifetime_fails_closed(client, monkeypatch):
    http, device = client
    token = create_device_access_token(device)
    monkeypatch.setattr(settings, "signaling_jwt_ttl_seconds", 3601, raising=False)
    assert request(http, token).status_code == 503


@pytest.mark.parametrize("name,value", [
    ("jwt_issuer", "i" * 257), ("jwt_issuer", "issuer with spaces"),
    ("signaling_jwt_audience", "audience with spaces"),
])
def test_backend_does_not_issue_credentials_rejected_by_sidecar_config(client, monkeypatch, name, value):
    http, device = client
    monkeypatch.setattr(settings, name, value)
    assert request(http, create_device_access_token(device)).status_code == 503


@pytest.mark.skipif(not os.getenv("MRD_REALTIME_TEST_BINARY"), reason="compiled realtime-server binary is not configured")
def test_backend_credential_registers_with_real_realtime_executable(client):
    from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
    from cryptography.hazmat.primitives.serialization import Encoding, PublicFormat
    from websockets.sync.client import connect

    http, device = client
    private_key = Ed25519PrivateKey.generate()
    public_key = private_key.public_key().public_bytes(Encoding.Raw, PublicFormat.Raw)
    key_id = hashlib.sha256(public_key).hexdigest()
    response = request(http, create_device_access_token(device), device_key_id=key_id)
    assert response.status_code == 200
    credential = response.json()["token"]
    binary = Path(os.environ["MRD_REALTIME_TEST_BINARY"]).resolve(strict=True)
    with socket.socket() as port_socket:
        port_socket.bind(("127.0.0.1", 0))
        port = port_socket.getsockname()[1]
    environment = {**os.environ, "MRD_REALTIME_BIND": f"127.0.0.1:{port}",
                   "MRD_REALTIME_DEPLOYED": "false", "MRD_REALTIME_TLS_TERMINATED": "false",
                   "MRD_REALTIME_JWT_SECRET": SECRET, "MRD_REALTIME_JWT_ISSUER": "rdesk-tests",
                   "MRD_REALTIME_JWT_AUDIENCE": "rdesk-signaling"}
    options = {"creationflags": subprocess.CREATE_NO_WINDOW} if os.name == "nt" else {}
    process = subprocess.Popen([str(binary)], env=environment, stdout=subprocess.DEVNULL,
                               stderr=subprocess.DEVNULL, **options)
    try:
        deadline = time.monotonic() + 10
        while True:
            assert process.poll() is None, "realtime-server exited before accepting connections"
            try:
                with urlopen(f"http://127.0.0.1:{port}/health", timeout=1) as health:
                    assert json.load(health)["status"] == "ok"
                break
            except OSError:
                assert time.monotonic() < deadline, "realtime-server did not become ready"
                time.sleep(0.05)
        # Both the backend JWT and Ed25519 proof are real, across the executable boundary.
        for counter, (signed_role, expected) in enumerate((("Controller", "registered"), ("Agent", "protocol_error")), start=1):
            with connect(f"ws://127.0.0.1:{port}/ws", open_timeout=3) as websocket:
                challenge = json.loads(websocket.recv(timeout=3))["message"]["payload"]
                now_ms = int(time.time() * 1000)
                payload = {
                    "claims": {"issuer_device_id": device.device_id, "issuer_key_id": key_id,
                               "intended_peer_device_id": "signal-server", "issued_at_ms": now_ms,
                               "expires_at_ms": now_ms + 5000, "counter": counter, "nonce": list(os.urandom(16))},
                    "role": signed_role, "device_name": "test-device",
                    "backend_device_token": credential, "challenge_id": challenge["challenge_id"],
                    "challenge_nonce": challenge["challenge_nonce"],
                }
                canonical = json.dumps(payload, separators=(",", ":")).encode()
                context = b"MRD_SIGNAL_REGISTER_V2"
                signed = (b"MRD_CONTEXT_SIGNATURE_V1" + struct.pack(">H", len(context)) + context
                          + struct.pack(">Q", len(canonical)) + canonical)
                envelope = {"version": 2, "message": {"type": "register", "payload": {
                    "payload": payload, "signer_public_key": list(public_key),
                    "signature": list(private_key.sign(signed)),
                }}}
                websocket.send(json.dumps(envelope, separators=(",", ":")))
                result = json.loads(websocket.recv(timeout=3))
                assert result["message"]["type"] == expected
                if expected == "protocol_error":
                    assert result["message"]["payload"]["reason"] == "authentication_failed"
    finally:
        process.terminate()
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=5)


@pytest.mark.skipif(not os.getenv("MRD_REALTIME_TEST_BINARY"), reason="compiled realtime-server binary is not configured")
def test_realtime_identity_and_counters_survive_real_process_restarts(client, tmp_path):
    from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey, Ed25519PublicKey
    from cryptography.hazmat.primitives.serialization import Encoding, PublicFormat, PrivateFormat, NoEncryption
    from websockets.sync.client import connect

    http, device = client
    machine = Ed25519PrivateKey.generate()
    machine_public = machine.public_key().public_bytes(Encoding.Raw, PublicFormat.Raw)
    response = request(http, create_device_access_token(device),
                       device_key_id=hashlib.sha256(machine_public).hexdigest(), role="Peer")
    assert response.status_code == 200
    identity = Ed25519PrivateKey.generate()
    expected_public = identity.public_key().public_bytes(Encoding.Raw, PublicFormat.Raw)
    expected_key_id = hashlib.sha256(expected_public).hexdigest()
    key_path = tmp_path / "signaling.pk8"
    counter_path = tmp_path / "counter.json"
    key_path.write_bytes(identity.private_bytes(Encoding.DER, PrivateFormat.PKCS8, NoEncryption()))
    counter_path.write_text(json.dumps({"format_version": 1, "key_id": expected_key_id,
                                        "reserved_through": 0}), encoding="utf-8")
    if os.name != "nt":
        tmp_path.chmod(0o700)
        key_path.chmod(0o600)
        counter_path.chmod(0o600)
    binary = Path(os.environ["MRD_REALTIME_TEST_BINARY"]).resolve(strict=True)
    counters = []
    for attempt in range(3):
        with socket.socket() as port_socket:
            port_socket.bind(("127.0.0.1", 0))
            port = port_socket.getsockname()[1]
        environment = {**os.environ, "MRD_REALTIME_BIND": f"127.0.0.1:{port}",
                       "MRD_REALTIME_DEPLOYED": "false", "MRD_REALTIME_TLS_TERMINATED": "false",
                       "MRD_REALTIME_JWT_SECRET": SECRET, "MRD_REALTIME_JWT_ISSUER": "rdesk-tests",
                       "MRD_REALTIME_JWT_AUDIENCE": "rdesk-signaling",
                       "MRD_REALTIME_IDENTITY_PKCS8_FILE": str(key_path),
                       "MRD_REALTIME_COUNTER_FILE": str(counter_path)}
        options = {"creationflags": subprocess.CREATE_NO_WINDOW} if os.name == "nt" else {}
        process = subprocess.Popen([str(binary)], env=environment, stdout=subprocess.DEVNULL,
                                   stderr=subprocess.DEVNULL, **options)
        try:
            deadline = time.monotonic() + 10
            while True:
                assert process.poll() is None, "realtime-server exited before accepting connections"
                try:
                    with urlopen(f"http://127.0.0.1:{port}/health", timeout=1):
                        break
                except OSError:
                    assert time.monotonic() < deadline
                    time.sleep(0.05)
            with connect(f"ws://127.0.0.1:{port}/ws", open_timeout=3) as websocket:
                challenge = json.loads(websocket.recv(timeout=3))["message"]["payload"]
                now_ms = int(time.time() * 1000)
                payload = {
                    "claims": {"issuer_device_id": device.device_id,
                               "issuer_key_id": hashlib.sha256(machine_public).hexdigest(),
                               "intended_peer_device_id": "signal-server", "issued_at_ms": now_ms,
                               "expires_at_ms": now_ms + 5000, "counter": attempt + 1,
                               "nonce": list(os.urandom(16))},
                    "role": "Peer", "device_name": "restart-test",
                    "backend_device_token": response.json()["token"],
                    "challenge_id": challenge["challenge_id"], "challenge_nonce": challenge["challenge_nonce"],
                }
                canonical = json.dumps(payload, separators=(",", ":")).encode()
                context = b"MRD_SIGNAL_REGISTER_V2"
                signed = (b"MRD_CONTEXT_SIGNATURE_V1" + struct.pack(">H", len(context)) + context
                          + struct.pack(">Q", len(canonical)) + canonical)
                websocket.send(json.dumps({"version": 2, "message": {"type": "register", "payload": {
                    "payload": payload, "signer_public_key": list(machine_public),
                    "signature": list(machine.sign(signed)),
                }}}, separators=(",", ":")))
                result = json.loads(websocket.recv(timeout=3))
                assert result["message"]["type"] == "registered"
                registration = result["message"]["payload"]
                assert bytes(registration["signer_public_key"]) == expected_public
                claims = registration["payload"]["claims"]
                assert claims["issuer_key_id"] == expected_key_id
                canonical = json.dumps(registration["payload"], separators=(",", ":")).encode()
                context = b"MRD_SIGNAL_REGISTERED_V2"
                signed = (b"MRD_CONTEXT_SIGNATURE_V1" + struct.pack(">H", len(context)) + context
                          + struct.pack(">Q", len(canonical)) + canonical)
                Ed25519PublicKey.from_public_bytes(expected_public).verify(bytes(registration["signature"]), signed)
                counters.append(claims["counter"])
        finally:
            # A hard process exit must not reuse any reserved signing counter.
            process.kill()
            process.wait(timeout=5)
    assert 0 < counters[0] < counters[1] < counters[2]
    assert len(set(counters)) == 3
