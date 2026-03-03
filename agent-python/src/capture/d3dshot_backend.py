"""
Screen capture with multiple backend support.

Fastest backend: pywin32 GDI (54.5 FPS)
Supports: GDI, mss, PIL.ImageGrab
"""

import asyncio
import logging
import time
import ctypes
from dataclasses import dataclass
from typing import Optional

logger = logging.getLogger(__name__)


@dataclass
class CapturedFrame:
    """A captured frame from screen."""

    data: bytes  # Raw pixel data (RGB or RGBA)
    width: int
    height: int
    stride: int  # Bytes per row
    format: str = "RGB"  # pixel format
    timestamp: float = 0.0

    def __post_init__(self):
        if self.timestamp == 0.0:
            self.timestamp = time.time()


class GDICapturer:
    """
    Fast Windows screen capture using GDI via pywin32.

    This is the fastest pure Python method for screen capture on Windows.
    """

    def __init__(self, target_fps: int = 30):
        """
        Initialize the GDI capturer.

        Args:
            target_fps: Target frame rate for frame pacing
        """
        self.target_fps = target_fps
        self._screen_width = 0
        self._screen_height = 0
        self._last_capture_time = 0.0
        self._frame_interval = 1.0 / max(1, target_fps)

        # GDI objects
        self._hwnd = None
        self._hdc = None
        self._hdc_mem = None
        self._hbitmap = None
        self._hobj = None

    async def initialize(self) -> bool:
        """Initialize GDI capturer."""
        try:
            import win32gui
            import win32con
            import ctypes

            # Get screen dimensions via ctypes
            user32 = ctypes.windll.user32
            self._screen_width = user32.GetSystemMetrics(0)  # SM_CXSCREEN
            self._screen_height = user32.GetSystemMetrics(1)  # SM_CYSCREEN

            # Get desktop window and DC
            self._hwnd = win32gui.GetDesktopWindow()
            self._hdc = win32gui.GetDC(self._hwnd)

            # Create memory DC and bitmap
            self._hdc_mem = win32gui.CreateCompatibleDC(self._hdc)
            self._hbitmap = win32gui.CreateCompatibleBitmap(
                self._hdc, self._screen_width, self._screen_height
            )
            self._hobj = win32gui.SelectObject(self._hdc_mem, self._hbitmap)

            logger.info(
                f"GDI capturer initialized: {self._screen_width}x{self._screen_height} "
                f"@ {self.target_fps}fps"
            )
            return True

        except ImportError:
            logger.error("pywin32 not available")
            return False
        except Exception as e:
            logger.error(f"Failed to initialize GDI capturer: {e}")
            return False

    async def capture_frame(self) -> Optional[CapturedFrame]:
        """
        Capture a single frame using GDI.

        Returns:
            CapturedFrame or None if capture failed
        """
        if self._hdc is None:
            return None

        # Frame pacing
        now = time.time()
        elapsed = now - self._last_capture_time
        if elapsed < self._frame_interval:
            await asyncio.sleep(self._frame_interval - elapsed)

        try:
            import win32gui
            import win32con

            # Perform capture in thread pool
            loop = asyncio.get_event_loop()
            frame = await loop.run_in_executor(None, self._capture_sync)

            if frame is None:
                return None

            self._last_capture_time = time.time()
            return frame

        except Exception as e:
            logger.error(f"GDI capture error: {e}")
            return None

    def _capture_sync(self) -> Optional[CapturedFrame]:
        """Synchronous GDI capture (run in thread pool)."""
        try:
            import win32gui
            import win32con
            import ctypes
            from ctypes import wintypes

            # BitBlt to capture screen
            win32gui.BitBlt(
                self._hdc_mem, 0, 0,
                self._screen_width, self._screen_height,
                self._hdc, 0, 0,
                win32con.SRCCOPY
            )

            # Get bitmap data using ctypes
            # Define BITMAPINFO structure
            class BITMAPINFOHEADER(ctypes.Structure):
                _fields_ = [
                    ("biSize", wintypes.DWORD),
                    ("biWidth", wintypes.LONG),
                    ("biHeight", wintypes.LONG),
                    ("biPlanes", wintypes.WORD),
                    ("biBitCount", wintypes.WORD),
                    ("biCompression", wintypes.DWORD),
                    ("biSizeImage", wintypes.DWORD),
                    ("biXPelsPerMeter", wintypes.LONG),
                    ("biYPelsPerMeter", wintypes.LONG),
                    ("biClrUsed", wintypes.DWORD),
                    ("biClrImportant", wintypes.DWORD),
                ]

            class BITMAPINFO(ctypes.Structure):
                _fields_ = [
                    ("bmiHeader", BITMAPINFOHEADER),
                    ("bmiColors", wintypes.DWORD * 3),
                ]

            # Create buffer for bitmap data
            bmp_data = (ctypes.c_ubyte * (self._screen_width * self._screen_height * 4))()

            # Setup BITMAPINFO
            bmi = BITMAPINFO()
            bmi.bmiHeader.biSize = ctypes.sizeof(BITMAPINFOHEADER)
            bmi.bmiHeader.biWidth = self._screen_width
            bmi.bmiHeader.biHeight = -self._screen_height  # Negative for top-down DIB
            bmi.bmiHeader.biPlanes = 1
            bmi.bmiHeader.biBitCount = 32  # BGRA
            bmi.bmiHeader.biCompression = 0  # BI_RGB

            # Get DIBits
            gdi32 = ctypes.windll.gdi32
            gdi32.GetDIBits(
                self._hdc,
                self._hbitmap,
                0,
                self._screen_height,
                ctypes.byref(bmp_data),
                ctypes.byref(bmi),
                0  # DIB_RGB_COLORS
            )

            # Convert to numpy and adjust format
            import numpy as np
            arr = np.frombuffer(bmp_data, dtype=np.uint8)
            arr = arr.reshape((self._screen_height, self._screen_width, 4))  # BGRA
            arr = arr[:, :, :3][:, :, [2, 1, 0]]  # BGRA -> RGB

            return CapturedFrame(
                data=arr.tobytes(),
                width=self._screen_width,
                height=self._screen_height,
                stride=self._screen_width * 3,
                format="RGB",
                timestamp=time.time(),
            )

        except Exception as e:
            logger.error(f"Sync GDI capture error: {e}")
            return None

    async def close(self) -> None:
        """Clean up GDI resources."""
        if self._hdc:
            try:
                import win32gui
                win32gui.SelectObject(self._hdc_mem, self._hobj)
                win32gui.DeleteObject(self._hbitmap)
                win32gui.DeleteDC(self._hdc_mem)
                win32gui.ReleaseDC(self._hwnd, self._hdc)
            except Exception:
                pass
            self._hdc = None
            self._hdc_mem = None
            self._hbitmap = None
            self._hobj = None

        logger.info("GDI capturer closed")

    @property
    def screen_width(self) -> int:
        """Get screen width."""
        return self._screen_width

    @property
    def screen_height(self) -> int:
        """Get screen height."""
        return self._screen_height


