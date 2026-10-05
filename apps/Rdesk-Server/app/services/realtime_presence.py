"""Read authenticated private realtime presence for already-visible device codes."""

import asyncio
import base64
import hashlib
import hmac
import ipaddress
import json
import re
import time
from dataclasses import dataclass
from urllib.parse import urlsplit

import httpx
from pydantic import SecretStr

from app.core.config import settings

TOTAL_TIMEOUT_SECONDS = 2.0
MAX_BATCH_SIZE = 128
MAX_RESPONSE_BYTES = 64 * 1024
MAX_HEARTBEAT_AGE_MS = 300_000  # Sidecar configurable TTL has this hard maximum.
_DEVICE_CODE = re.compile(r"[A-Za-z0-9._-]{1,128}", re.ASCII)
_SECRET = re.compile(r"[A-Za-z0-9_-]{43}", re.ASCII)
_CONTEXT = b"MRD_REALTIME_PRESENCE_QUERY_V1\0"


@dataclass(frozen=True, slots=True)
class DevicePresence:
    online: bool = False
    last_seen_ms: int | None = None


def _now_ms() -> int:
    return time.time_ns() // 1_000_000


async def query_realtime_presence(device_ids: list[str]) -> dict[str, DevicePresence] | None:
    """None preserves legacy status only when the private query is unconfigured.

    Every configured failure returns offline, including partial batch failures.
    Callers must finish user/tenant visibility checks before passing these codes.
    """
    secret = settings.realtime_presence_secret
    if secret is None:
        return None
    codes = list(dict.fromkeys(device_ids))
    offline = {code: DevicePresence() for code in codes}
    if not codes:
        return offline
    try:
        async with asyncio.timeout(TOTAL_TIMEOUT_SECONDS):
            if any(not isinstance(code, str) or _DEVICE_CODE.fullmatch(code) is None for code in codes):
                return offline
            url = _loopback_url(settings.realtime_presence_url)
            authorization = _authorization(secret)
            found: dict[str, DevicePresence] = {}
            async with httpx.AsyncClient(
                trust_env=False, follow_redirects=False, timeout=TOTAL_TIMEOUT_SECONDS,
            ) as client:
                for start in range(0, len(codes), MAX_BATCH_SIZE):
                    batch = codes[start:start + MAX_BATCH_SIZE]
                    async with client.stream("POST", url,
                        headers={"Authorization": authorization, "Accept-Encoding": "identity"},
                        json={"device_ids": batch},
                    ) as response:
                        if response.status_code != 200 or response.headers.get("content-encoding", "identity") != "identity":
                            raise ValueError("presence unavailable")
                        content_length = response.headers.get("content-length")
                        if content_length is not None and (not content_length.isascii() or not content_length.isdecimal() or int(content_length) > MAX_RESPONSE_BYTES):
                            raise ValueError("presence response too large")
                        body = bytearray()
                        async for chunk in response.aiter_bytes(chunk_size=8192):
                            if len(body) + len(chunk) > MAX_RESPONSE_BYTES:
                                raise ValueError("presence response too large")
                            body.extend(chunk)
                    decoded = json.loads(body.decode("utf-8"), object_pairs_hook=_unique_members,
                        parse_constant=_invalid_constant)
                    found.update(_parse_response(decoded, batch, _now_ms()))
            return found
    except (TimeoutError, httpx.HTTPError, ValueError, TypeError, OverflowError, RecursionError):
        return offline


def _loopback_url(value: str) -> str:
    if not isinstance(value, str) or any(character.isspace() or ord(character) < 32 for character in value):
        raise ValueError("invalid presence URL")
    parsed = urlsplit(value)
    if (parsed.scheme != "http" or parsed.username is not None or parsed.password is not None
        or "?" in value or "#" in value or parsed.path != "/internal/presence"
        or not ipaddress.ip_address(parsed.hostname).is_loopback
        or (parsed.port is not None and not 1 <= parsed.port <= 65535)):
        raise ValueError("invalid presence URL")
    return value


def _authorization(secret: SecretStr) -> str:
    if not isinstance(secret, SecretStr):
        raise ValueError("invalid presence secret")
    raw = secret.get_secret_value()
    if _SECRET.fullmatch(raw) is None:
        raise ValueError("invalid presence secret")
    key = base64.b64decode(raw + "=", altchars=b"-_", validate=True)
    if len(key) != 32 or base64.urlsafe_b64encode(key).rstrip(b"=").decode("ascii") != raw:
        raise ValueError("invalid presence secret")
    token = base64.urlsafe_b64encode(hmac.new(key, _CONTEXT, hashlib.sha256).digest()).rstrip(b"=").decode("ascii")
    return f"Bearer {token}"


def _unique_members(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError("duplicate presence member")
        result[key] = value
    return result


def _invalid_constant(_value):
    raise ValueError("invalid presence number")


def _timestamp(value) -> bool:
    return type(value) is int and 0 <= value <= 2**64 - 1


def _parse_response(value, requested: list[str], now_ms: int) -> dict[str, DevicePresence]:
    if (not isinstance(value, dict) or set(value) != {"version", "sampled_at_ms", "devices"}
        or type(value["version"]) is not int or value["version"] != 1
        or not _timestamp(value["sampled_at_ms"]) or not isinstance(value["devices"], list)):
        raise ValueError("invalid presence response")
    sampled = value["sampled_at_ms"]
    if sampled > now_ms + 2000 or now_ms - sampled > 5000:
        raise ValueError("stale presence response")
    requested_codes = set(requested)
    found = {}
    for row in value["devices"]:
        if not isinstance(row, dict) or set(row) != {"device_id", "online", "last_seen_ms"}:
            raise ValueError("invalid presence device")
        code, online, seen = row["device_id"], row["online"], row["last_seen_ms"]
        if not isinstance(code, str) or code not in requested_codes or code in found or type(online) is not bool:
            raise ValueError("invalid presence device")
        if seen is not None and (not _timestamp(seen) or seen > sampled):
            raise ValueError("invalid presence timestamp")
        if online and (seen is None or sampled - seen >= MAX_HEARTBEAT_AGE_MS):
            raise ValueError("expired presence heartbeat")
        found[code] = DevicePresence(online, seen)
    if set(found) != requested_codes:
        raise ValueError("incomplete presence response")
    return found
