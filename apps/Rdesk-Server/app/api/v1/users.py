import os
import uuid
from datetime import datetime
from pathlib import Path
from typing import Optional

from fastapi import APIRouter, Depends, HTTPException, UploadFile, File, Form
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

# Base URL for serving uploaded files
BASE_URL = os.getenv("RDESK_BASE_URL", "http://127.0.0.1:9530")


def get_avatar_url(filename: str) -> str:
    return f"{BASE_URL}/api/v1/users/avatar/{filename}"


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
    await db.commit()

    return {"message": "Password changed successfully"}


@router.post("/me/avatar", response_model=AvatarUploadResponse)
async def upload_avatar(
    file: UploadFile = File(...),
    current_user: User = Depends(get_current_user),
    db: AsyncSession = Depends(get_db),
) -> AvatarUploadResponse:
    # Ensure upload directory exists
    UPLOAD_DIR.mkdir(parents=True, exist_ok=True)

    # Validate file type
    if not file.content_type or not file.content_type.startswith("image/"):
        raise HTTPException(
            status_code=400,
            detail="File must be an image"
        )

    # Validate file size (max 5MB)
    MAX_FILE_SIZE = 5 * 1024 * 1024
    contents = await file.read()
    if len(contents) > MAX_FILE_SIZE:
        raise HTTPException(
            status_code=400,
            detail="File size must be less than 5MB"
        )

    # Generate unique filename
    ext = Path(file.filename).suffix or ".jpg"
    filename = f"{current_user.id}_{uuid.uuid4().hex[:8]}{ext}"
    file_path = UPLOAD_DIR / filename

    # Save file
    with open(file_path, "wb") as f:
        f.write(contents)

    # Update user avatar URL
    avatar_url = get_avatar_url(filename)
    current_user.avatar_url = avatar_url
    await db.commit()

    return AvatarUploadResponse(avatar_url=avatar_url)


@router.get("/avatar/{filename}")
async def get_avatar(filename: str):
    file_path = UPLOAD_DIR / filename

    if not file_path.exists():
        raise HTTPException(status_code=404, detail="Avatar not found")

    return FileResponse(
        file_path,
        media_type="image/jpeg",
        headers={"Cache-Control": "public, max-age=31536000"}
    )


@router.delete("/me/avatar")
async def delete_avatar(
    current_user: User = Depends(get_current_user),
    db: AsyncSession = Depends(get_db),
):
    if current_user.avatar_url:
        # Extract filename from URL
        filename = current_user.avatar_url.split("/")[-1]
        file_path = UPLOAD_DIR / filename

        # Delete file if exists
        if file_path.exists():
            file_path.unlink()

        # Clear avatar URL
        current_user.avatar_url = None
        await db.commit()

    return {"message": "Avatar deleted successfully"}
