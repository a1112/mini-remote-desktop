#!/usr/bin/env python3
"""One-time, root-only same-key relay recovery through the formal APIs.

No database writes, credential output, or normal Agent validation changes.
After a failure retain the private journal and service stop guard for resume.

HTTPS uses the installed curl's standard certificate, name and time checks
with only the protected original CA and an empty private CA directory. This
avoids Python's additional X509_STRICT legacy-AKI rejection; it never uses
insecure TLS options. Curl config and credentials travel only through stdin.
"""
from __future__ import annotations

import argparse
from datetime import datetime
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import subprocess
import tempfile
import threading
import time
import urllib.error
import urllib.parse
import urllib.request

REGISTRATION_FIELDS = ("node_id", "region", "failure_domain", "endpoints",
                       "max_allocations", "max_egress_bps")
UNIT = "mrd-relay-agent.service"
GUARD = Path("/run/systemd/system/mrd-relay-agent.service.d/90-identity-recovery.conf")
GUARD_BODY = b"[Unit]\nConditionPathExists=/run/mrd-relay-identity-recovery-permit-start\n"
PERMIT = Path("/run/mrd-relay-identity-recovery-permit-start")
MAX_BYTES = 1024 * 1024
CURL_TIMEOUT_SECONDS = 30
MAX_HEADER_BYTES = 65536


class RecoveryError(Exception):
    pass


def protected_read(path: Path, max_bytes: int = MAX_BYTES, owner: int = 0) -> bytes:
    path = Path(path)
    if not path.is_absolute() or ".." in path.parts:
        raise RecoveryError("recovery_path_invalid")
    for parent in (path.parent, *path.parents[1:]):
        metadata = parent.lstat()
        if (not stat.S_ISDIR(metadata.st_mode) or metadata.st_uid not in {0, owner}
                or metadata.st_mode & 0o022):
            raise RecoveryError("recovery_parent_unprotected")
    try:
        fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
        with os.fdopen(fd, "rb") as source:
            metadata = os.fstat(source.fileno())
            if (not stat.S_ISREG(metadata.st_mode) or metadata.st_uid != owner
                    or metadata.st_mode & 0o077 or metadata.st_nlink != 1
                    or metadata.st_size > max_bytes):
                raise RecoveryError("recovery_file_unprotected")
            data = source.read(max_bytes + 1)
            if len(data) > max_bytes:
                raise RecoveryError("recovery_file_too_large")
            return data
    except OSError:
        raise RecoveryError("recovery_file_read_failed") from None


def durable_write(path: Path, data: bytes, owner: int = 0, group: int = 0) -> None:
    # Only use this in a previously verified private directory.
    metadata = path.parent.lstat()
    if (not stat.S_ISDIR(metadata.st_mode) or metadata.st_uid != owner
            or metadata.st_mode & 0o077):
        raise RecoveryError("recovery_write_parent_unprotected")
    if path.exists() or path.is_symlink():
        protected_read(path, owner=owner)
    descriptor, temporary = tempfile.mkstemp(prefix=".recovery-", dir=path.parent)
    try:
        with os.fdopen(descriptor, "wb") as output:
            os.fchmod(output.fileno(), 0o600)
            os.fchown(output.fileno(), owner, group)
            output.write(data)
            output.flush()
            os.fsync(output.fileno())
        os.replace(temporary, path)
        directory = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
        try:
            os.fsync(directory)
        finally:
            os.close(directory)
    finally:
        Path(temporary).unlink(missing_ok=True)


def save_journal(path: Path, journal: dict) -> None:
    durable_write(path, json.dumps(journal, separators=(",", ":")).encode())


def build_enrollment(config: dict, node: dict, inspected: dict, token: str, secret: str) -> dict:
    if (inspected.get("node_id") != config.get("node_id") or node.get("revoked_at") is not None
            or any(node.get(k) != config.get(k) for k in REGISTRATION_FIELDS)
            or not node.get("physical_host_id") or not inspected.get("csr_pem")):
        raise RecoveryError("recovery_original_configuration_mismatch")
    return dict(token=token, turn_rest_secret=secret, csr_pem=inspected["csr_pem"],
                **{k: config[k] for k in REGISTRATION_FIELDS})


