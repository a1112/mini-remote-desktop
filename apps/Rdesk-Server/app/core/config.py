from pathlib import Path
from typing import Literal

from pydantic import Field, model_validator
from pydantic_settings import BaseSettings, SettingsConfigDict


_DEV_DB_URL = "postgresql+asyncpg://postgres@127.0.0.1:5432/rdesk_server"
_DEV_JWT_SECRET = "development-only-secret-change-before-production"
_EXAMPLE_DB_URL = "postgresql+asyncpg://rdesk:replace-me@127.0.0.1:5432/rdesk_server"
_EXAMPLE_JWT_SECRET = "replace-with-at-least-32-random-bytes"


def _repository_root() -> str:
    return str(Path(__file__).resolve().parents[4])


class Settings(BaseSettings):
    model_config = SettingsConfigDict(
        env_file=".env",
        env_file_encoding="utf-8",
        env_prefix="RDESK_",
        extra="ignore",
    )

    environment: Literal["development", "test", "production"] = "development"
    server_host: str = "127.0.0.1"
    server_port: int = 9530
    db_url: str = _DEV_DB_URL
    jwt_secret: str = _DEV_JWT_SECRET
    jwt_expire_minutes: int = 60
    signaling_ws_url: str = "ws://127.0.0.1:9542/ws"
    realtime_server_health_url: str = "http://127.0.0.1:9542/health"
    realtime_server_command: str = "cargo"
    realtime_server_args: str = "run -p realtime-server"
    realtime_server_workdir: str = Field(default_factory=_repository_root)
    cors_origins: str = "http://localhost:9531,http://127.0.0.1:9531"
    turn_urls: str = "turn:127.0.0.1:3478?transport=udp,turn:127.0.0.1:3478?transport=tcp,turns:127.0.0.1:5349?transport=tcp"
    turn_auth_secret: str = ""
    turn_credential_ttl_seconds: int = 600
    development_reload: bool = False

    initial_admin_username: str | None = None
    initial_admin_email: str | None = None
    initial_admin_password: str | None = None
    seed_demo_data: bool = False

    @model_validator(mode="after")
    def validate_security_boundary(self) -> "Settings":
        admin_fields = (
            self.initial_admin_username,
            self.initial_admin_email,
            self.initial_admin_password,
        )
        if any(admin_fields) and not all(admin_fields):
            raise ValueError(
                "RDESK_INITIAL_ADMIN_USERNAME, RDESK_INITIAL_ADMIN_EMAIL, and "
                "RDESK_INITIAL_ADMIN_PASSWORD must be configured together"
            )
        if self.initial_admin_password and len(self.initial_admin_password) < 12:
            raise ValueError("RDESK_INITIAL_ADMIN_PASSWORD must contain at least 12 characters")

        if self.environment == "production":
            if self.db_url in {_DEV_DB_URL, _EXAMPLE_DB_URL}:
                raise ValueError("RDESK_DB_URL must be configured for production")
            if self.jwt_secret in {_DEV_JWT_SECRET, _EXAMPLE_JWT_SECRET} or len(
                self.jwt_secret.encode("utf-8")
            ) < 32:
                raise ValueError(
                    "RDESK_JWT_SECRET must be a production-specific secret of at least 32 bytes"
                )

        return self


settings = Settings()
