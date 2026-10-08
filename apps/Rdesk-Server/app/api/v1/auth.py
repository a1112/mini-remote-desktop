import time
from collections import defaultdict, deque

from fastapi import APIRouter, Depends, HTTPException, Request, status
from sqlalchemy import or_, select
from sqlalchemy.exc import IntegrityError
from sqlalchemy.ext.asyncio import AsyncSession

from app.core.security import (
    create_access_token,
    hash_password,
    password_needs_rehash,
    verify_password,
)
from app.core.response_security import no_store_sensitive_response
from app.db.session import get_db
from app.models.user import User
from app.schemas.auth import LoginRequest, LoginResponse, RegisterRequest

router = APIRouter(
    prefix="/auth",
    tags=["auth"],
    dependencies=[Depends(no_store_sensitive_response)],
)

_ATTEMPT_WINDOW_SECONDS = 60.0
_ATTEMPT_LIMIT = 8
_ATTEMPT_BUCKET_LIMIT = 10_000
_attempts: dict[str, deque[float]] = defaultdict(deque)


def _allow_attempt(request: Request, username: str) -> None:
    """Apply a bounded source/account limit before PBKDF2 work begins."""

    now = time.monotonic()
    source = request.client.host if request.client else "unknown"
    keys = (f"ip:{source}", f"account:{username.strip().lower()}")
    for key in keys:
        bucket = _attempts[key]
        while bucket and now - bucket[0] >= _ATTEMPT_WINDOW_SECONDS:
            bucket.popleft()
        if len(bucket) >= _ATTEMPT_LIMIT:
            raise HTTPException(
                status_code=status.HTTP_429_TOO_MANY_REQUESTS,
                detail="Too many authentication attempts",
                headers={"Retry-After": "60"},
            )
    for key in keys:
        _attempts[key].append(now)
    if len(_attempts) > _ATTEMPT_BUCKET_LIMIT:
        oldest = min(_attempts, key=lambda key: _attempts[key][-1])
        _attempts.pop(oldest, None)


@router.post("/register", response_model=LoginResponse)
async def register(
    payload: RegisterRequest,
    request: Request,
    db: AsyncSession = Depends(get_db),
) -> LoginResponse:
    _allow_attempt(request, payload.username)
    username = payload.username.strip()
    email = payload.email.strip().lower()
    password = payload.password

    if len(username) < 3:
        raise HTTPException(
            status_code=status.HTTP_400_BAD_REQUEST,
            detail="Username must be at least 3 characters",
        )
    if "@" not in email or "." not in email:
        raise HTTPException(
            status_code=status.HTTP_400_BAD_REQUEST,
            detail="Invalid email",
        )
    if len(password) < 8:
        raise HTTPException(
            status_code=status.HTTP_400_BAD_REQUEST,
            detail="Password must be at least 8 characters",
        )

    existed = await db.scalar(
        select(User).where(or_(User.username == username, User.email == email))
    )
    if existed:
        # Do not let callers distinguish which unique identifier is present.
        raise HTTPException(
            status_code=status.HTTP_409_CONFLICT,
            detail="Registration could not be completed",
        )

    user = User(
        username=username,
        email=email,
        password_hash=hash_password(password),
        role="user",
    )
    db.add(user)
    try:
        await db.commit()
    except IntegrityError:
        # A concurrent registration can win between the lookup and commit;
        # keep that race generic as well.
        await db.rollback()
        raise HTTPException(
            status_code=status.HTTP_409_CONFLICT,
            detail="Registration could not be completed",
        ) from None
    await db.refresh(user)

    token = create_access_token(
        user.id, user.username, user.role, user.session_version
    )
    return LoginResponse(
        access_token=token,
        user_id=user.id,
        username=user.username,
        role=user.role,
    )


@router.post("/login", response_model=LoginResponse)
async def login(
    payload: LoginRequest,
    request: Request,
    db: AsyncSession = Depends(get_db),
) -> LoginResponse:
    # See the registration route for the same pre-hash request bound. The
    # caller's source is included in the key so account and IP limits compose.
    _allow_attempt(request, payload.username)
    user = await db.scalar(select(User).where(User.username == payload.username))
    if not user or not verify_password(payload.password, user.password_hash):
        raise HTTPException(
            status_code=status.HTTP_401_UNAUTHORIZED,
            detail="Invalid username or password",
        )
    token = create_access_token(
        user.id, user.username, user.role, user.session_version
    )
    if password_needs_rehash(user.password_hash):
        user.password_hash = hash_password(payload.password)
        await db.commit()
    return LoginResponse(
        access_token=token,
        user_id=user.id,
        username=user.username,
        role=user.role,
    )
