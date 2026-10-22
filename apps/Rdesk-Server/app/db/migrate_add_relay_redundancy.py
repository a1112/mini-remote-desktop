"""Add a grant-owned redundancy snapshot without weakening legacy grants."""
from __future__ import annotations

import hashlib

from sqlalchemy import Integer, inspect, text
from sqlalchemy.ext.asyncio import AsyncConnection

from app.db.migrate_add_relay_access import (
    RelayAccessMigrationError, _normalize_check_expression, _normalize_server_default, _table,
)


def verify_redundancy_column(connection: object, schema: str | None) -> None:
    columns = {item["name"]: item for item in inspect(connection).get_columns("session_requests", schema=schema)}
    item = columns.get("relay_max_backups")
    if (item is None or not isinstance(item["type"], Integer)
            or item["nullable"] or _normalize_server_default(item["default"]) != "1"):
        raise RelayAccessMigrationError("relay redundancy schema differs")


async def migrate(connection: AsyncConnection, *, schema: str | None = None) -> None:
    if connection.dialect.name != "postgresql":
        raise RelayAccessMigrationError("relay redundancy migration requires PostgreSQL")
    effective_schema = schema or await connection.scalar(text("SELECT current_schema()"))
    sessions = _table(effective_schema, "session_requests")
    versions = _table(effective_schema, "relay_redundancy_schema_migrations")
    lock = int.from_bytes(hashlib.sha256(b"MRD_RELAY_REDUNDANCY_V1\0" + effective_schema.encode()).digest()[:8], "big", signed=True)
    await connection.execute(text("SELECT pg_advisory_xact_lock(:key)"), {"key": lock})
    await connection.execute(text(f"CREATE TABLE IF NOT EXISTS {versions} (version INTEGER PRIMARY KEY, applied_at TIMESTAMPTZ NOT NULL DEFAULT now())"))
    applied = set((await connection.execute(text(f"SELECT version FROM {versions}"))).scalars())
    if applied not in (set(), {1}):
        raise RelayAccessMigrationError("relay redundancy migration versions differ")
    if not applied:
        await connection.execute(text(f"ALTER TABLE {sessions} ADD COLUMN IF NOT EXISTS relay_max_backups INTEGER NOT NULL DEFAULT 1"))
        await connection.run_sync(lambda sync: verify_redundancy_column(sync, effective_schema))
        await connection.execute(text(f"ALTER TABLE {sessions} ADD CONSTRAINT ck_session_requests_relay_max_backups CHECK (relay_max_backups BETWEEN 0 AND 7)"))
        await connection.execute(text(f"INSERT INTO {versions} (version) VALUES (1)"))
    await connection.run_sync(lambda sync: verify_redundancy_column(sync, effective_schema))
    expression = await connection.scalar(text("SELECT pg_get_expr(conbin, conrelid) FROM pg_constraint WHERE conrelid = CAST(:table AS regclass) AND conname = 'ck_session_requests_relay_max_backups' AND contype = 'c' AND convalidated"), {"table": f"{effective_schema}.session_requests"})
    if _normalize_check_expression(expression).replace("(", "").replace(")", "") != "relay_max_backups>=0andrelay_max_backups<=7":
        raise RelayAccessMigrationError("relay redundancy constraint differs")
