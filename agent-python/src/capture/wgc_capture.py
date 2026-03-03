"""
Windows Graphics Capture (WGC) Python 包装器

使用 Desktop Duplication API 实现:
- 屏幕捕获 (监视器索引)
- 窗口捕获 (HWND)
- GPU Direct (D3D11 纹理输出)
- 延迟: ~0-1ms
"""

import ctypes
import logging
from pathlib import Path
from typing import Optional, List, Tuple

logger = logging.getLogger(__name__)

# ============================================================================
# 结构体定义
# ============================================================================

class WgcCaptureType:
    MONITOR = 0
    WINDOW = 1


class WgcFrame(ctypes.Structure):
    """捕获帧信息"""
    _fields_ = [
        ("width", ctypes.c_int),
        ("height", ctypes.c_int),
        ("d3d11_texture", ctypes.c_void_p),  # ID3D11Texture2D*
        ("timestamp", ctypes.c_ulonglong),
        ("frame_id", ctypes.c_uint),
    ]


class WgcWindowInfo(ctypes.Structure):
    """窗口信息"""
    _fields_ = [
        ("hwnd", ctypes.c_void_p),
        ("title", ctypes.c_wchar * 256),
        ("is_visible", ctypes.c_int),
        ("rect_left", ctypes.c_int),
        ("rect_top", ctypes.c_int),
        ("rect_right", ctypes.c_int),
        ("rect_bottom", ctypes.c_int),
    ]

    @property
    def rect(self) -> Tuple[int, int, int, int]:
        return (self.rect_left, self.rect_top, self.rect_right, self.rect_bottom)

    @property
    def size(self) -> Tuple[int, int]:
        return (self.rect_right - self.rect_left, self.rect_bottom - self.rect_top)


class WgcMonitorInfo(ctypes.Structure):
    """监视器信息"""
    _fields_ = [
        ("hmon", ctypes.c_void_p),
        ("name", ctypes.c_wchar * 64),
        ("rect_left", ctypes.c_int),
        ("rect_top", ctypes.c_int),
        ("rect_right", ctypes.c_int),
        ("rect_bottom", ctypes.c_int),
        ("is_primary", ctypes.c_int),
    ]

    @property
    def rect(self) -> Tuple[int, int, int, int]:
        return (self.rect_left, self.rect_top, self.rect_right, self.rect_bottom)

    @property
    def size(self) -> Tuple[int, int]:
        return (self.rect_right - self.rect_left, self.rect_bottom - self.rect_top)


# ============================================================================
# WGC 捕获类
# ============================================================================

