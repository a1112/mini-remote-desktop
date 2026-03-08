#!/usr/bin/env python3
"""
测试所有可用的屏幕捕获库。

对比不同库的性能和可用性。
"""
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent / 'src'))

import numpy as np


def test_library_available():
    """检查各个库的可用性。"""
    print("="*70)
    print("屏幕捕获库可用性检查")
    print("="*70)

    libraries = []

    # 1. MSS (已知可用)
    print("\n1. MSS:")
    try:
        import mss
        print("   ✅ 已安装")
        libraries.append(("MSS", True, "mss"))
    except ImportError:
        print("   ❌ 未安装")

    # 2. d3dshot
    print("\n2. d3dshot:")
    try:
        import d3dshot
        print("   ✅ 已安装")
        libraries.append(("d3dshot", True, "d3dshot"))
    except ImportError:
        print("   ❌ 未安装 (与 Python 3.12 不兼容)")

    # 3. pyscreenshot
    print("\n3. pyscreenshot:")
    try:
        import pyscreenshot
        print("   ✅ 已安装")
        libraries.append(("pyscreenshot", True, "pyscreenshot"))
    except ImportError:
        print("   ❌ 未安装")

    # 4. pyautogui
    print("\n4. pyautogui:")
    try:
        import pyautogui
        print("   ✅ 已安装")
        libraries.append(("pyautogui", True, "pyautogui"))
    except ImportError:
        print("   ❌ 未安装")

    # 5. PIL.ImageGrab
    print("\n5. PIL.ImageGrab:")
    try:
        from PIL import ImageGrab
        print("   ✅ 已安装")
        libraries.append(("PIL.ImageGrab", True, "PIL"))
    except ImportError:
        print("   ❌ 未安装")

    # 6. win32gui (pywin32)
    print("\n6. win32gui (pywin32):")
    try:
        import win32gui
        import win32con
        import win32ui
        print("   ✅ 已安装")
        libraries.append(("win32gui", True, "win32"))
    except ImportError:
        print("   ❌ 未安装")

    # 7. d3d11
    print("\n7. d3d11 (DirectX 11):")
    try:
        import d3d11
        print("   ✅ 已安装")
        libraries.append(("d3d11", True, "d3d11"))
    except ImportError:
        print("   ❌ 未安装")

    # 8. d3d12
    print("\n8. d3d12 (DirectX 12):")
    try:
        import d3d12
        print("   ✅ 已安装")
        libraries.append(("d3d12", True, "d3d12"))
    except ImportError:
        print("   ❌ 未安装")

    # 9. moderngl (OpenGL)
    print("\n9. moderngl (OpenGL):")
    try:
        import moderngl
        print("   ✅ 已安装")
        libraries.append(("moderngl", True, "moderngl"))
    except ImportError:
        print("   ❌ 未安装")

    # 10. pygetwindow (辅助)
    print("\n10. pygetwindow:")
    try:
        import pygetwindow
        print("   ✅ 已安装")
    except ImportError:
        print("   ❌ 未安装")

    return libraries


def benchmark_library(name, duration=3):
    """基准测试单个库。"""
    print(f"\n{'='*70}")
    print(f"{name} 性能测试")
    print(f"{'='*70}")

    try:
        if name == "MSS":
            return benchmark_mss(duration)
        elif name == "PIL.ImageGrab":
            return benchmark_pil(duration)
        elif name == "pyautogui":
            return benchmark_pyautogui(duration)
        elif name == "pyscreenshot":
            return benchmark_pyscreenshot(duration)
        elif name == "win32gui":
            return benchmark_win32gui(duration)
        else:
            print(f"   暂不支持此库的测试")
            return 0, []

    except Exception as e:
        print(f"   错误: {e}")
        return 0, []


def benchmark_mss(duration=3):
    """MSS 基准测试。"""
    import mss
    import ctypes

    sct = mss.mss()

    user32 = ctypes.windll.user32
    screen_w = user32.GetSystemMetrics(0)
    screen_h = user32.GetSystemMetrics(1)

    # 测试 1080p
    scale = min(1920 / screen_w, 1080 / screen_h)
    w = int(screen_w * scale)
    h = int(screen_h * scale)

    monitor = {"left": 0, "top": 0, "width": w, "height": h}

    print(f"   分辨率: {w}x{h}")
    print("   测试中...")

    times = []
    start = time.time()

    while time.time() - start < duration:
        t0 = time.perf_counter()
        img = sct.grab(monitor)
        arr = np.frombuffer(img.rgb, dtype=np.uint8)
        frame = arr.reshape((h, w, 3))
        t1 = time.perf_counter()
        times.append((t1 - t0) * 1000)

    fps = len(times) / duration
    print(f"   FPS: {fps:.1f}")
    print(f"   平均: {sum(times)/len(times):.1f} ms")

    return fps, times


def benchmark_pil(duration=3):
    """PIL.ImageGrab 基准测试。"""
    from PIL import ImageGrab

    print("   测试中...")

    times = []
    start = time.time()

    while time.time() - start < duration:
        t0 = time.perf_counter()
        img = ImageGrab.grab()
        # 转换为 numpy
        arr = np.array(img)
        t1 = time.perf_counter()
        times.append((t1 - t0) * 1000)

    fps = len(times) / duration
    print(f"   FPS: {fps:.1f}")
    print(f"   平均: {sum(times)/len(times):.1f} ms")

    return fps, times


