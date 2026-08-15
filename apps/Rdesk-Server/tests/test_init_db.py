import unittest
from unittest.mock import patch

import app.models.device_network_group  # noqa: F401
import app.models.network_group  # noqa: F401
from app.db.init_db import seed_initial_data
from app.core.config import settings
from app.models.device import Device


class FakeSession:
    def __init__(self, scalar_results):
        self.scalar_results = iter(scalar_results)
        self.added = []
        self.added_batches = []
        self.commit_count = 0

    async def scalar(self, _statement):
        return next(self.scalar_results)

    def add(self, value):
        self.added.append(value)

    def add_all(self, values):
        self.added_batches.append(list(values))

    async def commit(self):
        self.commit_count += 1


class InitialDataTests(unittest.IsolatedAsyncioTestCase):
    async def test_demo_seed_is_idempotent_without_an_admin(self):
        session = FakeSession([None, None, None, object()])

        with (
            patch.object(settings, "initial_admin_username", None),
            patch.object(settings, "initial_admin_email", None),
            patch.object(settings, "initial_admin_password", None),
            patch.object(settings, "seed_demo_data", True),
        ):
            await seed_initial_data(session)
            await seed_initial_data(session)

        self.assertEqual(len(session.added_batches), 1)
        self.assertTrue(all(isinstance(device, Device) for device in session.added_batches[0]))
        self.assertEqual(session.commit_count, 1)

    async def test_existing_user_does_not_block_first_demo_seed(self):
        session = FakeSession([object(), None])

        with (
            patch.object(settings, "initial_admin_username", None),
            patch.object(settings, "initial_admin_email", None),
            patch.object(settings, "initial_admin_password", None),
            patch.object(settings, "seed_demo_data", True),
        ):
            await seed_initial_data(session)

        self.assertEqual(len(session.added_batches), 1)
        self.assertEqual(session.commit_count, 1)


if __name__ == "__main__":
    unittest.main()
