"""Identity mappings and short-lived, independently redeemable OIDC flows."""
from datetime import datetime

from sqlalchemy import Boolean, CheckConstraint, DateTime, ForeignKey, Index, Integer, String, Text
from sqlalchemy.orm import Mapped, mapped_column

from app.db.session import Base


class OidcIdentity(Base):
    __tablename__ = "oidc_identities"

    issuer: Mapped[str] = mapped_column(String(512), primary_key=True)
    subject: Mapped[str] = mapped_column(String(255), primary_key=True)
    user_id: Mapped[str] = mapped_column(
        String(36), ForeignKey("users.id", ondelete="RESTRICT"), unique=True, nullable=False
    )
    created_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), nullable=False)


class OidcLoginTransaction(Base):
    __tablename__ = "oidc_login_transactions"
    __table_args__ = (
        CheckConstraint("status IN ('pending', 'exchanging', 'complete', 'failed', 'consumed')", name="ck_oidc_flow_status"),
        Index("ix_oidc_flow_expires_at", "expires_at"),
    )

    completion_digest: Mapped[str] = mapped_column(String(64), primary_key=True)
    state_digest: Mapped[str] = mapped_column(String(64), unique=True, nullable=False)
    verifier_ciphertext: Mapped[str] = mapped_column(Text, nullable=False)
    nonce: Mapped[str] = mapped_column(String(43), nullable=False)
    issuer: Mapped[str] = mapped_column(String(512), nullable=False)
    client_id: Mapped[str] = mapped_column(String(256), nullable=False)
    redirect_uri: Mapped[str] = mapped_column(String(512), nullable=False)
    require_response_issuer: Mapped[bool] = mapped_column(Boolean, nullable=False)
    status: Mapped[str] = mapped_column(String(16), nullable=False)
    link_user_id: Mapped[str | None] = mapped_column(String(36), ForeignKey("users.id", ondelete="RESTRICT"))
    link_session_version: Mapped[int | None] = mapped_column(Integer)
    link_subject_ciphertext: Mapped[str | None] = mapped_column(Text)
    user_id: Mapped[str | None] = mapped_column(String(36), ForeignKey("users.id", ondelete="RESTRICT"))
    expires_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), nullable=False)
