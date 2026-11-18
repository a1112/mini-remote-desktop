"""Explicit opt-in PostgreSQL migration for independent OIDC identity state."""
import hashlib

from sqlalchemy import inspect, text
from sqlalchemy.ext.asyncio import AsyncConnection

from app.db.migrate_add_relay_access import _normalize_check_expression, _table
from app.db.session import Base


OIDC_TABLES = frozenset({"oidc_identities", "oidc_login_transactions"})


class OidcMigrationError(RuntimeError):
    pass


def _verify(connection: object, schema: str) -> None:
    inspector = inspect(connection)
    tables = Base.metadata.tables
    for name in OIDC_TABLES:
        expected = tables[name]
        actual = {col["name"]: col for col in inspector.get_columns(name, schema=schema)}
        if set(actual) != set(expected.columns.keys()):
            raise OidcMigrationError("OIDC schema columns differ")
        for col in expected.columns:
            found = actual[col.name]
            if not isinstance(found["type"], type(col.type)):
                raise OidcMigrationError("OIDC schema column type differs")
            if (getattr(found["type"], "length", None) != getattr(col.type, "length", None)
                    or found["nullable"] != col.nullable
                    or found["default"] is not None
                    or (col.name in {"created_at", "expires_at"} and not found["type"].timezone)):
                raise OidcMigrationError("OIDC schema column attributes differ")
        if inspector.get_pk_constraint(name, schema=schema)["constrained_columns"] != [c.name for c in expected.primary_key]:
            raise OidcMigrationError("OIDC schema primary key differs")
    unique_identity = inspector.get_unique_constraints("oidc_identities", schema=schema)
    unique_flow = inspector.get_unique_constraints("oidc_login_transactions", schema=schema)
    if ([item["column_names"] for item in unique_identity] != [["user_id"]]
            or [item["column_names"] for item in unique_flow] != [["state_digest"]]):
        raise OidcMigrationError("OIDC schema uniqueness differs")
    for name, expected_columns in (("oidc_identities", {"user_id"}),
                                   ("oidc_login_transactions", {"link_user_id", "user_id"})):
        foreign = inspector.get_foreign_keys(name, schema=schema)
        if (len(foreign) != len(expected_columns)
                or {item["constrained_columns"][0] for item in foreign} != expected_columns
                or any(item["referred_table"] != "users" or item["referred_columns"] != ["id"]
                       or item["referred_schema"] not in (None, schema)
                       or item.get("options", {}).get("ondelete") != "RESTRICT" for item in foreign)):
            raise OidcMigrationError("OIDC schema foreign keys differ")
    checks = inspector.get_check_constraints("oidc_login_transactions", schema=schema)
    expected_check = "status=anyarray['pending','exchanging','complete','failed','consumed']"
    if (len(checks) != 1 or checks[0]["name"] != "ck_oidc_flow_status"
            or _normalize_check_expression(checks[0]["sqltext"]).replace("(", "").replace(")", "") != expected_check):
        raise OidcMigrationError("OIDC schema checks differ")
    indexes = inspector.get_indexes("oidc_login_transactions", schema=schema)
    if not any(item["name"] == "ix_oidc_flow_expires_at" and item["column_names"] == ["expires_at"]
               and not item["unique"] for item in indexes):
        raise OidcMigrationError("OIDC schema expiry index differs")


async def migrate(connection: AsyncConnection, *, schema: str | None = None) -> None:
    if connection.dialect.name != "postgresql":
        raise OidcMigrationError("OIDC migration requires PostgreSQL")
    effective_schema = schema or await connection.scalar(text("SELECT current_schema()"))
    versions = _table(effective_schema, "oidc_schema_migrations")
    lock = int.from_bytes(hashlib.sha256(b"MRD_OIDC_MIGRATION_V1\0" + effective_schema.encode()).digest()[:8], "big", signed=True)
    await connection.execute(text("SELECT pg_advisory_xact_lock(:key)"), {"key": lock})
    await connection.execute(text(f"CREATE TABLE IF NOT EXISTS {versions} (version INTEGER PRIMARY KEY, applied_at TIMESTAMPTZ NOT NULL DEFAULT now())"))
    applied = set((await connection.execute(text(f"SELECT version FROM {versions}"))).scalars())
    if applied not in (set(), {1}):
        raise OidcMigrationError("OIDC schema versions differ")
    if not applied:
        if await connection.run_sync(lambda sync: any(inspect(sync).has_table(name, schema=effective_schema) for name in OIDC_TABLES)):
            raise OidcMigrationError("OIDC tables exist without migration")
        # Translation permits a reviewed non-default schema and keeps the ORM
        # definitions and explicit migration aligned for new installations.
        await connection.run_sync(lambda sync: Base.metadata.tables["oidc_identities"].create(
            sync.execution_options(schema_translate_map={None: effective_schema}), checkfirst=False))
        await connection.run_sync(lambda sync: Base.metadata.tables["oidc_login_transactions"].create(
            sync.execution_options(schema_translate_map={None: effective_schema}), checkfirst=False))
        await connection.run_sync(lambda sync: _verify(sync, effective_schema))
        await connection.execute(text(f"INSERT INTO {versions} (version) VALUES (1)"))
    await connection.run_sync(lambda sync: _verify(sync, effective_schema))
    identities, transactions = (_table(effective_schema, name) for name in ("oidc_identities", "oidc_login_transactions"))
    unvalidated = await connection.scalar(text("""
        SELECT count(*) FROM pg_constraint
        WHERE conrelid IN (CAST(:identities AS regclass), CAST(:transactions AS regclass))
          AND contype IN ('c', 'f') AND NOT convalidated
    """), {"identities": identities, "transactions": transactions})
    if unvalidated:
        raise OidcMigrationError("OIDC schema constraints are unvalidated")
