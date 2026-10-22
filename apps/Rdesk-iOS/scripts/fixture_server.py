#!/usr/bin/env python3
"""Loopback-only JPEG gateway fixture; records controls without injecting input.

Uses only Python's standard library. Never bind this test fixture to a LAN.
"""
import argparse
import base64
import hashlib
import http.server
import json
import socket
import struct
import threading
import time

EVENTS = []
LOCK = threading.Lock()


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
        if self.path in ("/health", "/events"):
            with LOCK:
                body = json.dumps(EVENTS if self.path == "/events" else {"fixture": True}).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
            return
        if self.path != "/mobile/desktop/ws" or self.headers.get("Upgrade", "").lower() != "websocket":
            self.send_error(404)
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


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--jpeg", required=True)
    args = parser.parse_args()
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 19534), Handler)
    with open(args.jpeg, "rb") as source:
        server.jpeg = source.read()
    print("Rdesk test fixture on 127.0.0.1:19534", flush=True)
    server.serve_forever()
