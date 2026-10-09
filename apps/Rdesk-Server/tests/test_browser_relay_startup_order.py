"""Exercise the production CHECK and catalog-state verifiers without a PG fixture.

Only unrelated relay/enrollment tables are omitted from this catalog fixture.
The device column is migrated by the real additive SQLite migration and its
CHECK is represented exactly as PostgreSQL's public catalog reports it.
"""
from types import SimpleNamespace

import pytest
from sqlalchemy import DateTime, create_engine, inspect, text

import app.db.migrate_add_relay_access as access
import app.db.migrate_add_browser_controllers as browser
from app.models.device import Device


BASE_DEVICE_CHECKS = {
    "ck_devices_tenant_id": "length(tenant_id) >= 1 AND length(tenant_id) <= 64",
    "ck_devices_tenant_id_canonical": "tenant_id ~ '^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$'",
    "ck_devices_bound_owner": "is_bound = FALSE AND bound_user_id IS NULL OR is_bound = TRUE AND bound_user_id IS NOT NULL",
    "ck_devices_auth_version": "auth_version >= 1",
    "ck_devices_serial_digest": "motherboard_serial_digest IS NULL OR length(motherboard_serial_digest) = 64",
    "ck_devices_plaintext_serial_cleared": "motherboard_serial IS NULL",
}
PRINCIPAL_CHECK = "principal_kind = ANY (ARRAY['physical', 'browser_controller'])"


class DeviceChecksVerified(Exception):
    pass


class Catalog:
    def __init__(self, connection, *, drift=None):
        self.connection = connection
        self.drift = drift

    def has_table(self, name, schema=None):
        return name == "browser_controllers"

    def get_columns(self, table, schema=None):
        defaults = {"tenant_id": "'default'", "status": "'requested'", "auth_version": "1", "session_version": "1"}
        result = [{"name": name, "type": kind(length) if length else kind(timezone=True) if kind is DateTime else kind(),
                   "nullable": nullable, "default": defaults.get(name)}
                  for name, (kind, length, nullable) in access._auth_specs()[table].items()]
        if table == "devices":
            result += [column for column in inspect(self.connection).get_columns("devices")
                       if column["name"] == "principal_kind"]
        return result

    def get_check_constraints(self, table, schema=None):
        if table == "users":
            checks = {"ck_users_tenant_id": "length(tenant_id) >= 1 AND length(tenant_id) <= 64",
                      "ck_users_tenant_id_canonical": "tenant_id ~ '^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$'"}
        elif table == "session_requests":
            raise DeviceChecksVerified
        else:
            checks = dict(BASE_DEVICE_CHECKS)
            if any(column["name"] == "principal_kind" for column in inspect(self.connection).get_columns("devices")):
                checks["ck_devices_principal_kind"] = PRINCIPAL_CHECK
            if self.drift == "unknown_check": checks["unrecognized_device_check"] = "auth_version > 0"
            if self.drift == "bounds": checks["ck_devices_principal_kind"] = "principal_kind <> 'forbidden'"
            if self.drift == "trivial_or": checks["ck_devices_principal_kind"] = "principal_kind IN ('physical', 'browser_controller') OR 1 = 1"
            if self.drift == "function": checks["ck_devices_principal_kind"] = "principal_kindin('physical', 'browser_controller')"
            if self.drift == "pg_wrapped":
                checks["ck_devices_principal_kind"] = "((principal_kind)::text = ANY ((ARRAY['physical'::character varying, 'browser_controller'::character varying])::text[]))"
        return [{"name": name, "sqltext": sql} for name, sql in checks.items()]


