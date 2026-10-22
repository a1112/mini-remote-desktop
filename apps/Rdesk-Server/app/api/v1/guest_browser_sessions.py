from datetime import UTC, datetime
import re
import base64
import json
import jwt
from fastapi import APIRouter, Depends, HTTPException, Request
from sqlalchemy.ext.asyncio import AsyncSession
from app.core.config import settings
from app.core.security import (
    _configured_device_jwt,
    capture_device_auth_snapshot,
    get_current_device,
)
from app.core.response_security import no_store_sensitive_response
from app.db.session import get_db
from app.models.device import Device
from app.schemas.guest_browser import (
    GuestBrowserSessionCreateIn,
    GuestBrowserSessionOut,
    GuestHTTPCredential,
    TemporaryAccessPublishIn,
    TemporaryAccessStatus,
)
from app.schemas.browser_session import BrowserRelayAccessIn
from app.schemas.session import (
    DeviceSessionId,
    DeviceSessionOut,
    DeviceSessionTransitionIn,
)
from app.services.device_sessions import DeviceSessionError, device_session_out
from app.services.guest_browser_sessions import GuestBrowserSessionService
from app.services.temporary_access import TemporaryAccessService
from app.services.browser_authority import utc
from app.services.device_self_enrollment import (
    enrollment_peer_ip,
    _unique_json_object,
    _reject_json_constant,
)
from app.services.device_enrollment import DeviceEnrollmentError
from app.services.relay_directory import RelayAccessService, RelayAccessError
from app.api.v1.browser_sessions import _response
from app.api.v1.device_sessions import (
    DeviceSessionAPIRoute,
    _commit,
    _raise,
    _raise_relay,
)
from app.api.v1.relays import (
    get_relay_access_service,
    RelayAccessResponse,
    NodeTurnCredentialOut,
)

HTTP_AUDIENCE = "rdesk-guest-browser-http"
router = APIRouter(
    prefix="/guest-browser-sessions",
    tags=["guest-browser-sessions"],
    route_class=DeviceSessionAPIRoute,
    dependencies=[Depends(no_store_sensitive_response)],
)
physical_router = APIRouter(
    prefix="/devices",
    tags=["devices"],
    route_class=DeviceSessionAPIRoute,
    dependencies=[Depends(no_store_sensitive_response)],
)


def _temporary(db):
    if settings.guest_browser_enabled is not True:
        raise HTTPException(503, detail={"code": "guest_access_unavailable"})
    try:
        return TemporaryAccessService(
            db,
            api_url=settings.public_api_url,
            pepper=bytes.fromhex(settings.device_serial_pepper.get_secret_value()),
        )
    except (TypeError, ValueError, AttributeError):
        raise HTTPException(503, detail={"code": "guest_access_unavailable"}) from None


def _http_credential(row, principal, shadow):
    configured = _configured_device_jwt()
    if configured is None or HTTP_AUDIENCE in {
        settings.jwt_audience,
        settings.device_jwt_audience,
        settings.signaling_jwt_audience,
    }:
        raise HTTPException(503, detail={"code": "guest_access_unavailable"})
    now = int(datetime.now(UTC).timestamp())
    expiry = min(now + 600, int(utc(principal.expires_at).timestamp()))
    if row.status == "approved":
        expiry = min(
            expiry,
            int(utc(row.grant_expires_at).timestamp()),
            int(utc(row.policy_expires_at).timestamp()),
        )
    if expiry <= now:
        raise HTTPException(404, detail={"code": "browser_session_not_found"})
    claims = dict(
        sub=shadow.device_id,
        device_id=shadow.device_id,
        device_key_id=principal.key_id,
        token_type="guest_browser_http",
        authority_kind="temporary_password",
        iss=configured[1],
        aud=HTTP_AUDIENCE,
        iat=now,
        exp=expiry,
        user_id=None,
        tenant_id=principal.tenant_id,
        session_id=row.id,
        target_device_id=row.request_payload["target_device_id"],
        allowed_scopes=principal.allowed_scopes,
        request_commitment=row.request_commitment,
        temporary_access_generation=principal.temporary_access_generation,
        target_auth_version=principal.target_auth_version,
    )
    return GuestHTTPCredential(
        token=jwt.encode(claims, configured[0], algorithm="HS256"),
        expires_at_ms=expiry * 1000,
    )


