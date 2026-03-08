#!/usr/bin/env python3
"""
Live display test - GDI capture for maximum performance (60 FPS @ 1080p).

Based on test_gdi_reuse.py which achieved 60 FPS.
"""
import sys
import time
import threading
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent / 'src'))

import cv2
import numpy as np

# GDI imports
try:
    import win32gui
    import win32con
    import ctypes
    from ctypes import wintypes

    user32 = ctypes.windll.user32

    HAS_GDI = True
except ImportError:
    HAS_GDI = False
    print("Warning: pywin32 not available")


class GDICapture:
    """Fast GDI screen capture with DC reuse."""

    def __init__(self, width=1920, height=1080):
        self.target_width = width
        self.target_height = height

        # Get screen dimensions
        self.src_width = user32.GetSystemMetrics(0)
        self.src_height = user32.GetSystemMetrics(1)

        # Initialize GDI objects
        self.hwnd = win32gui.GetDesktopWindow()
        self.hdc = win32gui.GetDC(self.hwnd)
        self.hdc_mem = win32gui.CreateCompatibleDC(self.hdc)
        self.hbitmap = win32gui.CreateCompatibleBitmap(self.hdc, width, height)
        self.hobj = win32gui.SelectObject(self.hdc_mem, self.hbitmap)

        # For GetDIBits
        self.bmp_info = self._create_bmpinfo(width, height)
        self.bmp_buffer = (ctypes.c_ubyte * (width * height * 4))()

    def _create_bmpinfo(self, width, height):
        """Create BITMAPINFO for GetDIBits."""
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
            _fields_ = [("bmiHeader", BITMAPINFOHEADER)]

        bmi = BITMAPINFO()
        bmi.bmiHeader.biSize = ctypes.sizeof(BITMAPINFOHEADER)
        bmi.bmiHeader.biWidth = width
        bmi.bmiHeader.biHeight = -height  # Top-down
        bmi.bmiHeader.biPlanes = 1
        bmi.bmiHeader.biBitCount = 32  # BGRA
        bmi.bmiHeader.biCompression = 0  # BI_RGB

        return bmi

    def capture(self):
        """Capture frame using GDI."""
        # Stretch to target resolution
        win32gui.StretchBlt(
            self.hdc_mem, 0, 0, self.target_width, self.target_height,
            self.hdc, 0, 0, self.src_width, self.src_height,
            win32con.SRCCOPY
        )

        # Get bits using simpler approach
        gdi32 = ctypes.windll.gdi32

        # Get bitmap data
        bits = gdi32.GetDIBits(
            int(self.hdc),
            int(self.hbitmap),
            0,
            self.target_height,
            ctypes.byref(self.bmp_buffer),
            ctypes.byref(self.bmp_info),
            0
        )

        # Convert to numpy
        arr = np.frombuffer(self.bmp_buffer, dtype=np.uint8)
        arr = arr.reshape((self.target_height, self.target_width, 4))

        # BGRA -> BGR
        return arr[:, :, :3]

    def close(self):
        """Clean up GDI resources."""
        if hasattr(self, 'hdc'):
            win32gui.SelectObject(self.hdc_mem, self.hobj)
            win32gui.DeleteObject(self.hbitmap)
            win32gui.DeleteDC(self.hdc_mem)
            win32gui.ReleaseDC(self.hwnd, self.hdc)


class LiveDisplayGDI:
    """Live display using GDI capture."""

    def __init__(self, width=1920, height=1080):
        self.width = width
        self.height = height
        self.running = False

        # Stats
        self.frame_count = 0
        self.start_time = 0
        self.current_fps = 0

        # Initialize capture
        if HAS_GDI:
            self.capture = GDICapture(width, height)
            print(f"GDI capture initialized: {width}x{height}")
        else:
            raise RuntimeError("GDI not available")

    def run(self):
        """Run display loop."""
        print("="*60)
        print("Live Display Test - GDI Capture")
        print("="*60)
        print("Press ESC or Q to exit")
        print("="*60)

        self.running = True
        self.start_time = time.time()
        last_fps_update = self.start_time

        cv2.namedWindow("GDI Live Capture", cv2.WINDOW_NORMAL)

        try:
            while self.running:
                loop_start = time.perf_counter()

                # Capture
                frame = self.capture.capture()
                if frame is None:
                    continue

                self.frame_count += 1

                # Update FPS
                now = time.time()
                if now - last_fps_update >= 0.1:  # Update more frequently
                    self.current_fps = self.frame_count / (now - self.start_time)
                    last_fps_update = now

                # Draw overlay
                h, w = frame.shape[:2]

                overlay = frame.copy()
                cv2.rectangle(overlay, (5, 5), (400, 150), (0, 0, 0), -1)
                frame = cv2.addWeighted(overlay, 0.6, frame, 0.4, 0)

                # FPS color based on performance
                if self.current_fps >= 50:
                    fps_color = (0, 200, 0)
                    rating = "⭐⭐⭐ Excellent"
                elif self.current_fps >= 30:
                    fps_color = (0, 200, 200)
                    rating = "⭐⭐ Good"
                elif self.current_fps >= 20:
                    fps_color = (0, 150, 255)
                    rating = "⭐ Fair"
                else:
                    fps_color = (0, 0, 255)
                    rating = "❌ Poor"

                cv2.putText(frame, f"GDI FPS: {self.current_fps:.1f}", (15, 40),
                           cv2.FONT_HERSHEY_SIMPLEX, 1.0, fps_color, 2)

                cv2.putText(frame, f"Frames: {self.frame_count}", (15, 70),
                           cv2.FONT_HERSHEY_SIMPLEX, 0.45, (255, 255, 255), 1)
                cv2.putText(frame, f"Resolution: {w}x{h}", (15, 90),
                           cv2.FONT_HERSHEY_SIMPLEX, 0.45, (255, 255, 255), 1)
                cv2.putText(frame, f"Backend: GDI (Fastest)", (15, 110),
                           cv2.FONT_HERSHEY_SIMPLEX, 0.45, (200, 255, 200), 1)
                cv2.putText(frame, rating, (15, 140),
                           cv2.FONT_HERSHEY_SIMPLEX, 0.5, fps_color, 1)

                # Show
                cv2.imshow("GDI Live Capture", frame)

                # Exit
                key = cv2.waitKey(1) & 0xFF
                if key == 27 or key == ord('q'):
                    break

        finally:
            self.capture.close()
            cv2.destroyAllWindows()

        # Final stats
        total_time = time.time() - self.start_time

        print("\n" + "="*60)
        print("Test Complete")
        print("="*60)
        print(f"Duration: {total_time:.1f}s")
        print(f"Frames: {self.frame_count}")
        print(f"Average FPS: {self.frame_count / total_time:.1f}")
        print(f"Resolution: {self.width}x{self.height}")
        print("="*60)


if __name__ == "__main__":
    import argparse

    parser = argparse.ArgumentParser()
    parser.add_argument("--width", type=int, default=1920)
    parser.add_argument("--height", type=int, default=1080)
    args = parser.parse_args()

    try:
        test = LiveDisplayGDI(width=args.width, height=args.height)
        test.run()
    except Exception as e:
        print(f"\nError: {e}")
        import traceback
        traceback.print_exc()
