"""Public, secret-free connection metadata authenticated by the HTTPS origin."""

import base64
import re
from urllib.parse import urlsplit

from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from cryptography.hazmat.primitives.serialization import Encoding, PublicFormat
from fastapi import APIRouter, HTTPException

from app.core.config import settings

router = APIRouter(prefix="/public", tags=["public"])


@router.get("/connection-config")
def connection_config() -> dict:
    try:
        api = urlsplit(settings.public_api_url)
        signal = urlsplit(settings.signaling_ws_url)
        if (
            api.scheme != "https"
            or signal.scheme != "wss"
            or not api.hostname
            or api.hostname != signal.hostname
            or (api.port or 443) != (signal.port or 443)
            or any([api.username, api.password, api.query, api.fragment,
                    signal.username, signal.password, signal.query, signal.fragment])
            or re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]{0,63}", settings.relay_directory_signing_key_id) is None
        ):
            raise ValueError("invalid public configuration")
        seed = base64.b64decode(settings.relay_directory_signing_private_key.get_secret_value(), validate=True)
        public_key = Ed25519PrivateKey.from_private_bytes(seed).public_key().public_bytes(Encoding.Raw, PublicFormat.Raw)
        return {
            "signaling_url": settings.signaling_ws_url,
            "signaling_server_device_id": settings.public_signal_server_device_id,
            "signaling_server_key_id": settings.public_signal_server_key_id or None,
            "relay_directory_url": settings.public_api_url.rstrip("/") + "/relays/access",
            "relay_directory_keys": {
                settings.relay_directory_signing_key_id: base64.b64encode(public_key).decode("ascii"),
            },
        }
    except (ValueError, TypeError):
        raise HTTPException(status_code=503, detail={"code": "public_connection_unavailable"}) from None
