import unittest

from pydantic import ValidationError

from app.core.config import Settings


class SecurityConfigurationTests(unittest.TestCase):
    def test_production_rejects_development_defaults(self) -> None:
        with self.assertRaises(ValidationError):
            Settings(_env_file=None, environment="production")

    def test_production_accepts_explicit_secrets(self) -> None:
        settings = Settings(
            _env_file=None,
            environment="production",
            db_url="postgresql+asyncpg://rdesk@db.internal/rdesk",
            jwt_secret="x" * 32,
        )
        self.assertEqual(settings.environment, "production")
        self.assertEqual(settings.jwt_expire_minutes, 60)

    def test_initial_admin_requires_complete_bootstrap_configuration(self) -> None:
        with self.assertRaises(ValidationError):
            Settings(_env_file=None, initial_admin_username="admin")

        with self.assertRaises(ValidationError):
            Settings(
                _env_file=None,
                initial_admin_username="admin",
                initial_admin_email="admin@example.test",
                initial_admin_password="too-short",
            )

    def test_web_topology_uses_distinct_default_ports(self) -> None:
        settings = Settings(_env_file=None)
        self.assertEqual(settings.server_port, 9530)
        self.assertIn(":9542/", settings.signaling_ws_url)
        self.assertIn(":9542/", settings.realtime_server_health_url)


if __name__ == "__main__":
    unittest.main()
