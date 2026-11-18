"""The durable automatic-grant column upgrades old PostgreSQL databases safely."""

import os
from uuid import uuid4

import pytest
from sqlalchemy import inspect, text
from sqlalchemy.ext.asyncio import create_async_engine

from app.db.migrate_add_same_account_authority import SameAccountMigrationError, migrate


DATABASE_URL = os.getenv("MRD_TEST_DATABASE_URL")


@pytest.mark.asyncio
@pytest.mark.skipif(not DATABASE_URL, reason="requires PostgreSQL migration fixture")
@pytest.mark.parametrize("preexisting", [False, True])
async def test_same_account_migration_upgrades_and_is_idempotent(preexisting):
    engine = create_async_engine(DATABASE_URL)
    schema = "mrd_same_" + uuid4().hex[:16]
    try:
        async with engine.begin() as connection:
            await connection.execute(text(f'CREATE SCHEMA "{schema}"'))
            column = ", same_account_authority JSONB" if preexisting else ""
            await connection.execute(text(f'CREATE TABLE "{schema}".session_requests (id TEXT PRIMARY KEY{column})'))
            await migrate(connection, schema=schema)
            columns = await connection.run_sync(lambda sync: inspect(sync).get_columns("session_requests", schema=schema))
            assert {item["name"] for item in columns} == {"id", "same_account_authority"}
            await connection.execute(text(f'INSERT INTO "{schema}".session_requests '
                                          "(id, same_account_authority) VALUES ('one', '{\"owner_user_id\":\"u\"}'::jsonb)"))
            await migrate(connection, schema=schema)
            assert await connection.scalar(text(f'SELECT count(*) FROM "{schema}".same_account_authority_schema_migrations')) == 1
    finally:
        async with engine.begin() as connection:
            await connection.execute(text(f'DROP SCHEMA IF EXISTS "{schema}" CASCADE'))
        await engine.dispose()


@pytest.mark.asyncio
@pytest.mark.skipif(not DATABASE_URL, reason="requires PostgreSQL migration fixture")
async def test_same_account_migration_rejects_existing_nonempty_unversioned_state():
    engine = create_async_engine(DATABASE_URL)
    schema = "mrd_same_" + uuid4().hex[:16]
    try:
        async with engine.begin() as connection:
            await connection.execute(text(f'CREATE SCHEMA "{schema}"'))
            await connection.execute(text(f'CREATE TABLE "{schema}".session_requests '
                                          "(id TEXT PRIMARY KEY, same_account_authority JSONB)"))
            await connection.execute(text(f'INSERT INTO "{schema}".session_requests '
                                          "(id, same_account_authority) VALUES ('one', '{}'::jsonb)"))
        with pytest.raises(SameAccountMigrationError, match="unversioned"):
            async with engine.begin() as connection:
                await migrate(connection, schema=schema)
    finally:
        async with engine.begin() as connection:
            await connection.execute(text(f'DROP SCHEMA IF EXISTS "{schema}" CASCADE'))
        await engine.dispose()
