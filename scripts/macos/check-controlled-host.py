#!/usr/bin/env python3
"""Read service readiness without exporting credentials or changing permissions."""

import argparse
import json
import os
import socket
import stat
import struct
import sys


def request(endpoint, command):
    metadata = os.lstat(endpoint)
    if not stat.S_ISSOCK(metadata.st_mode) or metadata.st_uid != os.geteuid():
        raise ValueError("IPC endpoint must be an owned Unix socket")
    body = json.dumps({"type": command}).encode()
    with socket.socket(socket.AF_UNIX) as stream:
        stream.settimeout(5)
        stream.connect(endpoint)
        stream.sendall(struct.pack("<I", len(body)) + body)

        def read_exact(size):
            result = bytearray()
            while len(result) < size:
                chunk = stream.recv(size - len(result))
                if not chunk:
                    raise EOFError("service closed its IPC response")
                result.extend(chunk)
            return result

        size = struct.unpack("<I", read_exact(4))[0]
        if size > 16 * 1024 * 1024:
            raise ValueError("IPC response exceeds the protocol limit")
        response = json.loads(read_exact(size))
        if response.get("type") == "Error":
            raise ValueError(response.get("code", "IPC request failed"))
        return response


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--endpoint", default=os.environ.get(
        "MRD_SERVICE_IPC_ENDPOINT", f"/tmp/mrd-service-{os.geteuid()}/service.sock"))
    parser.add_argument("--require-cloud", action="store_true")
    args = parser.parse_args()
    try:
        health = request(args.endpoint, "ServiceHealth")["status"]
        shell = request(args.endpoint, "GetShellStatus")["status"]
        lan = request(args.endpoint, "LanDiscoverySnapshot")["snapshot"]
        capabilities = request(args.endpoint, "CapabilitySnapshot")["snapshot"]["capabilities"]
        public = request(args.endpoint, "GetPublicServerStatus")["status"]
        permissions = {item["id"]: {"status": item["status"], "reason": item.get("reason")}
                       for item in capabilities
                       if item["id"] in ("capture.macos", "control.keyboard_mouse")}
        ui_alive = False
        if shell["ui_pid"]:
            try:
                os.kill(shell["ui_pid"], 0)
                ui_alive = True
            except OSError:
                pass
        local_ready = bool(health["running"] and health["healthy"] and lan["running"]
                           and ui_alive and not shell["last_error"]
                           and all(permissions.get(item, {}).get("status") == "available"
                                   for item in ("capture.macos", "control.keyboard_mouse")))
        cloud_ready = bool(public["api_reachable"] and public["device_registered"]
                           and public["signaling_state"] == "authenticated")
        print(json.dumps({"service_pid": health["pid"], "healthy": health["healthy"],
                          "ui_pid": shell["ui_pid"], "ui_alive": ui_alive, "lan_running": lan["running"],
                          "discovery_port": lan["discovery_port"], "permissions": permissions,
                          "local_host_ready": local_ready, "cloud_ready": cloud_ready,
                          "public": public}, ensure_ascii=False, indent=2))
        return 0 if local_ready and (cloud_ready or not args.require_cloud) else 1
    except (OSError, EOFError, ValueError, KeyError) as error:
        print(f"Controlled host readiness unavailable: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