def certificate_bundle(response: dict, headers, node_id: str, now: int) -> dict:
    try:
        expires = int(datetime.fromisoformat(response["expires_at"].replace("Z", "+00:00")).timestamp())
        pairs = headers.items() if hasattr(headers, "items") else headers
        values = [v for k, v in pairs if k.lower() == "x-relay-renew-at"]
        if len(values) != 1:
            raise ValueError()
        renew = int(values[0])
        if (response["node_id"] != node_id or response["status"] != "approved"
                or not response["certificate_pem"] or not response["ca_certificate_pem"]
                or not now < renew < expires):
            raise ValueError()
    except (KeyError, TypeError, ValueError, OverflowError):
        raise RecoveryError("recovery_certificate_response_invalid") from None
    return dict(certificate_pem=response["certificate_pem"],
                ca_certificate_pem=response["ca_certificate_pem"],
                expires_at_unix_seconds=expires, renew_at_unix_seconds=renew)


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        raise urllib.error.HTTPError(req.full_url if req else "", code, "redirect refused", headers, fp)


def curl_quote(value: str) -> str:
    # Curl config quoting is not JSON quoting: unrecognized backslash escapes
    # are discarded by curl. JSON has already been serialized to one line.
    if any(ord(character) < 32 or ord(character) == 127 for character in value):
        raise RecoveryError("recovery_api_config_invalid")
    return '"' + value.replace('\\', '\\\\').replace('"', '\\"') + '"'


def verify_empty_capath(path: Path) -> None:
    try:
        if not path.is_absolute() or ".." in path.parts or ":" in str(path):
            raise RecoveryError("recovery_api_trust_unprotected")
        for directory in (path, *path.parents):
            metadata = directory.lstat()
            if (not stat.S_ISDIR(metadata.st_mode) or metadata.st_uid != 0
                    or metadata.st_mode & 0o022):
                raise RecoveryError("recovery_api_trust_unprotected")
        if path.lstat().st_mode & 0o077 or any(path.iterdir()):
            raise RecoveryError("recovery_api_trust_unprotected")
    except OSError:
        raise RecoveryError("recovery_api_trust_unprotected") from None


def bounded_curl(config: bytes) -> bytes:
    # No inherited proxy, CA, key-log, curl-config or dynamic-loader settings.
    environment = {"PATH": os.environ.get("PATH", "/usr/bin:/bin"), "LANG": "C", "LC_ALL": "C"}
    try:
        process = subprocess.Popen(["/usr/bin/curl", "-q", "--config", "-"],
                                   shell=False, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                   stderr=subprocess.DEVNULL, env=environment)
    except OSError:
        raise RecoveryError("recovery_api_request_failed") from None
    outcome = {}

    def exchange():
        try:
            process.stdin.write(config)
            process.stdin.close()
            # Includes headers: unlike --max-filesize, this also bounds headers
            # and unknown-length responses on older curl installations.
            outcome["output"] = process.stdout.read(MAX_BYTES + 1)
        except (OSError, ValueError):
            outcome["failed"] = True

    deadline = time.monotonic() + CURL_TIMEOUT_SECONDS
    worker = threading.Thread(target=exchange, daemon=True)
    try:
        worker.start()
        worker.join(CURL_TIMEOUT_SECONDS)
        if worker.is_alive() or outcome.get("failed"):
            raise RecoveryError("recovery_api_request_failed")
        output = outcome.get("output", b"")
        if len(output) > MAX_BYTES:
            raise RecoveryError("recovery_api_response_too_large")
        if process.wait(timeout=max(0.01, deadline - time.monotonic())):
            raise RecoveryError("recovery_api_request_failed")
        return output
    except (OSError, RuntimeError, subprocess.TimeoutExpired):
        raise RecoveryError("recovery_api_request_failed") from None
    finally:
        # Stop the child before closing its pipes: it may be blocked on stdout
        # or still reading stdin. Always reap it, including size/timeout errors.
        try:
            if process.poll() is None:
                process.kill()
            process.wait(timeout=5)
        except (OSError, subprocess.TimeoutExpired):
            raise RecoveryError("recovery_api_request_failed") from None
        finally:
            if worker.ident is not None:
                worker.join(5)
            if not worker.is_alive():
                for pipe in (process.stdin, process.stdout):
                    try:
                        pipe.close()
                    except OSError:
                        pass


