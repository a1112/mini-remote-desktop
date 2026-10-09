from datetime import UTC, datetime
import re
import hashlib
import binascii
from urllib.parse import urlsplit

from fastapi import APIRouter, Depends, HTTPException
import jwt
from sqlalchemy.ext.asyncio import AsyncSession

from app.api.v1.device_sessions import DeviceSessionAPIRoute, _commit, _raise, _raise_relay
from app.api.v1.relays import get_relay_access_service, NodeTurnCredentialOut, RelayAccessResponse, _decode_secret_b64
from app.core.config import settings
from app.core.security import _configured_device_jwt, capture_device_auth_snapshot, get_current_user
from app.db.session import get_db
from app.models.user import User
from app.schemas.browser_session import BrowserRelayAccessIn, BrowserSessionCreateIn, BrowserSessionOut
from app.schemas.realtime import SignalingCredentialRequest, SignalingCredentialResponse
from app.schemas.session import DeviceSessionId, DeviceSessionOut, DeviceSessionTransitionIn
from app.services.browser_authority import utc
from app.services.browser_sessions import BrowserCredentialBinding, BrowserSessionService
from app.services.device_sessions import DeviceSessionError, device_session_out
from app.services.relay_directory import RelayAccessError, RelayAccessService
from app.services.signaling_credentials import issue_signaling_credential
from app.services.realtime_identities import query_realtime_target_key
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from cryptography.hazmat.primitives.serialization import Encoding, PublicFormat


router = APIRouter(prefix="/browser-sessions", tags=["browser-sessions"], route_class=DeviceSessionAPIRoute)


