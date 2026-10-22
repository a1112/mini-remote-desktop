from sqlalchemy import create_engine, text, inspect, MetaData, CheckConstraint
from sqlalchemy.schema import CreateTable
import pytest
from app.models.browser_controller import BrowserController
from app.models.session_request import SessionRequest


def _old(table, removed, check):
    from app.db.session import Base

    metadata = MetaData()
    for candidate in Base.metadata.sorted_tables:
        candidate.to_metadata(metadata)
    copy = metadata.tables[table.name]
    for constraint in list(copy.constraints):
        if isinstance(constraint, CheckConstraint) and constraint.name == check:
            copy.constraints.remove(constraint)
    for name in removed:
        copy._columns.remove(copy.c[name])
    if copy.name == "browser_controllers":
        copy.c.user_id.nullable = False
        copy.c.user_session_version.nullable = False
    else:
        copy.c.requester_user_id.nullable = False
    return copy


def test_guest_migration_upgrades_known_legacy_preserves_rows_and_repeats():
    from app.db.migrate_add_guest_temporary_access import migrate_connection

    engine = create_engine("sqlite:///:memory:")
    with engine.begin() as connection:
        connection.execute(text("CREATE TABLE users (id VARCHAR(36) PRIMARY KEY)"))
        connection.execute(text("CREATE TABLE devices (id VARCHAR(36) PRIMARY KEY)"))
        old_session = _old(
            SessionRequest.__table__,
            [
                "authority_kind",
                "temporary_access_generation",
                "target_auth_version",
                "authority_expires_at",
            ],
            "ck_session_requests_authority",
        )
        old_browser = _old(
            BrowserController.__table__,
            ["authority_kind", "temporary_access_generation", "target_auth_version"],
            "ck_browser_authority",
        )
        for table in (old_session, old_browser):
            connection.execute(
                text(str(CreateTable(table).compile(dialect=connection.dialect)))
            )
            for index in table.indexes:
                index.create(connection)
        migrate_connection(connection)
        migrate_connection(connection)
        assert inspect(connection).has_table("device_temporary_access")
        assert (
            next(
                c
                for c in inspect(connection).get_columns("session_requests")
                if c["name"] == "requester_user_id"
            )["nullable"]
            is True
        )
        assert (
            next(
                c
                for c in inspect(connection).get_columns("browser_controllers")
                if c["name"] == "user_id"
            )["nullable"]
            is True
        )


def test_guest_schema_rejects_null_account_and_missing_guest_versions(
    device_sessions_api,
):
    api = device_sessions_api
    for authority, fields in [
        ("account", {}),
        ("temporary_password", {"authority_expires_at": "2026-10-09 10:00:00"}),
    ]:
        with pytest.raises(Exception):
            api.session.execute(
                text(
                    "INSERT INTO session_requests (id,requester_user_id,target_device_id,signaling_room,tenant_id,status,authority_kind) VALUES ('bad',NULL,'target-row','bad','tenant-a','requested',:authority)"
                ),
                {"authority": authority},
            )
            api.session.commit()
        api.session.rollback()


from test_device_session_api import device_sessions_api


def test_guest_principal_requires_nonnull_generation(
    device_sessions_api, guest_config, trusted_realtime_identity
):
    from test_guest_browser_session_api import _publish, _guest
    from app.models.browser_controller import BrowserController

    api = device_sessions_api
    assert _publish(api).status_code == 200
    assert _guest(api).status_code == 200
    principal = api.session.scalar(__import__("sqlalchemy").select(BrowserController))
    principal.temporary_access_generation = None
    with pytest.raises(Exception):
        api.session.commit()
    api.session.rollback()


from test_guest_browser_session_api import guest_config, trusted_realtime_identity


def test_authority_parser_accepts_postgres_casts_and_scalar_parentheses():
    from app.db.migrate_add_guest_temporary_access import (
        authority_expression,
        expected_authority_check,
    )

    expression = expected_authority_check(BrowserController.__table__)
    postgres = (
        expression.replace("authority_kind", "(authority_kind)::text")
        .replace("'account'", "'account'::text")
        .replace("'temporary_password'", "'temporary_password'::text")
    )
    assert authority_expression(postgres) == authority_expression(expression)


def test_guest_upgrade_rejects_drifted_legacy_session_type():
    from app.db.migrate_add_guest_temporary_access import migrate_connection

    engine = create_engine("sqlite:///:memory:")
    with engine.begin() as connection:
        connection.execute(text("CREATE TABLE users (id VARCHAR(36) PRIMARY KEY)"))
        connection.execute(text("CREATE TABLE devices (id VARCHAR(36) PRIMARY KEY)"))
        old_session = _old(
            SessionRequest.__table__,
            [
                "authority_kind",
                "temporary_access_generation",
                "target_auth_version",
                "authority_expires_at",
            ],
            "ck_session_requests_authority",
        )
        old_browser = _old(
            BrowserController.__table__,
            ["authority_kind", "temporary_access_generation", "target_auth_version"],
            "ck_browser_authority",
        )
        for table in (old_session, old_browser):
            ddl = str(CreateTable(table).compile(dialect=connection.dialect))
            if table.name == "session_requests":
                ddl = ddl.replace("grant_expires_at DATETIME", "grant_expires_at TEXT")
            connection.execute(text(ddl))
            for index in table.indexes:
                index.create(connection)
        with pytest.raises(RuntimeError):
            migrate_connection(connection)