def parse_curl_response(output: bytes):
    try:
        head, data = output.split(b"\r\n\r\n", 1)
        if len(head) > MAX_HEADER_BYTES:
            raise ValueError()
        lines = head.split(b"\r\n")
        status = re.fullmatch(rb"HTTP/1\.[01] ([2-5][0-9]{2})(?: [\x20-\x7e]*)?", lines[0])
        if status is None or len(lines) > 257:
            raise ValueError()
        pairs = []
        for line in lines[1:]:
            header = re.fullmatch(rb"([!#$%&'*+.^_`|~0-9A-Za-z-]+):[ \t]*([\t\x20-\x7e\x80-\xff]*)", line)
            if header is None:
                raise ValueError()
            pairs.append((header[1].decode("ascii"), header[2].decode("latin-1").strip(" \t")))
        code = int(status[1])
        if code >= 300:
            raise RecoveryError("recovery_api_http_" + str(code))
        return json.loads(data), pairs
    except (ValueError, UnicodeError):
        raise RecoveryError("recovery_api_response_invalid") from None


class API:
    def __init__(self, base: str, ca: Path | None = None):
        parsed = urllib.parse.urlsplit(base)
        if (parsed.username or parsed.password or parsed.query or parsed.fragment
                or parsed.scheme not in {"http", "https"}
                or parsed.hostname not in {"127.0.0.1", "localhost"}
                or parsed.path not in {"/", "/api/v1/"}):
            raise RecoveryError("recovery_api_destination_invalid")
        if parsed.scheme == "http" and (parsed.hostname != "127.0.0.1" or parsed.port != 9530):
            raise RecoveryError("recovery_api_destination_invalid")
        handlers = [urllib.request.ProxyHandler({}), NoRedirect()]
        self.ca = None
        if parsed.scheme == "https":
            if ca is None:
                raise RecoveryError("recovery_api_ca_required")
            self.ca = Path(ca)
            self.ca_sha256 = hashlib.sha256(protected_read(self.ca, max_bytes=65536)).digest()
            trusted_executable(Path("/usr/bin/curl"))
        self.opener = urllib.request.build_opener(*handlers)
        self.base = base.rstrip("/") + ("/api/v1/" if parsed.path == "/" else "/")

    def _open(self, request):
        return self.opener.open(request, timeout=30)

    def request(self, path: str, payload: dict | None = None, token: str | None = None,
                receipt: str | None = None):
        if not re.fullmatch(r"[A-Za-z0-9._/-]+", path) or ".." in path or path.startswith("/"):
            raise RecoveryError("recovery_api_path_invalid")
        headers = {"Content-Type": "application/json"}
        if token:
            headers["Authorization"] = "Bearer " + token
        if receipt:
            headers["X-Relay-Enrollment-Receipt"] = receipt
        if self.ca is not None:
            return self._curl_request(path, payload, headers)
        request = urllib.request.Request(self.base + path, headers=headers,
                                         data=json.dumps(payload).encode() if payload is not None else None,
                                         method="POST" if payload is not None else "GET")
        try:
            with self._open(request) as response:
                data = response.read(MAX_BYTES + 1)
                if len(data) > MAX_BYTES:
                    raise RecoveryError("recovery_api_response_too_large")
                return json.loads(data), list(response.headers.items())
        except urllib.error.HTTPError as error:
            raise RecoveryError("recovery_api_http_" + str(error.code)) from None
        except (urllib.error.URLError, TimeoutError, ValueError):
            raise RecoveryError("recovery_api_request_failed") from None

    def _curl_request(self, path: str, payload: dict | None, headers: dict):
        if any(not re.fullmatch(r"[\x20-\x7e]+", value) for value in headers.values()):
            raise RecoveryError("recovery_api_header_invalid")
        if hashlib.sha256(protected_read(self.ca, max_bytes=65536)).digest() != self.ca_sha256:
            raise RecoveryError("recovery_api_ca_changed")
        body = json.dumps(payload, ensure_ascii=True) if payload is not None else None
        if body is not None and len(body) > MAX_BYTES:
            raise RecoveryError("recovery_api_request_too_large")
        # Use the CA's already verified parent, never a shared temporary root.
        with tempfile.TemporaryDirectory(prefix=".recovery-curl-ca-", dir=self.ca.parent) as directory:
            capath = Path(directory)
            verify_empty_capath(capath)
            options = [("url", self.base + path), ("cacert", str(self.ca)), ("capath", str(capath)),
                       ("proto", "=https"), ("proto-redir", "=https"), ("proxy", ""), ("noproxy", "*"),
                       ("max-time", "30"), ("connect-timeout", "10"), ("max-filesize", str(MAX_BYTES)),
                       ("request", "POST" if body is not None else "GET"), ("header", "Expect:")]
            options.extend(("header", name + ": " + value) for name, value in headers.items())
            if body is not None:
                options.append(("data-binary", body))
            config = "silent\ninclude\nhttp1.1\n" + "".join(name + " = " + curl_quote(value) + "\n"
                                                               for name, value in options)
            return parse_curl_response(bounded_curl(config.encode("utf-8")))


