"""Exercise the running mobile gateway with standard-library WebSocket clients.

No pairing secret is needed on a trusted private network. This sends no Windows input.
"""

import base64
import json
import os
import secrets
import socket
import struct
import time


HOST = os.environ.get("MRD_MOBILE_GATEWAY_HOST", "127.0.0.1")
PORT = int(os.environ.get("MRD_MOBILE_GATEWAY_PORT", "9534"))


class Ws:
    def __init__(self, path):
        self.sock = socket.create_connection((HOST, PORT), timeout=8)
        self.sock.settimeout(8)
        key = base64.b64encode(secrets.token_bytes(16)).decode()
        request = (
            f"GET {path} HTTP/1.1\r\nHost: {HOST}:{PORT}\r\n"
            f"Upgrade: websocket\r\nConnection: Upgrade\r\n"
            f"Sec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n"
        )
        self.sock.sendall(request.encode())
        response = b""
        while not response.endswith(b"\r\n\r\n"):
            response += self.sock.recv(1)
        assert response.startswith(b"HTTP/1.1 101"), response

    def send(self, opcode, data):
        if isinstance(data, str):
            data = data.encode()
        mask = secrets.token_bytes(4)
        length = len(data)
        header = bytes([0x80 | opcode])
        if length < 126:
            header += bytes([0x80 | length])
        elif length < 65536:
            header += bytes([0x80 | 126]) + struct.pack("!H", length)
        else:
            header += bytes([0x80 | 127]) + struct.pack("!Q", length)
        self.sock.sendall(header + mask + bytes(byte ^ mask[index % 4] for index, byte in enumerate(data)))

    def read(self):
        header = self._exact(2)
        opcode, length = header[0] & 0x0F, header[1] & 0x7F
        if length == 126:
            length = struct.unpack("!H", self._exact(2))[0]
        elif length == 127:
            length = struct.unpack("!Q", self._exact(8))[0]
        assert length <= 2 * 1024 * 1024, length
        if header[1] & 0x80:
            mask = self._exact(4)
            data = self._exact(length)
            data = bytes(byte ^ mask[index % 4] for index, byte in enumerate(data))
        else:
            data = self._exact(length)
        return opcode, data

    def _exact(self, count):
        data = b""
        while len(data) < count:
            part = self.sock.recv(count - len(data))
            if not part:
                raise ConnectionError("WebSocket closed")
            data += part
        return data

    def ready(self):
        opcode, data = self.read()
        assert opcode == 1 and json.loads(data)["type"] == "ready", (opcode, data)

    def close(self):
        self.sock.close()


def main():
    with socket.create_connection((HOST, PORT), timeout=8) as hostile:
        key = base64.b64encode(secrets.token_bytes(16)).decode()
        hostile.sendall((
            f"GET /mobile/phone/control/ws HTTP/1.1\r\nHost: {HOST}:{PORT}\r\n"
            f"Origin: https://evil.example\r\nUpgrade: websocket\r\n"
            f"Connection: Upgrade\r\nSec-WebSocket-Key: {key}\r\n"
            "Sec-WebSocket-Version: 13\r\n\r\n"
        ).encode())
        assert hostile.recv(128).startswith(b"HTTP/1.1 403"), "foreign browser origin accepted"
    print("foreign browser origin denied")

    with socket.create_connection((HOST, PORT), timeout=8) as page:
        page.sendall((
            f"GET /mobile/phone HTTP/1.1\r\nHost: {HOST}:{PORT}\r\n"
            "Connection: close\r\n\r\n"
        ).encode())
        headers = page.recv(4096).split(b"\r\n\r\n", 1)[0].lower()
        assert b"x-frame-options: deny" in headers, "phone page can be framed"
    print("phone page cannot be framed")

    publisher = Ws("/mobile/phone/publish/ws")
    publisher.ready()
    controller = Ws("/mobile/phone/control/ws")
    controller.ready()
    frame = b"\xff\xd8mobile-test\xff\xd9"
    publisher.send(2, frame)
    assert controller.read() == (2, frame)
    controller.send(1, json.dumps({"type": "tap", "x": 0.5, "y": 0.5}))
    opcode, payload = publisher.read()
    assert opcode == 1 and json.loads(payload)["type"] == "tap"
    publisher.close()
    opcode, payload = controller.read()
    assert opcode == 1 and json.loads(payload)["type"] == "offline"
    controller.close()
    print("phone frame and control relayed")

    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as probe:
        probe.settimeout(3)
        probe.sendto(b"MRD_DISCOVER_V1", (HOST, 9535))
        reply, _ = probe.recvfrom(512)
        announcement = json.loads(reply)
        assert announcement["type"] == "rdesk_gateway" and announcement["port"] == PORT
        assert "token" not in announcement
    print("LAN discovery response received without secret")

    desktop = Ws("/mobile/desktop/ws")
    desktop.ready()
    deadline = time.monotonic() + 8
    found_frame = False
    while time.monotonic() < deadline:
        opcode, payload = desktop.read()
        if opcode == 2:
            assert payload.startswith(b"\xff\xd8") and payload.endswith(b"\xff\xd9")
            assert len(payload) > 1000
            found_frame = True
            break
    desktop.close()
    assert found_frame, "no real desktop frame received"
    print("real desktop JPEG received")


if __name__ == "__main__":
    main()
