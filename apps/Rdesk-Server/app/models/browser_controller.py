from datetime import datetime

from sqlalchemy import CheckConstraint, DateTime, ForeignKey, Integer, JSON, LargeBinary, String
from sqlalchemy.dialects.postgresql import JSONB
from sqlalchemy.orm import Mapped, mapped_column

from app.db.session import Base


class BrowserController(Base):
    """One expiring browser key, represented by a non-physical device FK."""

    __tablename__ = "browser_controllers"
    __table_args__ = (
        CheckConstraint("user_session_version >= 1", name="ck_browser_user_version"),
        CheckConstraint("length(public_key) = 32", name="ck_browser_public_key"),
        CheckConstraint("length(key_id) = 64", name="ck_browser_key_id"),
        CheckConstraint("expires_at > created_at", name="ck_browser_lifetime"),
    )

    device_row_id: Mapped[str] = mapped_column(
        String(36), ForeignKey("devices.id", ondelete="CASCADE"), primary_key=True)
    session_id: Mapped[str] = mapped_column(
        String(36), ForeignKey("session_requests.id", ondelete="CASCADE"), unique=True)
    user_id: Mapped[str] = mapped_column(String(36), ForeignKey("users.id", ondelete="CASCADE"), index=True)
    tenant_id: Mapped[str] = mapped_column(String(64))
    user_session_version: Mapped[int] = mapped_column(Integer)
    target_device_row_id: Mapped[str] = mapped_column(
        String(36), ForeignKey("devices.id", ondelete="CASCADE"))
    public_key: Mapped[bytes] = mapped_column(LargeBinary(32))
    key_id: Mapped[str] = mapped_column(String(64))
    allowed_scopes: Mapped[list[str]] = mapped_column(JSON().with_variant(JSONB(), "postgresql"))
    created_at: Mapped[datetime] = mapped_column(DateTime(timezone=True))
    expires_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), index=True)
    revoked_at: Mapped[datetime | None] = mapped_column(DateTime(timezone=True), nullable=True)
