#!/usr/bin/env python3
"""
优化的 win32gui GDI 捕获测试。

使用正确的 API 获取位图数据。
"""
import sys
import time
import ctypes
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent / 'src'))

import numpy as np


def benchmark_win32gui_optimized(duration=5):
    """优化的 win32gui 捕获。"""
    import win32gui
    import win32con
    import win32ui

    user32 = ctypes.windll.user32
    screen_w = user32.GetSystemMetrics(0)
    screen_h = user32.GetSystemMetrics(1)

    # 目标分辨率
    target_w, target_h = 1920, 1080
    scale = min(target_w / screen_w, target_h / screen_h)
    capture_w = int(screen_w * scale)
    capture_h = int(screen_h * scale)

    print(f"屏幕: {screen_w}x{screen_h}")
    print(f"捕获: {capture_w}x{capture_h}")

    # 创建 DC
    hwnd = win32gui.GetDesktopWindow()
    hdc = win32gui.GetDC(hwnd)
    hdc_mem = win32gui.CreateCompatibleDC(hdc)
    hbitmap = win32gui.CreateCompatibleBitmap(hdc, capture_w, capture_h)
    hobj = win32gui.SelectObject(hdc_mem, hbitmap)

    # 定义 BITMAPINFOHEADER
    class BITMAPINFOHEADER(ctypes.Structure):
        _fields_ = [
            ("biSize", ctypes.c_uint32),
            ("biWidth", ctypes.c_int),
            ("biHeight", ctypes.c_int),
            ("biPlanes", ctypes.c_ushort),
            ("biBitCount", ctypes.c_ushort),
            ("biCompression", ctypes.c_uint32),
            ("biSizeImage", ctypes.c_uint32),
            ("biXPelsPerMeter", ctypes.c_long),
            ("biYPelsPerMeter", ctypes.c_long),
            ("biClrUsed", ctypes.c_uint32),
            ("biClrImportant", ctypes.c_uint32),
        ]

    class BITMAPINFO(ctypes.Structure):
        _fields_ = [
            ("bmiHeader", BITMAPINFOHEADER),
        ]

    print("\n测试中...")

    times = []
    start = time.time()

    # 准备 BITMAPINFO
    bmi = BITMAPINFO()
    bmi.bmiHeader.biSize = ctypes.sizeof(BITMAPINFOHEADER)
    bmi.bmiHeader.biWidth = capture_w
    bmi.bmiHeader.biHeight = -capture_h  # top-down
    bmi.bmiHeader.biPlanes = 1
    bmi.bmiHeader.biBitCount = 32  # BGRA
    bmi.bmiHeader.biCompression = 0  # BI_RGB

    buffer_size = capture_w * capture_h * 4
    bmp_buffer = (ctypes.c_ubyte * buffer_size)()

    while time.time() - start < duration:
        t0 = time.perf_counter()

        # 捕获并缩放
        win32gui.StretchBlt(
            hdc_mem, 0, 0, capture_w, capture_h,
            hdc, 0, 0, screen_w, screen_h,
            win32con.SRCCOPY
        )

        # 获取数据
        gdi32 = ctypes.windll.gdi32
        gdi32.GetDIBits(
            int(hdc),
            int(hbitmap),
            0,
            capture_h,
            ctypes.byref(bmp_buffer),
            ctypes.byref(bmi),
            0
        )

        # 转换为 numpy
        arr = np.frombuffer(bmp_buffer, dtype=np.uint8)
        frame = arr.reshape((capture_h, capture_w, 4))
        frame = frame[:, :, :3][:, :, [2, 1, 0]]  # BGRA -> RGB

        t1 = time.perf_counter()
        times.append((t1 - t0) * 1000)

    # 清理
    win32gui.SelectObject(hdc_mem, hobj)
    win32gui.DeleteObject(hbitmap)
    win32gui.DeleteDC(hdc_mem)
    win32gui.ReleaseDC(hwnd, hdc)

    fps = len(times) / duration

    print(f"\n结果:")
    print(f"  捕获帧数: {len(times)}")
    print(f"  FPS: {fps:.1f}")
    print(f"  平均: {sum(times)/len(times):.1f} ms")
    print(f"  最快: {min(times):.1f} ms")
    print(f"  最慢: {max(times):.1f} ms")

    return fps, times


if __name__ == "__main__":
    print("="*70)
    print("win32gui 优化捕获测试")
    print("="*70)

    try:
        fps, times = benchmark_win32gui_optimized(duration=5)

        if fps >= 50:
            rating = "🚀🚀🚀"
        elif fps >= 30:
            rating = "🚀🚀"
        elif fps >= 20:
            rating = "⚡⚡"
        else:
            rating = "⚡"

        print(f"\n评级: {rating}")

        # 与 MSS 对比
        print(f"\n对比:")
        print(f"  win32gui: {fps:.1f} FPS")
        print(f"  MSS:      30.3 FPS (之前的测试)")

    except Exception as e:
        print(f"错误: {e}")
        import traceback
        traceback.print_exc()
