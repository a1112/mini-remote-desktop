from __future__ import annotations

import asyncio
import re
from contextlib import asynccontextmanager
from datetime import UTC, datetime
from types import SimpleNamespace
from uuid import uuid4

import pytest
from pydantic import SecretStr
from sqlalchemy import func, select, text
from sqlalchemy.exc import IntegrityError
from sqlalchemy.ext.asyncio import AsyncSession, async_sessionmaker, create_async_engine

from app.db.session import Base
from app.models.device import Device, generate_device_id_from_digest
from app.models.device_enrollment import DeviceEnrollment
from app.services import device_enrollment
from app.services.device_enrollment import DeviceEnrollmentError, DeviceEnrollmentService
from test_device_ownership import (
    DATABASE_URL,
    EnrollmentSessionShim,
    SERIAL_PEPPER,
    _asyncpg_url,
    _enrollment_headers,
    _issue_device_enrollment,
    _register_payload,
    _user,
    device_api,
)


def _use_savepoints(device_api: SimpleNamespace) -> None:
    from app.db.session import get_db

    async def override_db():
        yield EnrollmentSessionShim(device_api.session)

    device_api.app.dependency_overrides[get_db] = override_db


@pytest.mark.parametrize("digest", ["0" * 64, "f" * 64, "A" * 64])
def test_new_device_codes_are_nine_ascii_digits(digest: str) -> None:
    code = generate_device_id_from_digest(digest)
    assert re.fullmatch(r"[0-9]{9}", code, flags=re.ASCII)
    assert generate_device_id_from_digest(digest) == code


def test_device_code_preserves_leading_zeroes() -> None:
    assert generate_device_id_from_digest("0" * 64) == "000000000"


@pytest.mark.parametrize("digest", ["f" * 63, "g" * 64, "\u0660" * 64, None])
def test_device_code_rejects_invalid_serial_digest(digest: object) -> None:
    with pytest.raises(ValueError, match="device serial digest is invalid"):
        generate_device_id_from_digest(digest)


def test_enrollment_allocates_nine_digit_code_and_reuses_exact_retry(
    device_api: SimpleNamespace,
) -> None:
    _use_savepoints(device_api)
    token = _issue_device_enrollment(device_api)
    payload = _register_payload("new-nine-digit-device")
    first = device_api.client.post(
        "/api/v1/devices/register", json=payload, headers=_enrollment_headers(token)
    )
    retry = device_api.client.post(
        "/api/v1/devices/register", json=payload, headers=_enrollment_headers(token)
    )
    assert first.status_code == retry.status_code == 200
    assert re.fullmatch(r"[0-9]{9}", first.json()["device_id"], flags=re.ASCII)
    assert retry.json()["device_id"] == first.json()["device_id"]
    assert device_api.session.scalar(select(func.count()).select_from(Device)) == 2


@pytest.mark.parametrize("legacy_code", ["0123456789", "012345678901"])
def test_exact_enrollment_retry_keeps_persisted_legacy_code(
    device_api: SimpleNamespace, legacy_code: str,
) -> None:
    _use_savepoints(device_api)
    token = _issue_device_enrollment(device_api)
    payload = _register_payload("legacy-retry-device")
    first = device_api.client.post(
        "/api/v1/devices/register", json=payload, headers=_enrollment_headers(token)
    )
    assert first.status_code == 200, first.text
    device = device_api.session.scalar(select(Device).where(
        Device.device_id == first.json()["device_id"]
    ))
    device.device_id = legacy_code
    device_api.session.commit()
    retry = device_api.client.post(
        "/api/v1/devices/register", json=payload, headers=_enrollment_headers(token)
    )
    assert retry.status_code == 200, retry.text
    assert retry.json()["device_id"] == legacy_code


def test_database_code_collision_retries_with_numeric_code(
    device_api: SimpleNamespace, monkeypatch: pytest.MonkeyPatch,
) -> None:
    _use_savepoints(device_api)
    occupied = "012345678"
    device_api.device.device_id = occupied
    device_api.session.commit()
    monkeypatch.setattr(device_enrollment, "generate_device_id_from_digest", lambda _: occupied)
    monkeypatch.setattr(device_enrollment.secrets, "randbelow", lambda _: 42)
    token = _issue_device_enrollment(device_api)
    payload = _register_payload("code-collision-device")
    response = device_api.client.post(
        "/api/v1/devices/register", json=payload, headers=_enrollment_headers(token)
    )
    assert response.status_code == 200, response.text
    assert response.json()["device_id"] == "000000042"
    retry = device_api.client.post(
        "/api/v1/devices/register", json=payload, headers=_enrollment_headers(token)
    )
    assert retry.json()["device_id"] == "000000042"
    device_api.session.refresh(device_api.device)
    assert device_api.device.device_id == occupied


