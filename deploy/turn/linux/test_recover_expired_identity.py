"""Operator recovery tests; Linux file-boundary tests run as root."""
import importlib.util
import copy
import io
import json
import os
from pathlib import Path
import re
import tempfile
import threading
import unittest
import urllib.error
from unittest.mock import patch

SCRIPT = Path(__file__).with_name("recover-expired-identity.py")
spec = importlib.util.spec_from_file_location("relay_recovery", SCRIPT)
recovery = importlib.util.module_from_spec(spec)
spec.loader.exec_module(recovery)


class CaptureInput(io.BytesIO):
    def close(self):
        if not self.closed:
            self.captured = self.getvalue()
        super().close()


class RecordedOutput(io.BytesIO):
    def read(self, count):
        self.read_limit = count
        return super().read(count)


class CurlProcess:
    def __init__(self, output=b'HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n{}', code=0):
        self.stdin = CaptureInput()
        self.stdout = RecordedOutput(output)
        self.returncode = code
        self.killed = False
        self.finished = False

    def wait(self, timeout=None):
        self.finished = True
        return self.returncode

    def poll(self):
        return self.returncode if self.finished else None

    def kill(self):
        self.killed = True
        self.finished = True


class CurlTransportTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.ca = Path(self.directory.name) / 'trusted-ca.pem'
        self.addCleanup(patch.stopall)
        patch.object(recovery, 'protected_read', return_value=b'PUBLIC CA').start()
        self.trusted = patch.object(recovery, 'trusted_executable').start()
        self.capath_check = patch.object(recovery, 'verify_empty_capath', create=True).start()
        # Let the old urllib implementation return a harmless success: the RED
        # failure must be failure to use the protected curl path, never network.
        patch('ssl.create_default_context').start()
        response = io.BytesIO(b'{}')
        response.headers = {}
        patch.object(recovery.API, '_open', return_value=response).start()

    def call(self, process, **arguments):
        with patch.object(recovery.subprocess, 'Popen', return_value=process) as spawn:
            api = recovery.API('https://127.0.0.1:9443/api/v1/', self.ca)
            result = api.request('relays/enroll', **arguments)
        return result, spawn

    def test_credentials_are_only_stdin_and_tls_uses_only_original_ca(self):
        process = CurlProcess()
        secrets = ['PRIVATE TOKEN', 'PRIVATE RECEIPT', 'PRIVATE REST SECRET']
        (body, headers), spawn = self.call(process, payload={'secret': secrets[2]},
                                          token=secrets[0], receipt=secrets[1])
        spawn.assert_called_once()
        argv = spawn.call_args.args[0]
        options = spawn.call_args.kwargs
        self.assertEqual(argv, ['/usr/bin/curl', '-q', '--config', '-'])
        self.assertIs(options['shell'], False)
        self.assertEqual(options['stderr'], recovery.subprocess.DEVNULL)
        self.assertEqual(body, {})
        config = process.stdin.captured.decode()
        for secret in secrets:
            self.assertIn(secret, config)
            self.assertNotIn(secret, repr(argv))
            self.assertNotIn(secret, repr(options['env']))
        for option in ('cacert = ', 'capath = ', 'proto = "=https"', 'max-time = "30"',
                       'max-filesize = "1048576"', 'proxy = ""', 'noproxy = "*"',
                       'header = "Expect:"'):
            self.assertIn(option, config)
        self.assertNotIn('insecure', config)
        self.assertNotIn('location', config)
        self.trusted.assert_called_with(Path('/usr/bin/curl'))
        self.capath_check.assert_called()

    def test_tls_proxy_keylog_and_loader_environment_are_not_forwarded(self):
        variables = dict(HTTPS_PROXY='proxy-secret', ALL_PROXY='proxy-secret', NO_PROXY='proxy-secret',
                         SSL_CERT_FILE='extra-ca', SSL_CERT_DIR='extra-ca', CURL_CA_BUNDLE='extra-ca',
                         SSLKEYLOGFILE='keylog', CURL_SSL_BACKEND='backend', LD_PRELOAD='loader',
                         CURL_HOME='config-directory')
        with patch.dict(os.environ, variables):
            _, spawn = self.call(CurlProcess())
        spawn.assert_called_once()
        environment = spawn.call_args.kwargs['env']
        for name in variables:
            self.assertNotIn(name, environment)
        self.assertTrue(environment['PATH'])

    def test_response_headers_preserve_duplicates_and_case(self):
        output = (b'HTTP/1.1 202 Accepted\r\nx-relay-renew-at: 100\r\n'
                  b'X-Relay-Renew-At: 101\r\n\r\n{"status":"pending"}')
        (body, headers), spawn = self.call(CurlProcess(output))
        spawn.assert_called_once()
        self.assertEqual(body, {'status': 'pending'})
        self.assertEqual(headers, [('x-relay-renew-at', '100'), ('X-Relay-Renew-At', '101')])

    def test_http_redirect_and_error_do_not_echo_secret_body(self):
        for code in (302, 409, 500):
            with self.subTest(code=code):
                process = CurlProcess(f'HTTP/1.1 {code} Error\r\nLocation: https://other.test\r\n\r\nPRIVATE BODY'.encode())
                with self.assertRaises(recovery.RecoveryError) as caught:
                    self.call(process, payload={'secret': 'PRIVATE BODY'})
                self.assertEqual(str(caught.exception), f'recovery_api_http_{code}')

    def test_malformed_or_multiple_header_blocks_are_rejected(self):
        for response in (b'HTTP/1.1 100 Continue\r\n\r\nHTTP/1.1 200 OK\r\n\r\n{}',
                         b'HTTP/1.1 200 OK\r\n folded: value\r\n\r\n{}',
                         b'HTTP/1.1 200 OK\r\nBad Header: x\r\n\r\n{}',
                         b'HTTP/1.1 200 OK\r\nGood: x\x00SECRET\r\n\r\n{}',
                         b'HTTP/1.1 200 OK\n\n{}'):
            with self.subTest(response=response):
                with self.assertRaisesRegex(recovery.RecoveryError, '^recovery_api_response_invalid$'):
                    self.call(CurlProcess(response))

    def test_oversize_output_is_bounded_and_child_is_reaped(self):
        process = CurlProcess(b'x' * (recovery.MAX_BYTES + 2))
        with self.assertRaisesRegex(recovery.RecoveryError, '^recovery_api_response_too_large$'):
            self.call(process)
        self.assertTrue(process.killed)
        self.assertEqual(process.stdout.read_limit, recovery.MAX_BYTES + 1)

    def test_failed_process_and_launch_errors_are_static(self):
        with self.assertRaisesRegex(recovery.RecoveryError, '^recovery_api_request_failed$'):
            self.call(CurlProcess(b'PRIVATE BODY', code=60))
        with patch.object(recovery.subprocess, 'Popen', side_effect=OSError('PRIVATE ERROR')):
            api = recovery.API('https://127.0.0.1:9443/api/v1/', self.ca)
            with self.assertRaisesRegex(recovery.RecoveryError, '^recovery_api_request_failed$'):
                api.request('relays')

    def test_header_injection_is_refused_before_launch(self):
        for value in ('PRIVATE\r\nX-Injected: yes', 'PRIVATE\x00', 'non-ascii-\u00e9'):
            with self.subTest(value=value), patch.object(recovery.subprocess, 'Popen') as spawn:
                api = recovery.API('https://127.0.0.1:9443/api/v1/', self.ca)
                with self.assertRaisesRegex(recovery.RecoveryError, '^recovery_api_header_invalid$'):
                    api.request('relays', receipt=value)
                spawn.assert_not_called()

    def test_config_quote_preserves_literal_json_backslashes_and_quotes(self):
        process = CurlProcess()
        _, spawn = self.call(process, payload={'csr_pem': 'line one\n"quoted"\\value\n\u00e9'})
        spawn.assert_called_once()
        config = process.stdin.captured.decode()
        line = next(line for line in config.splitlines() if line.startswith('data-binary = '))
        # Decode only curl's documented quoted-value escapes, not JSON escapes.
        raw = line.split(' = ', 1)[1][1:-1]
        decoded = re.sub(r'\\([\\"tnrv])', lambda m: {'t':'\t', 'n':'\n', 'r':'\r', 'v':'\v'}.get(m[1], m[1]), raw)
        self.assertEqual(json.loads(decoded), {'csr_pem': 'line one\n"quoted"\\value\n\u00e9'})

    def test_outer_timeout_kills_and_reaps_child_without_secret_exception(self):
        process = CurlProcess()
        stopped = threading.Event()
        class HangingOutput:
            def read(self, count):
                stopped.wait(2)
                return b''
            def close(self):
                pass
        process.stdout = HangingOutput()
        def kill():
            process.killed = True
            process.finished = True
            stopped.set()
        process.kill = kill
        with patch.object(recovery, 'CURL_TIMEOUT_SECONDS', 0.02, create=True):
            with self.assertRaisesRegex(recovery.RecoveryError, '^recovery_api_request_failed$'):
                self.call(process)
        self.assertTrue(process.killed)

    def test_changed_ca_and_untrusted_executable_refuse_before_launch(self):
        with patch.object(recovery, 'protected_read', side_effect=[b'original CA', b'changed CA']), \
                patch.object(recovery.subprocess, 'Popen') as spawn:
            api = recovery.API('https://127.0.0.1:9443/api/v1/', self.ca)
            with self.assertRaisesRegex(recovery.RecoveryError, '^recovery_api_ca_changed$'):
                api.request('relays')
            spawn.assert_not_called()
        with patch.object(recovery, 'trusted_executable', side_effect=recovery.RecoveryError('recovery_helper_untrusted')), \
                patch.object(recovery.subprocess, 'Popen') as spawn:
            with self.assertRaisesRegex(recovery.RecoveryError, '^recovery_helper_untrusted$'):
                recovery.API('https://127.0.0.1:9443/api/v1/', self.ca)
            spawn.assert_not_called()

    def test_nonempty_ca_directory_refuses_before_launch(self):
        self.capath_check.side_effect = recovery.RecoveryError('recovery_api_trust_unprotected')
        with patch.object(recovery.subprocess, 'Popen') as spawn:
            api = recovery.API('https://127.0.0.1:9443/api/v1/', self.ca)
            with self.assertRaisesRegex(recovery.RecoveryError, '^recovery_api_trust_unprotected$'):
                api.request('relays')
            spawn.assert_not_called()

    def test_process_exit_timeout_is_static_and_reaped(self):
        process = CurlProcess()
        original_wait = process.wait
        calls = []
        def wait(timeout=None):
            calls.append(timeout)
            if len(calls) == 1:
                raise recovery.subprocess.TimeoutExpired('PRIVATE COMMAND', timeout)
            return original_wait(timeout)
        process.wait = wait
        with self.assertRaisesRegex(recovery.RecoveryError, '^recovery_api_request_failed$'):
            self.call(process)
        self.assertTrue(process.killed)
        self.assertEqual(len(calls), 2)

    def test_response_header_limit_is_enforced(self):
        process = CurlProcess(b'HTTP/1.1 200 OK\r\nHeader: ' + b'x' * recovery.MAX_HEADER_BYTES + b'\r\n\r\n{}')
        with self.assertRaisesRegex(recovery.RecoveryError, '^recovery_api_response_invalid$'):
            self.call(process)

    def test_interrupt_during_wait_still_kills_and_reaps_child(self):
        process = CurlProcess()
        with patch.object(recovery.threading.Thread, 'join', side_effect=[KeyboardInterrupt, None]):
            with self.assertRaises(KeyboardInterrupt):
                self.call(process)
        self.assertTrue(process.killed)
        self.assertTrue(process.finished)

    def test_thread_start_failure_still_reaps_child(self):
        process = CurlProcess()
        with patch.object(recovery.threading.Thread, 'start', side_effect=RuntimeError('PRIVATE ERROR')):
            with self.assertRaisesRegex(recovery.RecoveryError, '^recovery_api_request_failed$'):
                self.call(process)
        self.assertTrue(process.killed)
        self.assertTrue(process.finished)


