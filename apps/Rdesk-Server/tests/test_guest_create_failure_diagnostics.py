"""Finite internal diagnostics preserve guest HTTP failures and secret custody."""

import inspect
import json
import logging
from types import FunctionType

import pytest
from fastapi.testclient import TestClient
from sqlalchemy import select

from app.core.config import settings
from app.models.device_temporary_access import DeviceTemporaryAccess
from app.services.device_sessions import DeviceSessionError
from app.services.device_self_enrollment import enrollment_peer_ip
from app.services.guest_browser_sessions import GuestBrowserSessionService
from app.services.temporary_access import TemporaryAccessService
from app.services.temporary_access import invalid as temporary_invalid
from test_guest_browser_session_api import (
    KEY_ID,
    PASSWORD,
    PUBLIC,
    SALT,
    _guest,
    _publish,
    device_sessions_api,
    guest_config,
    trusted_realtime_identity,
)
from test_browser_session_api import _browser_request


LOGGER = "rdesk.guest_browser_create"
EVENT = "guest_browser_create_invalid"
HTTP_INVALID = {
    "detail": {
        "code": "guest_access_invalid",
        "message": "Temporary access is unavailable",
    }
}


def _diagnostic(caplog):
    records = [record for record in caplog.records if record.name == LOGGER]
    assert len(records) == 1
    record = records[0]
    assert not record.exc_info
    assert record.stack_info is None
    message = record.getMessage()
    assert len(message.encode("utf-8")) <= 512
    value = json.loads(message)
    assert set(value) == {"event", "session_id", "target_device_id", "source", "line"}
    assert value["event"] == EVENT
    assert value["session_id"] == "guest-session-1"
    assert value["target_device_id"] == "unbound-1"
    assert type(value["line"]) is int
    assert 0 <= value["line"] <= 100_000
    for secret in (PASSWORD, "WRNG2345", KEY_ID, PUBLIC.hex(), SALT.hex()):
        assert secret not in message
    assert "temporary_password" not in message
    assert "Traceback" not in message
    return value


def _guest_invalid_lines():
    source, start = inspect.getsourcelines(GuestBrowserSessionService.create)
    return [start + offset for offset, line in enumerate(source) if "invalid()" in line]


def _guest_wrong_password_line():
    source, start = inspect.getsourcelines(GuestBrowserSessionService.create)
    return next(start + offset + 1 for offset, line in enumerate(source) if "not valid_password" in line)


def test_wrong_password_logs_direct_known_caller_without_changing_http(
    device_sessions_api, caplog
):
    api = device_sessions_api
    assert _publish(api).status_code == 200
    with caplog.at_level(logging.WARNING, logger=LOGGER):
        response = _guest(api, temporary_password="WRNG2345")
    assert response.status_code == 401
    assert response.json() == HTTP_INVALID
    value = _diagnostic(caplog)
    assert value["source"] == "guest_browser_service"
    assert value["line"] == _guest_wrong_password_line()


def test_changed_publication_logs_later_validation_caller(
    device_sessions_api, caplog, monkeypatch
):
    api = device_sessions_api
    assert _publish(api).status_code == 200
    delegate = TemporaryAccessService.verify_password

    async def change_after_password_check(self, password, access):
        valid = await delegate(self, password, access)
        if valid:
            row = api.session.scalar(select(DeviceTemporaryAccess))
            row.generation += 1
            api.session.commit()
        return valid

    monkeypatch.setattr(TemporaryAccessService, "verify_password", change_after_password_check)
    with caplog.at_level(logging.WARNING, logger=LOGGER):
        response = _guest(api)
    assert response.status_code == 401
    assert response.json() == HTTP_INVALID
    value = _diagnostic(caplog)
    assert value["source"] == "guest_browser_service"
    assert value["line"] == _guest_invalid_lines()[-1]


def test_unknown_source_hides_original_error_and_locals(
    device_sessions_api, caplog, monkeypatch
):
    api = device_sessions_api
    assert _publish(api).status_code == 200
    private_error = "synthetic-error-must-never-enter-diagnostic"

    async def unknown_create(self, payload):
        private_local = "synthetic-local-must-never-enter-diagnostic"
        assert private_local
        raise DeviceSessionError("guest_access_invalid", 401, private_error)

    monkeypatch.setattr(GuestBrowserSessionService, "create", unknown_create)
    with caplog.at_level(logging.WARNING, logger=LOGGER):
        response = _guest(api)
    assert response.status_code == 401
    assert response.json() == {
        "detail": {"code": "guest_access_invalid", "message": private_error}
    }
    value = _diagnostic(caplog)
    assert (value["source"], value["line"]) == ("unknown", 0)
    message = [record for record in caplog.records if record.name == LOGGER][0].getMessage()
    assert private_error not in message
    assert "synthetic-local-must-never-enter-diagnostic" not in message
    assert __file__ not in message


