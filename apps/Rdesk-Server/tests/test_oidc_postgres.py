"""Optional isolated PostgreSQL migration and single-use concurrency checks.

Set MRD_OIDC_TEST_SOCKET to an isolated fixture's Unix socket directory and
MRD_OIDC_TEST_PORT to its port. Never point this at a production database.
"""
import asyncio
import os
from datetime import UTC, datetime, timedelta
from urllib.parse import parse_qs, urlsplit
from uuid import uuid4

import pytest
from cryptography.fernet import Fernet
from pydantic import SecretStr
from sqlalchemy import func, select, text
from sqlalchemy.ext.asyncio import async_sessionmaker, create_async_engine

import app.models  # noqa: F401
from app.core.config import settings
from app.db.migrate_add_oidc import OidcMigrationError, migrate
from app.models.oidc import OidcIdentity, OidcLoginTransaction
from app.models.user import User
from app.services import oidc


@pytest.mark.asyncio
async def test_migration_identity_race_and_single_use_completion(monkeypatch):
    socket = os.environ.get("MRD_OIDC_TEST_SOCKET")
    if not socket:
        pytest.skip("requires an isolated PostgreSQL fixture socket")
    port = int(os.environ.get("MRD_OIDC_TEST_PORT", "59441"))
    schema = "oidc_test_" + uuid4().hex
    base = create_async_engine("postgresql+asyncpg://oidc_fixture@/postgres", connect_args={"host": socket, "port": port})
    async with base.begin() as connection:
        await connection.execute(text(f'CREATE SCHEMA "{schema}"'))
    engine = create_async_engine("postgresql+asyncpg://oidc_fixture@/postgres", connect_args={
        "host": socket, "port": port, "server_settings": {"search_path": schema}})
    try:
        async with engine.begin() as connection:
            await connection.run_sync(lambda sync: User.__table__.create(sync, checkfirst=False))
            await migrate(connection, schema=schema)
            await migrate(connection, schema=schema)
        monkeypatch.setattr(settings, "oidc_issuer", "https://identity.example.test/realms/test")
        monkeypatch.setattr(settings, "oidc_client_id", "test")
        monkeypatch.setattr(settings, "oidc_redirect_uri", "https://rdesk.example.test/api/v1/auth/oidc/callback")
        monkeypatch.setattr(settings, "jwt_secret", "super-secret-JWT-for-tests-12345!@#$")
        monkeypatch.setattr(settings, "jwt_issuer", "https://rdesk.example.test")
        monkeypatch.setattr(settings, "jwt_audience", "rdesk-api")
        monkeypatch.setattr(settings, "jwt_expire_minutes", 30)
        monkeypatch.setattr(settings, "jwt_max_lifetime_minutes", 60)
        sessions = async_sessionmaker(engine, expire_on_commit=False)

        async def login_identity():
            async with sessions() as db:
                user = await oidc._identity_user(db, "same-subject", None, None)
                await db.commit()
                return user.id

        first, second = await asyncio.gather(login_identity(), login_identity())
        assert first == second
        async with sessions() as db:
            assert await db.scalar(select(func.count()).select_from(User)) == 1
            assert await db.scalar(select(func.count()).select_from(OidcIdentity)) == 1
            completion = "a" * 43
            db.add(OidcLoginTransaction(completion_digest=oidc._digest(completion),
                state_digest="1" * 64, verifier_ciphertext="", nonce="b" * 43,
                issuer=settings.oidc_issuer, client_id="test", redirect_uri="https://rdesk.example.test/api/v1/auth/oidc/callback",
                require_response_issuer=True, status="complete", user_id=first,
                expires_at=datetime.now(UTC) + timedelta(minutes=1)))
            await db.commit()

        async def redeem():
            async with sessions() as db:
                try:
                    return await oidc.complete(db, completion)
                except oidc.OidcFailure:
                    return None

        results = await asyncio.gather(redeem(), redeem())
        assert sum(result is not None for result in results) == 1
        assert next(result for result in results if result is not None).user_id == first

        monkeypatch.setattr(settings, "oidc_client_id", "test-client")
        monkeypatch.setattr(settings, "oidc_redirect_uri", "https://rdesk.example.test/api/v1/auth/oidc/callback")
        monkeypatch.setattr(settings, "oidc_transaction_key", SecretStr(Fernet.generate_key().decode()))
        async def discover():
            return oidc.Provider("https://identity.example.test/auth", "https://identity.example.test/token",
                                 "https://identity.example.test/certs", True)
        async def verified_subject(_provider, _code, _verifier, _nonce):
            return "postgres-callback-subject"
        monkeypatch.setattr(oidc, "_discover", discover)
        monkeypatch.setattr(oidc, "_verified_subject", verified_subject)
        async with sessions() as db:
            started = await oidc.start(db)
            state = parse_qs(urlsplit(started["authorization_url"]).query)["state"][0]
            await oidc.callback(db, state=state, code="fixture-code",
                                response_issuer=settings.oidc_issuer, provider_error=False)
            login = await oidc.complete(db, started["completion_token"])
            assert login is not None and login.user_id != first
        async with sessions() as db:
            local = User(id=str(uuid4()), username="local", email="local@example.test", password_hash="!", role="user")
            db.add(local)
            await db.commit()
            local_id = local.id
            started = await oidc.start(db, local)
            state = parse_qs(urlsplit(started["authorization_url"]).query)["state"][0]
            monkeypatch.setattr(oidc, "_verified_subject", lambda *_args: _subject("postgres-link-subject"))
            await oidc.callback(db, state=state, code="fixture-code",
                                response_issuer=settings.oidc_issuer, provider_error=False)
            assert await db.scalar(select(OidcIdentity).where(OidcIdentity.subject == "postgres-link-subject")) is None
            with pytest.raises(oidc.OidcFailure):
                await oidc.complete(db, started["completion_token"])
        async def redeem_link():
            async with sessions() as db:
                local = await db.scalar(select(User).where(User.id == local_id))
                try:
                    return await oidc.complete(db, started["completion_token"], local)
                except oidc.OidcFailure:
                    return None
        link_results = await asyncio.gather(redeem_link(), redeem_link())
        assert sum(result is not None for result in link_results) == 1
        assert next(result for result in link_results if result is not None).user_id == local_id
        async with sessions() as db:
            assert (await db.scalar(select(OidcIdentity).where(
                OidcIdentity.subject == "postgres-link-subject"))).user_id == local_id
        async with engine.begin() as connection:
            await connection.execute(text("ALTER TABLE oidc_login_transactions ADD COLUMN unexpected TEXT"))
            with pytest.raises(OidcMigrationError):
                await migrate(connection, schema=schema)
    finally:
        await engine.dispose()
        async with base.begin() as connection:
            await connection.execute(text(f'DROP SCHEMA "{schema}" CASCADE'))
        await base.dispose()


async def _subject(value):
    return value