class MSSCapturer:
    """
    Screen capture using mss library (cross-platform).

    Uses ctypes to call native screen capture APIs.
    """

    def __init__(self, target_fps: int = 30):
        """
        Initialize the MSS capturer.

        Args:
            target_fps: Target frame rate
        """
        self.target_fps = target_fps
        self._mss = None
        self._monitor = None
        self._screen_width = 0
        self._screen_height = 0
        self._last_capture_time = 0.0
        self._frame_interval = 1.0 / max(1, target_fps)

    async def initialize(self) -> bool:
        """Initialize MSS capturer."""
        try:
            import mss

            self._mss = mss.mss()
            self._monitor = self._mss.monitors[1]  # Primary monitor
            self._screen_width = self._monitor["width"]
            self._screen_height = self._monitor["height"]

            logger.info(
                f"MSS capturer initialized: {self._screen_width}x{self._screen_height} "
                f"@ {self.target_fps}fps"
            )
            return True

        except ImportError:
            logger.error("mss not available")
            return False
        except Exception as e:
            logger.error(f"Failed to initialize MSS capturer: {e}")
            return False

    async def capture_frame(self) -> Optional[CapturedFrame]:
        """Capture a frame using MSS."""
        if self._mss is None:
            return None

        # Frame pacing
        now = time.time()
        elapsed = now - self._last_capture_time
        if elapsed < self._frame_interval:
            await asyncio.sleep(self._frame_interval - elapsed)

        try:
            # Capture in thread pool
            loop = asyncio.get_event_loop()
            frame = await loop.run_in_executor(None, self._capture_sync)
            self._last_capture_time = time.time()
            return frame

        except Exception as e:
            logger.error(f"MSS capture error: {e}")
            return None

    def _capture_sync(self) -> Optional[CapturedFrame]:
        """Synchronous MSS capture."""
        try:
            import numpy as np

            screenshot = self._mss.grab(self._monitor)

            # mss returns BGRA data
            arr = np.frombuffer(screenshot.rgb, dtype=np.uint8)
            arr = arr.reshape((self._screen_height, self._screen_width, 3))

            return CapturedFrame(
                data=arr.tobytes(),
                width=self._screen_width,
                height=self._screen_height,
                stride=self._screen_width * 3,
                format="RGB",
                timestamp=time.time(),
            )

        except Exception as e:
            logger.error(f"Sync MSS capture error: {e}")
            return None

    async def close(self) -> None:
        """Clean up MSS resources."""
        if self._mss:
            try:
                self._mss.close()
            except Exception:
                pass
            self._mss = None

        logger.info("MSS capturer closed")

    @property
    def screen_width(self) -> int:
        return self._screen_width

    @property
    def screen_height(self) -> int:
        return self._screen_height