class RecoveryContractTests(unittest.TestCase):
    def setUp(self):
        self.config = dict(node_id="relay-one", region="region-one", failure_domain="domain-one",
                           endpoints=["turn:relay.test:3478?transport=udp"],
                           max_allocations=16, max_egress_bps=200000000,
                           identity_path="/var/lib/mrd-relay-agent/identity.envelope",
                           runtime_state_path="/var/lib/mrd-relay-agent/runtime.envelope")
        self.node = dict(self.config, physical_host_id="host-one", revoked_at=None)
        self.inspect = dict(node_id="relay-one", csr_pem="PUBLIC CSR", identity_epoch=102,
                            request_sequence=4, sealed_sha256="a" * 64)

    def test_enrollment_preserves_original_registration_and_only_uses_original_csr(self):
        body = recovery.build_enrollment(self.config, self.node, self.inspect, "TOKEN", "SECRET")
        self.assertEqual(body, dict(token="TOKEN", turn_rest_secret="SECRET", csr_pem="PUBLIC CSR",
                                    **{k: self.config[k] for k in recovery.REGISTRATION_FIELDS}))
        self.assertNotIn("physical_host_id", body)
        self.assertEqual(self.inspect["request_sequence"], 4)

    def test_mismatched_remote_capacity_or_node_refuses_reenrollment(self):
        for field, value in (("node_id", "other"), ("max_allocations", 17),
                             ("endpoints", ["turn:other.test:3478"])):
            with self.subTest(field=field):
                with self.assertRaises(recovery.RecoveryError):
                    recovery.build_enrollment(self.config, dict(self.node, **{field: value}),
                                              self.inspect, "TOKEN", "SECRET")

    def test_reenrollment_refuses_revoked_node(self):
        with self.assertRaises(recovery.RecoveryError):
            recovery.build_enrollment(self.config, dict(self.node, revoked_at="yesterday"),
                                      self.inspect, "TOKEN", "SECRET")

    def test_pickup_requires_node_match_and_future_renewal_header(self):
        response = dict(node_id="relay-one", status="approved", certificate_pem="LEAF",
                        ca_certificate_pem="CA", expires_at="2030-01-01T00:10:00+00:00")
        now = 1893456000
        bundle = recovery.certificate_bundle(response, {"X-Relay-Renew-At": str(now + 300)},
                                             "relay-one", now)
        self.assertEqual(bundle["renew_at_unix_seconds"], now + 300)
        self.assertEqual(bundle["expires_at_unix_seconds"], now + 600)
        for headers in ({}, {"X-Relay-Renew-At": str(now)},
                        {"X-Relay-Renew-At": str(now + 600)}):
            with self.subTest(headers=headers):
                with self.assertRaises(recovery.RecoveryError):
                    recovery.certificate_bundle(response, headers, "relay-one", now)
        with self.assertRaises(recovery.RecoveryError):
            recovery.certificate_bundle(response, {"X-Relay-Renew-At": str(now + 300)},
                                        "wrong-node", now)

    def test_http_error_does_not_echo_credential_or_response_body(self):
        error = urllib.error.HTTPError("https://relay.test/api", 409, "secret-body", {}, None)
        with patch.object(recovery.API, "_open", side_effect=error):
            with self.assertRaises(recovery.RecoveryError) as caught:
                recovery.API("http://127.0.0.1:9530/api/v1/").request("relays", {"token": "SECRET"})
        self.assertEqual(str(caught.exception), "recovery_api_http_409")
        self.assertNotIn("SECRET", str(caught.exception))

    def test_redirect_is_refused(self):
        with self.assertRaises(urllib.error.HTTPError):
            recovery.NoRedirect().redirect_request(None, None, 302, "redirect", {}, "https://other.test")

    def test_renewal_header_is_case_insensitive_and_refuses_duplicates(self):
        response = dict(node_id="relay-one", status="approved", certificate_pem="LEAF",
                        ca_certificate_pem="CA", expires_at="2030-01-01T00:10:00+00:00")
        now = 1893456000
        bundle = recovery.certificate_bundle(response, {"x-relay-renew-at": str(now + 300)},
                                             "relay-one", now)
        self.assertEqual(bundle["renew_at_unix_seconds"], now + 300)
        with self.assertRaises(recovery.RecoveryError):
            recovery.certificate_bundle(response, [("X-Relay-Renew-At", str(now + 300)),
                                                   ("x-relay-renew-at", str(now + 301))], "relay-one", now)

    def test_retry_dispatched_enrollment_reuses_exact_token_and_payload(self):
        journal = dict(stage="enrollment_dispatched", node_id="relay-one", original_node=self.node,
                       inspect=self.inspect, token="SAME TOKEN")
        body = recovery.build_enrollment(self.config, self.node, self.inspect, "SAME TOKEN", "SECRET")
        journal["enrollment_payload_sha256"] = recovery.payload_sha256(body)
        requests = []
        class Public:
            def request(inner, path, payload=None, **kwargs):
                requests.append((path, payload))
                if path == "relays/enroll":
                    return dict(node_id="relay-one", status="pending", enrollment_id="one", receipt="PRIVATE"), {}
                return dict(node_id="relay-one", status="approved", certificate_pem="LEAF",
                            ca_certificate_pem="CA", expires_at="2030-01-01T00:10:00+00:00"), {"x-relay-renew-at": "1893456300"}
        class Admin:
            def request(inner, *args, **kwargs):
                raise AssertionError("already approved pickup must not approve again or issue a token")
        with patch.object(recovery, "save_journal"), patch.object(recovery, "durable_write"):
            recovery.recover_certificate(journal, self.config, "SECRET", Admin(), Public(), "BEARER",
                                         Path("journal"), Path("certificate"), now=1893456000)
        self.assertEqual(requests[0], ("relays/enroll", body))
        self.assertEqual(journal["stage"], "certificate_received")
        self.assertNotIn("token", journal)

    def test_retry_approval_success_before_journal_uses_receipt_pickup(self):
        journal = dict(stage="enrolled", node_id="relay-one", original_node=self.node,
                       inspect=self.inspect, enrollment_id="one", receipt="PRIVATE")
        calls = []
        class Public:
            def request(inner, path, payload=None, **kwargs):
                calls.append((path, kwargs.get("receipt")))
                return dict(node_id="relay-one", status="approved", certificate_pem="LEAF",
                            ca_certificate_pem="CA", expires_at="2030-01-01T00:10:00+00:00"), {"x-relay-renew-at": "1893456300"}
        class Admin:
            def request(inner, *args, **kwargs):
                raise AssertionError("must not repeat approval after its unknown successful outcome")
        with patch.object(recovery, "save_journal"), patch.object(recovery, "durable_write"):
            recovery.recover_certificate(journal, self.config, "SECRET", Admin(), Public(), "BEARER",
                                         Path("journal"), Path("certificate"), now=1893456000)
        self.assertEqual(calls, [("relays/enrollments/one/pickup", "PRIVATE")])
        self.assertEqual(journal["stage"], "certificate_received")

    def test_lost_approval_response_reconciles_without_second_approval(self):
        journal = dict(stage="enrolled", node_id="relay-one", original_node=self.node,
                       inspect=self.inspect, enrollment_id="one", receipt="PRIVATE")
        state = {"approved": False, "approve_count": 0}
        class Public:
            def request(inner, path, payload=None, **kwargs):
                if not state["approved"]:
                    return dict(node_id="relay-one", status="pending"), []
                return dict(node_id="relay-one", status="approved", certificate_pem="LEAF",
                            ca_certificate_pem="CA", expires_at="2030-01-01T00:10:00+00:00"), [("x-relay-renew-at", "1893456300")]
        class Admin:
            def request(inner, path, payload=None, token=None):
                state["approve_count"] += 1
                state["approved"] = True
                raise recovery.RecoveryError("recovery_api_request_failed")
        with patch.object(recovery, "save_journal"), patch.object(recovery, "durable_write"):
            with self.assertRaises(recovery.RecoveryError):
                recovery.recover_certificate(journal, self.config, "SECRET", Admin(), Public(), "BEARER",
                                             Path("journal"), Path("certificate"), now=1893456000)
            self.assertEqual(journal["stage"], "enrolled")
            recovery.recover_certificate(journal, self.config, "SECRET", Admin(), Public(), "BEARER",
                                         Path("journal"), Path("certificate"), now=1893456000)
        self.assertEqual(state["approve_count"], 1)
        self.assertEqual(journal["stage"], "certificate_received")

    def test_dispatched_retry_refuses_changed_turn_secret_before_api_call(self):
        body = recovery.build_enrollment(self.config, self.node, self.inspect, "TOKEN", "OLD SECRET")
        journal = dict(stage="enrollment_dispatched", node_id="relay-one", original_node=self.node,
                       inspect=self.inspect, token="TOKEN", enrollment_payload_sha256=recovery.payload_sha256(body))
        with patch.object(recovery, "save_journal") as save:
            with self.assertRaisesRegex(recovery.RecoveryError, "retry_payload_changed"):
                recovery.recover_certificate(journal, self.config, "NEW SECRET", None, None, "BEARER",
                                             Path("journal"), Path("certificate"))
        save.assert_not_called()

    def test_start_crash_is_durable_and_retry_does_not_stop_running_agent(self):
        journal = dict(stage="installed", node_id="relay-one", receipt="PRIVATE",
                       result=dict(identity_epoch=102, request_sequence=4))
        saved = []
        commands = []
        with tempfile.TemporaryDirectory() as directory:
            guard = Path(directory) / "guard"
            guard.write_bytes(recovery.GUARD_BODY)
            def command(*args):
                commands.append(args)
                if "start" in args and len([c for c in commands if "start" in c]) == 1:
                    raise SystemExit("simulated crash after successful start")
                return b""
            with patch.object(recovery, "GUARD", guard), patch.object(recovery, "checked", side_effect=command), \
                    patch.object(recovery, "save_journal", side_effect=lambda p, j: saved.append(copy.deepcopy(j))), \
                    patch("builtins.print"):
                with self.assertRaises(SystemExit):
                    recovery.finish_agent(Path("journal"), journal)
                self.assertEqual(saved[-1]["stage"], "starting")
                recovery.finish_agent(Path("journal"), journal)
        self.assertFalse(any("stop" in c for c in commands))
        self.assertEqual(saved[-1]["stage"], "complete")

    def test_started_resume_accepts_normal_sequence_progress_but_not_identity_change(self):
        original = dict(self.inspect, public_key_b64="KEY")
        journal = dict(stage="starting", inspect=original)
        current = dict(original, request_sequence=9, active_certificate_present=True,
                       active_certificate_expires_at_unix_seconds=300)
        self.assertTrue(recovery.started_resume_valid(journal, current, 100))
        self.assertFalse(recovery.started_resume_valid(journal, dict(current, public_key_b64="OTHER"), 100))
        self.assertFalse(recovery.started_resume_valid(journal, dict(current, request_sequence=3), 100))
        self.assertFalse(recovery.started_resume_valid(journal, current, 300))

    def test_installed_resume_only_accepts_exact_validated_identity_and_unchanged_runtime(self):
        inspected = dict(self.inspect, public_key_b64="ORIGINAL PUBLIC KEY")
        journal = dict(stage="installed", inspect=inspected, runtime_sha256="runtime",
                       result={"after_sha256": "new"})
        bundle = dict(renew_at_unix_seconds=200, expires_at_unix_seconds=300)
        self.assertTrue(recovery.installed_resume_valid(journal, "new", "runtime", inspected, bundle, 100))
        self.assertFalse(recovery.installed_resume_valid(journal, "other", "runtime", inspected, bundle, 100))
        self.assertFalse(recovery.installed_resume_valid(journal, "new", "changed", inspected, bundle, 100))
        self.assertFalse(recovery.installed_resume_valid(journal, "new", "runtime",
                                                        dict(inspected, identity_epoch=103), bundle, 100))
        self.assertFalse(recovery.installed_resume_valid(journal, "new", "runtime", inspected, bundle, 200))