def checked(*arguments: str, timeout: int = 60) -> bytes:
    try:
        result = subprocess.run(arguments, capture_output=True, timeout=timeout, check=False)
    except (OSError, subprocess.TimeoutExpired):
        raise RecoveryError("recovery_process_failed") from None
    if result.returncode or len(result.stdout) > MAX_BYTES:
        raise RecoveryError("recovery_process_failed")
    return result.stdout


def trusted_executable(path: Path) -> None:
    if not path.is_absolute() or ".." in path.parts:
        raise RecoveryError("recovery_helper_untrusted")
    for parent in path.parents:
        metadata = parent.lstat()
        if not stat.S_ISDIR(metadata.st_mode) or metadata.st_uid != 0 or metadata.st_mode & 0o022:
            raise RecoveryError("recovery_helper_untrusted")
    metadata = path.lstat()
    if (not stat.S_ISREG(metadata.st_mode) or metadata.st_uid != 0 or metadata.st_mode & 0o022
            or metadata.st_nlink != 1 or not metadata.st_mode & 0o111):
        raise RecoveryError("recovery_helper_untrusted")


def installed_resume_valid(journal: dict, current_sha: str, runtime_sha: str,
                           inspected: dict, bundle: dict, now: int) -> bool:
    original = journal["inspect"]
    result = journal["result"]
    return (journal["stage"] == "installed" and current_sha == result["after_sha256"]
            and runtime_sha == journal["runtime_sha256"]
            and all(inspected[k] == original[k] for k in
                    ("node_id", "public_key_b64", "identity_epoch", "request_sequence"))
            and now < bundle["renew_at_unix_seconds"] < bundle["expires_at_unix_seconds"])


def started_resume_valid(journal: dict, inspected: dict, now: int) -> bool:
    original = journal["inspect"]
    return (journal["stage"] == "starting"
            and all(inspected[k] == original[k] for k in ("node_id", "public_key_b64", "identity_epoch"))
            and inspected["request_sequence"] >= original["request_sequence"]
            and inspected.get("active_certificate_present") is True
            and inspected.get("active_certificate_expires_at_unix_seconds", 0) > now)


def finish_agent(journal_path: Path, journal: dict) -> None:
    if (GUARD.is_symlink() or (GUARD.exists() and GUARD.read_bytes() != GUARD_BODY)
            or (journal["stage"] == "installed" and not GUARD.exists())):
        raise RecoveryError("recovery_stop_guard_conflict")
    # Persist before allowing normal Agent to change its sequence/runtime. A retry
    # of this stage must never recreate the guard or stop an already-running Agent.
    journal["stage"] = "starting"
    save_journal(journal_path, journal)
    GUARD.unlink(missing_ok=True)
    checked("/usr/bin/systemctl", "daemon-reload")
    checked("/usr/bin/systemctl", "reset-failed", UNIT)
    checked("/usr/bin/systemctl", "start", UNIT)
    journal["stage"] = "complete"
    journal.pop("receipt", None)
    save_journal(journal_path, journal)
    result = journal["result"]
    print(json.dumps(dict(installed=True, node_id=journal["node_id"], identity_epoch=result["identity_epoch"],
                          request_sequence=result["request_sequence"], agent_started=True,
                          rotation_verified=False)))