def test_exhausted_code_collisions_leave_enrollment_unconsumed(
    device_api: SimpleNamespace, monkeypatch: pytest.MonkeyPatch,
) -> None:
    _use_savepoints(device_api)
    occupied = "000000042"
    device_api.device.device_id = occupied
    device_api.session.commit()
    attempts: list[int] = []
    monkeypatch.setattr(device_enrollment, "generate_device_id_from_digest", lambda _: occupied)

    def collision_candidate(bound: int) -> int:
        attempts.append(bound)
        return 42

    monkeypatch.setattr(device_enrollment.secrets, "randbelow", collision_candidate)
    token = _issue_device_enrollment(device_api)
    response = device_api.client.post(
        "/api/v1/devices/register",
        json=_register_payload("exhausted-code-device"),
        headers=_enrollment_headers(token),
    )
    assert response.status_code == 503, response.text
    assert response.json()["detail"]["code"] == "device_code_unavailable"
    assert 1 <= len(attempts) <= 64
    assert set(attempts) == {10**9}
    enrollment = device_api.session.scalar(select(DeviceEnrollment))
    assert enrollment.consumed_at is None
    assert enrollment.registered_device_id is None
    assert device_api.session.scalar(select(func.count()).select_from(Device)) == 1


@pytest.mark.parametrize("legacy_code", ["821456789", "0123456789", "012345678901", "123456789012-abcd"])
def test_authenticated_refresh_keeps_legacy_device_code(
    device_api: SimpleNamespace, legacy_code: str,
) -> None:
    from app.core.security import create_device_access_token

    device_api.device.device_id = legacy_code
    device_api.session.commit()
    token = create_device_access_token(device_api.device)
    response = device_api.client.post(
        "/api/v1/devices/register", json=_register_payload("serial-a"),
        headers={"X-Rdesk-Device-Authorization": f"Bearer {token}"},
    )
    assert response.status_code == 200, response.text
    assert response.json()["device_id"] == legacy_code


def test_database_unique_constraint_forbids_duplicate_public_code(
    device_api: SimpleNamespace,
) -> None:
    device_api.session.add(Device(
        name="duplicate-public-code", device_id=device_api.device.device_id, os="Linux",
    ))
    with pytest.raises(IntegrityError):
        device_api.session.flush()
    device_api.session.rollback()


def test_other_insert_constraint_errors_are_not_code_collisions(
    device_api: SimpleNamespace, monkeypatch: pytest.MonkeyPatch,
) -> None:
    token = _issue_device_enrollment(device_api)
    attempts: list[int] = []
    monkeypatch.setattr(device_enrollment.secrets, "randbelow", lambda bound: attempts.append(bound) or 42)
    registration = {**_register_payload("invalid-device-name"), "hostname": None, "device_name": None}

    async def register() -> None:
        await DeviceEnrollmentService(
            EnrollmentSessionShim(device_api.session),
            token_pepper=bytes.fromhex("66" * 32), serial_pepper=SERIAL_PEPPER,
            ttl_seconds=300,
        ).register(token=SecretStr(token), registration=registration)

    with pytest.raises(IntegrityError):
        asyncio.run(register())
    assert attempts == []
    device_api.session.rollback()


def test_serial_unique_constraint_never_retries_or_changes_existing_device(
    device_api: SimpleNamespace, monkeypatch: pytest.MonkeyPatch,
) -> None:
    from app.db.session import get_db

    attempts: list[int] = []
    monkeypatch.setattr(
        device_enrollment, "generate_device_id_from_digest", lambda _: "000000042"
    )
    monkeypatch.setattr(
        device_enrollment.secrets, "randbelow", lambda bound: attempts.append(bound) or 43
    )

    class ConcurrentSerialSessionShim(EnrollmentSessionShim):
        async def scalar(self, statement, *args, **kwargs):
            # Model a registration lookup immediately preceding a competing
            # commit, so the real unique constraint is the ownership backstop.
            condition = getattr(statement, "whereclause", None)
            left = getattr(condition, "left", None)
            if getattr(left, "name", None) == "motherboard_serial_digest":
                return None
            return await super().scalar(statement, *args, **kwargs)

    async def override_db():
        yield ConcurrentSerialSessionShim(device_api.session)

    token = _issue_device_enrollment(device_api)
    device_api.app.dependency_overrides[get_db] = override_db
    response = device_api.client.post(
        "/api/v1/devices/register", json=_register_payload("serial-a"),
        headers=_enrollment_headers(token),
    )
    assert response.status_code == 409, response.text
    assert response.json()["detail"]["code"] == "device_enrollment_conflict"
    assert attempts == []
    device_api.session.refresh(device_api.device)
    assert device_api.device.device_id == "100000000001"
    assert device_api.session.scalar(select(func.count()).select_from(Device)) == 1
    enrollment = device_api.session.scalar(select(DeviceEnrollment))
    assert enrollment.consumed_at is None


