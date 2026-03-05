from pydantic_settings import BaseSettings, SettingsConfigDict


class Settings(BaseSettings):
    model_config = SettingsConfigDict(
        env_file=".env",
        env_file_encoding="utf-8",
        env_prefix="RDESK_",
        extra="ignore",
    )

    server_host: str = "0.0.0.0"
    server_port: int = 9530
    db_url: str = "postgresql+asyncpg://postgres:519223@127.0.0.1:5432/rdesk_server"
    jwt_secret: str = "change_me_for_production"
    jwt_expire_minutes: int = 60 * 24 * 7
    signaling_ws_url: str = "ws://127.0.0.1:9527"
    cors_origins: str = "http://localhost:5173,http://127.0.0.1:5173"


settings = Settings()
