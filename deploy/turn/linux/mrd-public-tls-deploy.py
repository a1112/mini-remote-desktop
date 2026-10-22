#!/usr/bin/env python3
"""Deploy a renewed public IP certificate through the relay's restart owner."""
from __future__ import annotations

import json
import hashlib
import os
from pathlib import Path
import stat
import subprocess
import tempfile
import urllib.error
import urllib.request

LINEAGE = Path("/etc/letsencrypt/live/mrd-public-ip")
TLS_DIRECTORY = Path("/etc/mrd-relay-agent/tls")
CREDENTIALS = Path("/root/mrd-public-connectivity-20261005/credentials/admin.json")
NODE_ID = "relay-tencent-gz-1"
PUBLIC_IP = "175.178.16.90"
DEPLOYED_MARKER = Path("/var/lib/mrd-public-tls/deployed.sha256")


def protected_read(path: Path) -> bytes:
    for parent in (path.parent, *path.parents[1:]):
        metadata = parent.lstat()
        if not stat.S_ISDIR(metadata.st_mode) or metadata.st_uid != 0 or metadata.st_mode & 0o022:
            raise RuntimeError("unprotected credential parent")
    descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW)
    with os.fdopen(descriptor, "rb") as source:
        metadata = os.fstat(source.fileno())
        if not stat.S_ISREG(metadata.st_mode) or metadata.st_uid != 0 or metadata.st_mode & 0o077:
            raise RuntimeError("unprotected credential file")
        if metadata.st_size > 16384:
            raise RuntimeError("credential file too large")
        return source.read(16385)


def checked(*arguments: str) -> bytes:
    result = subprocess.run(arguments, capture_output=True, check=False)
    if result.returncode:
        raise RuntimeError("certificate or service validation failed")
    return result.stdout


def api(path: str, payload: dict, token: str | None = None) -> dict:
    headers = {"Content-Type": "application/json"}
    if token:
        headers["Authorization"] = "Bearer " + token
    request = urllib.request.Request(
        "http://127.0.0.1:9530/api/v1/" + path,
        data=json.dumps(payload).encode(), headers=headers, method="POST",
    )
    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            return json.load(response)
    except urllib.error.HTTPError as error:
        raise RuntimeError("relay certificate rotation request failed") from None


def main() -> None:
    if os.geteuid() != 0 or os.environ.get("RENEWED_LINEAGE") != str(LINEAGE):
        raise RuntimeError("unexpected certificate deployment context")
    fullchain, private_key = LINEAGE / "fullchain.pem", LINEAGE / "privkey.pem"
    checked("/usr/bin/openssl", "verify", "-verify_ip", PUBLIC_IP, "-CApath", "/etc/ssl/certs",
            "-untrusted", str(fullchain), str(fullchain))
    checked("/usr/bin/openssl", "x509", "-in", str(fullchain), "-checkend", "86400", "-noout")
    if (checked("/usr/bin/openssl", "x509", "-in", str(fullchain), "-pubkey", "-noout")
            != checked("/usr/bin/openssl", "pkey", "-in", str(private_key), "-pubout")):
        raise RuntimeError("certificate key differs")
    fingerprint = hashlib.sha256(fullchain.read_bytes()).hexdigest()
    if DEPLOYED_MARKER.exists() and protected_read(DEPLOYED_MARKER).decode().strip() == fingerprint:
        print("public_certificate_deployment_current")
        return
    checked("/usr/sbin/nginx", "-t")
    checked("/usr/bin/systemctl", "reload", "nginx.service")
    if not TLS_DIRECTORY.is_dir():
        print("public_certificate_deployed_no_relay")
        return
    credentials = json.loads(protected_read(CREDENTIALS))
    login = api("auth/login", {"username": credentials["username"], "password": credentials["password"]})
    # Validate credentials before replacing source files. systemd's running
    # LoadCredential snapshots stay valid until the broker's bounded restart.
    for source, target_name in ((fullchain, "fullchain.pem"), (private_key, "privkey.pem")):
        descriptor, temporary_name = tempfile.mkstemp(prefix=".renew-", dir=TLS_DIRECTORY)
        try:
            with os.fdopen(descriptor, "wb") as output:
                output.write(source.read_bytes())
                output.flush()
                os.fsync(output.fileno())
            os.replace(temporary_name, TLS_DIRECTORY / target_name)
        finally:
            Path(temporary_name).unlink(missing_ok=True)
    api("relays/" + NODE_ID + "/rotate-secret", {"credential_ttl_seconds": 600}, login["access_token"])
    DEPLOYED_MARKER.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    descriptor = os.open(DEPLOYED_MARKER, os.O_WRONLY | os.O_CREAT | os.O_TRUNC | os.O_NOFOLLOW, 0o600)
    with os.fdopen(descriptor, "w") as marker:
        marker.write(fingerprint + "\n")
        marker.flush()
        os.fsync(marker.fileno())
    print("public_certificate_deployed_relay_rotation_requested")


if __name__ == "__main__":
    try:
        main()
    except Exception:
        # Neither API errors nor subprocess output may expose credentials.
        raise SystemExit("public_certificate_deploy_failed") from None
