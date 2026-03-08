#!/usr/bin/env python3
"""Test GDI capture performance."""
import time
import statistics
import numpy as np
import win32gui
import win32con

# Initialize GDI
hwnd = win32gui.GetDesktopWindow()
hdc = win32gui.GetDC(hwnd)
width = win32gui.GetSystemMetrics(0)
height = win32gui.GetSystemMetrics(1)
hdc_mem = win32gui.CreateCompatibleDC(hdc)
hbitmap = win32gui.CreateCompatibleBitmap(hdc, width, height)
hobj = win32gui.SelectObject(hdc_mem, hbitmap)

print(f"Resolution: {width}x{height}")
print("Warming up...")
for _ in range(5):
    win32gui.BitBlt(hdc_mem, 0, 0, width, height, hdc, 0, 0, win32con.SRCCOPY)

# Benchmark
times = []
start = time.time()
duration = 3.0

while time.time() - start < duration:
    t0 = time.perf_counter()
    win32gui.BitBlt(hdc_mem, 0, 0, width, height, hdc, 0, 0, win32con.SRCCOPY)
    t1 = time.perf_counter()
    times.append((t1 - t0) * 1000)

fps = len(times) / duration
avg = statistics.mean(times)
p95 = statistics.quantiles(times, n=20)[18]
p99 = statistics.quantiles(times, n=100)[98]

print(f"\nGDI Capture Performance:")
print(f"  FPS: {fps:.1f}")
print(f"  Avg: {avg:.2f} ms")
print(f"  P95: {p95:.2f} ms")
print(f"  P99: {p99:.2f} ms")

# Cleanup
win32gui.SelectObject(hdc_mem, hobj)
win32gui.DeleteObject(hbitmap)
win32gui.DeleteDC(hdc_mem)
win32gui.ReleaseDC(hwnd, hdc)

# Rating
if fps >= 50:
    print(f"  Rating: ⭐⭐⭐ (Excellent)")
elif fps >= 30:
    print(f"  Rating: ⭐⭐ (Good)")
elif fps >= 15:
    print(f"  Rating: ⭐ (Fair)")
else:
    print(f"  Rating: ❌ (Poor)")