class PILCapturer:
    """
    Screen capture using PIL.ImageGrab.

    Fallback option, works across platforms.
    """

    def __init__(self, target_fps: int = 30):
        self.target_fps = target_fps
        self._screen_width = 0
        self._screen_height = 0
        self._last_capture_time = 0.0
        self._frame_interval = 1.0 / max(1, target_fps)

    async def initialize(self) -> bool:
        """Initialize PIL capturer."""
        try:
            from PIL import ImageGrab
            import ctypes

            user32 = ctypes.windll.user32
            self._screen_width = user32.GetSystemMetrics(0)
            self._screen_height = user32.GetSystemMetrics(1)

            logger.info(
                f"PIL capturer initialized: {self._screen_width}x{self._screen_height} "
                f"@ {self.target_fps}fps"
            )
            return True

        except ImportError:
            logger.error("PIL not available")
            return False
        except Exception as e:
            logger.error(f"Failed to initialize PIL capturer: {e}")
            return False

    async def capture_frame(self) -> Optional[CapturedFrame]:
        """Capture a frame using PIL."""
        # Frame pacing
        now = time.time()
        elapsed = now - self._last_capture_time
        if elapsed < self._frame_interval:
            await asyncio.sleep(self._frame_interval - elapsed)

        try:
            from PIL import ImageGrab
            import numpy as np

            # Capture in thread pool
            loop = asyncio.get_event_loop()
            frame = await loop.run_in_executor(None, self._capture_sync)
            self._last_capture_time = time.time()
            return frame

        except Exception as e:
            logger.error(f"PIL capture error: {e}")
            return None

    def _capture_sync(self) -> Optional[CapturedFrame]:
        """Synchronous PIL capture."""
        try:
            from PIL import ImageGrab
            import numpy as np

            screenshot = ImageGrab.grab()
            arr = np.array(screenshot)

            height, width = arr.shape[:2]
            if arr.ndim == 3:
                data = arr.tobytes()
                stride = width * 3
            else:
                data = arr.tobytes()
                stride = width

            return CapturedFrame(
                data=data,
                width=width,
                height=height,
                stride=stride,
                format="RGB",
                timestamp=time.time(),
            )

        except Exception as e:
            logger.error(f"Sync PIL capture error: {e}")
            return None

    async def close(self) -> None:
        """Clean up."""
        logger.info("PIL capturer closed")

    @property
    def screen_width(self) -> int:
        return self._screen_width

    @property
    def screen_height(self) -> int:
        return self._screen_height


