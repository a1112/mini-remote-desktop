from datetime import datetime
from uuid import uuid4
from sqlalchemy import (
    BigInteger,
    Boolean,
    CheckConstraint,
    DateTime,
    ForeignKey,
    JSON,
    LargeBinary,
    String,
)
from sqlalchemy.dialects.postgresql import JSONB
from sqlalchemy.orm import Mapped, mapped_column
from app.db.session import Base


class DeviceTemporaryAccess(Base):
    __tablename__ = "device_temporary_access"
    __table_args__ = (
        CheckConstraint(
            "generation >= 1 AND target_auth_version >= 1",
            name="ck_temporary_access_versions",
        ),
        CheckConstraint(
            "length(key_id) = 64 AND length(publication_digest) = 64",
            name="ck_temporary_access_identity",
        ),
        CheckConstraint(
            "(enabled = TRUE AND expires_at IS NOT NULL AND salt IS NOT NULL AND length(salt) = 16 AND verifier_hmac IS NOT NULL AND length(verifier_hmac) = 32) OR (enabled = FALSE AND expires_at IS NULL AND salt IS NULL AND verifier_hmac IS NULL)",
            name="ck_temporary_access_bundle",
        ),
    )
    device_row_id: Mapped[str] = mapped_column(
        String(36), ForeignKey("devices.id", ondelete="CASCADE"), primary_key=True
    )
    generation: Mapped[int] = mapped_column(BigInteger)
    target_auth_version: Mapped[int] = mapped_column(BigInteger)
    key_id: Mapped[str] = mapped_column(String(64))
    enabled: Mapped[bool] = mapped_column(Boolean)
    expires_at: Mapped[datetime | None] = mapped_column(
        DateTime(timezone=True), nullable=True
    )
    salt: Mapped[bytes | None] = mapped_column(LargeBinary(16), nullable=True)
    verifier_hmac: Mapped[bytes | None] = mapped_column(LargeBinary(32), nullable=True)
    publication_digest: Mapped[str] = mapped_column(String(64))
    allowed_scopes: Mapped[list[str]] = mapped_column(
        JSON().with_variant(JSONB(), "postgresql")
    )
    updated_at: Mapped[datetime] = mapped_column(DateTime(timezone=True))


class GuestAccessAttempt(Base):
    __tablename__ = "guest_access_attempts"
    id: Mapped[str] = mapped_column(
        String(36), primary_key=True, default=lambda: str(uuid4())
    )
    peer_digest: Mapped[str] = mapped_column(String(64), index=True)
    target_digest: Mapped[str] = mapped_column(String(64), index=True)
    issued_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), index=True)