async def _response(db, row, principal, shadow):
    target_id = row.request_payload["target_device_id"]
    binding = BrowserCredentialBinding(session_id=row.id, user_id=principal.user_id,
        user_session_version=principal.user_session_version, tenant_id=principal.tenant_id,
        controller_row_id=shadow.id, controller_device_id=shadow.device_id,
        controller_key_id=principal.key_id, target_row_id=row.target_device_id,
        target_device_id=target_id, request_commitment=row.request_commitment)
    server_key = settings.public_signal_server_key_id
    relay_key = settings.relay_directory_signing_key_id
    server_id = settings.public_signal_server_device_id
    signaling_url = settings.signaling_ws_url
    try:
        parsed = urlsplit(signaling_url)
        if parsed.port is not None and not 1 <= parsed.port <= 65535:
            raise ValueError("invalid signaling endpoint")
    except (ValueError, TypeError):
        raise HTTPException(status_code=503, detail={"code": "browser_identity_unavailable",
                            "message": "Trusted browser connection identity is unavailable"}) from None
    if (any(not isinstance(key, str) or re.fullmatch(r"[0-9a-f]{64}", key, re.ASCII) is None
            for key in (server_key, relay_key))
        or not isinstance(server_id, str) or re.fullmatch(r"[A-Za-z0-9._-]{1,128}", server_id, re.ASCII) is None
        or parsed.scheme not in {"ws", "wss"} or not parsed.hostname
        or parsed.username is not None or parsed.password is not None or parsed.fragment):
        raise HTTPException(status_code=503, detail={"code": "browser_identity_unavailable",
                            "message": "Trusted browser connection identity is unavailable"})
    target_key = await query_realtime_target_key(target_id)
    if not isinstance(target_key, str) or re.fullmatch(r"[0-9a-f]{64}", target_key, re.ASCII) is None:
        raise HTTPException(status_code=503, detail={"code": "browser_identity_unavailable",
                            "message": "Trusted target connection identity is unavailable"})
    try:
        seed = _decode_secret_b64(settings.relay_directory_signing_private_key, expected_length=32)
        relay_public = Ed25519PrivateKey.from_private_bytes(seed).public_key().public_bytes(Encoding.Raw, PublicFormat.Raw)
        if hashlib.sha256(relay_public).hexdigest() != relay_key:
            raise ValueError("directory key binding differs")
    except (ValueError, TypeError, binascii.Error):
        raise HTTPException(status_code=503, detail={"code": "browser_identity_unavailable",
                            "message": "Trusted relay directory identity is unavailable"}) from None
    row, principal, shadow = await BrowserSessionService(db).revalidate_for_credential(binding)
    base = issue_signaling_credential(shadow, SignalingCredentialRequest(
        device_key_id=principal.key_id, role="Controller"))
    configured = _configured_device_jwt()
    now = int(datetime.now(UTC).timestamp())
    expires = min(base.expires_at_ms // 1000, int(utc(principal.expires_at).timestamp()))
    if row.status == "approved":
        expires = min(expires, int(utc(row.grant_expires_at).timestamp()), int(utc(row.policy_expires_at).timestamp()))
    if expires <= now:
        BrowserSessionService.unavailable()
    claims = {"sub": shadow.device_id, "device_id": shadow.device_id,
        "device_key_id": principal.key_id, "role": "Controller", "token_type": "browser_signaling",
        "iss": configured[1], "aud": settings.signaling_jwt_audience.strip(), "iat": now, "exp": expires,
        "user_id": principal.user_id, "tenant_id": principal.tenant_id,
        "session_id": row.id, "target_device_id": row.request_payload["target_device_id"],
        "allowed_scopes": list(row.approved_scopes if row.status == "approved" else principal.allowed_scopes)}
    return BrowserSessionOut(controller_device_id=shadow.device_id, controller_key_id=principal.key_id,
        signaling_url=signaling_url, signaling_server_device_id=server_id,
        signaling_server_key_id=server_key, target_key_id=target_key, relay_directory_key_id=relay_key,
        relay_directory_public_key=list(relay_public),
        expires_at_ms=int(utc(principal.expires_at).timestamp() * 1000), session=device_session_out(row),
        credential=SignalingCredentialResponse(token=jwt.encode(claims, configured[0], algorithm="HS256"),
            expires_at_ms=expires * 1000, device_id=shadow.device_id,
            device_key_id=principal.key_id, role="Controller"))


@router.post("", response_model=BrowserSessionOut)
async def create_browser_session(payload: BrowserSessionCreateIn,
    user: User = Depends(get_current_user), db: AsyncSession = Depends(get_db)):
    version = user.session_version
    try:
        row, principal, shadow = await BrowserSessionService(db).create(
            user=user, user_version=version, payload=payload)
        response = await _response(db, row, principal, shadow)
        await _commit(db)
        return response
    except DeviceSessionError as error:
        await db.rollback()
        _raise(error)
    except HTTPException:
        await db.rollback()
        raise


@router.get("/{session_id}", response_model=BrowserSessionOut)
async def inspect_browser_session(session_id: DeviceSessionId,
    user: User = Depends(get_current_user), db: AsyncSession = Depends(get_db)):
    try:
        return await _response(db, *await BrowserSessionService(db).inspect(session_id=session_id, user=user))
    except DeviceSessionError as error:
        await db.rollback()
        _raise(error)


@router.post("/{session_id}/close", response_model=DeviceSessionOut)
async def close_browser_session(session_id: DeviceSessionId, _: DeviceSessionTransitionIn,
    user: User = Depends(get_current_user), db: AsyncSession = Depends(get_db)):
    try:
        row = await BrowserSessionService(db).close(session_id=session_id, user=user)
        response = device_session_out(row)
        await _commit(db)
        return response
    except DeviceSessionError as error:
        await db.rollback()
        _raise(error)


@router.post("/{session_id}/relay-access", response_model=RelayAccessResponse)
async def browser_relay_access(session_id: DeviceSessionId, payload: BrowserRelayAccessIn,
    user: User = Depends(get_current_user), db: AsyncSession = Depends(get_db),
    relay: RelayAccessService = Depends(get_relay_access_service)):
    try:
        row, principal, shadow = await BrowserSessionService(db).inspect(session_id=session_id, user=user)
        if row.status != "approved":
            BrowserSessionService.unavailable()
        generation = payload.generation if payload.generation is not None else row.active_relay_generation
        result = await relay.issue_authenticated_access(current_device=shadow,
            auth_snapshot=capture_device_auth_snapshot(shadow), session_id=row.id,
            policy_revision=row.policy_revision, intended_peer_id=row.request_payload["target_device_id"],
            generation=generation, refresh=payload.refresh)
        return RelayAccessResponse(generation=result.generation, directory_id=result.directory.payload.directory_id,
            relay_url_digest=result.relay_url_digest, directory=result.directory,
            credentials=[NodeTurnCredentialOut(node_id=item.node_id, urls=list(item.urls), username=item.username,
                credential=item.credential, expires_at_unix_seconds=item.expires_at_unix_seconds)
                for item in result.credentials])
    except DeviceSessionError as error:
        await db.rollback()
        _raise(error)
    except RelayAccessError as error:
        await db.rollback()
        _raise_relay(error)
