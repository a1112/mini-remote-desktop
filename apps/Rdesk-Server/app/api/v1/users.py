import asyncio
import mimetypes
import os
import re
import uuid
from datetime import datetime
from pathlib import Path
from typing import Optional

from fastapi import APIRouter, Depends, HTTPException, UploadFile, File
from sqlalchemy import select
from sqlalchemy.ext.asyncio import AsyncSession
from fastapi.responses import FileResponse

from app.core.security import verify_password, hash_password, get_current_user
from app.db.session import get_db
from app.models.user import User
from app.schemas.user import (
    UserProfileResponse,
    UpdateProfileRequest,
    ChangePasswordRequest,
    AvatarUploadResponse,
)

router = APIRouter(prefix="/users", tags=["users"])

# Upload directory for avatars
UPLOAD_DIR = Path("uploads/avatars")
_AVATAR_NAME = re.compile(
    r"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}_[0-9a-f]{8}\.(?:jpg|jpeg|png|gif|webp)$",
    re.ASCII,
)
_AVATAR_EXTENSIONS = {".jpg", ".jpeg", ".png", ".gif", ".webp"}
_MAX_AVATAR_BYTES = 5 * 1024 * 1024
_AVATAR_CHUNK_BYTES = 64 * 1024
_avatar_locks: dict[str, asyncio.Lock] = {}
_avatar_overflow_lock = asyncio.Lock()

# Base URL for serving uploaded files
BASE_URL = os.getenv("RDESK_BASE_URL", "http://127.0.0.1:9530")


def get_avatar_url(filename: str) -> str:
    return f"{BASE_URL}/api/v1/users/avatar/{filename}"


def _avatar_root() -> Path:
    return UPLOAD_DIR.resolve()


def _owned_avatar_path(filename: str) -> Path | None:
    if not _AVATAR_NAME.fullmatch(filename):
        return None
    root = _avatar_root()
    path = (root / filename).resolve()
    return path if path.parent == root else None


def _avatar_filename_from_url(value: str | None) -> str | None:
    if not value:
        return None
    filename = value.rsplit("/", 1)[-1]
    return filename if _owned_avatar_path(filename) is not None else None


def _avatar_lock(user_id: str) -> asyncio.Lock:
    # User ids are server-generated and bounded by the user table. Avoid
    # unbounded per-request allocations if a caller supplies a large id in a
    # test or migration fixture.
    lock = _avatar_locks.get(user_id)
    if lock is not None:
        return lock
    if len(_avatar_locks) >= 10_000:
        return _avatar_overflow_lock
    lock = asyncio.Lock()
    _avatar_locks[user_id] = lock
    return lock


@router.get("/me", response_model=UserProfileResponse)
async def get_current_user_profile(
    current_user: User = Depends(get_current_user),
) -> UserProfileResponse:
    return UserProfileResponse(
        id=current_user.id,
        username=current_user.username,
        email=current_user.email,
        role=current_user.role,
        avatar_url=current_user.avatar_url,
    )


@router.put("/me", response_model=UserProfileResponse)
async def update_profile(
    payload: UpdateProfileRequest,
    current_user: User = Depends(get_current_user),
    db: AsyncSession = Depends(get_db),
) -> UserProfileResponse:
    # Check if username is taken by another user
    if payload.username != current_user.username:
        existing = await db.scalar(
            select(User).where(User.username == payload.username)
        )
        if existing:
            raise HTTPException(
                status_code=409,
                detail="Username already exists"
            )

    # Check if email is taken by another user
    if payload.email != current_user.email:
        existing = await db.scalar(
            select(User).where(User.email == payload.email)
        )
        if existing:
            raise HTTPException(
                status_code=409,
                detail="Email already exists"
            )

    # Update user
    current_user.username = payload.username
    current_user.email = payload.email
    await db.commit()
    await db.refresh(current_user)

    return UserProfileResponse(
        id=current_user.id,
        username=current_user.username,
        email=current_user.email,
        role=current_user.role,
        avatar_url=current_user.avatar_url,
    )


@router.post("/me/change-password")
async def change_password(
    payload: ChangePasswordRequest,
    current_user: User = Depends(get_current_user),
    db: AsyncSession = Depends(get_db),
):
    if not verify_password(payload.current_password, current_user.password_hash):
        raise HTTPException(
            status_code=401,
            detail="Current password is incorrect"
        )

    current_user.password_hash = hash_password(payload.new_password)
    current_user.session_version += 1
    await db.commit()

    return {"message": "Password changed successfully"}


@router.post("/me/avatar", response_model=AvatarUploadResponse)
async def upload_avatar(
    file: UploadFile = File(...),
    current_user: User = Depends(get_current_user),
    db: AsyncSession = Depends(get_db),
) -> AvatarUploadResponse:
    root = _avatar_root()
    root.mkdir(parents=True, exist_ok=True)

    # Validate file type
    if not file.content_type or not file.content_type.startswith("image/"):
        raise HTTPException(
            status_code=400,
            detail="File must be an image"
        )

    filename_hint = Path(file.filename or "").suffix.lower()
    ext = filename_hint if filename_hint in _AVATAR_EXTENSIONS else ".jpg"
    filename = f"{uuid.uuid4()}_{uuid.uuid4().hex[:8]}{ext}"
    file_path = _owned_avatar_path(filename)
    assert file_path is not None
    old_filename = _avatar_filename_from_url(current_user.avatar_url)
    old_path = _owned_avatar_path(old_filename) if old_filename else None
    avatar_url = get_avatar_url(filename)

    async with _avatar_lock(current_user.id):
        size = 0
        try:
            with file_path.open("wb") as output:
                while chunk := await file.read(_AVATAR_CHUNK_BYTES):
                    size += len(chunk)
                    if size > _MAX_AVATAR_BYTES:
                        raise HTTPException(
                            status_code=400,
                            detail="File size must be less than 5MB",
                        )
                    output.write(chunk)
            current_user.avatar_url = avatar_url
            await db.commit()
        except Exception:
            file_path.unlink(missing_ok=True)
            raise
        if old_path is not None and old_path != file_path:
            old_path.unlink(missing_ok=True)

    return AvatarUploadResponse(avatar_url=avatar_url)


@router.get("/avatar/{filename}")
async def get_avatar(filename: str):
    file_path = _owned_avatar_path(filename)
    if file_path is None:
        raise HTTPException(status_code=404, detail="Avatar not found")

    if not file_path.exists():
        raise HTTPException(status_code=404, detail="Avatar not found")

    return FileResponse(
        file_path,
        media_type=mimetypes.guess_type(file_path.name)[0] or "application/octet-stream",
        headers={"Cache-Control": "public, max-age=31536000"}
    )


@router.delete("/me/avatar")
async def delete_avatar(
    current_user: User = Depends(get_current_user),
    db: AsyncSession = Depends(get_db),
):
    if current_user.avatar_url:
        # Extract filename from URL
        filename = _avatar_filename_from_url(current_user.avatar_url)
        file_path = _owned_avatar_path(filename) if filename else None

        # Delete file if exists
        if file_path is not None:
            file_path.unlink(missing_ok=True)

        # Clear avatar URL
        current_user.avatar_url = None
        await db.commit()

    return {"message": "Avatar deleted successfully"}
