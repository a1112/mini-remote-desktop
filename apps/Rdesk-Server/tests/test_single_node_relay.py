from dataclasses import replace
from datetime import UTC, datetime
from types import SimpleNamespace
import json

import pytest
from sqlalchemy import delete, select

from app.models.relay_node import RelayNode
from app.models.relay_node_registration import RelayNodeRegistration
from app.models.session_request import SessionRequest
from app.services.relay_directory import _policy_from_grant
from app.services.session_grants import (
    SessionGrantError,
    bind_session_grant_policy,
    configured_session_grant_policy,
    validate_session_grant_policy,
)
from test_wan_relay_access import (
    WanRelayAPI, _access, _approve, _create, _headers, wan_relay_api,
)


def test_explicit_single_node_policy_is_persisted_in_the_approved_grant(wan_relay_api: WanRelayAPI):
    policy = replace(wan_relay_api.service.current_policy, max_backups=0)
    grant = SessionRequest()
    bind_session_grant_policy(grant=grant, target_device_id="target", policy=policy, now=datetime.now(UTC))
    assert grant.relay_max_backups == 0
    assert _policy_from_grant(grant).max_backups == 0


@pytest.mark.parametrize("route_policy", ["relay_only", "direct_first"])
def test_single_node_generation_is_signed_and_policy_drift_denies_credentials(wan_relay_api: WanRelayAPI, route_policy):
    api = wan_relay_api
    api.service._current_policy = replace(api.service.current_policy, max_backups=0)
    keep = api.session.scalar(select(RelayNode.node_id).order_by(RelayNode.node_id))
    api.session.execute(delete(RelayNodeRegistration).where(RelayNodeRegistration.node_id != keep))
    api.session.execute(delete(RelayNode).where(RelayNode.node_id != keep))
    api.session.commit()
    created = _create(api)
    assert created.status_code == 200
    session_id = "wan-session-1"
    if route_policy == "direct_first":
        session_id = "wan-session-direct-first"
        payload = json.loads(created.request.content)
        payload.update(session_id=session_id, idempotency_key=[8] * 16, route_policy=route_policy)
        assert api.client.post(created.request.url.path, headers=_headers(api, "controller-1"), json=payload).status_code == 200
    approved = _approve(api, session_id=session_id)
    assert approved.status_code == 200, approved.text
    row = api.session.get(SessionRequest, session_id)
    assert row.relay_max_backups == 0
    def access_request():
        return api.client.post("/api/v1/relays/access", headers=_headers(api, "controller-1"), json={"session_id": session_id, "policy_revision": 29, "intended_peer_id": "target-1", "generation": 0, "refresh": False})
    access = access_request()
    assert access.status_code == 200, access.text
    assert len(access.json()["directory"]["payload"]["candidates"]) == 1
    api.service._current_policy = replace(api.service.current_policy, max_backups=1)
    assert access_request().status_code == 403


@pytest.mark.parametrize("max_backups", [-1, 8, True, "0"])
def test_redundancy_configuration_fails_closed(wan_relay_api: WanRelayAPI, max_backups):
    with pytest.raises(SessionGrantError):
        validate_session_grant_policy(replace(wan_relay_api.service.current_policy, max_backups=max_backups))


def test_configured_policy_has_strict_default_and_explicit_zero():
    assert configured_session_grant_policy(SimpleNamespace()).max_backups == 1
    assert configured_session_grant_policy(SimpleNamespace(relay_max_backups=0)).max_backups == 0
