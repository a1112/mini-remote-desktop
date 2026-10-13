"""Distinct internal deny sites retain the same password work and HTTP mask."""

import json
import logging

from app.models.session_request import SessionRequest
from app.services.temporary_access import TemporaryAccessService
from test_guest_browser_session_api import (
    _guest,
    _publish,
    device_sessions_api,
    guest_config,
    trusted_realtime_identity,
)

LOGGER = "rdesk.guest_browser_create"
MASKED_INVALID = {
    "detail": {
        "code": "guest_access_invalid",
        "message": "Temporary access is unavailable",
    }
}


def test_missing_target_missing_access_and_wrong_password_have_distinct_deny_sites(
    device_sessions_api, monkeypatch, caplog
):
    api = device_sessions_api
    stages = []
    verification_calls = []
    delegate = TemporaryAccessService.verify_password

    async def observed_verification(self, password, access):
        stages.append("verify_started")
        valid = await delegate(self, password, access)
        verification_calls.append((access is None, valid))
        stages.append("verify_finished")
        return valid

    class DenialObserver(logging.Handler):
        def emit(self, record):
            if record.name == LOGGER:
                stages.append("diagnostic")

    monkeypatch.setattr(TemporaryAccessService, "verify_password", observed_verification)
    logger = logging.getLogger(LOGGER)
    observer = DenialObserver()
    logger.addHandler(observer)
    sites = []
    try:
        for changes in (
            {"target_device_id": "missing"},
            {},
            {"temporary_password": "WRNG2345"},
        ):
            if "temporary_password" in changes:
                assert _publish(api).status_code == 200
            stages.clear()
            caplog.clear()
            with caplog.at_level(logging.WARNING, logger=LOGGER):
                response = _guest(api, **changes)
            assert response.status_code == 401
            assert response.json() == MASKED_INVALID
            assert stages == ["verify_started", "verify_finished", "diagnostic"]
            assert api.session.query(SessionRequest).count() == 0
            records = [record for record in caplog.records if record.name == LOGGER]
            assert len(records) == 1
            diagnostic = json.loads(records[0].getMessage())
            assert diagnostic["source"] == "guest_browser_service"
            sites.append(diagnostic["line"])
        assert len(set(sites)) == 3, "each denial requires its own finite source line"
    finally:
        logger.removeHandler(observer)

    assert verification_calls == [(True, False), (True, False), (False, False)]
    stages.clear()
    response = _guest(api, session_id="valid-paired-after-denials", access_mode="paired")
    assert response.status_code == 200, response.text
    assert response.json()["session"]["request"]["access_mode"] == "paired"
    assert stages == ["verify_started", "verify_finished"]
    assert verification_calls[-1] == (False, True)