def benchmark_pyautogui(duration=3):
    """pyautogui 基准测试。"""
    import pyautogui

    print("   测试中...")

    times = []
    start = time.time()

    while time.time() - start < duration:
        t0 = time.perf_counter()
        img = pyautogui.screenshot()
        arr = np.array(img)
        t1 = time.perf_counter()
        times.append((t1 - t0) * 1000)

    fps = len(times) / duration
    print(f"   FPS: {fps:.1f}")
    print(f"   平均: {sum(times)/len(times):.1f} ms")

    return fps, times


def benchmark_pyscreenshot(duration=3):
    """pyscreenshot 基准测试。"""
    import pyscreenshot

    print("   测试中...")

    times = []
    start = time.time()

    while time.time() - start < duration:
        t0 = time.perf_counter()
        img = pyscreenshot.grab()
        arr = np.array(img)
        t1 = time.perf_counter()
        times.append((t1 - t0) * 1000)

    fps = len(times) / duration
    print(f"   FPS: {fps:.1f}")
    print(f"   平均: {sum(times)/len(times):.1f} ms")

    return fps, times


def benchmark_win32gui(duration=3):
    """win32gui GDI 基准测试。"""
    import win32gui
    import win32con
    import win32ui
    import ctypes

    user32 = ctypes.windll.user32
    screen_w = user32.GetSystemMetrics(0)
    screen_h = user32.GetSystemMetrics(1)

    w, h = 1920, 1080
    scale = min(w / screen_w, h / screen_h)
    capture_w = int(screen_w * scale)
    capture_h = int(screen_h * scale)

    hwnd = win32gui.GetDesktopWindow()
    hdc = win32gui.GetDC(hwnd)
    hdc_mem = win32gui.CreateCompatibleDC(hdc)
    hbitmap = win32gui.CreateCompatibleBitmap(hdc, capture_w, capture_h)
    hobj = win32gui.SelectObject(hdc_mem, hbitmap)

    print(f"   分辨率: {capture_w}x{capture_h}")
    print("   测试中...")

    times = []
    start = time.time()

    while time.time() - start < duration:
        t0 = time.perf_counter()

        # 捕获
        win32gui.StretchBlt(
            hdc_mem, 0, 0, capture_w, capture_h,
            hdc, 0, 0, screen_w, screen_h,
            win32con.SRCCOPY
        )

        # 获取数据
        bmpinfo = win32gui.GetObject(hbitmap)
        bmpstr = win32gui.GetBitmapBits(hbitmap, capture_w * capture_h * 4)
        arr = np.frombuffer(bmpstr, dtype=np.uint8)
        frame = arr.reshape((capture_h, capture_w, 4))

        t1 = time.perf_counter()
        times.append((t1 - t0) * 1000)

    # 清理
    win32gui.SelectObject(hdc_mem, hobj)
    win32gui.DeleteObject(hbitmap)
    win32gui.DeleteDC(hdc_mem)
    win32gui.ReleaseDC(hwnd, hdc)

    fps = len(times) / duration
    print(f"   FPS: {fps:.1f}")
    print(f"   平均: {sum(times)/len(times):.1f} ms")

    return fps, times


def compare_all():
    """对比所有可用库的性能。"""
    libraries = test_library_available()

    if not libraries:
        print("\n❌ 没有可用的捕获库")
        return

    print("\n" + "="*70)
    print("性能对比 (3秒测试)")
    print("="*70)

    results = []

    for name, available, module in libraries:
        if available:
            fps, times = benchmark_library(name, duration=3)
            if fps > 0:
                results.append((name, fps, times))

    # 总结
    print("\n" + "="*70)
    print("总结")
    print("="*70)

    if results:
        results.sort(key=lambda x: x[1], reverse=True)

        print(f"\n{'库':<20} {'FPS':<10} {'评级':<10}")
        print("-"*50)

        for name, fps, times in results:
            if fps >= 50:
                rating = "🚀🚀🚀"
            elif fps >= 30:
                rating = "🚀🚀"
            elif fps >= 20:
                rating = "⚡⚡"
            elif fps >= 10:
                rating = "⚡"
            else:
                rating = "💻"

            print(f"{name:<20} {fps:<10.1f} {rating}")

        winner = results[0]
        print(f"\n🏆 最快: {winner[0]} ({winner[1]:.1f} FPS)")


def show_installation_commands():
    """显示安装命令。"""
    print("\n" + "="*70)
    print("安装命令")
    print("="*70)
    print("""
# 已有
pip install mss
pip install pywin32  # win32gui

# 可尝试
pip install pillow          # PIL.ImageGrab
pip install pyautogui       # pyautogui
pip install pyscreenshot    # pyscreenshot

# 实验性/需要编译
pip install d3d11           # DirectX 11
pip install d3d12           # DirectX 12
pip install moderngl        # OpenGL

# 不兼容 Python 3.12
# pip install d3dshot       # 需要 pillow<7.2
    """)


if __name__ == "__main__":
    compare_all()
    show_installation_commands()