def _claims(request: Request):
    configured = _configured_device_jwt()
    auth = request.headers.getlist("authorization")
    try:
        if (
            configured is None
            or len(auth) != 1
            or len(auth[0]) > 8192
            or re.fullmatch(
                r"Bearer [A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+", auth[0]
            )
            is None
        ):
            raise ValueError()
        # A valid MAC must not turn duplicate fields or multiple audiences into
        # an ambiguous grant at a different verifier. Parse the same compact JWT
        # with duplicate rejection before applying PyJWT's signature checks.
        parts = auth[0][7:].split(".")
        parsed = []
        for part in parts[:2]:
            raw = base64.b64decode(
                part + "=" * (-len(part) % 4), altchars=b"-_", validate=True
            )
            parsed.append(
                json.loads(
                    raw.decode("utf-8"),
                    object_pairs_hook=_unique_json_object,
                    parse_constant=_reject_json_constant,
                )
            )
        if (
            not isinstance(parsed[0], dict)
            or parsed[0] != {"alg": "HS256", "typ": "JWT"}
            or not isinstance(parsed[1], dict)
        ):
            raise ValueError()
        claims = jwt.decode(
            auth[0][7:],
            configured[0],
            algorithms=["HS256"],
            issuer=configured[1],
            audience=HTTP_AUDIENCE,
            options={"require": ["sub", "iat", "exp", "iss", "aud"]},
        )
        expected = {
            "sub",
            "device_id",
            "device_key_id",
            "token_type",
            "authority_kind",
            "iss",
            "aud",
            "iat",
            "exp",
            "user_id",
            "tenant_id",
            "session_id",
            "target_device_id",
            "allowed_scopes",
            "request_commitment",
            "temporary_access_generation",
            "target_auth_version",
        }
        if (
            set(claims) != expected
            or claims["token_type"] != "guest_browser_http"
            or claims["authority_kind"] != "temporary_password"
            or claims["user_id"] is not None
            or claims["aud"] != HTTP_AUDIENCE
        ):
            raise ValueError()
        if (
            any(
                type(claims[k]) is not int
                for k in (
                    "iat",
                    "exp",
                    "temporary_access_generation",
                    "target_auth_version",
                )
            )
            or not 0 < claims["exp"] - claims["iat"] <= 600
        ):
            raise ValueError()
        if any(
            not 1 <= claims[k] <= 2**63 - 1
            for k in ("temporary_access_generation", "target_auth_version")
        ):
            raise ValueError()
        if (
            re.fullmatch(r"browser_[0-9a-f]{32}", claims["device_id"]) is None
            or claims["sub"] != claims["device_id"]
        ):
            raise ValueError()
        if any(
            re.fullmatch(r"[0-9a-f]{64}", claims[k]) is None
            for k in ("device_key_id", "request_commitment")
        ):
            raise ValueError()
        if (
            not isinstance(claims["tenant_id"], str)
            or not 1 <= len(claims["tenant_id"]) <= 64
        ):
            raise ValueError()
        scopes = claims["allowed_scopes"]
        if (
            not isinstance(scopes, list)
            or scopes != sorted(set(scopes))
            or "screen.view" not in scopes
            or any(
                s not in {"screen.view", "input.keyboard", "input.pointer"}
                for s in scopes
            )
        ):
            raise ValueError()
        if any(
            not isinstance(claims[k], str)
            or re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]{0,63}", claims[k]) is None
            for k in ("tenant_id", "target_device_id")
        ):
            raise ValueError()
        if (
            not isinstance(claims["session_id"], str)
            or re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]{0,35}", claims["session_id"])
            is None
        ):
            raise ValueError()
        return claims
    except (jwt.PyJWTError, ValueError, TypeError, KeyError, RecursionError):
        raise HTTPException(
            401,
            detail={
                "code": "guest_access_invalid",
                "message": "Temporary access is unavailable",
            },
        ) from None


@physical_router.post("/temporary-access", response_model=TemporaryAccessStatus)
async def publish_temporary_access(
    payload: TemporaryAccessPublishIn,
    device: Device = Depends(get_current_device),
    db: AsyncSession = Depends(get_db),
):
    snapshot = capture_device_auth_snapshot(device)
    try:
        result = await _temporary(db).publish(snapshot=snapshot, payload=payload)
        await _commit(db)
        return result
    except DeviceSessionError as error:
        await db.rollback()
        _raise(error)


