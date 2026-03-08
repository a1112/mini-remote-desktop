#!/usr/bin/env python3
"""
WGC 捕获性能瓶颈分析

详细分析各阶段的耗时
"""

import sys
import time
import ctypes
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent / 'src'))

from src.capture.wgc_capture import WGCCapture

print("=" * 70)
print("WGC 捕获性能瓶颈分析")
print("=" * 70)
print()

capture = WGCCapture()

if not capture.start_monitor(0):
    print("✗ 无法启动捕获")
    sys.exit(1)

print(f"✓ 捕获已启动: {capture.width}x{capture.height}")
print()

# 等待首帧
print("等待首帧...")
for _ in range(10):
    frame = capture.capture_frame()
    if frame:
        break
    time.sleep(0.1)

print(f"✓ 首帧: {frame.width}x{frame.height}")
print()

# 缓冲区
buffer_size = capture.width * capture.height * 4
buffer = (ctypes.c_ubyte * buffer_size)()

print("-" * 70)
print("性能分析 (捕获 100 帧)...")
print("-" * 70)

# 各阶段耗时统计
capture_only_times = []
copy_times = []
total_times = []

# 预热
for _ in range(10):
    frame = capture.capture_frame()
    if frame:
        capture.copy_to_cpu(buffer)

print(f"{'帧号':<8} {'捕获':<12} {'复制':<12} {'总计':<12}")
print("-" * 70)

for i in range(100):
    # 1. 测量捕获时间
    t1 = time.perf_counter()
    frame = capture.capture_frame()
    t2 = time.perf_counter()
    capture_time = (t2 - t1) * 1000

    if frame:
        # 2. 测量复制时间
        t3 = time.perf_counter()
        capture.copy_to_cpu(buffer)
        t4 = time.perf_counter()
        copy_time = (t4 - t3) * 1000

        total_time = (t4 - t1) * 1000

        capture_only_times.append(capture_time)
        copy_times.append(copy_time)
        total_times.append(total_time)

        if i < 10 or (i + 1) % 20 == 0:
            print(f"{i+1:<8} {capture_time:<12.3f} {copy_time:<12.3f} {total_time:<12.3f}")

print()
print("=" * 70)
print("统计结果")
print("=" * 70)

if capture_only_times:
    import statistics

    avg_capture = statistics.mean(capture_only_times)
    avg_copy = statistics.mean(copy_times)
    avg_total = statistics.mean(total_times)

    p95_capture = statistics.quantiles(capture_only_times, n=20)[18]  # 95th percentile
    p95_copy = statistics.quantiles(copy_times, n=20)[18]
    p95_total = statistics.quantiles(total_times, n=20)[18]

    print("捕获阶段 (capture_frame):")
    print(f"  平均: {avg_capture:.3f} ms")
    print(f"  P95:  {p95_capture:.3f} ms")
    print()

    print("复制阶段 (copy_to_cpu):")
    print(f"  平均: {avg_copy:.3f} ms")
    print(f"  P95:  {p95_copy:.3f} ms")
    print()

    print("总计:")
    print(f"  平均: {avg_total:.3f} ms")
    print(f"  P95:  {p95_total:.3f} ms")
    print()

    # 计算理论 FPS
    theoretical_fps = 1000 / avg_total
    print(f"理论 FPS (基于平均延迟): {theoretical_fps:.1f}")
    print()

    # 瓶颈分析
    print("瓶颈分析:")
    capture_pct = (avg_capture / avg_total) * 100
    copy_pct = (avg_copy / avg_total) * 100
    print(f"  捕获占用: {capture_pct:.1f}%")
    print(f"  复制占用: {copy_pct:.1f}%")

    if avg_capture > 5:
        print()
        print("  ⚠ 主要瓶颈: WGC 捕获")
        print("     可能原因:")
        print("     - AcquireNextFrame 超时等待")
        print("     - 屏幕更新频率低")
        print("     - DXGI 独占模式竞争")

    if avg_copy > 2:
        print()
        print("  ⚠ 次要瓶颈: CPU 复制")
        print("     可能原因:")
        print("     - Map/Unmap 开销")
        print("     - 内存带宽限制")
        print("     - 跨越 GPU-CPU 边界")

print()
print("=" * 70)
print("优化建议:")
print("=" * 70)
print()

print("1. 减少 AcquireNextFrame 超时:")
print("   - 当前: 1ms (在 C++ 中)")
print("   - 建议: 0ms (立即返回) 用于性能测试")

print()
print("2. 使用 D3D11 纹理直接编码 (GPU Direct):")
print("   - 当前: WGC → CPU → NVENC")
print("   - 目标: WGC (D3D11) → NVENC")
print("   - 节省: ~3-5ms 的复制时间")

print()
print("3. 多线程处理:")
print("   - 捕获线程和编码线程分离")
print("   - 允许流水线并行")

print()
print("4. 减少 Python 开销:")
print("   - 使用 ctypes 更高效的调用")
print("   - 批处理帧")

capture.stop()
