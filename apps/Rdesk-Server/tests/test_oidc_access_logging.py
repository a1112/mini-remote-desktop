import io
import logging

from uvicorn.logging import AccessFormatter

from app.core.uvicorn_logging import OidcCallbackAccessFilter, uvicorn_logging_config


def test_builtin_logging_config_redacts_only_callback_query():
    config = uvicorn_logging_config()
    assert config["loggers"]["uvicorn.access"]["filters"] == ["oidc_callback_redaction"]
    assert config["filters"]["oidc_callback_redaction"]["()"] is OidcCallbackAccessFilter
    output = io.StringIO()
    handler = logging.StreamHandler(output)
    handler.setFormatter(AccessFormatter(fmt='%(client_addr)s - "%(request_line)s" %(status_code)s'))
    logger = logging.Logger("oidc-access-test")
    logger.addFilter(OidcCallbackAccessFilter())
    logger.addHandler(handler)
    logger.info('%s - "%s %s HTTP/%s" %d', "127.0.0.1", "GET",
                "/api/v1/auth/oidc/callback?code=secret-code&state=secret-state", "1.1", 200)
    logger.info('%s - "%s %s HTTP/%s" %d', "127.0.0.1", "GET",
                "/api/v1/auth/oidc/callback/?code=redirect-secret", "1.1", 307)
    logger.info('%s - "%s %s HTTP/%s" %d', "127.0.0.1", "GET", "/health?x=ok", "1.1", 200)
    content = output.getvalue()
    assert "secret-code" not in content and "secret-state" not in content and "redirect-secret" not in content
    assert "/api/v1/auth/oidc/callback?[redacted]" in content
    assert "/health?x=ok" in content
    count = len(content)
    logger.info("unexpected /api/v1/auth/oidc/callback?code=another-secret")
    assert len(output.getvalue()) == count
