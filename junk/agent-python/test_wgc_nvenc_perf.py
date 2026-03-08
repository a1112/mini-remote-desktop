#!/usr/bin/env python3
"""
WGC + NVENC 性能测试

测试监视器捕获和窗口捕获的性能
"""

import asyncio
import ctypes
import logging
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent / 'src'))

from src.capture.wgc_capture import WGCCapture
from src.config import AgentConfig

logging.basicConfig(level=logging.WARNING)


async def test_monitor_performance():
    """测试监视器捕获性能"""
    print("=" * 70)
    print("监视器 WGC + NVENC 性能测试")
    print("=" * 70)
    print()

    capture = WGCCapture()

    if not capture.start_monitor(0):
        print("✗ 无法启动监视器捕获")
        print("  提示: 关闭 Game Bar / NVIDIA Share")
        return False

    print(f"✓ 监视器捕获已启动")
    print(f"  分辨率: {capture.width}x{capture.height}")
    print(f"  D3D11 设备: {hex(capture.d3d11_device)}")
    print()

    # 等待首帧
    print("等待屏幕更新...")
    for _ in range(10):
        frame = capture.capture_frame()
        if frame:
            print(f"✓ 首帧捕获: {frame.width}x{frame.height}")
            break
        await asyncio.sleep(0.1)

    # 性能测试
    print()
    print("捕获性能测试 (5秒，无 sleep)...")
    print("-" * 70)

    buffer_size = capture.width * capture.height * 4
    buffer = (ctypes.c_ubyte * buffer_size)()

    frames = 0
    capture_times = []
    start_time = time.perf_counter()

    while time.perf_counter() - start_time < 5.0:
        loop_start = time.perf_counter()

        frame = capture.capture_frame()
        if frame:
            if capture.copy_to_cpu(buffer):
                frames += 1
                capture_time = (time.perf_counter() - loop_start) * 1000
                capture_times.append(capture_time)

                if frames <= 5 or frames % 60 == 0:
                    print(f"  帧 {frames}: 捕获 {capture_time:.3f}ms")

    total_time = time.perf_counter() - start_time
    fps = frames / total_time if total_time > 0 else 0

    print()
    print("性能统计:")
    print(f"  总帧数: {frames}")
    print(f"  实际 FPS: {fps:.1f}")

    if capture_times:
        avg_time = sum(capture_times) / len(capture_times)
        max_time = max(capture_times)
        min_time = min(capture_times)
        p95_time = sorted(capture_times)[int(len(capture_times) * 0.95)] if capture_times else 0

        print(f"  捕获延迟:")
        print(f"    平均: {avg_time:.3f} ms")
        print(f"    最小: {min_time:.3f} ms")
        print(f"    最大: {max_time:.3f} ms")
        print(f"    P95:  {p95_time:.3f} ms")

    # 评级
    if fps >= 120:
        rating = "🚀 A+ - 超过 120fps!"
    elif fps >= 60:
        rating = "✓ A - 优秀 (超过 60fps)"
    elif fps >= 30:
        rating = "⚠ B - 良好"
    else:
        rating = "✗ C - 需优化"

    print()
    print(f"评级: {rating}")

    capture.stop()
    return True


async def test_window_performance(hwnd: int = None):
    """测试窗口捕获性能"""
    if hwnd is None:
        print()
    print("=" * 70)
    print("窗口 WGC + NVENC 性能测试")
    print("=" * 70)
    print()

    capture = WGCCapture()

    # 如果没有指定 HWND，让用户选择
    if hwnd is None:
        windows = WGCCapture.enum_windows()
        print(f"发现 {len(windows)} 个窗口:")
        print()

        for i, w in enumerate(windows[:20]):
            marker = " <- 主窗口" if w.is_visible else ""
            title_short = (w.title[:50] + "...") if len(w.title) > 50 else w.title
            print(f"  [{i}] 0x{w.hwnd:X} - {title_short}{marker}")

        print()
        try:
            choice = input("选择窗口编号 (或输入 HWND 如 0x123456): ").strip()
            if choice.startswith("0x") or choice.startswith("0X"):
                hwnd = int(choice, 16)
            else:
                idx = int(choice)
                hwnd = windows[idx].hwnd
        except (ValueError, IndexError):
            print("无效输入")
            return False

    print(f"目标 HWND: 0x{hwnd:X}")
    print()

    if not capture.start_window(hwnd):
        print("✗ 无法启动窗口捕获")
        return False

    print(f"✓ 窗口捕获已启动")
    print(f"  分辨率: {capture.width}x{capture.height}")
    print()

    # 等待首帧
    print("等待窗口更新...")
    for _ in range(20):
        frame = capture.capture_frame()
        if frame:
            print(f"✓ 首帧捕获: {frame.width}x{frame.height}")
            break
        await asyncio.sleep(0.1)

    # 性能测试
    print()
    print("捕获性能测试 (5秒)...")
    print("-" * 70)

    buffer_size = capture.width * capture.height * 4
    buffer = (ctypes.c_ubyte * buffer_size)()

    frames = 0
    capture_times = []
    start_time = time.perf_counter()

    while time.perf_counter() - start_time < 5.0:
        loop_start = time.perf_counter()

        frame = capture.capture_frame()
        if frame:
            if capture.copy_to_cpu(buffer):
                frames += 1
                capture_time = (time.perf_counter() - loop_start) * 1000
                capture_times.append(capture_time)

                if frames <= 5 or frames % 30 == 0:
                    print(f"  帧 {frames}: 捕获 {capture_time:.3f}ms")

    total_time = time.perf_counter() - start_time
    fps = frames / total_time if total_time > 0 else 0

    print()
    print("性能统计:")
    print(f"  总帧数: {frames}")
    print(f"  实际 FPS: {fps:.1f}")

    if capture_times:
        avg_time = sum(capture_times) / len(capture_times)
        print(f"  平均捕获延迟: {avg_time:.3f} ms")

    capture.stop()
    return True


async def main():
    """主函数"""
    import argparse

    parser = argparse.ArgumentParser(description="WGC + NVENC 性能测试")
    parser.add_argument("--mode", choices=["monitor", "window", "both"], default="monitor",
                       help="捕获模式")
    parser.add_argument("--hwnd", type=lambda x: int(x, 16), help="窗口 HWND (十六进制)")

    args = parser.parse_args()

    if args.mode in ["monitor", "both"]:
        await test_monitor_performance()

    if args.mode in ["window", "both"]:
        await test_window_performance(args.hwnd)


if __name__ == "__main__":
    try:
        asyncio.run(main())
    except KeyboardInterrupt:
        print("\n测试中断")