@unittest.skipUnless(hasattr(os, "geteuid") and getattr(os, "geteuid", lambda: -1)() == 0,
                     "requires Linux root file ownership")
class ProtectedFileTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="mrd-recovery-test-", dir="/root")
        self.root = Path(self.directory.name)
        self.addCleanup(self.directory.cleanup)

    def test_private_read_refuses_symlink_hardlink_public_mode_and_oversize(self):
        target = self.root / "credential"
        target.write_bytes(b"secret")
        target.chmod(0o600)
        self.assertEqual(recovery.protected_read(target, max_bytes=10), b"secret")
        target.chmod(0o644)
        with self.assertRaises(recovery.RecoveryError):
            recovery.protected_read(target)
        target.chmod(0o600)
        with self.assertRaises(recovery.RecoveryError):
            recovery.protected_read(target, max_bytes=3)
        alias = self.root / "alias"
        alias.symlink_to(target)
        with self.assertRaises(recovery.RecoveryError):
            recovery.protected_read(alias)
        alias.unlink()
        os.link(target, alias)
        with self.assertRaises(recovery.RecoveryError):
            recovery.protected_read(target)

    def test_journal_is_private_atomic_and_preserves_completed_api_stage(self):
        path = self.root / "journal.json"
        original = dict(schema=1, stage="enrolled", receipt="PRIVATE", enrollment_id="one")
        recovery.save_journal(path, original)
        self.assertEqual(json.loads(recovery.protected_read(path)), original)
        self.assertEqual(path.stat().st_mode & 0o777, 0o600)
        recovery.save_journal(path, dict(original, stage="approved"))
        self.assertEqual(json.loads(recovery.protected_read(path))["receipt"], "PRIVATE")

    def test_ca_directory_requires_empty_private_root_owned_unlinked_path(self):
        directory = self.root / 'ca-directory'
        directory.mkdir(mode=0o700)
        recovery.verify_empty_capath(directory)
        extra_ca = directory / '00000000.0'
        extra_ca.write_bytes(b'OTHER TRUST ROOT')
        with self.assertRaisesRegex(recovery.RecoveryError, '^recovery_api_trust_unprotected$'):
            recovery.verify_empty_capath(directory)
        extra_ca.unlink()
        directory.chmod(0o755)
        with self.assertRaises(recovery.RecoveryError):
            recovery.verify_empty_capath(directory)
        directory.chmod(0o700)
        alias = self.root / 'ca-alias'
        alias.symlink_to(directory)
        with self.assertRaises(recovery.RecoveryError):
            recovery.verify_empty_capath(alias)
        os.chown(directory, 65534, 65534)
        with self.assertRaises(recovery.RecoveryError):
            recovery.verify_empty_capath(directory)
        os.chown(directory, 0, 0)
        colon_path = self.root / 'ca:directory'
        colon_path.mkdir(mode=0o700)
        with self.assertRaises(recovery.RecoveryError):
            recovery.verify_empty_capath(colon_path)


if __name__ == "__main__":
    unittest.main()