def test_excessive_traceback_depth_has_no_guessed_source(
    device_sessions_api, caplog, monkeypatch
):
    api = device_sessions_api
    assert _publish(api).status_code == 200

    def nested(depth):
        if depth:
            return nested(depth - 1)
        raise DeviceSessionError("guest_access_invalid", 401, "Temporary access is unavailable")

    async def deep_create(self, payload):
        nested(40)

    monkeypatch.setattr(GuestBrowserSessionService, "create", deep_create)
    with caplog.at_level(logging.WARNING, logger=LOGGER):
        response = _guest(api)
    assert response.status_code == 401
    assert response.json() == HTTP_INVALID
    value = _diagnostic(caplog)
    assert (value["source"], value["line"]) == ("unknown", 0)


@pytest.mark.parametrize("code,status", [("guest_access_invalid", 403), ("other_failure", 401)])
def test_other_errors_do_not_emit_the_guest_invalid_diagnostic(
    device_sessions_api, caplog, monkeypatch, code, status
):
    api = device_sessions_api
    assert _publish(api).status_code == 200

    async def other_create(self, payload):
        raise DeviceSessionError(code, status, "Unchanged synthetic HTTP error")

    monkeypatch.setattr(GuestBrowserSessionService, "create", other_create)
    with caplog.at_level(logging.WARNING, logger=LOGGER):
        response = _guest(api)
    assert response.status_code == status
    assert response.json() == {
        "detail": {"code": code, "message": "Unchanged synthetic HTTP error"}
    }
    assert not [record for record in caplog.records if record.name == LOGGER]


def test_logging_failure_cannot_change_http_denial(device_sessions_api, monkeypatch):
    api = device_sessions_api
    assert _publish(api).status_code == 200

    class BrokenHandler(logging.Handler):
        def emit(self, record):
            raise RuntimeError("synthetic logging failure")

    logger = logging.getLogger(LOGGER)
    handler = BrokenHandler()
    logger.addHandler(handler)
    try:
        response = _guest(api, temporary_password="WRNG2345")
    finally:
        logger.removeHandler(handler)
    assert response.status_code == 401
    assert response.json() == HTTP_INVALID


def test_real_bad_trusted_forwarding_logs_masked_entrypoint_denial(
    device_sessions_api, caplog, monkeypatch
):
    from app.api.v1 import guest_browser_sessions

    api = device_sessions_api
    assert _publish(api).status_code == 200
    monkeypatch.setattr(guest_browser_sessions, "enrollment_peer_ip", enrollment_peer_ip)
    monkeypatch.setattr(settings, "device_self_enrollment_trusted_proxies", "127.0.0.1/32")
    with TestClient(api.app, client=("127.0.0.1", 54321)) as client:
        with caplog.at_level(logging.WARNING, logger=LOGGER):
            response = client.post(
                "/api/v1/guest-browser-sessions",
                headers={"X-Real-IP": "not-an-ip"},
                json={
                    **_browser_request(
                        target_device_id="unbound-1", session_id="guest-session-1"
                    ),
                    "temporary_password": PASSWORD,
                },
            )
    assert response.status_code == 401
    assert response.json() == HTTP_INVALID
    value = _diagnostic(caplog)
    assert value["source"] == "device_self_enrollment_service"
    assert value["line"] > 0
    assert "not-an-ip" not in caplog.records[-1].getMessage()


def test_equal_code_clone_is_not_misidentified_as_the_known_invalid_function(
    device_sessions_api, caplog, monkeypatch
):
    api = device_sessions_api
    assert _publish(api).status_code == 200
    cloned_code = temporary_invalid.__code__.replace()
    assert cloned_code == temporary_invalid.__code__
    assert cloned_code is not temporary_invalid.__code__
    cloned_invalid = FunctionType(cloned_code, temporary_invalid.__globals__)

    async def unknown_create(self, payload):
        cloned_invalid()

    monkeypatch.setattr(GuestBrowserSessionService, "create", unknown_create)
    with caplog.at_level(logging.WARNING, logger=LOGGER):
        response = _guest(api)
    assert response.status_code == 401
    assert response.json() == HTTP_INVALID
    value = _diagnostic(caplog)
    assert value["source"] == "temporary_access_service"
    assert value["line"] > 0
