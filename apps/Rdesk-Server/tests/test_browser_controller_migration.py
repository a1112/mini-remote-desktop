import asyncio
import pytest
from sqlalchemy import create_engine, inspect, text


def test_additive_browser_migration_preserves_physical_identity_and_is_idempotent():
    from app.db.migrate_add_browser_controllers import migrate_connection
    engine = create_engine("sqlite:///:memory:")
    with engine.begin() as connection:
        connection.execute(text("CREATE TABLE users (id VARCHAR(36) PRIMARY KEY)"))
        connection.execute(text("CREATE TABLE devices (id VARCHAR(36) PRIMARY KEY)"))
        connection.execute(text("CREATE TABLE session_requests (id VARCHAR(36) PRIMARY KEY)"))
        connection.execute(text("INSERT INTO devices (id) VALUES ('physical-before-upgrade')"))
        migrate_connection(connection)
        migrate_connection(connection)
        assert connection.scalar(text("SELECT principal_kind FROM devices")) == "physical"
        assert inspect(connection).has_table("browser_controllers")
        columns = {column["name"]: column for column in inspect(connection).get_columns("devices")}
        assert not columns["principal_kind"]["nullable"]
        with pytest.raises(Exception):
            connection.execute(text("INSERT INTO devices VALUES ('invalid', 'unknown')"))
    engine.dispose()


def test_browser_migration_rejects_incompatible_existing_principal_table():
    from app.db.migrate_add_browser_controllers import migrate_connection
    engine = create_engine("sqlite:///:memory:")
    with engine.begin() as connection:
        connection.execute(text("CREATE TABLE users (id VARCHAR(36) PRIMARY KEY)"))
        connection.execute(text("CREATE TABLE devices (id VARCHAR(36) PRIMARY KEY)"))
        connection.execute(text("CREATE TABLE session_requests (id VARCHAR(36) PRIMARY KEY)"))
        connection.execute(text("CREATE TABLE browser_controllers (device_row_id VARCHAR(36) PRIMARY KEY)"))
        with pytest.raises(RuntimeError, match="browser principal schema differs"):
            migrate_connection(connection)
    engine.dispose()


@pytest.mark.parametrize("column", [
    "VARCHAR(32) NOT NULL DEFAULT 'physical'",
    "VARCHAR(24) DEFAULT 'physical'",
    "VARCHAR(24) NOT NULL DEFAULT 'browser_controller'",
    "VARCHAR(24) NOT NULL",
    "INTEGER NOT NULL DEFAULT 1",
])
def test_browser_migration_rejects_drifted_principal_column(column):
    from app.db.migrate_add_browser_controllers import migrate_connection
    engine = create_engine("sqlite:///:memory:")
    with engine.begin() as connection:
        connection.execute(text("CREATE TABLE devices (id VARCHAR(36) PRIMARY KEY, principal_kind " + column +
            " CONSTRAINT ck_devices_principal_kind CHECK (principal_kind IN ('physical', 'browser_controller')))"))
        with pytest.raises(RuntimeError, match="browser device principal schema differs"):
            migrate_connection(connection)
    engine.dispose()



def _browser_schema(connection, *, rewrite=lambda ddl: ddl, create_indexes=True):
    from sqlalchemy.schema import CreateTable
    from app.models.browser_controller import BrowserController
    for table in ("users", "devices", "session_requests"):
        connection.execute(text(f"CREATE TABLE {table} (id VARCHAR(36) PRIMARY KEY)"))
    ddl = str(CreateTable(BrowserController.__table__).compile(dialect=connection.dialect))
    connection.execute(text(rewrite(ddl)))
    if create_indexes:
        for index in BrowserController.__table__.indexes:
            index.create(connection)