@asynccontextmanager
async def _postgres_sessions(*, session_class: type[AsyncSession] = AsyncSession):
    assert DATABASE_URL is not None
    schema = "device_codes_" + uuid4().hex
    admin_engine = create_async_engine(_asyncpg_url(DATABASE_URL))
    async with admin_engine.begin() as connection:
        await connection.execute(text(f'CREATE SCHEMA "{schema}"'))
    engine = create_async_engine(
        _asyncpg_url(DATABASE_URL),
        connect_args={"server_settings": {"search_path": schema}},
    )
    sessions = async_sessionmaker(engine, class_=session_class, expire_on_commit=False)
    try:
        async with engine.begin() as connection:
            await connection.run_sync(Base.metadata.create_all)
        async with sessions.begin() as setup:
            setup.add(_user("code-test-admin", "default", role="admin"))
        yield sessions
    finally:
        await engine.dispose()
        async with admin_engine.begin() as connection:
            await connection.execute(text(f'DROP SCHEMA "{schema}" CASCADE'))
        await admin_engine.dispose()


@pytest.mark.skipif(not DATABASE_URL, reason="MRD_TEST_DATABASE_URL is not configured")
@pytest.mark.anyio
async def test_concurrent_different_devices_with_one_candidate_get_unique_codes(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    both_inserting = asyncio.Event()
    initial_inserts = 0
    occupied_candidate = "000000042"

    class RacingSession(AsyncSession):
        async def flush(self, objects=None) -> None:
            nonlocal initial_inserts
            if any(
                isinstance(row, Device) and row.device_id == occupied_candidate
                for row in self.new
            ):
                initial_inserts += 1
                if initial_inserts == 2:
                    both_inserting.set()
                await asyncio.wait_for(both_inserting.wait(), timeout=5)
            await super().flush(objects)

    monkeypatch.setattr(
        device_enrollment, "generate_device_id_from_digest", lambda _: occupied_candidate
    )
    monkeypatch.setattr(device_enrollment.secrets, "randbelow", lambda _: 43)
    now = datetime(2026, 10, 5, 0, 0, tzinfo=UTC)
    tokens = ("A" * 43, "B" * 43)

    def service(session: AsyncSession, raw_token: str | None = None):
        kwargs = {"token_source": lambda _: raw_token} if raw_token else {}
        return DeviceEnrollmentService(
            session, token_pepper=bytes.fromhex("66" * 32),
            serial_pepper=SERIAL_PEPPER, ttl_seconds=300, now=lambda: now, **kwargs,
        )

    async with _postgres_sessions(session_class=RacingSession) as sessions:
        for raw_token in tokens:
            async with sessions.begin() as setup:
                await service(setup, raw_token).issue(admin_user_id="code-test-admin")

        async def consume(index: int):
            async with sessions.begin() as session:
                result = await service(session).register(
                    token=SecretStr(tokens[index]),
                    registration=_register_payload(f"different-physical-device-{index}"),
                )
                return result.device.id, result.device.device_id, result.recovered

        results = await asyncio.wait_for(
            asyncio.gather(consume(0), consume(1)), timeout=10,
        )
        assert initial_inserts == 2
        assert {result[1] for result in results} == {"000000042", "000000043"}
        assert len({result[0] for result in results}) == 2
        assert not any(result[2] for result in results)
        for index in range(2):
            retry = await consume(index)
            assert retry[:2] == results[index][:2]
            assert retry[2] is True
        async with sessions() as verification:
            assert await verification.scalar(select(func.count()).select_from(Device)) == 2
            assert await verification.scalar(
                select(func.count()).select_from(DeviceEnrollment)
                .where(DeviceEnrollment.consumed_at.is_not(None))
            ) == 2


@pytest.mark.skipif(not DATABASE_URL, reason="MRD_TEST_DATABASE_URL is not configured")
@pytest.mark.anyio
async def test_postgres_exhausted_collisions_preserve_unconsumed_enrollment(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    occupied_candidate = "000000042"
    monkeypatch.setattr(
        device_enrollment, "generate_device_id_from_digest", lambda _: occupied_candidate
    )
    monkeypatch.setattr(device_enrollment.secrets, "randbelow", lambda _: 42)
    now = datetime(2026, 10, 5, 0, 0, tzinfo=UTC)
    raw_token = "A" * 43

    def service(session: AsyncSession):
        return DeviceEnrollmentService(
            session, token_pepper=bytes.fromhex("66" * 32),
            serial_pepper=SERIAL_PEPPER, ttl_seconds=300, now=lambda: now,
            token_source=lambda _: raw_token,
        )

    async with _postgres_sessions() as sessions:
        async with sessions.begin() as setup:
            setup.add(Device(name="occupied", device_id=occupied_candidate, os="Linux"))
            await service(setup).issue(admin_user_id="code-test-admin")
        async with sessions() as session:
            with pytest.raises(DeviceEnrollmentError) as failure:
                await service(session).register(
                    token=SecretStr(raw_token), registration=_register_payload("exhaustion"),
                )
            assert failure.value.code == "device_code_unavailable"
            assert failure.value.status_code == 503
            # A failed candidate rolls back only its savepoint. Even before the
            # request transaction rollback, the session and token remain usable.
            assert await session.scalar(select(func.count()).select_from(Device)) == 1
            enrollment = await session.scalar(select(DeviceEnrollment))
            assert enrollment.consumed_at is None
            await session.rollback()
        async with sessions() as verification:
            enrollment = await verification.scalar(select(DeviceEnrollment))
            assert enrollment.consumed_at is None
            assert enrollment.registered_device_id is None
