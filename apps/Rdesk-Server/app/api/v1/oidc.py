"""OIDC entry and one-use completion endpoints for browser and native clients."""
from collections import deque
import time

from fastapi import APIRouter, Depends, HTTPException, Request
from fastapi.responses import HTMLResponse, JSONResponse
from pydantic import BaseModel, ConfigDict, Field
from sqlalchemy.ext.asyncio import AsyncSession

from app.core.client_ip import InvalidForwardedAddress, trusted_client_ip
from app.core.config import settings
from app.core.response_security import no_store_sensitive_response
from app.core.security import get_current_user, get_current_user_optional
from app.db.session import get_db
from app.models.user import User
from app.services import oidc


router = APIRouter(prefix="/auth/oidc", tags=["auth"],
                   dependencies=[Depends(no_store_sensitive_response)])
_attempts: dict[str, deque[float]] = {}
_global_attempts: deque[float] = deque()


class Completion(BaseModel):
    model_config = ConfigDict(extra="forbid")
    completion_token: str = Field(min_length=43, max_length=43)


def _enabled() -> None:
    if not settings.oidc_enabled:
        raise HTTPException(status_code=404, detail="Not found")


def _admit(request: Request) -> None:
    now = time.monotonic()
    try:
        key = trusted_client_ip(request, settings.oidc_trusted_proxies)
    except InvalidForwardedAddress:
        raise HTTPException(status_code=400, detail="Invalid proxy client address") from None
    except ValueError:
        if settings.oidc_trusted_proxies:
            raise HTTPException(status_code=503, detail="Account login unavailable") from None
        key = "unknown"
    bucket = _attempts.get(key, deque())
    for items in (bucket, _global_attempts):
        while items and now - items[0] >= 60:
            items.popleft()
    if len(bucket) >= 10 or len(_global_attempts) >= 300:
        raise HTTPException(status_code=429, detail="Too many authentication attempts", headers={"Retry-After": "60"})
    bucket.append(now)
    _attempts[key] = bucket
    _global_attempts.append(now)
    if len(_attempts) > 10_000:
        oldest = min(_attempts, key=lambda item: _attempts[item][-1])
        del _attempts[oldest]


@router.get("/capabilities")
async def capabilities() -> dict[str, bool]:
    return {"enabled": settings.oidc_enabled}


@router.post("/start")
async def start(request: Request, db: AsyncSession = Depends(get_db)) -> dict[str, object]:
    _enabled()
    _admit(request)
    try:
        return await oidc.start(db)
    except oidc.OidcFailure:
        raise HTTPException(status_code=503, detail="Account login unavailable") from None


@router.post("/link/start")
async def link_start(request: Request, user: User = Depends(get_current_user),
                     db: AsyncSession = Depends(get_db)) -> dict[str, object]:
    _enabled()
    _admit(request)
    try:
        return await oidc.start(db, user)
    except oidc.OidcFailure:
        raise HTTPException(status_code=503, detail="Account login unavailable") from None


@router.get("/callback", response_class=HTMLResponse)
async def callback(request: Request, db: AsyncSession = Depends(get_db)) -> HTMLResponse:
    _enabled()
    params = request.query_params
    fields = {name: params.getlist(name) for name in ("state", "code", "error", "iss")}
    valid = (len(fields["state"]) == 1 and len(fields["code"]) <= 1
             and len(fields["error"]) <= 1 and len(fields["iss"]) <= 1
             and (bool(fields["code"]) != bool(fields["error"]))
             and all(len(value) <= 4096 for values in fields.values() for value in values))
    if valid:
        try:
            await oidc.callback(db, state=fields["state"][0],
                code=fields["code"][0] if fields["code"] else None,
                response_issuer=fields["iss"][0] if fields["iss"] else None,
                provider_error=bool(fields["error"]))
        except oidc.OidcFailure:
            valid = False
    body = ("已收到登录结果，请返回 Rdesk 完成操作。" if valid else "登录未完成。请返回 Rdesk 重试。")
    return HTMLResponse("<!doctype html><html lang=\"zh-CN\"><meta charset=\"utf-8\"><title>Rdesk</title><body><p>"
                        + body + "</p></body></html>", status_code=200 if valid else 400,
                        headers={"Referrer-Policy": "no-referrer", "X-Content-Type-Options": "nosniff",
                                 "Content-Security-Policy": "default-src 'none'; base-uri 'none'; frame-ancestors 'none'"})


@router.post("/complete")
async def complete(payload: Completion, db: AsyncSession = Depends(get_db),
                   current_user: User | None = Depends(get_current_user_optional)):
    _enabled()
    try:
        result = await oidc.complete(db, payload.completion_token, current_user)
    except oidc.OidcFailure:
        raise HTTPException(status_code=400, detail="Account login could not be completed") from None
    if result is None:
        return JSONResponse({"status": "pending"}, status_code=202,
                            headers={"Cache-Control": "no-store, private", "Pragma": "no-cache"})
    return result
