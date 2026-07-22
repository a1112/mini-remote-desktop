import ctypes

from src.capture.hybrid_capture import D3D11HybridCapture


def test_capture_frame_returns_real_d3d11_pointer():
    capture = D3D11HybridCapture()
    capture._initialized = True
    capture._handle = ctypes.c_void_p(1)

    class FrameStruct(ctypes.Structure):
        _fields_ = [
            ("width", ctypes.c_int),
            ("height", ctypes.c_int),
            ("stride", ctypes.c_int),
            ("format", ctypes.c_int),
            ("timestamp", ctypes.c_ulonglong),
            ("d3d11_resource", ctypes.c_void_p),
            ("d3d12_resource", ctypes.c_void_p),
        ]

    class FakeDll:
        def capture_hybrid_frame(self, _handle, frame_ptr):
            frame = ctypes.cast(frame_ptr, ctypes.POINTER(FrameStruct)).contents
            frame.width = 1920
            frame.height = 1080
            frame.stride = 7680
            frame.format = 87
            frame.timestamp = 42
            frame.d3d11_resource = ctypes.c_void_p(0x12345678)
            frame.d3d12_resource = ctypes.c_void_p(0)
            return 1

    capture._dll = FakeDll()
    capture._FrameStruct = FrameStruct

    frame_info = capture.capture_frame()

    assert frame_info is not None
    assert frame_info.d3d11_resource_ptr == 0x12345678
