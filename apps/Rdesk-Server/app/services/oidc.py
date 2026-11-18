"""A single configured, confidential OIDC client; never trusts browser claims."""
from __future__ import annotations

import base64
import asyncio
import hashlib
import hmac
import json
import re
import secrets
from dataclasses import dataclass
from datetime import UTC, datetime, timedelta
from urllib.parse import quote_plus, urlencode, urlsplit
from uuid import uuid4

import httpx
import jwt
from cryptography.fernet import Fernet, InvalidToken
from cryptography.hazmat.primitives.asymmetric.rsa import RSAPublicKey
from sqlalchemy import delete, select, update
from sqlalchemy.exc import IntegrityError
from sqlalchemy.ext.asyncio import AsyncSession

from app.core.config import settings
from app.core.security import create_access_token
from app.models.oidc import OidcIdentity, OidcLoginTransaction
from app.models.user import User
from app.schemas.auth import LoginResponse


FLOW_SECONDS = 300
PROVIDER_DEADLINE_SECONDS = 10
_RANDOM = re.compile(r"^[A-Za-z0-9_-]{43}$", re.ASCII)
_SUBJECT = re.compile(r"^[\x21-\x7e]{1,255}$", re.ASCII)


class OidcFailure(Exception):
    """A deliberately non-descriptive failure safe to expose to callers."""


def _digest(value: str) -> str:
    return hashlib.sha256(value.encode("ascii")).hexdigest()


def _endpoint(value: object) -> str:
    if not isinstance(value, str) or not 1 <= len(value) <= 2048:
        raise OidcFailure()
    try:
        endpoint, issuer = urlsplit(value), urlsplit(settings.oidc_issuer)
        if (endpoint.scheme != "https" or endpoint.hostname != issuer.hostname
                or (endpoint.port or 443) != (issuer.port or 443)
                or endpoint.username is not None or endpoint.password is not None
                or endpoint.query or endpoint.fragment or "\\" in value
                or not re.fullmatch(r"[\x21-\x7e]+", value)):
            raise OidcFailure()
    except ValueError:
        raise OidcFailure() from None
    return value


def _json_object(pairs: list[tuple[str, object]]) -> dict[str, object]:
    result: dict[str, object] = {}
    for key, value in pairs:
        if key in result:
            raise OidcFailure()
        result[key] = value
    return result


async def _json_request(method: str, url: str, **kwargs: object) -> dict[str, object]:
    """Bound provider responses and do not follow redirects or inherit proxy env."""
    try:
        async with asyncio.timeout(PROVIDER_DEADLINE_SECONDS):
            async with httpx.AsyncClient(timeout=8.0, follow_redirects=False, trust_env=False) as client:
                async with client.stream(method, url, **kwargs) as response:
                    if response.status_code != 200:
                        raise OidcFailure()
                    body = bytearray()
                    async for chunk in response.aiter_bytes():
                        body.extend(chunk)
                        if len(body) > 131_072:
                            raise OidcFailure()
        payload = json.loads(body, object_pairs_hook=_json_object)
        if not isinstance(payload, dict):
            raise OidcFailure()
        return payload
    except (httpx.HTTPError, TimeoutError, ValueError, UnicodeError):
        raise OidcFailure() from None


@dataclass(frozen=True)
class Provider:
    authorization_endpoint: str
    token_endpoint: str
    jwks_uri: str
    response_issuer_required: bool


async def _discover() -> Provider:
    data = await _json_request("GET", settings.oidc_issuer + "/.well-known/openid-configuration")
    if data.get("issuer") != settings.oidc_issuer:
        raise OidcFailure()
    if (not isinstance(data.get("response_types_supported"), list)
            or "code" not in data["response_types_supported"]
            or not isinstance(data.get("code_challenge_methods_supported"), list)
            or "S256" not in data["code_challenge_methods_supported"]
            or not isinstance(data.get("id_token_signing_alg_values_supported"), list)
            or "RS256" not in data["id_token_signing_alg_values_supported"]):
        raise OidcFailure()
    methods = data.get("token_endpoint_auth_methods_supported", ["client_secret_basic"])
    response_iss = data.get("authorization_response_iss_parameter_supported", False)
    if not isinstance(methods, list) or "client_secret_basic" not in methods or not isinstance(response_iss, bool):
        raise OidcFailure()
    return Provider(
        _endpoint(data.get("authorization_endpoint")), _endpoint(data.get("token_endpoint")),
        _endpoint(data.get("jwks_uri")), response_iss,
    )