@physical_router.get("/temporary-access", response_model=TemporaryAccessStatus)
async def temporary_access_status(
    device: Device = Depends(get_current_device), db: AsyncSession = Depends(get_db)
):
    return await _temporary(db).inspect(device)


@router.post("", response_model=GuestBrowserSessionOut)
async def create_guest_browser(
    payload: GuestBrowserSessionCreateIn,
    request: Request,
    db: AsyncSession = Depends(get_db),
):
    temporary = _temporary(db)
    try:
        peer = enrollment_peer_ip(
            request, settings.device_self_enrollment_trusted_proxies
        )
        await temporary.record_attempt(
            peer_ip=peer,
            target_device_id=payload.target_device_id,
            limits=(
                settings.guest_browser_global_per_minute,
                settings.guest_browser_ip_per_minute,
                settings.guest_browser_device_per_minute,
            ),
        )
        service = GuestBrowserSessionService(db, temporary)
        row, principal, shadow = await service.create(payload)
        response = await _response(
            db, row, principal, shadow, authority_service=service
        )
        result = GuestBrowserSessionOut(
            **response.model_dump(),
            http_credential=_http_credential(row, principal, shadow)
        )
        await _commit(db)
        return result
    except DeviceSessionError as error:
        await db.rollback()
        _raise(error)
    except DeviceEnrollmentError as error:
        await db.rollback()
        raise HTTPException(
            error.status_code,
            detail={
                "code": "guest_access_invalid",
                "message": "Temporary access is unavailable",
            },
        ) from None
    except Exception:
        await db.rollback()
        raise


@router.get("/{session_id}", response_model=GuestBrowserSessionOut)
async def inspect_guest_browser(
    session_id: DeviceSessionId, request: Request, db: AsyncSession = Depends(get_db)
):
    service = GuestBrowserSessionService(db, _temporary(db))
    claims = _claims(request)
    try:
        row, principal, shadow = await service.inspect(
            session_id=session_id, claims=claims
        )
        response = await _response(
            db, row, principal, shadow, authority_service=service
        )
        return GuestBrowserSessionOut(
            **response.model_dump(),
            http_credential=_http_credential(row, principal, shadow)
        )
    except DeviceSessionError as error:
        await db.rollback()
        _raise(error)


@router.post("/{session_id}/close", response_model=DeviceSessionOut)
async def close_guest_browser(
    session_id: DeviceSessionId,
    payload: DeviceSessionTransitionIn,
    request: Request,
    db: AsyncSession = Depends(get_db),
):
    service = GuestBrowserSessionService(db, _temporary(db))
    claims = _claims(request)
    try:
        row = await service.close(session_id=session_id, claims=claims)
        result = device_session_out(row)
        await _commit(db)
        return result
    except DeviceSessionError as error:
        await db.rollback()
        _raise(error)


@router.post("/{session_id}/relay-access", response_model=RelayAccessResponse)
async def guest_relay_access(
    session_id: DeviceSessionId,
    payload: BrowserRelayAccessIn,
    request: Request,
    db: AsyncSession = Depends(get_db),
    relay: RelayAccessService = Depends(get_relay_access_service),
):
    service = GuestBrowserSessionService(db, _temporary(db))
    claims = _claims(request)
    try:
        row, principal, shadow = await service.inspect(
            session_id=session_id, claims=claims
        )
        if row.status != "approved":
            raise DeviceSessionError(
                "browser_session_not_found", 404, "Browser session is unavailable"
            )
        generation = (
            payload.generation
            if payload.generation is not None
            else row.active_relay_generation
        )
        result = await relay.issue_authenticated_access(
            current_device=shadow,
            auth_snapshot=capture_device_auth_snapshot(shadow),
            session_id=row.id,
            policy_revision=row.policy_revision,
            intended_peer_id=row.request_payload["target_device_id"],
            generation=generation,
            refresh=payload.refresh,
        )
        return RelayAccessResponse(
            generation=result.generation,
            directory_id=result.directory.payload.directory_id,
            relay_url_digest=result.relay_url_digest,
            directory=result.directory,
            credentials=[
                NodeTurnCredentialOut(
                    node_id=item.node_id,
                    urls=list(item.urls),
                    username=item.username,
                    credential=item.credential,
                    expires_at_unix_seconds=item.expires_at_unix_seconds,
                )
                for item in result.credentials
            ],
        )
    except DeviceSessionError as error:
        await db.rollback()
        _raise(error)
    except RelayAccessError as error:
        await db.rollback()
        _raise_relay(error)
