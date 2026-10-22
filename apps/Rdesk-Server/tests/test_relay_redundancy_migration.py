import asyncio
import os
from uuid import uuid4

import pytest
from sqlalchemy import create_engine, text
from sqlalchemy.exc import IntegrityError
from sqlalchemy.ext.asyncio import create_async_engine

from app.db.migrate_add_relay_redundancy import verify_redundancy_column
from app.db.migrate_add_relay_redundancy import migrate


@pytest.mark.skipif(not os.getenv("MRD_TEST_DATABASE_URL"), reason="PostgreSQL migration test requires an isolated database")
@pytest.mark.anyio
@pytest.mark.parametrize("drift", [None, "bounds", "default", "unknown_check"])
async def test_access_then_redundancy_migrations_restart_with_exact_schema(drift):
    from app.db.migrate_add_relay_access import migrate as migrate_access
    from app.db.migrate_add_relay_control import migrate as migrate_control
    from app.db.session import Base
    import app.models  # register the production tables
    schema = "relay_restart_" + uuid4().hex
    admin = create_async_engine(os.environ["MRD_TEST_DATABASE_URL"])
    async with admin.begin() as connection:
        await connection.execute(text(f'CREATE SCHEMA "{schema}"'))
    engine = create_async_engine(os.environ["MRD_TEST_DATABASE_URL"], connect_args={"server_settings": {"search_path": schema}})
    try:
        async with engine.begin() as connection:
            await migrate_control(connection)
            await connection.run_sync(Base.metadata.create_all)
        await migrate_access(engine)
        async with engine.begin() as connection:
            await migrate(connection)
            if drift == "bounds":
                await connection.execute(text("ALTER TABLE session_requests DROP CONSTRAINT ck_session_requests_relay_max_backups"))
                await connection.execute(text("ALTER TABLE session_requests ADD CONSTRAINT ck_session_requests_relay_max_backups CHECK (relay_max_backups BETWEEN 0 AND 8)"))
            elif drift == "default":
                await connection.execute(text("ALTER TABLE session_requests ALTER COLUMN relay_max_backups SET DEFAULT 0"))
            elif drift == "unknown_check":
                await connection.execute(text("ALTER TABLE session_requests ADD CONSTRAINT unexpected_check CHECK (relay_max_backups >= 0)"))
        if drift:
            with pytest.raises(RuntimeError, match="(checks differ|redundancy schema differs)"):
                await migrate_access(engine)
        else:
            await migrate_access(engine)
            async with engine.begin() as connection:
                await migrate(connection)
            await migrate_access(engine)
    finally:
        await engine.dispose()
        async with admin.begin() as connection:
            await connection.execute(text(f'DROP SCHEMA IF EXISTS "{schema}" CASCADE'))
        await admin.dispose()


@pytest.mark.parametrize("spec", ["INTEGER DEFAULT 1", "TEXT NOT NULL DEFAULT '1'", "INTEGER NOT NULL DEFAULT 0"])
def test_redundancy_schema_drift_is_rejected(spec):
    engine = create_engine("sqlite:///:memory:")
    with engine.begin() as conn:
        conn.execute(text(f"CREATE TABLE session_requests (relay_max_backups {spec})"))
        with pytest.raises(RuntimeError, match="relay redundancy schema differs"):
            verify_redundancy_column(conn, None)
    engine.dispose()


@pytest.mark.skipif(not os.getenv("MRD_TEST_DATABASE_URL"), reason="PostgreSQL migration test requires an isolated database")
def test_postgres_redundancy_upgrade_preserves_legacy_grants_and_rejects_drift():
    async def check():
        engine = create_async_engine(os.environ["MRD_TEST_DATABASE_URL"])
        schema = "relay_redundancy_" + uuid4().hex
        try:
            async with engine.begin() as connection:
                await connection.execute(text(f'CREATE SCHEMA "{schema}"'))
                await connection.execute(text(f'CREATE TABLE "{schema}".session_requests (id TEXT PRIMARY KEY)'))
                await connection.execute(text(f'INSERT INTO "{schema}".session_requests (id) VALUES (\'legacy\')'))
                await migrate(connection, schema=schema)
                await migrate(connection, schema=schema)
                assert await connection.scalar(text(f'SELECT relay_max_backups FROM "{schema}".session_requests WHERE id = \'legacy\'')) == 1
                await connection.execute(text(f'INSERT INTO "{schema}".session_requests VALUES (\'single\', 0)'))
                assert await connection.scalar(text(f'SELECT relay_max_backups FROM "{schema}".session_requests WHERE id = \'single\'')) == 0
                savepoint = await connection.begin_nested()
                try:
                    with pytest.raises(IntegrityError):
                        await connection.execute(text(f'INSERT INTO "{schema}".session_requests VALUES (\'invalid\', 8)'))
                    # PostgreSQL marks the savepoint aborted after a constraint
                    # violation; roll it back before checking catalog drift.
                finally:
                    await savepoint.rollback()
            async with engine.begin() as connection:
                await connection.execute(text(f'ALTER TABLE "{schema}".session_requests ALTER COLUMN relay_max_backups SET DEFAULT 0'))
                with pytest.raises(RuntimeError, match="relay redundancy schema differs"):
                    await migrate(connection, schema=schema)
        finally:
            async with engine.begin() as connection:
                await connection.execute(text(f'DROP SCHEMA IF EXISTS "{schema}" CASCADE'))
            await engine.dispose()
    asyncio.run(check())


def test_redundancy_column_preserves_strict_legacy_default():
    engine = create_engine("sqlite:///:memory:")
    with engine.begin() as conn:
        conn.execute(text("CREATE TABLE session_requests (relay_max_backups INTEGER NOT NULL DEFAULT 1)"))
        verify_redundancy_column(conn, None)
    engine.dispose()