def payload_sha256(payload: dict) -> str:
    return hashlib.sha256(json.dumps(payload, sort_keys=True, separators=(",", ":")).encode()).hexdigest()


def recover_certificate(journal: dict, config: dict, secret: str, admin_api: API, public_api: API,
                        bearer: str, journal_path: Path, certificate_path: Path, now: int | None = None) -> None:
    node_id = journal["node_id"]
    if journal["stage"] == "prepared":
        issued, _ = admin_api.request("relays/enrollment-tokens", {"ttl_seconds": 600}, bearer)
        journal.update(stage="token_created", token=issued["token"])
        save_journal(journal_path, journal)
    if journal["stage"] in {"token_created", "enrollment_dispatched"}:
        body = build_enrollment(config, journal["original_node"], journal["inspect"], journal["token"], secret)
        digest = payload_sha256(body)
        if journal["stage"] == "enrollment_dispatched" and journal["enrollment_payload_sha256"] != digest:
            raise RecoveryError("recovery_enrollment_retry_payload_changed")
        journal.update(stage="enrollment_dispatched", enrollment_payload_sha256=digest)
        save_journal(journal_path, journal)
        # The backend idempotently returns the original receipt for this same
        # token + request digest, including a used token after a lost response.
        enrolled, _ = public_api.request("relays/enroll", body)
        if (enrolled["node_id"] != node_id or enrolled["status"] != "pending"
                or not enrolled["enrollment_id"] or not enrolled["receipt"]):
            raise RecoveryError("recovery_enrollment_response_invalid")
        journal.update(stage="enrolled", enrollment_id=enrolled["enrollment_id"], receipt=enrolled["receipt"])
        journal.pop("token", None)
        save_journal(journal_path, journal)
    if journal["stage"] in {"enrolled", "approved"}:
        path = "relays/enrollments/" + journal["enrollment_id"] + "/pickup"
        picked, headers = public_api.request(path, {}, receipt=journal["receipt"])
        if picked.get("node_id") != node_id or picked.get("status") not in {"pending", "approved"}:
            raise RecoveryError("recovery_pickup_response_invalid")
        if picked["status"] == "pending":
            if journal["stage"] != "enrolled":
                raise RecoveryError("recovery_approval_outcome_mismatch")
            node = journal["original_node"]
            approved, _ = admin_api.request("relays/" + node_id + "/approve",
                                           {"failure_domain": node["failure_domain"],
                                            "physical_host_id": node["physical_host_id"]}, bearer)
            if approved["node_id"] != node_id or approved["status"] != "approved":
                raise RecoveryError("recovery_approval_response_invalid")
            journal["stage"] = "approved"
            save_journal(journal_path, journal)
            picked, headers = public_api.request(path, {}, receipt=journal["receipt"])
        bundle = certificate_bundle(picked, headers, node_id, int(time.time()) if now is None else now)
        durable_write(certificate_path, json.dumps(bundle).encode())
        journal["stage"] = "certificate_received"
        save_journal(journal_path, journal)
    if journal["stage"] != "certificate_received":
        raise RecoveryError("recovery_journal_stage_invalid")


def private_directory(path: Path, owner: int = 0, group: int = 0) -> None:
    if not path.is_absolute() or ".." in path.parts:
        raise RecoveryError("recovery_path_invalid")
    # Refuse an existing unsafe directory rather than chmod/chown it.
    if not path.exists():
        path.mkdir(mode=0o700)
        os.chown(path, owner, group)
    metadata = path.lstat()
    if (not stat.S_ISDIR(metadata.st_mode) or metadata.st_uid != owner
            or metadata.st_mode & 0o077):
        raise RecoveryError("recovery_directory_unprotected")
    for parent in path.parents:
        info = parent.lstat()
        if not stat.S_ISDIR(info.st_mode) or info.st_uid not in {0, owner} or info.st_mode & 0o022:
            raise RecoveryError("recovery_parent_unprotected")


