"""Public machine keys and bounded, one-use self-enrollment challenges."""
from datetime import datetime

from sqlalchemy import CheckConstraint, DateTime, ForeignKey, Index, String
from sqlalchemy.orm import Mapped, mapped_column

from app.db.session import Base


class DeviceMachineIdentity(Base):
    __tablename__ = "device_machine_identities"
    __table_args__ = (
        CheckConstraint("length(key_id) = 64", name="ck_device_machine_key_id"),
        CheckConstraint("length(public_key) = 64", name="ck_device_machine_public_key"),
    )

    key_id: Mapped[str] = mapped_column(String(64), primary_key=True)
    public_key: Mapped[str] = mapped_column(String(64), nullable=False)
    device_row_id: Mapped[str] = mapped_column(
        String(36), ForeignKey("devices.id", ondelete="RESTRICT"), nullable=False, unique=True
    )
    created_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), nullable=False)


class DeviceSelfEnrollmentChallenge(Base):
    __tablename__ = "device_self_enrollment_challenges"
    __table_args__ = (
        CheckConstraint("length(challenge_id) = 32", name="ck_device_self_challenge_id"),
        CheckConstraint("length(nonce_digest) = 64", name="ck_device_self_nonce_digest"),
        CheckConstraint("length(key_id) = 64", name="ck_device_self_key_id"),
        CheckConstraint("length(public_key) = 64", name="ck_device_self_public_key"),
        CheckConstraint("length(peer_digest) = 64", name="ck_device_self_peer_digest"),
        CheckConstraint("expires_at > issued_at", name="ck_device_self_expiry"),
        Index("ix_device_self_issued_at", "issued_at"),
        Index("ix_device_self_peer_issued", "peer_digest", "issued_at"),
        Index("ix_device_self_key_issued", "key_id", "issued_at"),
        Index("ix_device_self_expiry", "expires_at"),
    )

    challenge_id: Mapped[str] = mapped_column(String(32), primary_key=True)
    nonce_digest: Mapped[str] = mapped_column(String(64), nullable=False)
    key_id: Mapped[str] = mapped_column(String(64), nullable=False)
    public_key: Mapped[str] = mapped_column(String(64), nullable=False)
    api_url: Mapped[str] = mapped_column(String(2048), nullable=False)
    peer_digest: Mapped[str] = mapped_column(String(64), nullable=False)
    issued_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), nullable=False)
    expires_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), nullable=False)
    consumed_at: Mapped[datetime | None] = mapped_column(DateTime(timezone=True), nullable=True)
