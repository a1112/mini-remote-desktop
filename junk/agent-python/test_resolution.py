#!/usr/bin/env python3
"""Test capture performance at different resolutions."""
import time
import statistics
import win32gui
import win32con
import ctypes
import numpy as np

def test_resolution(target_width, target_height, duration=2.0):
    """Test capture at specific resolution."""
    user32 = ctypes.windll.user32

    # Create DC for target resolution
    hwnd = win32gui.GetDesktopWindow()
    hdc = win32gui.GetDC(hwnd)
    hdc_mem = win32gui.CreateCompatibleDC(hdc)
    hbitmap = win32gui.CreateCompatibleBitmap(hdc, target_width, target_height)
    hobj = win32gui.SelectObject(hdc_mem, hbitmap)

    # Source dimensions
    src_width = user32.GetSystemMetrics(0)
    src_height = user32.GetSystemMetrics(1)

    print(f"{target_width}x{target_height}: ", end="")

    # Warmup
    for _ in range(3):
        win32gui.BitBlt(hdc_mem, 0, 0, target_width, target_height,
                      hdc, 0, 0, win32con.SRCCOPY)

    # Benchmark
    times = []
    start = time.time()

    while time.time() - start < duration:
        t0 = time.perf_counter()
        win32gui.BitBlt(hdc_mem, 0, 0, target_width, target_height,
                      hdc, 0, 0, win32con.SRCCOPY)
        t1 = time.perf_counter()
        times.append((t1 - t0) * 1000)

    fps = len(times) / duration
    avg = statistics.mean(times)
    p95 = statistics.quantiles(times, n=20)[18] if times else 0

    print(f"{fps:5.1f} FPS, {avg:5.1f} ms avg, {p95:5.1f} ms P95")

    # Cleanup
    win32gui.SelectObject(hdc_mem, hobj)
    win32gui.DeleteObject(hbitmap)
    win32gui.DeleteDC(hdc_mem)
    win32gui.ReleaseDC(hwnd, hdc)

    return fps

print("="*50)
print("GDI Capture Performance by Resolution")
print("="*50)

# Test different resolutions
resolutions = [
    (1920, 1080, "1080p"),
    (2560, 1440, "1440p"),
    (1280, 720, "720p"),
    (640, 480, "480p"),
    (320, 240, "240p"),
]

print(f"{'Resolution':<12} {'FPS':<8} {'Avg':<10} {'P95':<10}")
print("-"*50)

results = []
for w, h, name in resolutions:
    fps = test_resolution(w, h)
    results.append((name, fps, w, h))

print("\n" + "="*50)
print("Summary")
print("="*50)
print(f"Resolution can significantly impact performance!")
print(f"\nFor 30 FPS @ 1080p or lower, GDI performs well.")
print(f"Full screen 1440p+ requires more optimization.")


# Calculate pixel counts and throughput
print("\n" + "="*50)
print("Pixel Processing Rate")
print("="*50)

for name, fps, w, h in results:
    pixels = w * h
    mpps = (fps * pixels) / 1_000_000
    print(f"{name:<12} {fps:5.1f} FPS = {mpps:5.1f} M pixels/sec")