@pytest.mark.parametrize("fragment", [
    "PRIMARY KEY (device_row_id)",
    "UNIQUE (session_id)",
    "FOREIGN KEY(device_row_id) REFERENCES devices (id) ON DELETE CASCADE",
    "FOREIGN KEY(session_id) REFERENCES session_requests (id) ON DELETE CASCADE",
    "FOREIGN KEY(user_id) REFERENCES users (id) ON DELETE CASCADE",
    "FOREIGN KEY(target_device_row_id) REFERENCES devices (id) ON DELETE CASCADE",
    "CONSTRAINT ck_browser_user_version CHECK (user_session_version >= 1)",
    "CONSTRAINT ck_browser_public_key CHECK (length(public_key) = 32)",
    "CONSTRAINT ck_browser_key_id CHECK (length(key_id) = 64)",
    "CONSTRAINT ck_browser_lifetime CHECK (expires_at > created_at)",
])
def test_browser_migration_rejects_same_shape_table_with_missing_constraint(fragment):
    from app.db.migrate_add_browser_controllers import migrate_connection
    engine = create_engine("sqlite:///:memory:")
    with engine.begin() as connection:
        _browser_schema(connection, rewrite=lambda ddl: ddl.replace(
            "\t" + fragment + ", \n", "").replace(
            ", \n\t" + fragment + "\n", "\n"))
        with pytest.raises(RuntimeError, match="browser principal schema differs"):
            migrate_connection(connection)
    engine.dispose()


@pytest.mark.parametrize("before,after", [
    ("PRIMARY KEY (device_row_id)", "PRIMARY KEY (session_id)"),
    ("UNIQUE (session_id)", "UNIQUE (key_id)"),
    ("REFERENCES session_requests (id)", "REFERENCES devices (id)"),
    ("ON DELETE CASCADE", "ON DELETE RESTRICT"),
    ("ON DELETE CASCADE", "ON DELETE CASCADE ON UPDATE CASCADE"),
    ("ON DELETE CASCADE", "ON DELETE CASCADE DEFERRABLE INITIALLY DEFERRED"),
    ("user_session_version >= 1", "user_session_version >= 0"),
    ("length(public_key) = 32", "length(public_key) >= 32"),
    ("length(key_id) = 64", "length(key_id) = 64 OR 1 = 1"),
    ("expires_at > created_at", "expires_at >= created_at"),
    ("\tPRIMARY KEY (device_row_id),", "\tCONSTRAINT unknown_browser_check CHECK (1 = 1),\n\tPRIMARY KEY (device_row_id),"),
    ("\tUNIQUE (session_id),", "\tUNIQUE (session_id),\n\tUNIQUE (user_id),"),
])
def test_browser_migration_rejects_drifted_identity_and_lifecycle_constraints(before, after):
    from app.db.migrate_add_browser_controllers import migrate_connection
    engine = create_engine("sqlite:///:memory:")
    with engine.begin() as connection:
        _browser_schema(connection, rewrite=lambda ddl: ddl.replace(before, after))
        with pytest.raises(RuntimeError, match="browser principal schema differs"):
            migrate_connection(connection)
    engine.dispose()


@pytest.mark.parametrize("before,after", [
    ("session_id VARCHAR(36)", "session_id VARCHAR(40)"),
    ("tenant_id VARCHAR(64)", "tenant_id VARCHAR(100)"),
    ("user_session_version INTEGER", "user_session_version BIGINT"),
    ("public_key BLOB", "public_key VARCHAR(32)"),
    ("allowed_scopes JSON", "allowed_scopes TEXT"),
    ("revoked_at DATETIME", "revoked_at DATETIME DEFAULT CURRENT_TIMESTAMP"),
    ("user_session_version INTEGER", "user_session_version INTEGER DEFAULT 1"),
])
def test_browser_migration_rejects_drifted_principal_table_column_contract(before, after):
    from app.db.migrate_add_browser_controllers import migrate_connection
    engine = create_engine("sqlite:///:memory:")
    with engine.begin() as connection:
        _browser_schema(connection, rewrite=lambda ddl: ddl.replace(before, after))
        with pytest.raises(RuntimeError, match="browser principal schema differs"):
            migrate_connection(connection)
    engine.dispose()


