"""Version 9 broadens only a verified WAN constraint, retaining historical rows."""
import os
from types import SimpleNamespace
from uuid import uuid4

import pytest
from sqlalchemy import inspect, text
from sqlalchemy.ext.asyncio import create_async_engine

import app.db.migrate_add_relay_access as migration
from app.db.migrate_add_relay_control import migrate as migrate_relay_control
from app.db.session import Base
from app.models.user import User
from app.models.device import Device
from app.models.session_request import SessionRequest
from app.schemas.session import DeviceSessionCanonicalRequest
from app.services.device_sessions import wan_request_commitment

ATTENDED = ("requester_device_id IS NULL OR requester_device_id <> target_device_id "
    "AND access_mode = 'attended' AND (route_policy = ANY (ARRAY['relay_only', 'direct_first']))")
PAIRED = ("requester_device_id IS NULL OR requester_device_id <> target_device_id "
    "AND (access_mode = ANY (ARRAY['attended', 'paired'])) AND "
    "(route_policy = ANY (ARRAY['relay_only', 'direct_first']))")


@pytest.mark.parametrize("expression,legacy", [(ATTENDED, True), (PAIRED, False), (None, False)])
def test_paired_upgrade_accepts_only_exact_old_or_new_constraint(monkeypatch, expression, legacy):
    checks = [] if expression is None else [{"name": "ck_session_requests_wan_values", "sqltext": expression}]
    monkeypatch.setattr(migration, "inspect", lambda _: SimpleNamespace(get_check_constraints=lambda *a, **k: checks))
    assert migration._wan_paired_constraint_is_legacy(object(), None, allow_missing=expression is None) is legacy


@pytest.mark.parametrize("expression", [None, ATTENDED.replace("= 'attended'", "<> 'forbidden'"),
    PAIRED.replace("'paired'", "'unattended'")])
def test_paired_upgrade_rejects_missing_or_drifted_constraints(monkeypatch, expression):
    checks = [] if expression is None else [{"name": "ck_session_requests_wan_values", "sqltext": expression}]
    monkeypatch.setattr(migration, "inspect", lambda _: SimpleNamespace(get_check_constraints=lambda *a, **k: checks))
    with pytest.raises(migration.RelayAccessMigrationError, match="WAN paired access constraint differs"):
        migration._wan_paired_constraint_is_legacy(object(), None, allow_missing=False)


def test_paired_upgrade_has_a_new_closed_ledger_version():
    assert migration._VERSIONS == tuple(range(1, 10))


