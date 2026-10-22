"""Resolve a target key through the authenticated, private sidecar boundary."""
import asyncio
import json
import re
import time
from urllib.parse import urlsplit, urlunsplit
import httpx

from app.core.config import settings
from app.services.realtime_presence import (
    MAX_RESPONSE_BYTES, TOTAL_TIMEOUT_SECONDS, _authorization, _invalid_constant,
    _loopback_url, _timestamp, _unique_members,
)


def _now_ms():
    return time.time_ns() // 1_000_000


async def query_realtime_target_key(device_id: str) -> str | None:
    if not isinstance(device_id, str) or re.fullmatch(r"[A-Za-z0-9._-]{1,128}", device_id, re.ASCII) is None:
        return None
    try:
        async with asyncio.timeout(TOTAL_TIMEOUT_SECONDS):
            parsed = urlsplit(_loopback_url(settings.realtime_presence_url))
            url = urlunsplit((parsed.scheme, parsed.netloc, "/internal/identities", "", ""))
            authorization = _authorization(settings.realtime_presence_secret)
            async with httpx.AsyncClient(trust_env=False, follow_redirects=False,
                                         timeout=TOTAL_TIMEOUT_SECONDS) as client:
                async with client.stream("POST", url, headers={"Authorization": authorization,
                    "Accept-Encoding": "identity"}, json={"device_ids": [device_id]}) as response:
                    if response.status_code != 200 or response.headers.get("content-encoding", "identity") != "identity":
                        return None
                    declared = response.headers.get("content-length")
                    if declared is not None and (not declared.isascii() or not declared.isdecimal()
                                                or int(declared) > MAX_RESPONSE_BYTES):
                        return None
                    body = bytearray()
                    async for chunk in response.aiter_bytes(chunk_size=8192):
                        if len(body) + len(chunk) > MAX_RESPONSE_BYTES:
                            return None
                        body.extend(chunk)
            payload = json.loads(body.decode("utf8"), object_pairs_hook=_unique_members,
                                 parse_constant=_invalid_constant)
            if (not isinstance(payload, dict) or set(payload) != {"version", "sampled_at_ms", "identities"}
                or type(payload["version"]) is not int or payload["version"] != 1
                or not _timestamp(payload["sampled_at_ms"])
                or not -2000 <= _now_ms() - payload["sampled_at_ms"] <= 5000
                or not isinstance(payload["identities"], list) or len(payload["identities"]) != 1):
                return None
            identity = payload["identities"][0]
            if (not isinstance(identity, dict) or set(identity) != {"device_id", "device_key_id", "role"}
                or identity["device_id"] != device_id or identity["role"] not in {"Agent", "Peer"}
                or not isinstance(identity["device_key_id"], str)
                or re.fullmatch(r"[0-9a-f]{64}", identity["device_key_id"], re.ASCII) is None):
                return None
            return identity["device_key_id"]
    except (TimeoutError, httpx.HTTPError, ValueError, TypeError, OverflowError, RecursionError):
        return None
