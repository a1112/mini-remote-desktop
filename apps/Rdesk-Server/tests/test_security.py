from unittest import TestCase

import jwt

from app.core.config import settings
from app.core.security import create_access_token


class SecurityTests(TestCase):
    def test_access_token_roundtrip_with_pyjwt(self) -> None:
        token = create_access_token("device-user", "tester", "user")

        payload = jwt.decode(token, settings.jwt_secret, algorithms=["HS256"])

        self.assertEqual(payload["sub"], "device-user")
        self.assertEqual(payload["username"], "tester")
        self.assertEqual(payload["role"], "user")
