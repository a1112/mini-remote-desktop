import ctypes

from src.encoder.nvenc_encoder import NVENCConfig, NVENCEncoder


class _FrameStruct(ctypes.Structure):
    _fields_ = [
        ("data", ctypes.POINTER(ctypes.c_ubyte)),
        ("size", ctypes.c_int),
        ("key_frame", ctypes.c_int),
        ("timestamp", ctypes.c_longlong),
    ]


def _build_encoder_with_fake_dll(fake_dll):
    encoder = NVENCEncoder(
        d3d11_device=1,
        d3d11_context=2,
        config=NVENCConfig(width=1920, height=1080, framerate=60),
    )
    encoder._dll = fake_dll
    encoder._handle = ctypes.c_void_p(1)
    encoder._FrameStruct = _FrameStruct
    return encoder


def test_encode_d3d11_prefers_zerocopy_when_available():
    calls = {"zerocopy": 0, "d3d11": 0}
    encoded = (ctypes.c_ubyte * 4)(1, 2, 3, 4)

    class FakeDll:
        def encode_nvenc_frame_d3d11_zerocopy(self, _h, _tex, _ts, _kf):
            calls["zerocopy"] += 1
            return 1

        def encode_nvenc_frame_d3d11(self, _h, _tex, _ts, _kf):
            calls["d3d11"] += 1
            return 1

        def get_nvenc_encoded_frame(self, _h, frame_ptr):
            frame = ctypes.cast(frame_ptr, ctypes.POINTER(_FrameStruct)).contents
            frame.data = ctypes.cast(encoded, ctypes.POINTER(ctypes.c_ubyte))
            frame.size = len(encoded)
            frame.key_frame = 1
            frame.timestamp = 123
            return 1

    encoder = _build_encoder_with_fake_dll(FakeDll())
    frame = encoder.encode_d3d11(0x1234)

    assert frame is not None
    assert calls["zerocopy"] == 1
    assert calls["d3d11"] == 0


def test_encode_d3d11_falls_back_when_zerocopy_fails():
    calls = {"zerocopy": 0, "d3d11": 0}
    encoded = (ctypes.c_ubyte * 3)(9, 8, 7)

    class FakeDll:
        def encode_nvenc_frame_d3d11_zerocopy(self, _h, _tex, _ts, _kf):
            calls["zerocopy"] += 1
            return 0

        def encode_nvenc_frame_d3d11(self, _h, _tex, _ts, _kf):
            calls["d3d11"] += 1
            return 1

        def get_nvenc_encoded_frame(self, _h, frame_ptr):
            frame = ctypes.cast(frame_ptr, ctypes.POINTER(_FrameStruct)).contents
            frame.data = ctypes.cast(encoded, ctypes.POINTER(ctypes.c_ubyte))
            frame.size = len(encoded)
            frame.key_frame = 0
            frame.timestamp = 456
            return 1

    encoder = _build_encoder_with_fake_dll(FakeDll())
    frame = encoder.encode_d3d11(0x1234)

    assert frame is not None
    assert calls["zerocopy"] == 1
    assert calls["d3d11"] == 1


def test_encode_d3d11_disables_zerocopy_after_failure():
    calls = {"zerocopy": 0, "d3d11": 0}
    encoded = (ctypes.c_ubyte * 2)(1, 1)

    class FakeDll:
        def encode_nvenc_frame_d3d11_zerocopy(self, _h, _tex, _ts, _kf):
            calls["zerocopy"] += 1
            return 0

        def encode_nvenc_frame_d3d11(self, _h, _tex, _ts, _kf):
            calls["d3d11"] += 1
            return 1

        def get_nvenc_encoded_frame(self, _h, frame_ptr):
            frame = ctypes.cast(frame_ptr, ctypes.POINTER(_FrameStruct)).contents
            frame.data = ctypes.cast(encoded, ctypes.POINTER(ctypes.c_ubyte))
            frame.size = len(encoded)
            frame.key_frame = 0
            frame.timestamp = 1
            return 1

    encoder = _build_encoder_with_fake_dll(FakeDll())
    frame1 = encoder.encode_d3d11(0x1)
    frame2 = encoder.encode_d3d11(0x2)

    assert frame1 is not None and frame2 is not None
    assert calls["zerocopy"] == 1
    assert calls["d3d11"] == 2


def test_initialize_prefers_zerocopy_init_when_available(monkeypatch):
    calls = {"zc_init": 0, "normal_init": 0}

    class FakeDll:
        def init_nvenc_encoder_d3d11_zerocopy(self, _dev, _ctx, _cfg):
            calls["zc_init"] += 1
            return ctypes.c_void_p(9)

        def init_nvenc_encoder_d3d11(self, _dev, _ctx, _cfg):
            calls["normal_init"] += 1
            return ctypes.c_void_p(3)

    fake = FakeDll()
    monkeypatch.setattr("src.encoder.nvenc_encoder.ctypes.CDLL", lambda _p: fake)
    monkeypatch.setattr("src.encoder.nvenc_encoder.Path.exists", lambda _p: True)
    monkeypatch.setattr(
        NVENCEncoder,
        "_setup_function_signatures",
        lambda self: (
            setattr(self, "_has_zerocopy_d3d11", True),
            setattr(self, "_has_zerocopy_init_d3d11", True),
        ),
    )
    monkeypatch.setattr(NVENCEncoder, "_create_config_struct", lambda self: ctypes.c_int(0))

    encoder = NVENCEncoder(
        d3d11_device=1,
        d3d11_context=2,
        config=NVENCConfig(width=1920, height=1080, framerate=60),
    )
    assert encoder.initialize() is True
    assert calls["zc_init"] == 1
    assert calls["normal_init"] == 0
