from datetime import UTC, datetime

import pytest

from app.api.v1 import devices as routes
from app.models.device import DeviceStatus
from app.services.realtime_presence import DevicePresence
from test_device_ownership import _device, device_api


def own(api):
    api.device.is_bound = True
    api.device.bound_user_id = api.owner.id
    api.device.tenant_id = api.owner.tenant_id
    api.device.status = DeviceStatus(status="offline", last_seen="legacy", device_id=api.device.id)
    api.session.commit()


def test_device_list_queries_only_visible_codes_and_filters_after_overlay(device_api, monkeypatch):
    own(device_api)
    foreign = _device(owner=device_api.other)
    foreign.id, foreign.device_id, foreign.motherboard_serial_digest = "foreign-row", "foreign-code", None
    device_api.session.add(foreign)
    device_api.session.commit()
    calls = []
    async def sidecar(ids):
        calls.append(ids)
        return {code: DevicePresence(True, 1_780_000_000_000) for code in ids}
    monkeypatch.setattr(routes, "query_realtime_presence", sidecar, raising=False)
    result = device_api.client.get("/api/v1/devices?status=online", headers={"Authorization": f"Bearer {device_api.user_token}"})
    assert result.status_code == 200
    assert [item["device_id"] for item in result.json()] == [device_api.device.device_id]
    assert result.json()[0]["status"] == "online"
    assert result.json()[0]["last_seen"] == datetime.fromtimestamp(1_780_000_000, UTC).isoformat(timespec="milliseconds").replace("+00:00", "Z")
    assert calls == [[device_api.device.device_id]]


def test_visible_device_detail_uses_presence_and_denied_detail_never_queries(device_api, monkeypatch):
    own(device_api)
    calls = []
    async def sidecar(ids):
        calls.append(ids)
        return {code: DevicePresence(True, 1_780_000_000_000) for code in ids}
    monkeypatch.setattr(routes, "query_realtime_presence", sidecar, raising=False)
    path = f"/api/v1/devices/{device_api.device.id}"
    denied = device_api.client.get(path, headers={"Authorization": f"Bearer {device_api.other_token}"})
    assert denied.status_code == 404
    assert calls == []
    visible = device_api.client.get(path, headers={"Authorization": f"Bearer {device_api.user_token}"})
    assert visible.json()["status"] == "online"
    assert calls == [[device_api.device.device_id]]


def test_configured_presence_unavailable_overrides_legacy_online(device_api, monkeypatch):
    own(device_api)
    device_api.device.status.status = "online"
    device_api.session.commit()
    async def unavailable(ids):
        return {code: DevicePresence() for code in ids}
    monkeypatch.setattr(routes, "query_realtime_presence", unavailable, raising=False)
    result = device_api.client.get("/api/v1/devices?status=offline", headers={"Authorization": f"Bearer {device_api.user_token}"})
    assert result.json()[0]["status"] == "offline"
    assert result.json()[0]["last_seen"] == "离线"


def test_absent_presence_configuration_preserves_legacy_status(device_api, monkeypatch):
    own(device_api)
    device_api.device.status.status = "online"
    device_api.session.commit()
    async def absent(_ids): return None
    monkeypatch.setattr(routes, "query_realtime_presence", absent, raising=False)
    result = device_api.client.get("/api/v1/devices", headers={"Authorization": f"Bearer {device_api.user_token}"})
    assert result.json()[0]["status"] == "online"
    assert result.json()[0]["last_seen"] == "legacy"
