from typing import Literal
from pydantic import (
    BaseModel,
    ConfigDict,
    Field,
    SecretStr,
    field_validator,
    model_validator,
)
from app.schemas.browser_session import BrowserSessionCreateIn, BrowserSessionOut


class TemporaryAccessPublishIn(BaseModel):
    model_config = ConfigDict(extra="forbid", strict=True)
    key_id: str = Field(pattern=r"^[0-9a-f]{64}$")
    public_key: str = Field(pattern=r"^[0-9a-f]{64}$")
    access_json: str = Field(min_length=1, max_length=8192, repr=False)
    signature: str = Field(pattern=r"^[0-9a-f]{128}$", repr=False)


class TemporaryAccessDocument(BaseModel):
    model_config = ConfigDict(extra="forbid", strict=True)
    device_id: str = Field(min_length=1, max_length=64)
    auth_version: int = Field(ge=1, le=2**63 - 1)
    generation: int = Field(ge=1, le=2**63 - 1)
    enabled: bool
    expires_at_ms: int | None = Field(ge=1, default=None)
    salt: str | None = Field(pattern=r"^[0-9a-f]{32}$", default=None, repr=False)
    verifier: str | None = Field(pattern=r"^[0-9a-f]{64}$", default=None, repr=False)
    allowed_scopes: list[Literal["input.keyboard", "input.pointer", "screen.view"]] = (
        Field(max_length=3)
    )

    @model_validator(mode="after")
    def valid_bundle(self):
        if self.enabled:
            if (
                self.expires_at_ms is None
                or self.salt is None
                or self.verifier is None
                or "screen.view" not in self.allowed_scopes
                or self.allowed_scopes != sorted(set(self.allowed_scopes))
            ):
                raise ValueError("invalid temporary access")
        elif (
            any(
                value is not None
                for value in (self.expires_at_ms, self.salt, self.verifier)
            )
            or self.allowed_scopes
        ):
            raise ValueError("invalid temporary access")
        return self


class TemporaryAccessStatus(BaseModel):
    enabled: bool
    ready: bool
    expires_at_ms: int | None
    generation: int
    reason: str | None = None


class GuestBrowserSessionCreateIn(BrowserSessionCreateIn):
    temporary_password: SecretStr = Field(repr=False)

    @field_validator("temporary_password")
    @classmethod
    def password(cls, value):
        raw = value.get_secret_value()
        if len(raw) != 8 or any(
            c not in "ABCDEFGHJKLMNPQRSTUVWXYZ23456789" for c in raw
        ):
            raise ValueError("invalid temporary access")
        return value


class GuestHTTPCredential(BaseModel):
    token: str = Field(repr=False)
    expires_at_ms: int


class GuestBrowserSessionOut(BrowserSessionOut):
    http_credential: GuestHTTPCredential