@pytest.mark.parametrize("index_ddl", [
    None,
    "CREATE INDEX unknown_browser_index ON browser_controllers (tenant_id)",
    "CREATE UNIQUE INDEX unknown_browser_unique ON browser_controllers (key_id)",
    "CREATE INDEX ix_browser_controllers_user_id ON browser_controllers (tenant_id)",
    "CREATE INDEX ix_browser_controllers_user_id ON browser_controllers (user_id) WHERE revoked_at IS NULL",
])
def test_browser_migration_rejects_missing_or_drifted_index_contract(index_ddl):
    from app.db.migrate_add_browser_controllers import migrate_connection
    engine = create_engine("sqlite:///:memory:")
    with engine.begin() as connection:
        _browser_schema(connection)
        if index_ddl is None or "ix_browser_controllers_user_id" in index_ddl:
            connection.execute(text("DROP INDEX ix_browser_controllers_user_id"))
        if index_ddl:
            connection.execute(text(index_ddl))
        with pytest.raises(RuntimeError, match="browser principal schema differs"):
            migrate_connection(connection)
    engine.dispose()


class _BrowserPostgresCatalog:
    """Exact PostgreSQL reflection shapes, backed by real SQLite model DDL."""
    default_schema_name = "public"

    def __init__(self, inspector):
        from copy import deepcopy
        from sqlalchemy import DateTime
        from sqlalchemy.dialects.postgresql import BYTEA, JSONB
        self.columns = deepcopy(inspector.get_columns("browser_controllers"))
        for column in self.columns:
            if column["name"] in ("created_at", "expires_at", "revoked_at"):
                column["type"] = DateTime(timezone=True)
            elif column["name"] == "public_key":
                column["type"] = BYTEA()
            elif column["name"] == "allowed_scopes":
                column["type"] = JSONB()
        self.device_columns = inspector.get_columns("devices")
        self.device_checks = inspector.get_check_constraints("devices")
        self.pk = inspector.get_pk_constraint("browser_controllers")
        self.pk["name"] = "browser_controllers_pkey"
        self.unique = inspector.get_unique_constraints("browser_controllers")
        self.unique[0]["name"] = "browser_controllers_session_id_key"
        self.fks = inspector.get_foreign_keys("browser_controllers")
        for key in self.fks:
            key["name"] = "browser_controllers_" + key["constrained_columns"][0] + "_fkey"
            key["referred_schema"] = "public"
        self.checks = inspector.get_check_constraints("browser_controllers")
        # Actual PostgreSQL CHECK spelling includes casts and redundant parentheses.
        for check in self.checks:
            if check["name"] == "ck_browser_key_id":
                check["sqltext"] = "(length((key_id)::text) = 64)"
            else:
                check["sqltext"] = "(" + check["sqltext"] + ")"
        self.indexes = inspector.get_indexes("browser_controllers")
        self.indexes.append({"name": "browser_controllers_session_id_key",
            "column_names": ["session_id"], "unique": True,
            "duplicates_constraint": "browser_controllers_session_id_key"})
        self.states = {self.pk["name"]: ("p", True, False, False),
            **{item["name"]: ("u", True, False, False) for item in self.unique},
            **{item["name"]: ("f", True, False, False) for item in self.fks},
            **{item["name"]: ("c", True, False, False) for item in self.checks}}

    def has_table(self, table, **_):
        return True

    def get_columns(self, table, **_):
        return self.device_columns if table == "devices" else self.columns

    def get_check_constraints(self, table, **_):
        return self.device_checks if table == "devices" else self.checks

    def get_pk_constraint(self, *_args, **_):
        return self.pk

    def get_unique_constraints(self, *_args, **_):
        return self.unique

    def get_foreign_keys(self, *_args, **_):
        return self.fks

    def get_indexes(self, *_args, **_):
        return self.indexes


class _BrowserPostgresConnection:
    from sqlalchemy.dialects.postgresql import dialect as postgres_dialect
    dialect = postgres_dialect()

    def __init__(self, catalog):
        self.catalog = catalog

    def execute(self, statement, parameters=None):
        from types import SimpleNamespace
        assert "FROM pg_constraint" in str(statement)
        assert parameters == {"schema": "public", "table_name": "browser_controllers"}
        return [SimpleNamespace(conname=name, contype=kind, convalidated=valid,
            condeferrable=deferrable, condeferred=deferred)
            for name, (kind, valid, deferrable, deferred) in self.catalog.states.items()]

    def scalar(self, statement, parameters=None):
        assert "JOIN pg_am" in str(statement)
        return "btree"


