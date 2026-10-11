from datetime import UTC, datetime, timedelta
from types import SimpleNamespace
from unittest import IsolatedAsyncioTestCase, TestCase
from unittest.mock import AsyncMock, patch

import jwt
from fastapi import HTTPException
from fastapi.security import HTTPAuthorizationCredentials

from app.core.config import settings
from app.core.security import (
    create_access_token,
    get_current_user,
    get_current_user_optional,
)


class SecurityTests(TestCase):
    def test_access_token_roundtrip_with_pyjwt(self) -> None:
        secret = "vY7!qP2@kL9#sX4$mR8%tN6&wC3*eH5-zB1+uD0="
        issuer = "https://auth.rdesk.test"
        audience = "rdesk-api"
        with patch.dict(
            settings.__dict__,
            {
                "jwt_secret": secret,
                "jwt_issuer": issuer,
                "jwt_audience": audience,
                "jwt_max_lifetime_minutes": 60,
                "jwt_future_iat_skew_seconds": 60,
            },
        ):
            token = create_access_token("device-user", "tester", "user")

        payload = jwt.decode(
            token,
            secret,
            algorithms=["HS256"],
            issuer=issuer,
            audience=audience,
        )

        self.assertEqual(payload["sub"], "device-user")
        self.assertEqual(payload["username"], "tester")
        self.assertEqual(payload["role"], "user")


class TokenValidationTests(IsolatedAsyncioTestCase):
    def setUp(self) -> None:
        self.secret = "vY7!qP2@kL9#sX4$mR8%tN6&wC3*eH5-zB1+uD0="
        self.issuer = "https://auth.rdesk.test"
        self.audience = "rdesk-api"
        configured = patch.dict(
            settings.__dict__,
            {
                "jwt_secret": self.secret,
                "jwt_issuer": self.issuer,
                "jwt_audience": self.audience,
                "jwt_max_lifetime_minutes": 60,
                "jwt_future_iat_skew_seconds": 60,
            },
        )
        configured.start()
        self.addCleanup(configured.stop)

    async def test_valid_token_returns_the_matching_user(self) -> None:
        user = SimpleNamespace(id="device-user", role="user", session_version=1)
        db = SimpleNamespace(scalar=AsyncMock(return_value=user))
        token = create_access_token(user.id, "tester", user.role)
        credentials = HTTPAuthorizationCredentials(scheme="Bearer", credentials=token)

        self.assertIs(await get_current_user(credentials, db), user)
        self.assertIs(await get_current_user_optional(credentials, db), user)

    async def test_invalid_tokens_are_rejected_before_database_access(self) -> None:
        now = datetime.now(UTC)
        claims = {
            "sub": "device-user",
            "role": "user",
            "session_version": 1,
            "iss": self.issuer,
            "aud": self.audience,
            "iat": now - timedelta(minutes=2),
            "exp": now + timedelta(minutes=5),
        }
        expired = jwt.encode(
            {**claims, "exp": now - timedelta(seconds=1)}, self.secret, algorithm="HS256"
        )
        bad_signature = jwt.encode(claims, "test-only-wrong-key-" + "x" * 32)
        for token in ("not-a-token", expired, bad_signature):
            with self.subTest(token_kind=token.split(".")[0]):
                db = SimpleNamespace(scalar=AsyncMock())
                credentials = HTTPAuthorizationCredentials(scheme="Bearer", credentials=token)
                with self.assertRaises(HTTPException) as denied:
                    await get_current_user(credentials, db)
                self.assertEqual(denied.exception.status_code, 401)
                self.assertEqual(denied.exception.headers, {"WWW-Authenticate": "Bearer"})
                self.assertIsNone(await get_current_user_optional(credentials, db))
                db.scalar.assert_not_awaited()