async def start(db: AsyncSession, user: User | None = None) -> dict[str, object]:
    provider = await _discover()
    state, nonce, verifier, completion = (secrets.token_urlsafe(32) for _ in range(4))
    challenge = base64.urlsafe_b64encode(hashlib.sha256(verifier.encode("ascii")).digest()).rstrip(b"=").decode("ascii")
    cipher = Fernet(settings.oidc_transaction_key.get_secret_value().encode("ascii"))
    now = datetime.now(UTC)
    await db.execute(delete(OidcLoginTransaction).where(
        OidcLoginTransaction.expires_at < now).execution_options(synchronize_session=False))
    db.add(OidcLoginTransaction(
        completion_digest=_digest(completion), state_digest=_digest(state),
        verifier_ciphertext=cipher.encrypt(verifier.encode("ascii")).decode("ascii"), nonce=nonce,
        issuer=settings.oidc_issuer, client_id=settings.oidc_client_id,
        redirect_uri=settings.oidc_redirect_uri,
        require_response_issuer=provider.response_issuer_required, status="pending",
        link_user_id=user.id if user else None,
        link_session_version=user.session_version if user else None,
        expires_at=now + timedelta(seconds=FLOW_SECONDS),
    ))
    await db.commit()
    query = urlencode({
        "response_type": "code", "client_id": settings.oidc_client_id,
        "redirect_uri": settings.oidc_redirect_uri, "scope": "openid",
        "state": state, "nonce": nonce, "code_challenge": challenge, "code_challenge_method": "S256",
    })
    return {"authorization_url": provider.authorization_endpoint + "?" + query,
            "completion_token": completion, "expires_in": FLOW_SECONDS}


async def _verified_subject(provider: Provider, code: str, verifier: str, nonce: str) -> str:
    client_id, secret = settings.oidc_client_id, settings.oidc_client_secret.get_secret_value()
    basic = base64.b64encode(f"{quote_plus(client_id)}:{quote_plus(secret)}".encode("ascii")).decode("ascii")
    tokens = await _json_request("POST", provider.token_endpoint,
        headers={"Authorization": "Basic " + basic}, data={
            "grant_type": "authorization_code", "code": code,
            "redirect_uri": settings.oidc_redirect_uri, "code_verifier": verifier,
        })
    token = tokens.get("id_token")
    if not isinstance(token, str) or not 1 <= len(token) <= 16_384:
        raise OidcFailure()
    try:
        header = jwt.get_unverified_header(token)
        kid = header.get("kid")
        if header.get("alg") != "RS256" or not isinstance(kid, str) or not 1 <= len(kid) <= 256:
            raise OidcFailure()
        jwks = await _json_request("GET", provider.jwks_uri)
        keys = jwks.get("keys")
        if not isinstance(keys, list) or len(keys) > 100:
            raise OidcFailure()
        matches = [key for key in keys if isinstance(key, dict) and key.get("kid") == kid]
        if len(matches) != 1:
            raise OidcFailure()
        jwk = matches[0]
        if (jwk.get("kty") != "RSA" or jwk.get("use", "sig") != "sig"
                or jwk.get("alg", "RS256") != "RS256"
                or ("key_ops" in jwk and (not isinstance(jwk["key_ops"], list) or "verify" not in jwk["key_ops"]))):
            raise OidcFailure()
        key = jwt.PyJWK.from_dict(jwk, algorithm="RS256").key
        if not isinstance(key, RSAPublicKey) or key.key_size < 2048:
            raise OidcFailure()
        claims = jwt.decode(token, key, algorithms=["RS256"], issuer=settings.oidc_issuer,
            audience=client_id, options={"require": ["iss", "sub", "aud", "exp", "iat", "nonce"]})
        subject, audience, azp = claims.get("sub"), claims.get("aud"), claims.get("azp")
        if (not isinstance(subject, str) or not _SUBJECT.fullmatch(subject)
                or not isinstance(claims.get("nonce"), str)
                or not hmac.compare_digest(claims["nonce"], nonce)
                or (isinstance(audience, list) and (len(audience) != 1 and azp != client_id))
                or (azp is not None and azp != client_id)
                or not isinstance(claims["iat"], int) or isinstance(claims["iat"], bool)
                or not isinstance(claims["exp"], int) or isinstance(claims["exp"], bool)
                or claims["exp"] <= claims["iat"]):
            raise OidcFailure()
        return subject
    except (jwt.PyJWTError, ValueError, TypeError):
        raise OidcFailure() from None


