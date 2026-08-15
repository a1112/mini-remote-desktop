from __future__ import annotations

import base64
import hashlib
import hmac
import re
import time
from dataclasses import dataclass
from typing import Callable


SCOPE_PATTERN = re.compile(r"^[A-Za-z0-9._-]{1,128}$")


class TurnCredentialExpired(ValueError):
    pass


class TurnCredentialConfigurationError(RuntimeError):
    pass


@dataclass(frozen=True)
class TurnCredential:
    urls: tuple[str, ...]
    username: str
    credential: str
    expires_at_unix_seconds: int
    ttl_seconds: int
    transport_policy: str = "relay"


class TurnCredentialService:
    def __init__(
        self,
        *,
        auth_secret: str,
        urls: list[str],
        ttl_seconds: int,
        now: Callable[[], int] | None = None,
    ) -> None:
        if not auth_secret:
            raise TurnCredentialConfigurationError("TURN auth secret is not configured")
        if not urls or any(
            not url.startswith(("turn:", "turns:")) for url in urls
        ):
            raise TurnCredentialConfigurationError("TURN URLs are not configured correctly")
        if not 1 <= ttl_seconds <= 86_400:
            raise TurnCredentialConfigurationError(
                "TURN credential TTL must be between 1 and 86400 seconds"
            )
        self._auth_secret = auth_secret.encode("utf-8")
        self._urls = tuple(urls)
        self._ttl_seconds = ttl_seconds
        self._now = now or (lambda: int(time.time()))

    def issue(
        self,
        *,
        user_id: str,
        session_id: str,
        credential_deadline_unix_seconds: int,
    ) -> TurnCredential:
        self._validate_scope("user_id", user_id)
        self._validate_scope("session_id", session_id)
        now = self._now()
        if credential_deadline_unix_seconds <= now:
            raise TurnCredentialExpired("session authorization has expired")
        expires_at = min(
            credential_deadline_unix_seconds, now + self._ttl_seconds
        )
        username = f"{expires_at}:{user_id}:{session_id}"
        credential = base64.b64encode(
            # TURN REST temporary credentials use HMAC-SHA1 for coturn
            # interoperability. This is a protocol MAC, not password hashing.
            # lgtm[py/weak-sensitive-data-hashing]
            hmac.new(self._auth_secret, username.encode("utf-8"), hashlib.sha1).digest()
        ).decode("ascii")
        return TurnCredential(
            urls=self._urls,
            username=username,
            credential=credential,
            expires_at_unix_seconds=expires_at,
            ttl_seconds=expires_at - now,
        )

    def verify(self, username: str, credential: str, now: int | None = None) -> bool:
        try:
            expires_at = int(username.split(":", 1)[0])
        except (ValueError, IndexError):
            return False
        if expires_at <= (self._now() if now is None else now):
            return False
        expected = base64.b64encode(
            # TURN REST temporary credentials use HMAC-SHA1 for coturn
            # interoperability. This is a protocol MAC, not password hashing.
            # lgtm[py/weak-sensitive-data-hashing]
            hmac.new(self._auth_secret, username.encode("utf-8"), hashlib.sha1).digest()
        ).decode("ascii")
        return hmac.compare_digest(expected, credential)

    @staticmethod
    def _validate_scope(name: str, value: str) -> None:
        if not SCOPE_PATTERN.fullmatch(value):
            raise ValueError(
                f"{name} must contain only letters, digits, '.', '_' or '-'"
            )
