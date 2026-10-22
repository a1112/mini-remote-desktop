import asyncio
import base64
import hashlib
import hmac
import json
import time

import httpx
import pytest
from pydantic import SecretStr

from app.core.config import Settings, settings
from app.services import realtime_presence as presence

NOW = 1_780_000_000_000
KEY = bytes(range(32))
SECRET = base64.urlsafe_b64encode(KEY).rstrip(b"=").decode("ascii")
CLIENT = httpx.AsyncClient


@pytest.fixture
def anyio_backend():
    return "asyncio"


@pytest.fixture(autouse=True)
def configured(monkeypatch):
    monkeypatch.setitem(settings.__dict__, "realtime_presence_secret", SecretStr(SECRET))
    monkeypatch.setitem(settings.__dict__, "realtime_presence_url", "http://127.0.0.1:9542/internal/presence")
    monkeypatch.setattr(presence, "_now_ms", lambda: NOW)


def install_sidecar(monkeypatch, handler):
    def client(**options):
        assert options["trust_env"] is False
        assert options["follow_redirects"] is False
        assert 0 < options["timeout"] <= 2.0
        return CLIENT(transport=httpx.MockTransport(handler), **options)
    monkeypatch.setattr(presence.httpx, "AsyncClient", client)


def payload(ids, *, online=True, seen=NOW):
    return {"version": 1, "sampled_at_ms": NOW, "devices": [
        {"device_id": code, "online": online, "last_seen_ms": seen} for code in ids
    ]}


def test_presence_settings_secret_is_optional_and_redacted():
    empty = Settings(_env_file=None)
    assert getattr(empty, "realtime_presence_secret", None) is None
    configured = Settings(_env_file=None, realtime_presence_secret=SECRET)
    assert configured.realtime_presence_secret.get_secret_value() == SECRET
    assert SECRET not in repr(configured)
    assert configured.realtime_presence_url == "http://127.0.0.1:9542/internal/presence"


@pytest.mark.anyio
async def test_queries_exact_unique_visible_codes_with_context_hmac(monkeypatch):
    calls = []
    def handler(request):
        ids = json.loads(request.content)["device_ids"]
        calls.append(ids)
        assert request.method == "POST"
        assert str(request.url) == "http://127.0.0.1:9542/internal/presence"
        token = base64.urlsafe_b64encode(hmac.new(KEY, b"MRD_REALTIME_PRESENCE_QUERY_V1\0", hashlib.sha256).digest()).rstrip(b"=").decode()
        assert request.headers["authorization"] == f"Bearer {token}"
        assert SECRET not in str(request.url)
        return httpx.Response(200, json=payload(ids))
    install_sidecar(monkeypatch, handler)
    result = await presence.query_realtime_presence(["1501515774", "1501515774", "code_2"])
    assert calls == [["1501515774", "code_2"]]
    assert result == {code: presence.DevicePresence(True, NOW) for code in calls[0]}


@pytest.mark.anyio
async def test_missing_configuration_keeps_legacy_status_without_network(monkeypatch):
    monkeypatch.setitem(settings.__dict__, "realtime_presence_secret", None)
    install_sidecar(monkeypatch, lambda _: pytest.fail("not configured: no sidecar call"))
    assert await presence.query_realtime_presence(["visible"]) is None


@pytest.mark.anyio
@pytest.mark.parametrize("url", [
    "https://127.0.0.1:9542/internal/presence", "http://localhost:9542/internal/presence",
    "http://192.168.1.1:9542/internal/presence", "http://127.0.0.1.evil/internal/presence",
    "http://user:secret@127.0.0.1:9542/internal/presence", "http://127.0.0.1:9542/internal/presence?q=1",
    "http://127.0.0.1:9542/internal/presence?", "http://127.0.0.1:9542/internal/presence#fragment",
    "http://127.0.0.1:99999/internal/presence", "http://127.0.0.1:\n9542/internal/presence",
])
async def test_invalid_url_fails_closed_without_sending_credentials(monkeypatch, url):
    monkeypatch.setitem(settings.__dict__, "realtime_presence_url", url)
    install_sidecar(monkeypatch, lambda _: pytest.fail("invalid URL: no sidecar call"))
    assert await presence.query_realtime_presence(["visible"]) == {"visible": presence.DevicePresence()}


@pytest.mark.anyio
@pytest.mark.parametrize("secret", ["", "a" * 43, SECRET + "=", "b" * 42, "\n" + SECRET, "?" * 43])
async def test_invalid_secret_fails_closed_without_network(monkeypatch, secret):
    monkeypatch.setitem(settings.__dict__, "realtime_presence_secret", SecretStr(secret))
    install_sidecar(monkeypatch, lambda _: pytest.fail("invalid secret: no sidecar call"))
    assert await presence.query_realtime_presence(["visible"]) == {"visible": presence.DevicePresence()}