async def _identity_user(db: AsyncSession, subject: str, link_id: str | None, link_version: int | None) -> User:
    issuer = settings.oidc_issuer
    if link_id is not None:
        # The locally authenticated account is the first lock in this path.
        user = await db.scalar(select(User).where(User.id == link_id).with_for_update()
                               .execution_options(populate_existing=True))
        if user is None or user.session_version != link_version:
            raise OidcFailure()
        mapping = await db.scalar(select(OidcIdentity).where(
            OidcIdentity.issuer == issuer, OidcIdentity.subject == subject))
        by_user = await db.scalar(select(OidcIdentity).where(OidcIdentity.user_id == user.id))
        if ((mapping is not None and mapping.user_id != user.id)
                or (by_user is not None and (by_user.issuer != issuer or by_user.subject != subject))):
            raise OidcFailure()
        if mapping is None:
            db.add(OidcIdentity(issuer=issuer, subject=subject, user_id=user.id, created_at=datetime.now(UTC)))
            await db.flush()
        return user

    mapping = await db.scalar(select(OidcIdentity).where(
        OidcIdentity.issuer == issuer, OidcIdentity.subject == subject))
    if mapping is None:
        unique = uuid4().hex
        user = User(id=str(uuid4()), username="oidc-" + unique,
                    email="oidc-" + unique + "@identity.invalid", password_hash="!", role="user")
        db.add(user)
        try:
            # Insert the referenced user before the unrelated mapper's identity
            # row; the whole transaction still rolls back if the mapping races.
            await db.flush()
            db.add(OidcIdentity(issuer=issuer, subject=subject, user_id=user.id, created_at=datetime.now(UTC)))
            await db.flush()
            return user
        except IntegrityError:
            # Concurrent first logins may race on the issuer+subject key.
            # Roll back the losing provisional user, then use only the winner.
            await db.rollback()
            mapping = await db.scalar(select(OidcIdentity).where(
                OidcIdentity.issuer == issuer, OidcIdentity.subject == subject))
            if mapping is None:
                raise OidcFailure() from None
    user = await db.scalar(select(User).where(User.id == mapping.user_id).with_for_update())
    if user is None:
        raise OidcFailure()
    return user


async def callback(db: AsyncSession, *, state: str, code: str | None,
                   response_issuer: str | None, provider_error: bool) -> None:
    if not _RANDOM.fullmatch(state):
        raise OidcFailure()
    row = (await db.execute(update(OidcLoginTransaction).where(
        OidcLoginTransaction.state_digest == _digest(state),
        OidcLoginTransaction.status == "pending",
        OidcLoginTransaction.expires_at > datetime.now(UTC),
    ).values(status="exchanging").execution_options(synchronize_session=False).returning(
        OidcLoginTransaction.completion_digest, OidcLoginTransaction.verifier_ciphertext,
        OidcLoginTransaction.nonce, OidcLoginTransaction.require_response_issuer,
        OidcLoginTransaction.link_user_id, OidcLoginTransaction.link_session_version,
        OidcLoginTransaction.issuer, OidcLoginTransaction.client_id,
        OidcLoginTransaction.redirect_uri,
    ))).first()
    if row is None:
        raise OidcFailure()
    completion_digest, ciphertext, nonce, require_iss, link_id, link_version, issuer, client_id, redirect_uri = row
    # Claim the state before talking to the provider so callbacks cannot replay.
    await db.commit()
    try:
        if (provider_error or code is None or not 1 <= len(code) <= 4096
                or issuer != settings.oidc_issuer or client_id != settings.oidc_client_id
                or redirect_uri != settings.oidc_redirect_uri
                or (require_iss and response_issuer is None)
                or (response_issuer is not None and response_issuer != settings.oidc_issuer)):
            raise OidcFailure()
        verifier = Fernet(settings.oidc_transaction_key.get_secret_value().encode("ascii")).decrypt(ciphertext.encode("ascii")).decode("ascii")
        if not _RANDOM.fullmatch(verifier):
            raise OidcFailure()
        provider = await _discover()
        subject = await _verified_subject(provider, code, verifier, nonce)
        link_subject_ciphertext = None
        if link_id is not None:
            # Do not attach a verified identity merely because the browser
            # completed: the initiating account must still redeem the flow.
            user = await db.scalar(select(User).where(User.id == link_id).with_for_update()
                                   .execution_options(populate_existing=True))
            if user is None or user.session_version != link_version:
                raise OidcFailure()
            binding = json.dumps({"completion_digest": completion_digest, "subject": subject},
                                 separators=(",", ":")).encode("ascii")
            link_subject_ciphertext = Fernet(settings.oidc_transaction_key.get_secret_value().encode("ascii")).encrypt(binding).decode("ascii")
        else:
            user = await _identity_user(db, subject, None, None)
        completed = await db.execute(update(OidcLoginTransaction).where(
            OidcLoginTransaction.completion_digest == completion_digest,
            OidcLoginTransaction.status == "exchanging",
            OidcLoginTransaction.expires_at > datetime.now(UTC),
        ).values(status="complete", user_id=user.id, verifier_ciphertext="",
                 link_subject_ciphertext=link_subject_ciphertext).execution_options(synchronize_session=False))
        if completed.rowcount != 1:
            raise OidcFailure()
        await db.commit()
    except (OidcFailure, InvalidToken, IntegrityError, UnicodeError, ValueError):
        await db.rollback()
        await db.execute(update(OidcLoginTransaction).where(
            OidcLoginTransaction.completion_digest == completion_digest,
            OidcLoginTransaction.status == "exchanging",
        ).values(status="failed", verifier_ciphertext="").execution_options(synchronize_session=False))
        await db.commit()
        raise OidcFailure() from None


