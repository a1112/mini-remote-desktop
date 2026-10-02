from datetime import datetime, timezone
import re
import unicodedata

from fastapi import HTTPException, status
import jwt

from app.core.config import settings
from app.core.security import _configured_device_jwt
from app.models.device import Device
from app.schemas.realtime import SignalingCredentialRequest, SignalingCredentialResponse


def _valid_sidecar_identifier(value: str) -> bool:
    return (
        bool(value)
        and len(value.encode("utf-8")) <= 256
        and not any(character.isspace() or unicodedata.category(character) == "Cc" for character in value)
    )


def issue_signaling_credential(
    device: Device, request: SignalingCredentialRequest
) -> SignalingCredentialResponse:
    """Exchange authenticated device identity for one signed signaling role/key."""
    configured = _configured_device_jwt()
    audience = settings.signaling_jwt_audience.strip()
    issuer = settings.jwt_issuer.strip()
    lifetime = settings.signaling_jwt_ttl_seconds
    if (
        configured is None
        or not _valid_sidecar_identifier(audience)
        or not _valid_sidecar_identifier(issuer)
        or audience in {settings.jwt_audience.strip(), settings.device_jwt_audience.strip()}
        or not isinstance(lifetime, int)
        or isinstance(lifetime, bool)
        or not 1 <= lifetime <= 3600
        or re.fullmatch(r"[A-Za-z0-9_-]{1,64}", device.device_id, re.ASCII) is None
    ):
        raise HTTPException(
            status_code=status.HTTP_503_SERVICE_UNAVAILABLE,
            detail={"code": "signaling_unavailable", "message": "signaling authentication is not configured"},
        )
    secret, issuer, _, _ = configured
    issued_at = int(datetime.now(timezone.utc).timestamp())
    expires_at = issued_at + lifetime
    claims = {
        "sub": device.device_id,
        "device_id": device.device_id,
        "device_key_id": request.device_key_id,
        "role": request.role,
        "token_type": "signaling",
        "iss": issuer,
        "aud": audience,
        "iat": issued_at,
        "exp": expires_at,
    }
    return SignalingCredentialResponse(
        token=jwt.encode(claims, secret, algorithm="HS256"),
        expires_at_ms=expires_at * 1000,
        device_id=device.device_id,
        device_key_id=request.device_key_id,
        role=request.role,
    )