class WGCCapture:
    """Windows Graphics Capture 捕获器"""

    def __init__(self, dll_path: Optional[str] = None):
        """
        初始化 WGC 捕获器

        Args:
            dll_path: wgc_capture.dll 路径，默认为 cpp_capture/wgc_capture.dll
        """
        if dll_path is None:
            dll_path = str(Path(__file__).parent.parent.parent / "cpp_capture" / "wgc_capture.dll")

        self._dll = ctypes.CDLL(dll_path)
        self._handle = None
        self._width = 0
        self._height = 0

        # 设置函数签名
        self._setup_functions()

        logger.info(f"WGC Capture initialized: {dll_path}")

    def _setup_functions(self):
        """设置 DLL 函数签名"""

        # int wgc_enum_monitors(WgcMonitorInfo* monitors, int max_count)
        self._dll.wgc_enum_monitors.argtypes = [ctypes.POINTER(WgcMonitorInfo), ctypes.c_int]
        self._dll.wgc_enum_monitors.restype = ctypes.c_int

        # int wgc_enum_windows(WgcWindowInfo* windows, int max_count)
        self._dll.wgc_enum_windows.argtypes = [ctypes.POINTER(WgcWindowInfo), ctypes.c_int]
        self._dll.wgc_enum_windows.restype = ctypes.c_int

        # HWgcSession wgc_create_session(WgcCaptureType type, void* target)
        self._dll.wgc_create_session.argtypes = [ctypes.c_int, ctypes.c_void_p]
        self._dll.wgc_create_session.restype = ctypes.c_void_p

        # int wgc_start(HWgcSession session)
        self._dll.wgc_start.argtypes = [ctypes.c_void_p]
        self._dll.wgc_start.restype = ctypes.c_int

        # void wgc_stop(HWgcSession session)
        self._dll.wgc_stop.argtypes = [ctypes.c_void_p]
        self._dll.wgc_stop.restype = None

        # int wgc_get_frame(HWgcSession session, WgcFrame* frame)
        self._dll.wgc_get_frame.argtypes = [ctypes.c_void_p, ctypes.POINTER(WgcFrame)]
        self._dll.wgc_get_frame.restype = ctypes.c_int

        # int wgc_copy_to_cpu(HWgcSession session, void* buffer, int size)
        self._dll.wgc_copy_to_cpu.argtypes = [ctypes.c_void_p, ctypes.c_void_p, ctypes.c_int]
        self._dll.wgc_copy_to_cpu.restype = ctypes.c_int

        # void* wgc_get_d3d11_device(HWgcSession session)
        self._dll.wgc_get_d3d11_device.argtypes = [ctypes.c_void_p]
        self._dll.wgc_get_d3d11_device.restype = ctypes.c_void_p

        # void* wgc_get_d3d11_context(HWgcSession session)
        self._dll.wgc_get_d3d11_context.argtypes = [ctypes.c_void_p]
        self._dll.wgc_get_d3d11_context.restype = ctypes.c_void_p

        # void wgc_free_session(HWgcSession session)
        self._dll.wgc_free_session.argtypes = [ctypes.c_void_p]
        self._dll.wgc_free_session.restype = None

    @staticmethod
    def enum_monitors(dll_path: Optional[str] = None) -> List[WgcMonitorInfo]:
        """
        枚举所有监视器

        Returns:
            监视器信息列表
        """
        if dll_path is None:
            dll_path = str(Path(__file__).parent.parent.parent / "cpp_capture" / "wgc_capture.dll")

        dll = ctypes.CDLL(dll_path)
        dll.wgc_enum_monitors.argtypes = [ctypes.POINTER(WgcMonitorInfo), ctypes.c_int]
        dll.wgc_enum_monitors.restype = ctypes.c_int

        # 先获取数量
        temp = (WgcMonitorInfo * 1)()
        count = dll.wgc_enum_monitors(temp, 0)

        # 获取所有监视器
        monitors = (WgcMonitorInfo * count)()
        dll.wgc_enum_monitors(monitors, count)

        return list(monitors)

    @staticmethod
    def enum_windows(dll_path: Optional[str] = None, max_count: int = 1000) -> List[WgcWindowInfo]:
        """
        枚举所有可见窗口

        Args:
            max_count: 最大窗口数量

        Returns:
            窗口信息列表
        """
        if dll_path is None:
            dll_path = str(Path(__file__).parent.parent.parent / "cpp_capture" / "wgc_capture.dll")

        dll = ctypes.CDLL(dll_path)
        dll.wgc_enum_windows.argtypes = [ctypes.POINTER(WgcWindowInfo), ctypes.c_int]
        dll.wgc_enum_windows.restype = ctypes.c_int

        # 先获取数量
        temp = (WgcWindowInfo * 1)()
        count = min(dll.wgc_enum_windows(temp, 0), max_count)

        # 获取所有窗口
        windows = (WgcWindowInfo * count)()
        dll.wgc_enum_windows(windows, count)

        return list(windows)

    def start_monitor(self, monitor_index: int = 0) -> bool:
        """
        开始监视器捕获

        Args:
            monitor_index: 监视器索引 (0 = 主监视器)

        Returns:
            成功返回 True
        """
        if self._handle:
            self.stop()

        logger.info(f"Starting monitor capture: index={monitor_index}")

        self._handle = self._dll.wgc_create_session(
            WgcCaptureType.MONITOR,
            ctypes.c_void_p(monitor_index)
        )

        if not self._handle:
            logger.error("Failed to create capture session")
            return False

        if self._dll.wgc_start(self._handle) == 0:
            logger.error("Failed to start capture session")
            self._dll.wgc_free_session(self._handle)
            self._handle = None
            return False

        logger.info("Capture session started")
        return True

    def start_window(self, hwnd: int) -> bool:
        """
        开始窗口捕获

        Args:
            hwnd: 窗口句柄

        Returns:
            成功返回 True
        """
        if self._handle:
            self.stop()

        logger.info(f"Starting window capture: hwnd={hex(hwnd)}")

        self._handle = self._dll.wgc_create_session(
            WgcCaptureType.WINDOW,
            ctypes.c_void_p(hwnd)
        )

        if not self._handle:
            logger.error("Failed to create capture session")
            return False

        if self._dll.wgc_start(self._handle) == 0:
            logger.error("Failed to start capture session")
            self._dll.wgc_free_session(self._handle)
            self._handle = None
            return False

        logger.info("Capture session started")
        return True

    def stop(self):
        """停止捕获"""
        if self._handle:
            self._dll.wgc_stop(self._handle)
            self._dll.wgc_free_session(self._handle)
            self._handle = None
            logger.info("Capture session stopped")

    def capture_frame(self) -> Optional[WgcFrame]:
        """
        捕获一帧

        Returns:
            WgcFrame 对象，如果没有新帧返回 None
        """
        if not self._handle:
            return None

        frame = WgcFrame()
        result = self._dll.wgc_get_frame(self._handle, ctypes.byref(frame))

        if result == 1:
            self._width = frame.width
            self._height = frame.height
            return frame
        elif result == 0:
            # 暂无新帧
            return None
        else:
            # 错误
            logger.error("Capture frame failed")
            return None

    def copy_to_cpu(self, buffer) -> bool:
        """
        复制当前帧到 CPU 内存

        Args:
            buffer: ctypes 缓冲区 (至少 width * height * 4 字节)
                    例如: ctypes.create_string_buffer(width * height * 4)

        Returns:
            成功返回 True
        """
        if not self._handle:
            return False

        result = self._dll.wgc_copy_to_cpu(
            self._handle,
            ctypes.c_void_p(ctypes.addressof(buffer)),
            len(buffer)
        )
        return result == 1

    @property
    def d3d11_device(self) -> Optional[int]:
        """
        获取 D3D11 设备指针 (用于 GPU Direct)

        Returns:
            D3D11 设备指针，或 None
        """
        if not self._handle:
            return None
        ptr = self._dll.wgc_get_d3d11_device(self._handle)
        return ptr if ptr else None

    @property
    def d3d11_context(self) -> Optional[int]:
        """
        获取 D3D11 上下文指针 (用于 GPU Direct)

        Returns:
            D3D11 上下文指针，或 None
        """
        if not self._handle:
            return None
        ptr = self._dll.wgc_get_d3d11_context(self._handle)
        return ptr if ptr else None

    @property
    def d3d11_texture(self) -> Optional[int]:
        """
        获取当前捕获的 D3D11 纹理指针 (用于 GPU Direct)

        Returns:
            D3D11 纹理指针，或 None
        """
        frame = self.capture_frame()
        if frame:
            return frame.d3d11_texture
        return None

    @property
    def width(self) -> int:
        """捕获宽度"""
        return self._width

    @property
    def height(self) -> int:
        """捕获高度"""
        return self._height

    @property
    def resolution(self) -> Tuple[int, int]:
        """捕获分辨率"""
        return (self._width, self._height)

    def __enter__(self):
        return self

    def __exit__(self, exc_type, exc_val, exc_tb):
        self.stop()


# ============================================================================
# 辅助函数
# ============================================================================

def print_available_monitors():
    """打印所有可用监视器"""
    monitors = WGCCapture.enum_monitors()
    print(f"\n发现 {len(monitors)} 个监视器:")
    print("=" * 60)
    for i, m in enumerate(monitors):
        primary = " [主显示器]" if m.is_primary else ""
        print(f"  [{i}] {m.name}{primary}")
        print(f"      分辨率: {m.size[0]}x{m.size[1]}")
        print(f"      位置: ({m.rect[0]}, {m.rect[1]}) - ({m.rect[2]}, {m.rect[3]})")


def print_available_windows():
    """打印所有可用窗口"""
    windows = WGCCapture.enum_windows()
    print(f"\n发现 {len(windows)} 个窗口:")
    print("=" * 60)
    for w in windows[:20]:  # 只显示前 20 个
        visible = "[可见]" if w.is_visible else "[隐藏]"
        print(f"  HWND: {hex(w.hwnd)} - {w.title} {visible}")
        print(f"      大小: {w.size[0]}x{w.size[1]}")
    if len(windows) > 20:
        print(f"  ... 还有 {len(windows) - 20} 个窗口")
