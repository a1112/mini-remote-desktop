"""Screen capture module."""

from .d3dshot_backend import ScreenCapturer, D3DShotCapturer, CapturedFrame
from .dxgi_backend import DXGIDuplicator, FastDXGICapture, create_duplicator, create_fast_capture

# C++ DXGI (最快，需要编译 DLL)
try:
    from .dxgi_cpp import DXGICapture, create_dxgi_capture
    HAS_DXGI_CPP = True
except ImportError:
    HAS_DXGI_CPP = False
    DXGICapture = None
    create_dxgi_capture = None

__all__ = [
    "ScreenCapturer",
    "D3DShotCapturer",
    "CapturedFrame",
    "DXGIDuplicator",
    "FastDXGICapture",
    "create_duplicator",
    "create_fast_capture",
    # C++ DXGI
    "DXGICapture",
    "create_dxgi_capture",
    "HAS_DXGI_CPP",
]
