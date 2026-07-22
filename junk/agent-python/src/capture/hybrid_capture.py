"""
GPU Direct Hybrid Capture - DXGI + NVENC Zero Copy.

使用 C++ d3d12_hybrid_capture.dll 实现:
1. DXGI Desktop Duplication 捕获到 D3D11 纹理
2. 纹理直接传递给 NVENC (无 CPU 拷贝)
3. 完整的 GPU 管道: Screen → GPU Memory → NVENC → H.264

性能:
- 零 CPU 拷贝
- 120+ FPS @ 1080p (理论)
- 延迟: <5ms 端到端
"""

import ctypes
import logging
import time
from pathlib import Path
from typing import Optional, Tuple
from dataclasses import dataclass

logger = logging.getLogger(__name__)


@dataclass
class HybridFrameInfo:
    """混合捕获帧信息。"""
    width: int
    height: int
    stride: int
    format: int
    timestamp: int
    d3d11_resource_ptr: int  # ID3D11Texture2D* 指针
    d3d12_resource_ptr: int = 0


class D3D11HybridCapture:
    """
    D3D11 混合捕获器 - GPU Direct 路径。

    使用 C++ DLL 实现 DXGI 捕获，返回 D3D11 纹理指针。
    """

    def __init__(self, monitor_index: int = 0):
        """
        初始化混合捕获器。

        Args:
            monitor_index: 显示器索引 (0 = 主显示器)
        """
        self.monitor_index = monitor_index
        self._handle: Optional[int] = None
        self._dll: Optional[ctypes.CDLL] = None
        self._initialized = False
        self._d3d11_device_ptr: Optional[int] = None
        self._d3d11_context_ptr: Optional[int] = None

    def initialize(self) -> bool:
        """
        初始化混合捕获器。

        Returns:
            True if successful
        """
        dll_path = Path(__file__).parent.parent.parent / 'cpp_capture' / 'd3d12_hybrid_capture.dll'

        if not dll_path.exists():
            logger.error(f"Hybrid capture DLL not found: {dll_path}")
            logger.info("请先编译 C++ DLL: cd cpp_capture && build.bat")
            return False

        try:
            self._dll = ctypes.CDLL(str(dll_path))
            self._setup_function_signatures()

            # 初始化捕获器 (enable_d3d12=0, 只用 D3D11)
            self._handle = self._dll.init_hybrid_capture(self.monitor_index, 0)

            if not self._handle:
                logger.error("Failed to initialize hybrid capture")
                return False

            # 获取 D3D11 设备和上下文 (用于 NVENC)
            self._d3d11_device_ptr = self._dll.get_hybrid_d3d11_device(self._handle)
            self._d3d11_context_ptr = self._dll.get_hybrid_d3d11_context(self._handle)

            if not self._d3d11_device_ptr or not self._d3d11_context_ptr:
                logger.error("Failed to get D3D11 device/context")
                return False

            self._initialized = True
            logger.info(f"✅ GPU Direct Hybrid Capture initialized")
            logger.info(f"   D3D11 Device: 0x{self._d3d11_device_ptr:X}")
            logger.info(f"   D3D11 Context: 0x{self._d3d11_context_ptr:X}")

            return True

        except Exception as e:
            logger.error(f"Failed to load hybrid capture DLL: {e}")
            return False

    def _setup_function_signatures(self) -> None:
        """设置 ctypes 函数签名。"""
        if not self._dll:
            return

        # D3D12HybridFrame 结构
        class D3D12HybridFrameStruct(ctypes.Structure):
            _fields_ = [
                ("width", ctypes.c_int),
                ("height", ctypes.c_int),
                ("stride", ctypes.c_int),
                ("format", ctypes.c_int),
                ("timestamp", ctypes.c_ulonglong),
                ("d3d11_resource", ctypes.c_void_p),
                ("d3d12_resource", ctypes.c_void_p),
            ]

        self._dll.init_hybrid_capture.argtypes = [ctypes.c_int, ctypes.c_int]
        self._dll.init_hybrid_capture.restype = ctypes.c_void_p

        self._dll.capture_hybrid_frame.argtypes = [
            ctypes.c_void_p,
            ctypes.POINTER(D3D12HybridFrameStruct)
        ]
        self._dll.capture_hybrid_frame.restype = ctypes.c_int

        self._dll.get_hybrid_d3d11_device.argtypes = [ctypes.c_void_p]
        self._dll.get_hybrid_d3d11_device.restype = ctypes.c_void_p

        self._dll.get_hybrid_d3d11_context.argtypes = [ctypes.c_void_p]
        self._dll.get_hybrid_d3d11_context.restype = ctypes.c_void_p

        self._dll.get_hybrid_d3d11_resource.argtypes = [ctypes.c_void_p]
        self._dll.get_hybrid_d3d11_resource.restype = ctypes.c_void_p

        self._dll.free_hybrid_capture.argtypes = [ctypes.c_void_p]
        self._dll.free_hybrid_capture.restype = None

        # 存储结构类
        self._FrameStruct = D3D12HybridFrameStruct

    def capture_frame(self) -> Optional[HybridFrameInfo]:
        """
        捕获一帧 (GPU Direct 路径)。

        返回 D3D11 纹理指针，可直接传递给 NVENC。

        Returns:
            HybridFrameInfo with D3D11 texture pointer, or None
        """
        if not self._initialized or not self._handle:
            return None

        try:
            frame_info = self._FrameStruct()
            result = self._dll.capture_hybrid_frame(self._handle, ctypes.byref(frame_info))

            if result != 1:
                return None

            return HybridFrameInfo(
                width=frame_info.width,
                height=frame_info.height,
                stride=frame_info.stride,
                format=frame_info.format,
                timestamp=frame_info.timestamp,
                # c_void_p fields already hold pointer values; do not take Python object address.
                d3d11_resource_ptr=int(frame_info.d3d11_resource) if frame_info.d3d11_resource else 0,
                d3d12_resource_ptr=int(frame_info.d3d12_resource) if frame_info.d3d12_resource else 0,
            )

        except Exception as e:
            logger.error(f"Capture error: {e}")
            return None

    def get_d3d11_device(self) -> Optional[int]:
        """获取 D3D11 设备指针 (用于 NVENC 初始化)。"""
        return self._d3d11_device_ptr

    def get_d3d11_context(self) -> Optional[int]:
        """获取 D3D11 上下文指针 (用于 NVENC 初始化)。"""
        return self._d3d11_context_ptr

    def get_texture_ptr(self) -> Optional[int]:
        """
        获取当前捕获的 D3D11 纹理指针。

        注意: 每次调用 capture_frame() 后纹理指针会变化。
        """
        if not self._handle:
            return None
        return self._dll.get_hybrid_d3d11_resource(self._handle)

    def close(self) -> None:
        """关闭混合捕获器。"""
        if self._handle and self._dll:
            self._dll.free_hybrid_capture(self._handle)
            self._handle = None
            self._initialized = False
            logger.info("Hybrid capture closed")


def create_hybrid_capture(monitor_index: int = 0) -> D3D11HybridCapture:
    """创建混合捕获器。"""
    return D3D11HybridCapture(monitor_index)


def is_gpu_direct_available() -> bool:
    """
    检查 GPU Direct 是否可用。

    Returns:
        True if d3d12_hybrid_capture.dll exists
    """
    dll_path = Path(__file__).parent.parent.parent / 'cpp_capture' / 'd3d12_hybrid_capture.dll'
    return dll_path.exists()
