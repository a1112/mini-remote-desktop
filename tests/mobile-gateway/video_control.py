"""Check video session liveness and decoder recovery, without injecting input."""
import json
import time
from smoke import Ws


def main():
    client = Ws('/mobile/desktop/video/ws')
    try:
        client.ready()
        client.send(9, b'video-test')
        client.send(1, '{"type":"ping","sent_us":123456}')
        pong, ping_reply, delta, requested, recovered = False, False, False, False, False
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            kind, data = client.read()
            if kind == 10:
                pong = data == b'video-test'
            elif kind == 1:
                message = json.loads(data)
                assert message.get('type') != 'error', message
                if message.get('type') == 'pong':
                    ping_reply = message['sent_us'] == 123456
            elif kind == 2:
                assert data[:8] == b'MRDWC01\0'
                n = int.from_bytes(data[8:12], 'little')
                h = json.loads(data[12:12+n])
                if requested and h['keyframe']:
                    recovered = True
                if not h['keyframe'] and not requested:
                    delta = True
                    requested = True
                    client.send(1, '{"type":"request_keyframe"}')
            if pong and ping_reply and delta and recovered:
                break
        assert pong and ping_reply, 'WebSocket or application keepalive failed'
        assert delta and recovered, 'requested keyframe did not recover the decoder stream'
        print('PASS: WebSocket ping, application ping, delta stream and requested IDR')
    finally:
        client.close()
    # A new session must wait for the previous DXGI duplication to be released.
    for _ in range(10):
        client = Ws('/mobile/desktop/video/ws')
        try:
            client.ready()
            while True:
                kind, data = client.read()
                if kind == 1:
                    message = json.loads(data)
                    assert message.get('type') != 'error', message
                if kind == 2:
                    assert data[:8] == b'MRDWC01\0'
                    break
        finally:
            client.close()
    print('PASS: 10 immediate reconnects receive real H.264 frames')


if __name__ == '__main__':
    main()