async def complete(db: AsyncSession, completion: str, current_user: User | None = None) -> LoginResponse | None:
    if not _RANDOM.fullmatch(completion):
        raise OidcFailure()
    digest = _digest(completion)
    row = (await db.execute(update(OidcLoginTransaction).where(
        OidcLoginTransaction.completion_digest == digest,
        OidcLoginTransaction.status == "complete",
        OidcLoginTransaction.expires_at > datetime.now(UTC),
    ).values(status="consumed").execution_options(synchronize_session=False).returning(
        OidcLoginTransaction.user_id, OidcLoginTransaction.link_user_id,
        OidcLoginTransaction.link_session_version, OidcLoginTransaction.link_subject_ciphertext,
        OidcLoginTransaction.issuer, OidcLoginTransaction.client_id,
        OidcLoginTransaction.redirect_uri))).first()
    if row is None:
        pending = await db.scalar(select(OidcLoginTransaction.completion_digest).where(
            OidcLoginTransaction.completion_digest == digest,
            OidcLoginTransaction.status.in_(["pending", "exchanging"]),
            OidcLoginTransaction.expires_at > datetime.now(UTC)))
        if pending is None:
            raise OidcFailure()
        return None
    user_id, link_id, link_version, ciphertext, issuer, client_id, redirect_uri = row
    try:
        if (issuer != settings.oidc_issuer or client_id != settings.oidc_client_id
                or redirect_uri != settings.oidc_redirect_uri):
            raise OidcFailure()
        if link_id is not None:
            if current_user is None or current_user.id != link_id or current_user.session_version != link_version or not ciphertext:
                raise OidcFailure()
            plaintext = Fernet(settings.oidc_transaction_key.get_secret_value().encode("ascii")).decrypt(ciphertext.encode("ascii"))
            binding = json.loads(plaintext, object_pairs_hook=_json_object)
            if (not isinstance(binding, dict) or set(binding) != {"completion_digest", "subject"}
                    or binding["completion_digest"] != digest or not isinstance(binding["subject"], str)
                    or not _SUBJECT.fullmatch(binding["subject"])):
                raise OidcFailure()
            user = await _identity_user(db, binding["subject"], link_id, link_version)
            if user.id != user_id:
                raise OidcFailure()
            await db.execute(update(OidcLoginTransaction).where(
                OidcLoginTransaction.completion_digest == digest,
                OidcLoginTransaction.status == "consumed",
            ).values(link_subject_ciphertext=None).execution_options(synchronize_session=False))
        else:
            user = await db.scalar(select(User).where(User.id == user_id).with_for_update()
                                   .execution_options(populate_existing=True))
            if user is None:
                raise OidcFailure()
        token = create_access_token(user.id, user.username, user.role, user.session_version)
        response = LoginResponse(access_token=token, user_id=user.id, username=user.username, role=user.role)
        await db.commit()
        return response
    except (OidcFailure, IntegrityError, InvalidToken, ValueError, UnicodeError):
        await db.rollback()
        raise OidcFailure() from None
