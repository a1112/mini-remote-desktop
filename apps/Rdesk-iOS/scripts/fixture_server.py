#!/usr/bin/env python3
"""Loopback-only TLS JPEG gateway fixture. Records controls without injecting input.

Uses only Python's standard library. Never bind this test fixture to a LAN.
"""
import argparse
import base64
import hashlib
import hmac
import http.server
import json
import secrets
import socket
import ssl
import struct
import threading
import time
from urllib.parse import parse_qs, urlsplit

EVENTS = []
LOCK = threading.Lock()
UPGRADES = 0


def packet(opcode, payload):
    size = len(payload)
    if size < 126:
        header = bytes([0x80 | opcode, size])
    elif size <= 65535:
        header = bytes([0x80 | opcode, 126]) + struct.pack("!H", size)
    else:
        header = bytes([0x80 | opcode, 127]) + struct.pack("!Q", size)
    return header + payload


class Handler(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *_):
        pass

    def do_GET(self):
        global UPGRADES
        target = urlsplit(self.path)
        if target.path != "/mobile/desktop/ws" or self.headers.get("Upgrade", "").lower() != "websocket":
            self.send_error(404)
            return
        with LOCK:
            UPGRADES += 1
        host = self.headers.get("Host")
        origin = self.headers.get("Origin")
        token = self.headers.get("X-MRD-Pairing-Token")
        if token is None:
            token = parse_qs(target.query).get("token", [""])[0]
        if (
            host != "127.0.0.1:19534"
            or (origin is not None and origin.lower() not in (f"http://{host}", f"https://{host}"))
            or not hmac.compare_digest(token.encode(), self.server.pairing_token.encode())
        ):
            self.send_error(403)
            return
        key = self.headers.get("Sec-WebSocket-Key", "")
        accept = base64.b64encode(hashlib.sha1((key + "258EAFA5-E914-47DA-95CA-C5AB0DC85B11").encode()).digest()).decode()
        self.send_response(101)
        self.send_header("Upgrade", "websocket")
        self.send_header("Connection", "Upgrade")
        self.send_header("Sec-WebSocket-Accept", accept)
        self.end_headers()
        self.close_connection = True
        with LOCK:
            EVENTS.clear()
        conn = self.connection
        conn.settimeout(0.05)
        incoming = bytearray()
        try:
            conn.sendall(packet(1, b'{"type":"ready"}'))
            conn.sendall(packet(1, b'{"type":"dimensions","width":1024,"height":1024}'))
            last_frame = 0
            while True:
                now = time.monotonic()
                if now - last_frame >= 0.15:
                    conn.sendall(packet(2, self.server.jpeg))
                    last_frame = now
                try:
                    data = conn.recv(8192)
                    if not data:
                        return
                    incoming.extend(data)
                except socket.timeout:
                    continue
                while len(incoming) >= 2:
                    opcode = incoming[0] & 15
                    masked = bool(incoming[1] & 128)
                    length = incoming[1] & 127
                    offset = 2
                    if length == 126:
                        if len(incoming) < 4:
                            break
                        length = struct.unpack("!H", incoming[2:4])[0]
                        offset = 4
                    elif length == 127:
                        if len(incoming) < 10:
                            break
                        length = struct.unpack("!Q", incoming[2:10])[0]
                        offset = 10
                    if not masked or length > 8192:
                        return
                    if len(incoming) < offset + 4 + length:
                        break
                    mask = incoming[offset:offset + 4]
                    payload = bytes(byte ^ mask[i % 4] for i, byte in enumerate(incoming[offset + 4:offset + 4 + length]))
                    del incoming[:offset + 4 + length]
                    if opcode == 8:
                        conn.sendall(packet(8, payload))
                        with LOCK:
                            EVENTS.append({"type": "closed"})
                        return
                    if opcode == 9:
                        conn.sendall(packet(10, payload))
                    if opcode == 1:
                        event = json.loads(payload)
                        with LOCK:
                            EVENTS.append(event)
        except (OSError, ValueError):
            return


class ControlHandler(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def do_GET(self):
        with LOCK:
            if self.path == "/health":
                response = {**self.server.fixture_info, "upgrades": UPGRADES}
            elif self.path == "/events":
                response = EVENTS.copy()
            else:
                self.send_error(404)
                return
        body = json.dumps(response).encode()
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--jpeg", required=True)
    parser.add_argument("--cert", required=True)
    parser.add_argument("--key", required=True)
    args = parser.parse_args()
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 19534), Handler)
    with open(args.jpeg, "rb") as source:
        server.jpeg = source.read()
    with open(args.cert, encoding="ascii") as source:
        certificate = ssl.PEM_cert_to_DER_cert(source.read())
    server.pairing_token = secrets.token_hex(32)
    context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    context.minimum_version = ssl.TLSVersion.TLSv1_2
    context.load_cert_chain(args.cert, args.key)
    server.socket = context.wrap_socket(server.socket, server_side=True)

    control = http.server.ThreadingHTTPServer(("127.0.0.1", 19535), ControlHandler)
    control.fixture_info = {
        "fingerprint": hashlib.sha256(certificate).hexdigest(),
        "token": server.pairing_token,
    }
    threading.Thread(target=control.serve_forever, daemon=True).start()
    print("Rdesk TLS fixture and test control on loopback ports 19534 and 19535", flush=True)
    server.serve_forever()