@pytest.mark.anyio
@pytest.mark.parametrize("mutation", [
    "bool_version", "bool_sample", "bool_seen", "numeric_online", "foreign", "duplicate", "missing",
    "extra_root", "extra_row", "old_sample", "future_sample", "old_online", "future_seen", "no_online_seen",
    "after_sample",
])
async def test_untrusted_or_stale_response_is_offline(monkeypatch, mutation):
    response = payload(["visible"])
    row = response["devices"][0]
    if mutation == "bool_version": response["version"] = True
    elif mutation == "bool_sample": response["sampled_at_ms"] = True
    elif mutation == "bool_seen": row["last_seen_ms"] = True
    elif mutation == "numeric_online": row["online"] = 1
    elif mutation == "foreign": row["device_id"] = "other-tenant"
    elif mutation == "duplicate": response["devices"].append(dict(row))
    elif mutation == "missing": response["devices"] = []
    elif mutation == "extra_root": response["unexpected"] = True
    elif mutation == "extra_row": row["unexpected"] = True
    elif mutation == "old_sample": response["sampled_at_ms"] -= 5001
    elif mutation == "future_sample": response["sampled_at_ms"] += 2001
    elif mutation == "old_online": row["last_seen_ms"] -= 300_000
    elif mutation == "future_seen": row["last_seen_ms"] += 2001
    elif mutation == "no_online_seen": row["last_seen_ms"] = None
    elif mutation == "after_sample": row["last_seen_ms"] += 1
    install_sidecar(monkeypatch, lambda _: httpx.Response(200, json=response))
    assert await presence.query_realtime_presence(["visible"]) == {"visible": presence.DevicePresence()}


@pytest.mark.anyio
@pytest.mark.parametrize("status", [302, 401, 403, 500, 503])
async def test_sidecar_http_reachability_is_not_device_online(monkeypatch, status):
    install_sidecar(monkeypatch, lambda _: httpx.Response(status, json=payload(["visible"]), headers={"Location": "http://evil.invalid"}))
    assert await presence.query_realtime_presence(["visible"]) == {"visible": presence.DevicePresence()}


@pytest.mark.anyio
async def test_unknown_or_disconnected_device_keeps_false_presence(monkeypatch):
    install_sidecar(monkeypatch, lambda _: httpx.Response(200, json=payload(["visible"], online=False, seen=None)))
    assert await presence.query_realtime_presence(["visible"]) == {"visible": presence.DevicePresence()}


@pytest.mark.anyio
async def test_batching_never_sends_more_than_128_codes(monkeypatch):
    calls = []
    def handler(request):
        ids = json.loads(request.content)["device_ids"]
        calls.append(ids)
        return httpx.Response(200, json=payload(ids))
    install_sidecar(monkeypatch, handler)
    ids = [f"visible_{index}" for index in range(300)]
    result = await presence.query_realtime_presence(ids)
    assert [len(batch) for batch in calls] == [128, 128, 44]
    assert len(result) == 300
    assert all(item.online for item in result.values())


@pytest.mark.anyio
async def test_total_deadline_covers_all_batches_and_fails_closed(monkeypatch):
    monkeypatch.setattr(presence, "TOTAL_TIMEOUT_SECONDS", .05)
    calls = []
    async def handler(request):
        ids = json.loads(request.content)["device_ids"]
        calls.append(ids)
        if len(calls) == 2: await asyncio.sleep(.2)
        return httpx.Response(200, json=payload(ids))
    install_sidecar(monkeypatch, handler)
    ids = [f"visible_{index}" for index in range(129)]
    started = time.monotonic()
    result = await presence.query_realtime_presence(ids)
    assert .03 <= time.monotonic() - started < .2
    assert len(calls) == 2
    assert result == {code: presence.DevicePresence() for code in ids}


@pytest.mark.anyio
async def test_response_stream_is_capped_before_full_body_is_read(monkeypatch):
    chunks_read = []
    class Body(httpx.AsyncByteStream):
        async def __aiter__(self):
            for index in range(10):
                chunks_read.append(index)
                yield b" " * (32 * 1024)
    install_sidecar(monkeypatch, lambda _: httpx.Response(200, stream=Body()))
    assert await presence.query_realtime_presence(["visible"]) == {"visible": presence.DevicePresence()}
    assert chunks_read == [0, 1, 2]


@pytest.mark.anyio
async def test_duplicate_json_members_fail_closed(monkeypatch):
    content = json.dumps(payload(["visible"])).replace('"online": true', '"online": false, "online": true')
    install_sidecar(monkeypatch, lambda _: httpx.Response(200, content=content))
    assert await presence.query_realtime_presence(["visible"]) == {"visible": presence.DevicePresence()}


@pytest.mark.anyio
@pytest.mark.parametrize("body", [b"[" * 20000 + b"0" + b"]" * 20000, b"{malformed", b"\xff", b'{"version":NaN}'], ids=["deep", "malformed", "invalid_utf8", "nan"])
async def test_malformed_json_always_fails_closed(monkeypatch, body):
    install_sidecar(monkeypatch, lambda _: httpx.Response(200, content=body))
    assert await presence.query_realtime_presence(["visible"]) == {"visible": presence.DevicePresence()}


@pytest.mark.anyio
async def test_offline_historical_heartbeat_is_not_promoted_to_online(monkeypatch):
    install_sidecar(monkeypatch, lambda _: httpx.Response(200, json=payload(["visible"], online=False, seen=NOW-86400000)))
    assert await presence.query_realtime_presence(["visible"]) == {"visible": presence.DevicePresence(False, NOW-86400000)}