class CatalogConnection:
    def __init__(self, catalog):
        self.catalog = catalog

    def scalar(self, statement):
        assert "current_schema()" in str(statement)
        return "public"

    def execute(self, statement, parameters):
        assert "pg_constraint" in str(statement)
        assert parameters == {"schema": "public", "table_name": "devices"}
        names = ["devices_pkey", "devices_bound_user_id_fkey", *BASE_DEVICE_CHECKS]
        if "principal_kind" in {column["name"] for column in self.catalog.get_columns("devices")}:
            names += ["ck_devices_principal_kind"]
        if self.catalog.drift == "unknown_state": names += ["unknown_state"]
        return [SimpleNamespace(conname=name, contype="p" if name == "devices_pkey" else
                "f" if name == "devices_bound_user_id_fkey" else "c",
                convalidated=not (name == "ck_devices_principal_kind" and self.catalog.drift == "not_valid"),
                condeferrable=False, condeferred=False) for name in names]


def verify_access_device_boundary(monkeypatch, connection, drift=None):
    catalog = Catalog(connection, drift=drift)
    wrapped = CatalogConnection(catalog)
    monkeypatch.setattr(access, "inspect", lambda _: catalog)
    monkeypatch.setattr(access, "assert_relay_schema_conforms", lambda *_: None)
    monkeypatch.setattr(access, "_verify_device_enrollment_table", lambda *_, **__: None)
    monkeypatch.setattr(access, "_verify_relay_access_generation_table", lambda *_, **__: None)
    # Stop after the actual production user/device CHECK comparison succeeds.
    with pytest.raises(DeviceChecksVerified):
        access._verify(wrapped, None)
    expected_types = {"devices_pkey": "p", "devices_bound_user_id_fkey": "f",
                      **{name: "c" for name in BASE_DEVICE_CHECKS}}
    if "principal_kind" in {column["name"] for column in catalog.get_columns("devices")}:
        expected_types["ck_devices_principal_kind"] = "c"
    access._assert_constraint_states(wrapped, schema="public", table_name="devices",
                                     expected_types=expected_types, exact=True)


@pytest.mark.parametrize("initial", ["legacy", "fresh"])
def test_access_browser_access_startup_order_survives_repeated_starts(monkeypatch, initial):
    engine = create_engine("sqlite:///:memory:")
    with engine.begin() as connection:
        connection.execute(text("CREATE TABLE users (id VARCHAR(36) PRIMARY KEY)"))
        connection.execute(text("CREATE TABLE session_requests (id VARCHAR(36) PRIMARY KEY)"))
        if initial == "legacy":
            connection.execute(text("CREATE TABLE devices (id VARCHAR(36) PRIMARY KEY)"))
        else:
            Device.__table__.create(connection)
        for _ in range(2):
            verify_access_device_boundary(monkeypatch, connection)
            browser.migrate_connection(connection)
        verify_access_device_boundary(monkeypatch, connection)
    engine.dispose()


@pytest.mark.parametrize("drift", ["unknown_check", "bounds", "trivial_or", "function", "unknown_state", "not_valid"])
def test_browser_device_schema_extension_keeps_checks_and_states_closed(monkeypatch, drift):
    engine = create_engine("sqlite:///:memory:")
    with engine.begin() as connection:
        connection.execute(text("CREATE TABLE users (id VARCHAR(36) PRIMARY KEY)"))
        connection.execute(text("CREATE TABLE session_requests (id VARCHAR(36) PRIMARY KEY)"))
        connection.execute(text("CREATE TABLE devices (id VARCHAR(36) PRIMARY KEY)"))
        browser.migrate_connection(connection)
        with pytest.raises(access.RelayAccessMigrationError):
            verify_access_device_boundary(monkeypatch, connection, drift)
    engine.dispose()


def test_postgres_casts_and_parentheses_keep_the_exact_browser_domain(monkeypatch):
    engine = create_engine("sqlite:///:memory:")
    with engine.begin() as connection:
        connection.execute(text("CREATE TABLE users (id VARCHAR(36) PRIMARY KEY)"))
        connection.execute(text("CREATE TABLE session_requests (id VARCHAR(36) PRIMARY KEY)"))
        Device.__table__.create(connection)
        verify_access_device_boundary(monkeypatch, connection, "pg_wrapped")
    engine.dispose()
