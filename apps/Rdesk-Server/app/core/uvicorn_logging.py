"""Keep OAuth callback query parameters out of built-in Uvicorn access logs."""
from copy import deepcopy
import logging

from uvicorn.config import LOGGING_CONFIG


class OidcCallbackAccessFilter(logging.Filter):
    def filter(self, record: logging.LogRecord) -> bool:
        # Uvicorn's h11 and httptools access loggers both use this exact
        # five-argument shape; change only the request target, not its status.
        if isinstance(record.args, tuple) and len(record.args) == 5 and isinstance(record.args[2], str):
            path, separator, _query = record.args[2].partition("?")
            if separator and path.rstrip("/").endswith("/api/v1/auth/oidc/callback"):
                args = list(record.args)
                args[2] = path + "?[redacted]"
                record.args = tuple(args)
        elif "/api/v1/auth/oidc/callback" in record.getMessage():
            # Fail closed if a future Uvicorn version changes its log shape.
            return False
        return True


def uvicorn_logging_config() -> dict:
    config = deepcopy(LOGGING_CONFIG)
    config["filters"] = {"oidc_callback_redaction": {"()": OidcCallbackAccessFilter}}
    config["loggers"]["uvicorn.access"]["filters"] = ["oidc_callback_redaction"]
    return config
