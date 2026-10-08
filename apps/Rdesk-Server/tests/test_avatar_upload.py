"""Avatar replacement keeps the database row and files in lockstep."""

import asyncio
from io import BytesIO
from types import SimpleNamespace

from fastapi import UploadFile
from sqlalchemy import create_engine, select
from sqlalchemy.orm import Session
from sqlalchemy.pool import StaticPool

from app.api.v1 import users as user_routes
from app.db.session import Base
from app.models.user import User
from test_relay_node_api import AsyncSessionShim


def _upload(name: str, payload: bytes) -> UploadFile:
    return UploadFile(
        file=BytesIO(payload),
        filename=name,
        headers={"content-type": "image/png"},
    )


def test_concurrent_avatar_replacements_remove_each_superseded_file(
    tmp_path, monkeypatch
):
    upload_root = tmp_path / "avatars"
    monkeypatch.setattr(user_routes, "UPLOAD_DIR", upload_root)
    monkeypatch.setattr(user_routes, "BASE_URL", "http://avatars.test")

    engine = create_engine(
        "sqlite:///:memory:",
        connect_args={"check_same_thread": False},
        poolclass=StaticPool,
    )
    Base.metadata.create_all(engine)
    session = Session(engine, expire_on_commit=False)
    user = User(
        id="avatar-race-user",
        username="avatar-race",
        email="avatar-race@example.test",
        password_hash="unused",
        tenant_id="tenant-a",
    )
    old_name = "11111111-1111-1111-1111-111111111111_11111111.png"
    user.avatar_url = user_routes.get_avatar_url(old_name)
    session.add(user)
    session.commit()
    upload_root.mkdir(parents=True)
    (upload_root / old_name).write_bytes(b"old")
    db = AsyncSessionShim(session)

    # Both requests intentionally hold the stale dependency snapshot. The
    # route must re-read the current row after acquiring the per-user lock.
    first_user = SimpleNamespace(id=user.id, avatar_url=user.avatar_url)
    second_user = SimpleNamespace(id=user.id, avatar_url=user.avatar_url)

    async def replace() -> None:
        await asyncio.gather(
            user_routes.upload_avatar(
                _upload("first.png", b"first"), current_user=first_user, db=db
            ),
            user_routes.upload_avatar(
                _upload("second.png", b"second"), current_user=second_user, db=db
            ),
        )

    asyncio.run(replace())
    current = session.scalar(select(User).where(User.id == user.id))
    assert current is not None
    current_name = user_routes._avatar_filename_from_url(current.avatar_url)
    assert current_name is not None
    files = sorted(path.name for path in upload_root.iterdir())
    assert files == [current_name]
    assert (upload_root / current_name).read_bytes() in {b"first", b"second"}

    session.close()
    engine.dispose()