@pytest.fixture
def browser_postgres_catalog(monkeypatch):
    from app.db import migrate_add_browser_controllers as migration
    from app.models.browser_controller import BrowserController
    engine = create_engine("sqlite:///:memory:")
    with engine.begin() as connection:
        _browser_schema(connection)
        connection.execute(text("ALTER TABLE devices ADD COLUMN principal_kind VARCHAR(24) "
            "NOT NULL DEFAULT 'physical' CONSTRAINT ck_devices_principal_kind "
            "CHECK (principal_kind IN ('physical', 'browser_controller'))"))
        catalog = _BrowserPostgresCatalog(inspect(connection))
    engine.dispose()
    monkeypatch.setattr(migration, "inspect", lambda _connection: catalog)
    # Table creation is exercised with actual model DDL above and in legacy tests.
    # This fixture isolates PostgreSQL-only catalog/state verification without a DB server.
    monkeypatch.setattr(BrowserController.__table__, "create", lambda *_args, **_: None)
    return catalog, _BrowserPostgresConnection(catalog)


def test_browser_migration_accepts_exact_postgres_catalog_and_repeat_startup(browser_postgres_catalog):
    from app.db.migrate_add_browser_controllers import migrate_connection
    _catalog, connection = browser_postgres_catalog
    migrate_connection(connection)
    migrate_connection(connection)


@pytest.mark.parametrize("drift", [
    "fk_other_schema", "fk_other_column", "fk_match_full", "fk_unknown_option",
    "unique_nulls_not_distinct", "pk_include_column", "unknown_check",
    "check_not_valid", "foreign_key_not_valid", "unique_deferrable", "unknown_state",
    "timestamp_without_timezone", "json_not_jsonb", "computed_column", "identity_column",
])
def test_browser_migration_rejects_postgres_catalog_drift(browser_postgres_catalog, drift):
    from sqlalchemy import DateTime, JSON
    from app.db.migrate_add_browser_controllers import migrate_connection
    catalog, connection = browser_postgres_catalog
    if drift == "fk_other_schema":
        catalog.fks[0]["referred_schema"] = "untrusted"
    elif drift == "fk_other_column":
        catalog.fks[0]["referred_columns"] = ["tenant_id"]
    elif drift == "fk_match_full":
        catalog.fks[0]["options"]["match"] = "FULL"
    elif drift == "fk_unknown_option":
        catalog.fks[0]["options"]["unexpected"] = True
    elif drift == "unique_nulls_not_distinct":
        catalog.unique[0]["dialect_options"] = {"postgresql_nulls_not_distinct": True}
    elif drift == "pk_include_column":
        catalog.pk["dialect_options"] = {"postgresql_include": ["tenant_id"]}
    elif drift == "unknown_check":
        catalog.checks.append({"name": "unknown", "sqltext": "1 = 1"})
    elif drift in ("check_not_valid", "foreign_key_not_valid", "unique_deferrable"):
        name = (catalog.checks[0]["name"] if drift == "check_not_valid" else
            catalog.fks[0]["name"] if drift == "foreign_key_not_valid" else catalog.unique[0]["name"])
        kind = catalog.states[name][0]
        catalog.states[name] = (kind, drift == "unique_deferrable", drift == "unique_deferrable", False)
    elif drift == "unknown_state":
        catalog.states["unknown_exclusion"] = ("x", True, False, False)
    elif drift == "timestamp_without_timezone":
        next(column for column in catalog.columns if column["name"] == "created_at")["type"] = DateTime()
    elif drift == "json_not_jsonb":
        next(column for column in catalog.columns if column["name"] == "allowed_scopes")["type"] = JSON()
    else:
        catalog.columns[0]["computed" if drift == "computed_column" else "identity"] = {"always": True}
    with pytest.raises(RuntimeError, match="browser principal schema differs"):
        migrate_connection(connection)
