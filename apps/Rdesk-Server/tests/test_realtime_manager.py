import json
import unittest
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
from threading import Barrier, BrokenBarrierError, Lock
from unittest.mock import MagicMock, patch

from app.services.realtime_manager import RealtimeSidecarManager


REALTIME_HEALTH_PAYLOAD = (
    Path(__file__).resolve().parents[2]
    / "realtime-server/tests/fixtures/health.json"
).read_bytes()


def _health_payload(**overrides: object) -> bytes:
    payload = json.loads(REALTIME_HEALTH_PAYLOAD)
    payload.update(overrides)
    return json.dumps(payload).encode("utf-8")


def _health_response(payload: bytes) -> MagicMock:
    response = MagicMock()
    response.read.return_value = payload
    context = MagicMock()
    context.__enter__.return_value = response
    context.__exit__.return_value = False
    return context


class RealtimeManagerTests(unittest.TestCase):
    def manager(self) -> RealtimeSidecarManager:
        return RealtimeSidecarManager(
            health_url="http://127.0.0.1:9542/health",
            command=["realtime-server"],
            workdir=".",
        )

    @patch("app.services.realtime_manager.urlopen")
    def test_accepts_health_payload_emitted_by_realtime_server(
        self, urlopen: MagicMock
    ) -> None:
        urlopen.return_value = _health_response(REALTIME_HEALTH_PAYLOAD)
        status = self.manager().status()
        self.assertTrue(status.reachable)
        self.assertEqual(status.status, "ok")

    @patch("app.services.realtime_manager.urlopen")
    def test_accepts_newer_primary_version_with_required_protocols(
        self, urlopen: MagicMock
    ) -> None:
        urlopen.return_value = _health_response(
            _health_payload(protocol_version=4, supported_protocol_versions=[2, 3, 4])
        )
        self.assertTrue(self.manager().status().reachable)

    @patch("app.services.realtime_manager.urlopen")
    def test_rejects_health_response_from_wrong_local_service(
        self, urlopen: MagicMock
    ) -> None:
        urlopen.return_value = _health_response(
            _health_payload(service="mrd-service")
        )
        status = self.manager().status()
        self.assertFalse(status.reachable)
        self.assertEqual(status.status, "unexpected-service")

    @patch("app.services.realtime_manager.urlopen")
    def test_rejects_incompatible_protocol_version(self, urlopen: MagicMock) -> None:
        for overrides in [
            {"protocol_version": 1, "supported_protocol_versions": [1]},
            {"protocol_version": 2, "supported_protocol_versions": [2]},
            {"protocol_version": 3, "supported_protocol_versions": [3]},
            {"protocol_version": 4, "supported_protocol_versions": [2, 3]},
            {"protocol_version": True, "supported_protocol_versions": [True, 2, 3]},
            {"supported_protocol_versions": ["2", "3"]},
            {"supported_protocol_versions": [2, 3, True]},
            {"supported_protocol_versions": "2,3"},
            {"supported_protocol_versions": None},
        ]:
            with self.subTest(overrides=overrides):
                urlopen.return_value = _health_response(_health_payload(**overrides))
                status = self.manager().status()
                self.assertFalse(status.reachable)
                self.assertEqual(status.status, "unexpected-service")

    @patch("app.services.realtime_manager.urlopen")
    def test_rejects_health_without_advertised_protocols(
        self, urlopen: MagicMock
    ) -> None:
        urlopen.return_value = _health_response(
            b'{"status":"ok","service":"realtime-server","protocol_version":3}'
        )
        self.assertFalse(self.manager().status().reachable)

    @patch("app.services.realtime_manager.urlopen")
    def test_rejects_non_object_health_payload(self, urlopen: MagicMock) -> None:
        urlopen.return_value = _health_response(b"[]")
        status = self.manager().status()
        self.assertFalse(status.reachable)
        self.assertEqual(status.status, "unexpected-service")

    @patch("app.services.realtime_manager.urlopen")
    def test_concurrent_starts_spawn_at_most_one_process(
        self, urlopen: MagicMock
    ) -> None:
        urlopen.return_value = _health_response(REALTIME_HEALTH_PAYLOAD)
        rendezvous = Barrier(2)
        calls: list[object] = []
        calls_lock = Lock()

        def spawn(_command: list[str], _cwd: Path):
            process = MagicMock()
            process.poll.return_value = None
            process.pid = 4200 + len(calls)
            with calls_lock:
                calls.append(process)
            try:
                rendezvous.wait(timeout=0.2)
            except BrokenBarrierError:
                pass
            return process

        manager = RealtimeSidecarManager(
            health_url="http://127.0.0.1:9542/health",
            command=["realtime-server"],
            workdir=".",
            spawner=spawn,
        )
        with ThreadPoolExecutor(max_workers=2) as executor:
            statuses = list(executor.map(lambda _: manager.start(), range(2)))

        self.assertEqual(len(calls), 1)
        self.assertTrue(all(status.running for status in statuses))


if __name__ == "__main__":
    unittest.main()
