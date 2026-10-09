"""Anonymous code allocation proves only a machine key, never screen consent."""
from fastapi import APIRouter, Depends, HTTPException, Request
from sqlalchemy.ext.asyncio import AsyncSession

from app.core.config import settings
from app.core.response_security import no_store_sensitive_response
from app.core.security import create_device_access_token, create_device_refresh_token
from app.db.session import get_db
from app.schemas.device import DeviceRegisterResponse
from app.schemas.device_self_enrollment import (
    DeviceSelfEnrollmentChallengeRequest, DeviceSelfEnrollmentChallengeResponse,
    DeviceSelfRegisterRequest,
)
from app.services.device_enrollment import DeviceEnrollmentError
from app.services.device_self_enrollment import DeviceSelfEnrollmentService, enrollment_peer_ip

router = APIRouter(prefix="/devices", tags=["devices"], dependencies=[Depends(no_store_sensitive_response)])


def _service(db: AsyncSession) -> DeviceSelfEnrollmentService:
    if settings.device_self_enrollment_enabled is not True:
        raise HTTPException(503, detail={"code": "device_self_enrollment_disabled"})
    try:
        return DeviceSelfEnrollmentService(
            db, api_url=settings.public_api_url,
            serial_pepper=bytes.fromhex(settings.device_serial_pepper.get_secret_value()),
            ttl_seconds=settings.device_self_enrollment_ttl_seconds,
            global_per_minute=settings.device_self_enrollment_global_per_minute,
            ip_per_minute=settings.device_self_enrollment_ip_per_minute,
            key_per_minute=settings.device_self_enrollment_key_per_minute,
        )
    except (AttributeError, TypeError, ValueError):
        raise HTTPException(503, detail={"code": "device_self_enrollment_unavailable"}) from None


def _raise(error: DeviceEnrollmentError) -> None:
    headers = {"Retry-After": "60"} if error.status_code == 429 else None
    raise HTTPException(error.status_code, detail={"code": error.code, "message": str(error)}, headers=headers)


@router.post("/self-enrollment-challenge", response_model=DeviceSelfEnrollmentChallengeResponse)
async def issue_self_enrollment_challenge(
    payload: DeviceSelfEnrollmentChallengeRequest, request: Request,
    db: AsyncSession = Depends(get_db),
) -> DeviceSelfEnrollmentChallengeResponse:
    service = _service(db)
    try:
        peer = enrollment_peer_ip(request, settings.device_self_enrollment_trusted_proxies)
        result = await service.issue(payload, peer_ip=peer)
        await db.commit()
        return result
    except DeviceEnrollmentError as error:
        await db.rollback()
        _raise(error)
    except Exception:
        await db.rollback()
        raise


@router.post("/self-register", response_model=DeviceRegisterResponse)
async def self_register_device(
    payload: DeviceSelfRegisterRequest, db: AsyncSession = Depends(get_db),
) -> DeviceRegisterResponse:
    service = _service(db)
    try:
        device = await service.register(payload)
        # Mint before commit: unavailable JWT configuration must not consume a
        # challenge or commit an identity that the resident cannot persist.
        result = DeviceRegisterResponse(
            device_id=device.device_id, device_name=device.name,
            access_token=create_device_access_token(device),
            refresh_token=create_device_refresh_token(device),
        )
        await db.commit()
        return result
    except DeviceEnrollmentError as error:
        await db.rollback()
        _raise(error)
    except Exception:
        await db.rollback()
        raise
