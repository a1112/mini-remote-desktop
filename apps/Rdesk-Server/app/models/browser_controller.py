from datetime import datetime

from sqlalchemy import BigInteger, CheckConstraint, DateTime, ForeignKey, Integer, JSON, LargeBinary, String, text
from sqlalchemy.dialects.postgresql import JSONB
from sqlalchemy.orm import Mapped, mapped_column

from app.db.session import Base


class BrowserController(Base):
    """One expiring browser key, represented by a non-physical device FK."""

    __tablename__ = "browser_controllers"
    __table_args__ = (
        CheckConstraint("user_session_version >= 1", name="ck_browser_user_version"),
        CheckConstraint("(authority_kind = 'account' AND user_id IS NOT NULL AND user_session_version IS NOT NULL AND user_session_version >= 1 AND temporary_access_generation IS NULL AND target_auth_version IS NULL) OR (authority_kind = 'temporary_password' AND user_id IS NULL AND user_session_version IS NULL AND temporary_access_generation IS NOT NULL AND target_auth_version IS NOT NULL AND temporary_access_generation >= 1 AND target_auth_version >= 1)", name="ck_browser_authority"),
        CheckConstraint("length(public_key) = 32", name="ck_browser_public_key"),
        CheckConstraint("length(key_id) = 64", name="ck_browser_key_id"),
        CheckConstraint("expires_at > created_at", name="ck_browser_lifetime"),
    )

    device_row_id: Mapped[str] = mapped_column(
        String(36), ForeignKey("devices.id", ondelete="CASCADE"), primary_key=True)
    session_id: Mapped[str] = mapped_column(
        String(36), ForeignKey("session_requests.id", ondelete="CASCADE"), unique=True)
    user_id: Mapped[str | None] = mapped_column(String(36), ForeignKey("users.id", ondelete="CASCADE"), nullable=True, index=True)
    tenant_id: Mapped[str] = mapped_column(String(64))
    user_session_version: Mapped[int | None] = mapped_column(Integer, nullable=True)
    authority_kind: Mapped[str] = mapped_column(String(24), nullable=False, default="account", server_default=text("'account'"))
    temporary_access_generation: Mapped[int | None] = mapped_column(BigInteger, nullable=True)
    target_auth_version: Mapped[int | None] = mapped_column(BigInteger, nullable=True)
    target_device_row_id: Mapped[str] = mapped_column(
        String(36), ForeignKey("devices.id", ondelete="CASCADE"))
    public_key: Mapped[bytes] = mapped_column(LargeBinary(32))
    key_id: Mapped[str] = mapped_column(String(64))
    allowed_scopes: Mapped[list[str]] = mapped_column(JSON().with_variant(JSONB(), "postgresql"))
    created_at: Mapped[datetime] = mapped_column(DateTime(timezone=True))
    expires_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), index=True)
    revoked_at: Mapped[datetime | None] = mapped_column(DateTime(timezone=True), nullable=True)
