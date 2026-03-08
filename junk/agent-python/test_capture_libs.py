#!/usr/bin/env python3
"""
Screen Capture Library Performance Comparison

Tests various Python screen capture libraries with C/C++ bindings.
"""
import sys
import time
import statistics
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent / "src"))


def test_d3dshot():
    """Test d3dshot (DirectX, fastest on Windows)."""
    print("\n[1] d3dshot (DirectX)")
    print("-" * 40)

    try:
        import d3dshot

        d3d = d3dshot.create(capture_output="numpy")
        if d3d is None:
            print("  ❌ d3dshot not available")
            return None, None

        print(f"  Resolution: {d3d.display_resolution}")
        print("  Warming up...")
        for _ in range(5):
            d3d.screenshot()

        # Benchmark
        times = []
        start = time.time()
        duration = 2.0

        while time.time() - start < duration:
            t0 = time.perf_counter()
            img = d3d.screenshot()
            t1 = time.perf_counter()
            if img is not None:
                times.append((t1 - t0) * 1000)

        if times:
            fps = len(times) / duration
            avg = statistics.mean(times)
            p95 = statistics.quantiles(times, n=20)[18]
            print(f"  ✅ FPS: {fps:.1f}")
            print(f"     Avg: {avg:.2f} ms, P95: {p95:.2f} ms")
            return fps, avg

    except ImportError:
        print("  ❌ d3dshot not installed")
        return None, None
    except Exception as e:
        print(f"  ❌ Error: {e}")
        return None, None


def test_mss():
    """Test mss (cross-platform, uses ctypes)."""
    print("\n[2] mss (cross-platform)")
    print("-" * 40)

    try:
        import mss

        ms = mss.mss()
        mon = ms.monitors[1]  # Primary monitor
        print(f"  Resolution: {mon['width']}x{mon['height']}")
        print("  Warming up...")
        for _ in range(5):
            ms.grab(mon)

        # Benchmark
        times = []
        start = time.time()
        duration = 2.0

        while time.time() - start < duration:
            t0 = time.perf_counter()
            img = ms.grab(mon)
            t1 = time.perf_counter()
            times.append((t1 - t0) * 1000)

        fps = len(times) / duration
        avg = statistics.mean(times)
        p95 = statistics.quantiles(times, n=20)[18]
        print(f"  ✅ FPS: {fps:.1f}")
        print(f"     Avg: {avg:.2f} ms, P95: {p95:.2f} ms")
        return fps, avg

    except ImportError:
        print("  ❌ mss not installed")
        return None, None
    except Exception as e:
        print(f"  ❌ Error: {e}")
        return None, None


def test_pil():
    """Test PIL.ImageGrab (pure Python)."""
    print("\n[3] PIL.ImageGrab (pure Python)")
    print("-" * 40)

    try:
        from PIL import ImageGrab
        import ctypes

        user32 = ctypes.windll.user32
        width = user32.GetSystemMetrics(0)
        height = user32.GetSystemMetrics(1)
        print(f"  Resolution: {width}x{height}")
        print("  Warming up...")
        for _ in range(5):
            ImageGrab.grab()

        # Benchmark
        times = []
        start = time.time()
        duration = 2.0

        while time.time() - start < duration:
            t0 = time.perf_counter()
            img = ImageGrab.grab()
            t1 = time.perf_counter()
            times.append((t1 - t0) * 1000)

        fps = len(times) / duration
        avg = statistics.mean(times)
        p95 = statistics.quantiles(times, n=20)[18]
        print(f"  ✅ FPS: {fps:.1f}")
        print(f"     Avg: {avg:.2f} ms, P95: {p95:.2f} ms")
        return fps, avg

    except ImportError:
        print("  ❌ PIL not installed")
        return None, None
    except Exception as e:
        print(f"  ❌ Error: {e}")
        return None, None


def test_windows_capture():
    """Test windows-capture (WinRT, newest)."""
    print("\n[4] windows-capture (WinRT API)")
    print("-" * 40)

    try:
        from windows_capture import WindowsCapture, Frame
        import threading

        # This is async-based, simpler test
        print("  Testing windows_capture...")

        class CaptureTester:
            def __init__(self):
                self.results = []
                self.running = False

            def on_frame_arrived(self, frame):
                t1 = time.perf_counter()
                if self.start_time:
                    self.results.append((t1 - self.start_time) * 1000)
                    if len(self.results) >= 30:  # Collect 30 frames
                        self.running = False

        tester = CaptureTester()

        # Simple test
        def capture_thread():
            try:
                wc = WindowsCapture()
                wc.register_frame_arrived_callback(tester.on_frame_arrived)
                wc.start_capture()

                # Capture for 2 seconds
                start = time.time()
                while time.time() - start < 2.0:
                    tester.start_time = time.perf_counter()
                    time.sleep(0.01)

                wc.stop_capture()
            except Exception as e:
                print(f"     Error: {e}")

        # Run in thread
        import threading
        thread = threading.Thread(target=capture_thread)
        thread.start()
        thread.join(timeout=5)

        if tester.results:
            fps = len(tester.results) / 2.0
            avg = statistics.mean(tester.results)
            print(f"  ✅ FPS: {fps:.1f}")
            print(f"     Avg: {avg:.2f} ms")
            return fps, avg
        else:
            print("  ⚠️  No frames captured (may need specific setup)")
            return None, None

    except ImportError:
        print("  ❌ windows-capture not installed (pip install windows-capture)")
        return None, None
    except Exception as e:
        print(f"  ❌ Error: {e}")
        return None, None


