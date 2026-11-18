"""Version the durable authorization snapshot on WAN session grants."""

import hashlib
import re

from sqlalchemy import inspect, text
from sqlalchemy.dialects.postgresql import JSONB
from sqlalchemy.ext.asyncio import AsyncConnection

from app.db.migrate_add_relay_access import _table


class SameAccountMigrationError(RuntimeError):
    pass


def _column(connection, schema):
    inspector = inspect(connection)
    return next((item for item in inspector.get_columns("session_requests", schema=schema)
                 if item["name"] == "same_account_authority"), None)


def _verify(connection, schema):
    column = _column(connection, schema)
    if (column is None or not isinstance(column["type"], JSONB)
        or column["nullable"] is not True or column["default"] is not None):
        raise SameAccountMigrationError("same-account authority column differs")


async def migrate(connection: AsyncConnection, *, schema: str | None = None) -> None:
    if connection.dialect.name != "postgresql":
        raise SameAccountMigrationError("same-account migration requires PostgreSQL")
    effective_schema = schema or await connection.scalar(text("SELECT current_schema()"))
    if (not isinstance(effective_schema, str)
        or re.fullmatch(r"[a-z_][a-z0-9_]{0,62}", effective_schema) is None):
        raise SameAccountMigrationError("same-account schema is invalid")
    lock = int.from_bytes(hashlib.sha256(
        b"MRD_SAME_ACCOUNT_AUTHORITY_V1\0" + effective_schema.encode()
    ).digest()[:8], "big", signed=True)
    await connection.execute(text("SELECT pg_advisory_xact_lock(:key)"), {"key": lock})
    versions = _table(effective_schema, "same_account_authority_schema_migrations")
    sessions = _table(effective_schema, "session_requests")
    await connection.execute(text(
        f"CREATE TABLE IF NOT EXISTS {versions} "
        "(version INTEGER PRIMARY KEY, applied_at TIMESTAMPTZ NOT NULL DEFAULT now())"
    ))
    applied = set((await connection.execute(text(f"SELECT version FROM {versions}"))).scalars())
    if applied not in (set(), {1}):
        raise SameAccountMigrationError("same-account migration versions differ")
    if not applied:
        if await connection.run_sync(lambda sync: _column(sync, effective_schema)) is None:
            await connection.execute(text(
                f"ALTER TABLE {sessions} ADD COLUMN same_account_authority JSONB"
            ))
        await connection.run_sync(lambda sync: _verify(sync, effective_schema))
        # Fresh databases may already contain the column from metadata.create_all;
        # it must not contain unversioned authorization state.
        if await connection.scalar(text(
            f"SELECT EXISTS (SELECT 1 FROM {sessions} WHERE same_account_authority IS NOT NULL)"
        )):
            raise SameAccountMigrationError("unversioned same-account authority exists")
        await connection.execute(text(f"INSERT INTO {versions} (version) VALUES (1)"))
    await connection.run_sync(lambda sync: _verify(sync, effective_schema))