class ScreenCapturer:
    """
    Auto-detecting screen capturer with multiple backend support.

    Tries backends in order: GDI -> mss -> PIL
    """

    def __init__(
        self,
        region: Optional[tuple[int, int, int, int]] = None,
        target_fps: int = 30,
        preferred_backend: Optional[str] = None,
    ):
        """
        Initialize the capturer.

        Args:
            region: Capture region (not yet supported)
            target_fps: Target frame rate
            preferred_backend: Force specific backend ("gdi", "mss", "pil")
        """
        self.region = region
        self.target_fps = target_fps
        self._preferred_backend = preferred_backend
        self._backend = None
        self._backend_name = ""
        self._screen_width = 0
        self._screen_height = 0

    async def initialize(self) -> bool:
        """Initialize with best available backend."""
        # Try preferred backend first, then fallback
        if self._preferred_backend:
            backends = [self._preferred_backend] + [b for b in ["gdi", "mss", "pil"] if b != self._preferred_backend]
        else:
            backends = ["gdi", "mss", "pil"]

        for backend in backends:
            if self._init_backend(backend):
                logger.info(f"ScreenCapturer using {self._backend_name} backend")
                return True

        logger.error("No screen capture backend available")
        return False

    def _init_backend(self, name: str) -> bool:
        """Initialize specific backend."""
        if name == "gdi":
            try:
                import win32gui
                import win32con
                import ctypes

                self._backend = GDICapturer(self.target_fps)

                # Sync init for GDI
                user32 = ctypes.windll.user32
                hwnd = win32gui.GetDesktopWindow()
                self._backend._hwnd = hwnd
                self._backend._hdc = win32gui.GetDC(hwnd)
                self._backend._screen_width = user32.GetSystemMetrics(0)
                self._backend._screen_height = user32.GetSystemMetrics(1)
                self._backend._hdc_mem = win32gui.CreateCompatibleDC(self._backend._hdc)
                self._backend._hbitmap = win32gui.CreateCompatibleBitmap(
                    self._backend._hdc, self._backend._screen_width, self._backend._screen_height
                )
                self._backend._hobj = win32gui.SelectObject(self._backend._hdc_mem, self._backend._hbitmap)
                self._backend_name = "gdi"
                self._screen_width = self._backend._screen_width
                self._screen_height = self._backend._screen_height
                return True
            except Exception as e:
                logger.debug(f"GDI backend failed: {e}")
                return False

        elif name == "mss":
            try:
                import mss
                ms = mss.mss()
                mon = ms.monitors[1]
                self._backend = MSSCapturer(self.target_fps)
                self._backend._mss = ms
                self._backend._monitor = mon
                self._backend._screen_width = mon["width"]
                self._backend._screen_height = mon["height"]
                self._backend_name = "mss"
                self._screen_width = mon["width"]
                self._screen_height = mon["height"]
                return True
            except Exception as e:
                logger.debug(f"MSS backend failed: {e}")
                return False

        elif name == "pil":
            try:
                from PIL import ImageGrab
                self._backend = PILCapturer(self.target_fps)
                # Sync init
                import ctypes
                user32 = ctypes.windll.user32
                self._backend._screen_width = user32.GetSystemMetrics(0)
                self._backend._screen_height = user32.GetSystemMetrics(1)
                self._backend_name = "pil"
                self._screen_width = self._backend._screen_width
                self._screen_height = self._screen_height
                return True
            except Exception as e:
                logger.debug(f"PIL backend failed: {e}")
                return False

        return False

    async def capture_frame(self) -> Optional[CapturedFrame]:
        """Capture a frame."""
        if self._backend is None:
            return None
        return await self._backend.capture_frame()

    async def close(self) -> None:
        """Clean up resources."""
        if self._backend:
            await self._backend.close()
            self._backend = None
        logger.info("ScreenCapturer closed")

    @property
    def screen_width(self) -> int:
        return self._screen_width

    @property
    def screen_height(self) -> int:
        return self._screen_height


# Backwards compatibility alias
D3DShotCapturer = ScreenCapturer