def test_pywin32_gdi():
    """Test pywin32 with direct GDI calls (fast for simple cases)."""
    print("\n[5] pywin32 GDI (Windows API)")
    print("-" * 40)

    try:
        import win32gui
        import win32ui
        import win32con

        hwnd = win32gui.GetDesktopWindow()
        hdc = win32gui.GetDC(hwnd)
        hdc_mem = win32gui.CreateCompatibleDC(hdc)
        hbitmap = win32gui.CreateCompatibleBitmap(hdc, 1920, 1080)
        hobj = win32gui.SelectObject(hdc_mem, hbitmap)

        print("  Warming up...")
        for _ in range(5):
            win32gui.BitBlt(hdc_mem, 0, 0, 1920, 1080, hdc, 0, 0, win32con.SRCCOPY)

        # Benchmark
        times = []
        start = time.time()
        duration = 2.0

        while time.time() - start < duration:
            t0 = time.perf_counter()
            win32gui.BitBlt(hdc_mem, 0, 0, 1920, 1080, hdc, 0, 0, win32con.SRCCOPY)
            t1 = time.perf_counter()
            times.append((t1 - t0) * 1000)

        # Cleanup
        win32gui.SelectObject(hdc_mem, hobj)
        win32gui.DeleteObject(hbitmap)
        win32gui.DeleteDC(hdc_mem)
        win32gui.ReleaseDC(hwnd, hdc)

        fps = len(times) / duration
        avg = statistics.mean(times)
        p95 = statistics.quantiles(times, n=20)[18]
        print(f"  ✅ FPS: {fps:.1f}")
        print(f"     Avg: {avg:.2f} ms, P95: {p95:.2f} ms")
        print(f"     Note: Fixed 1920x1080 resolution")
        return fps, avg

    except ImportError:
        print("  ❌ pywin32 not installed")
        return None, None
    except Exception as e:
        print(f"  ❌ Error: {e}")
        return None, None


def test_pyautogui():
    """Test pyautogui (wrapper around PIL)."""
    print("\n[6] pyautogui (PIL wrapper)")
    print("-" * 40)

    try:
        import pyautogui

        print("  Warming up...")
        for _ in range(3):
            pyautogui.screenshot()

        # Benchmark
        times = []
        start = time.time()
        duration = 2.0

        while time.time() - start < duration:
            t0 = time.perf_counter()
            img = pyautogui.screenshot()
            t1 = time.perf_counter()
            times.append((t1 - t0) * 1000)

        fps = len(times) / duration
        avg = statistics.mean(times)
        print(f"  ✅ FPS: {fps:.1f}")
        print(f"     Avg: {avg:.2f} ms")
        return fps, avg

    except ImportError:
        print("  ❌ pyautogui not installed")
        return None, None
    except Exception as e:
        print(f"  ❌ Error: {e}")
        return None, None


def main():
    """Run all tests and compare."""
    print("=" * 50)
    print("Python Screen Capture Library Performance")
    print("=" * 50)
    print(f"Platform: {sys.platform}")

    results = {}

    # Run all tests
    fps_d3d, avg_d3d = test_d3dshot()
    results['d3dshot'] = (fps_d3d, avg_d3d)

    fps_mss, avg_mss = test_mss()
    results['mss'] = (fps_mss, avg_mss)

    fps_pil, avg_pil = test_pil()
    results['PIL'] = (fps_pil, avg_pil)

    fps_wc, avg_wc = test_windows_capture()
    results['windows_capture'] = (fps_wc, avg_wc)

    fps_gdi, avg_gdi = test_pywin32_gdi()
    results['GDI'] = (fps_gdi, avg_gdi)

    fps_pg, avg_pg = test_pyautogui()
    results['pyautogui'] = (fps_pg, avg_pg)

    # Summary
    print("\n" + "=" * 50)
    print("PERFORMANCE SUMMARY")
    print("=" * 50)
    print(f"{'Library':<20} {'FPS':<10} {'Avg (ms)':<10} {'Rating'}")
    print("-" * 50)

    valid_results = [(k, v[0], v[1]) for k, v in results.items() if v[0] is not None]
    valid_results.sort(key=lambda x: x[1], reverse=True)

    for name, fps, avg in valid_results:
        rating = "⭐⭐⭐" if fps >= 50 else "⭐⭐" if fps >= 30 else "⭐" if fps >= 15 else "❌"
        print(f"{name:<20} {fps:<10.1f} {avg:<10.1f} {rating}")

    # Recommendations
    print("\n" + "=" * 50)
    print("RECOMMENDATIONS")
    print("=" * 50)

    best = valid_results[0] if valid_results else (None, 0, 0)
    if best[0]:
        best_fps = best[1]
        print(f"\n🏆 FASTEST: {best[0]} ({best_fps:.1f} FPS)")
        print(f"   pip install {best[0]}")

    print("\n📦 Installation:")
    print("   pip install d3dshot       # DirectX, Windows only")
    print("   pip install mss           # Cross-platform")
    print("   pip install windows-capture  # WinRT, Win10+")
    print("   pip install pywin32       # Windows API")


if __name__ == "__main__":
    main()