@pytest.mark.skipif(not os.getenv("MRD_TEST_DATABASE_URL"), reason="PostgreSQL migration verification required")
@pytest.mark.anyio
async def test_paired_v9_upgrades_v8_idempotently_and_preserves_attended_rows():
    url = os.environ["MRD_TEST_DATABASE_URL"].replace("postgresql://", "postgresql+asyncpg://", 1)
    schema = "wan_paired_v9_" + uuid4().hex
    admin = create_async_engine(url)
    async with admin.begin() as db:
        await db.execute(text(f'CREATE SCHEMA "{schema}"'))
    engine = create_async_engine(url, connect_args={"server_settings": {"search_path": schema}})
    try:
        async with engine.begin() as db:
            await migrate_relay_control(db)
            await db.run_sync(Base.metadata.create_all)
        await migration.migrate(engine)
        async with engine.begin() as db:
            await db.execute(text("DELETE FROM relay_access_schema_migrations WHERE version = 9"))
            await db.execute(text("ALTER TABLE session_requests DROP CONSTRAINT ck_session_requests_wan_values"))
            await db.execute(text("ALTER TABLE session_requests DROP CONSTRAINT ck_session_requests_paired_keys"))
            await db.execute(text("ALTER TABLE session_requests DROP COLUMN paired_controller_key_id"))
            await db.execute(text("ALTER TABLE session_requests DROP COLUMN paired_target_key_id"))
            await db.execute(text("ALTER TABLE session_requests ADD CONSTRAINT ck_session_requests_wan_values CHECK (" + ATTENDED + ")"))
            await db.execute(User.__table__.insert().values(id="old-owner", username="old-owner",
                email="old@example.test", password_hash="unused", role="user", tenant_id="tenant-old"))
            for name in ("old-target", "old-controller"):
                await db.execute(Device.__table__.insert().values(id=name, name=name, device_id=name,
                    os="Linux", is_bound=True, bound_user_id="old-owner", tenant_id="tenant-old"))
            canonical = DeviceSessionCanonicalRequest(session_id="old-session", idempotency_key=[7]*16,
                controller_device_id="old-controller", target_device_id="old-target", access_mode="attended",
                requested_scopes=["screen.view"], requested_profile=None, route_policy="relay_only")
            commitment = wan_request_commitment(canonical)
            await db.execute(SessionRequest.__table__.insert().values(id="old-session", requester_user_id="old-owner",
                requester_device_id="old-controller", target_device_id="old-target", signaling_room="old-room",
                tenant_id="tenant-old", access_mode="attended", route_policy="relay_only",
                request_payload=canonical.model_dump(mode="json"), request_commitment=commitment,
                requested_scopes=["screen.view"]))
        await migration.migrate(engine)
        await migration.migrate(engine)
        async with engine.connect() as db:
            versions = list((await db.execute(text("SELECT version FROM relay_access_schema_migrations ORDER BY version"))).scalars())
            row = (await db.execute(text("SELECT id,requester_user_id,access_mode,paired_controller_key_id,paired_target_key_id,request_commitment FROM session_requests WHERE id='old-session'"))).one()
            checks = await db.run_sync(lambda sync: inspect(sync).get_check_constraints("session_requests"))
        assert versions == list(range(1, 10))
        assert tuple(row) == ("old-session", "old-owner", "attended", None, None, commitment)
        assert "paired" in next(c["sqltext"] for c in checks if c["name"] == "ck_session_requests_wan_values")
    finally:
        await engine.dispose()
        async with admin.begin() as db:
            await db.execute(text(f'DROP SCHEMA "{schema}" CASCADE'))
        await admin.dispose()


@pytest.mark.skipif(not os.getenv("MRD_TEST_DATABASE_URL"), reason="PostgreSQL migration verification required")
@pytest.mark.anyio
async def test_paired_v9_drift_rejection_rolls_back_columns_and_ledger():
    url = os.environ["MRD_TEST_DATABASE_URL"].replace("postgresql://", "postgresql+asyncpg://", 1)
    schema = "wan_paired_drift_" + uuid4().hex
    admin = create_async_engine(url)
    async with admin.begin() as db:
        await db.execute(text(f'CREATE SCHEMA "{schema}"'))
    engine = create_async_engine(url, connect_args={"server_settings": {"search_path": schema}})
    try:
        async with engine.begin() as db:
            await migrate_relay_control(db)
            await db.run_sync(Base.metadata.create_all)
        await migration.migrate(engine)
        async with engine.begin() as db:
            await db.execute(text("DELETE FROM relay_access_schema_migrations WHERE version = 9"))
            await db.execute(text("ALTER TABLE session_requests DROP CONSTRAINT ck_session_requests_wan_values"))
            await db.execute(text("ALTER TABLE session_requests DROP CONSTRAINT ck_session_requests_paired_keys"))
            await db.execute(text("ALTER TABLE session_requests DROP COLUMN paired_controller_key_id"))
            await db.execute(text("ALTER TABLE session_requests DROP COLUMN paired_target_key_id"))
            await db.execute(text("ALTER TABLE session_requests ADD CONSTRAINT ck_session_requests_wan_values CHECK (" + ATTENDED.replace("= 'attended'", "<> 'forbidden'") + ")"))
        with pytest.raises(migration.RelayAccessMigrationError, match="WAN paired access constraint differs"):
            await migration.migrate(engine)
        async with engine.connect() as db:
            versions = list((await db.execute(text("SELECT version FROM relay_access_schema_migrations ORDER BY version"))).scalars())
            columns = await db.run_sync(lambda sync: {c["name"] for c in inspect(sync).get_columns("session_requests")})
        assert versions == list(range(1, 9))
        assert "paired_controller_key_id" not in columns and "paired_target_key_id" not in columns
    finally:
        await engine.dispose()
        async with admin.begin() as db:
            await db.execute(text(f'DROP SCHEMA "{schema}" CASCADE'))
        await admin.dispose()