def run_worker(helper: Path, config: Path, command: str, ca: Path | None = None,
               certificate: Path | None = None, original_sha: str | None = None,
               backup: Path | None = None) -> dict:
    suffix = os.urandom(6).hex()
    unit = "mrd-relay-identity-" + suffix
    credential_root = "/run/credentials/" + unit + ".service/"
    args = ["/usr/bin/systemd-run", "--quiet", "--wait", "--pipe", "--collect", "--unit=" + unit,
            "-p", "User=mrd-relay", "-p", "Group=mrd-relay",
            "-p", "LoadCredential=agent-config:" + str(config)]
    worker = [str(helper), command, "--config", credential_root + "agent-config"]
    if command == "install":
        args.extend(["-p", "LoadCredential=trusted-ca:" + str(ca),
                     "-p", "LoadCredential=node-certificate:" + str(certificate)])
        worker.extend(["--trusted-ca", credential_root + "trusted-ca",
                       "--certificate-json", credential_root + "node-certificate",
                       "--expected-sealed-sha256", original_sha, "--backup-file", str(backup)])
    try:
        value = json.loads(checked(*(args + worker)))
        if not isinstance(value, dict):
            raise ValueError()
        return value
    except (ValueError, TypeError):
        raise RecoveryError("recovery_worker_response_invalid") from None


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--apply", action="store_true")
    parser.add_argument("--node-id", required=True)
    parser.add_argument("--helper", type=Path, required=True)
    parser.add_argument("--config", type=Path, default=Path("/etc/mrd-relay-agent/agent.json"))
    parser.add_argument("--admin", type=Path, default=Path("/root/mrd-public-connectivity-20261005/credentials/admin.json"))
    parser.add_argument("--directory", type=Path, default=Path("/root/mrd-relay-identity-recovery-20261011"))
    args = parser.parse_args()
    if os.geteuid() != 0 or not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]{0,127}", args.node_id):
        raise RecoveryError("recovery_context_invalid")
    trusted_executable(args.helper)
    config_bytes = protected_read(args.config, max_bytes=65536)
    config = json.loads(config_bytes)
    if config["node_id"] != args.node_id or config["target"] != "linux-systemd":
        raise RecoveryError("recovery_context_invalid")
    import pwd
    account = pwd.getpwnam("mrd-relay")
    identity = Path(config["identity_path"])
    runtime = Path(config["runtime_state_path"])
    ca = Path("/etc/mrd-relay-agent/secrets/trusted-ca.pem")
    secret_path = Path("/etc/mrd-relay-agent/secrets/turn-rest-secret")
    secret = protected_read(secret_path, max_bytes=512).decode().strip()
    credentials = json.loads(protected_read(args.admin, max_bytes=16384))
    admin_api = API("http://127.0.0.1:9530/api/v1/")
    login, _ = admin_api.request("auth/login", {"username": credentials["username"], "password": credentials["password"]})
    bearer = login["access_token"]
    nodes, _ = admin_api.request("relays", token=bearer)
    matches = [n for n in nodes if n["node_id"] == args.node_id]
    if len(matches) != 1:
        raise RecoveryError("recovery_original_node_missing")
    node = matches[0]
    inspected = run_worker(args.helper, args.config, "inspect")
    build_enrollment(config, node, inspected, "preflight-only", secret)
    if not args.apply:
        print(json.dumps(dict(preflight=True, node_id=args.node_id, identity_epoch=inspected["identity_epoch"],
                              request_sequence=inspected["request_sequence"])))
        return
    private_directory(args.directory)
    journal_path = args.directory / "journal.json"
    journal = json.loads(protected_read(journal_path)) if journal_path.exists() else {}
    config_sha = hashlib.sha256(config_bytes).hexdigest()
    if journal and (journal.get("schema") != 1 or journal.get("node_id") != args.node_id
                    or journal.get("config_sha256") != config_sha):
        raise RecoveryError("recovery_journal_context_mismatch")
    if not journal and (not inspected.get("active_certificate_present")
                        or inspected.get("active_certificate_expires_at_unix_seconds", 0) > int(time.time())):
        raise RecoveryError("recovery_original_certificate_not_expired")
    if journal.get("stage") == "complete":
        print(json.dumps(dict(installed=True, node_id=args.node_id, resumed_complete=True)))
        return
    if journal.get("stage") == "starting":
        if not started_resume_valid(journal, inspected, int(time.time())):
            raise RecoveryError("recovery_started_identity_mismatch")
        finish_agent(journal_path, journal)
        return
    # Persist an explicit start condition so manual/automatic restarts cannot race the identity write.
    if PERMIT.exists() or PERMIT.is_symlink():
        raise RecoveryError("recovery_stop_guard_conflict")
    GUARD.parent.mkdir(mode=0o755, parents=True, exist_ok=True)
    for parent in GUARD.parents:
        metadata = parent.lstat()
        if not stat.S_ISDIR(metadata.st_mode) or metadata.st_uid != 0 or metadata.st_mode & 0o022:
            raise RecoveryError("recovery_stop_guard_unprotected")
    if GUARD.exists():
        info = GUARD.lstat()
        if (not stat.S_ISREG(info.st_mode) or info.st_uid != 0 or info.st_mode & 0o022
                or info.st_nlink != 1 or GUARD.read_bytes() != GUARD_BODY):
            raise RecoveryError("recovery_stop_guard_conflict")
    else:
        fd = os.open(GUARD, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o644)
        with os.fdopen(fd, "wb") as output:
            output.write(GUARD_BODY)
            output.flush()
            os.fsync(output.fileno())
    checked("/usr/bin/systemctl", "daemon-reload")
    checked("/usr/bin/systemctl", "stop", UNIT)
    if checked("/usr/bin/systemctl", "show", UNIT, "--property=MainPID", "--value").strip() != b"0":
        raise RecoveryError("recovery_agent_not_stopped")
    identity_bytes = protected_read(identity, owner=account.pw_uid)
    runtime_bytes = protected_read(runtime, owner=account.pw_uid)
    original_sha = hashlib.sha256(identity_bytes).hexdigest()
    if journal.get("stage") == "installed":
        bundle = json.loads(protected_read(args.directory / "certificate.json"))
        if not installed_resume_valid(journal, original_sha, hashlib.sha256(runtime_bytes).hexdigest(),
                                      inspected, bundle, int(time.time())):
            raise RecoveryError("recovery_installed_outcome_mismatch")
        finish_agent(journal_path, journal)
        return
    if not journal:
        if original_sha != inspected["sealed_sha256"]:
            raise RecoveryError("recovery_identity_changed")
        durable_write(args.directory / "identity.original", identity_bytes)
        durable_write(args.directory / "runtime.original", runtime_bytes)
        journal = dict(schema=1, stage="prepared", node_id=args.node_id, config_sha256=config_sha,
                       original_sha256=original_sha, runtime_sha256=hashlib.sha256(runtime_bytes).hexdigest(),
                       inspect=inspected, original_node=node)
        save_journal(journal_path, journal)
    if (original_sha != journal["original_sha256"]
            or hashlib.sha256(runtime_bytes).hexdigest() != journal["runtime_sha256"]
            or hashlib.sha256(protected_read(args.directory / "identity.original")).hexdigest() != original_sha):
        raise RecoveryError("recovery_identity_changed_or_install_outcome_unknown")
    public_api = API(config["backend_url"], ca=ca)
    recover_certificate(journal, config, secret, admin_api, public_api, bearer,
                        journal_path, args.directory / "certificate.json")
    stage_dir = identity.parent / "recovery-20261011"
    private_directory(stage_dir, account.pw_uid, account.pw_gid)
    worker_backup = stage_dir / "identity.original"
    durable_write(worker_backup, protected_read(args.directory / "identity.original"), account.pw_uid, account.pw_gid)
    installed = run_worker(args.helper, args.config, "install", ca, args.directory / "certificate.json",
                           journal["original_sha256"], worker_backup)
    if (installed.get("installed") is not True or installed["node_id"] != args.node_id
            or installed["identity_epoch"] != journal["inspect"]["identity_epoch"]
            or installed["request_sequence"] != journal["inspect"]["request_sequence"]):
        raise RecoveryError("recovery_install_response_invalid")
    if hashlib.sha256(protected_read(runtime, owner=account.pw_uid)).hexdigest() != journal["runtime_sha256"]:
        raise RecoveryError("recovery_runtime_changed")
    journal.update(stage="installed", result=installed)
    save_journal(journal_path, journal)
    finish_agent(journal_path, journal)


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        code = str(error) if isinstance(error, RecoveryError) else "recovery_failed"
        raise SystemExit(code) from None
