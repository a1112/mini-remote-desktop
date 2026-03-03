#!/usr/bin/env python3
"""
DXGI Desktop Duplication 性能测试。

对比 d3dshot (DirectX) 和 MSS (GDI) 的性能。
"""
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent / 'src'))

import cv2
import numpy as np


def test_d3dshot_available():
    """检查 d3dshot 是否可用。"""
    print("="*70)
    print("d3dshot 可用性检查")
    print("="*70)

    try:
        import d3dshot

        print("\n✅ d3dshot 已安装")

        # 创建实例
        d3d = d3dshot.create()

        print(f"   显示器: {d3d.displays}")
        print(f"   分辨率: {d3d.display_resolution}")
        print(f"   捕获模式: {d3d.capture_output}")

        return True

    except ImportError:
        print("\n❌ d3dshot 未安装")
        print("\n安装方法:")
        print("  pip install d3dshot")
        return False
    except Exception as e:
        print(f"\n⚠️  d3dshot 可用但有错误: {e}")
        return False


def test_d3dshot_performance(duration=5):
    """测试 d3dshot 性能。"""
    print("\n" + "="*70)
    print("d3dshot 性能测试")
    print("="*70)

    try:
        import d3dshot

        d3d = d3dshot.create(capture_output="numpy")

        print(f"\n捕获模式: {d3d.capture_output}")
        print(f"测试时长: {duration} 秒")
        print("测试中...")

        frames = []
        times = []
        start = time.time()

        while time.time() - start < duration:
            t0 = time.perf_counter()
            frame = d3d.capture()
            t1 = time.perf_counter()

            if frame is not None:
                frames.append(frame)
                times.append((t1 - t0) * 1000)

        elapsed = time.time() - start
        fps = len(frames) / elapsed

        print(f"\n结果:")
        print(f"  捕获帧数: {len(frames)}")
        print(f"  实际 FPS: {fps:.1f}")
        print(f"  平均捕获时间: {sum(times)/len(times):.1f} ms")
        print(f"  最快捕获: {min(times):.1f} ms")
        print(f"  最慢捕获: {max(times):.1f} ms")

        if fps >= 50:
            print(f"  评级: 🚀🚀🚀 极快!")
        elif fps >= 30:
            print(f"  评级: 🚀🚀 非常快")
        elif fps >= 20:
            print(f"  评级: ⚡ 快")
        else:
            print(f"  评级: 💻 一般")

        return fps, times

    except Exception as e:
        print(f"\n❌ d3dshot 测试失败: {e}")
        import traceback
        traceback.print_exc()
        return 0, []


def test_mss_performance(duration=5):
    """测试 MSS 性能作为对比。"""
    print("\n" + "="*70)
    print("MSS 性能测试 (对比)")
    print("="*70)

    try:
        import mss
        import ctypes

        sct = mss.mss()

        # 计算区域
        user32 = ctypes.windll.user32
        screen_w = user32.GetSystemMetrics(0)
        screen_h = user32.GetSystemMetrics(1)

        target_w = 1920
        target_h = 1080
        scale = min(target_w / screen_w, target_h / screen_h)
        capture_w = int(screen_w * scale)
        capture_h = int(screen_h * scale)

        monitor = {
            "left": (screen_w - capture_w) // 2,
            "top": (screen_h - capture_h) // 2,
            "width": capture_w,
            "height": capture_h,
        }

        print(f"\n捕获区域: {capture_w}x{capture_h}")
        print(f"测试时长: {duration} 秒")
        print("测试中...")

        frames = []
        times = []
        start = time.time()

        while time.time() - start < duration:
            t0 = time.perf_counter()
            screenshot = sct.grab(monitor)
            arr = np.frombuffer(screenshot.rgb, dtype=np.uint8)
            frame = arr.reshape((capture_h, capture_w, 3))
            t1 = time.perf_counter()

            frames.append(frame)
            times.append((t1 - t0) * 1000)

        elapsed = time.time() - start
        fps = len(frames) / elapsed

        print(f"\n结果:")
        print(f"  捕获帧数: {len(frames)}")
        print(f"  实际 FPS: {fps:.1f}")
        print(f"  平均捕获时间: {sum(times)/len(times):.1f} ms")
        print(f"  最快捕获: {min(times):.1f} ms")
        print(f"  最慢捕获: {max(times):.1f} ms")

        return fps, times

    except Exception as e:
        print(f"\n❌ MSS 测试失败: {e}")
        return 0, []


def compare_performance(d3dshot_fps, mss_fps):
    """对比性能。"""
    print("\n" + "="*70)
    print("性能对比")
    print("="*70)

    if d3dshot_fps > 0 and mss_fps > 0:
        print(f"\n{'方法':<15} {'FPS':<10} {'提升':<10}")
        print("-"*40)
        print(f"{'d3dshot':<15} {d3dshot_fps:<10.1f} 🚀")
        print(f"{'MSS':<15} {mss_fps:<10.1f}")

        if d3dshot_fps > mss_fps:
            improvement = (d3dshot_fps / mss_fps - 1) * 100
            print(f"\n✅ d3dshot 比 MSS 快 {improvement:.1f}%")
        else:
            decline = (mss_fps / d3dshot_fps - 1) * 100
            print(f"\n⚠️  MSS 比 d3dshot 快 {decline:.1f}%")

    elif d3dshot_fps > 0:
        print(f"\nd3dshot FPS: {d3dshot_fps:.1f}")
        print("(MSS 未测试)")

    else:
        print(f"\nMSS FPS: {mss_fps:.1f}")
        print("(d3dshot 不可用)")


def test_dxgi_backend():
    """测试 DXGI 后端模块。"""
    print("\n" + "="*70)
    print("DXGI 后端模块测试")
    print("="*70)

    try:
        from capture.dxgi_backend import FastDXGICapture

        print("\n创建 FastDXGICapture...")
        capture = FastDXGICapture(width=1280, height=720, fps=60)

        print("初始化...")
        # 注意: 这是同步测试，实际应该用 asyncio
        # 对于简单测试，我们直接使用内部方法

        backend_type = capture.backend_type
        print(f"后端类型: {backend_type}")

        # 测试 10 帧
        print("\n测试 10 帧捕获...")
        times = []

        for i in range(10):
            t0 = time.perf_counter()
            frame = capture.capture_frame_sync()
            t1 = time.perf_counter()

            if frame is not None:
                times.append((t1 - t0) * 1000)
                print(f"  帧 {i+1}: {frame.shape}, {times[-1]:.1f} ms")

        if times:
            avg = sum(times) / len(times)
            print(f"\n平均: {avg:.1f} ms/frame")
            print(f"理论 FPS: {1000/avg:.1f}")

    except Exception as e:
        print(f"\n❌ DXGI 后端测试失败: {e}")
        import traceback
        traceback.print_exc()


def show_installation_instructions():
    """显示安装说明。"""
    print("\n" + "="*70)
    print("d3dshot 安装说明")
    print("="*70)
    print("""
d3dshot 是一个使用 DirectX 进行屏幕捕获的 Python 库。

安装:
  pip install d3dshot

特点:
  • 使用 DirectX 11/12
  • 零拷贝 GPU 捕获
  • 速度比 GDI 快 2-3 倍

注意事项:
  • 需要 Windows 10+
  • 需要支持 DirectX 11 的显卡
  • 某些游戏/应用可能需要管理员权限

如果 d3dshot 不可用，系统会自动回退到 MSS 捕获。
    """)


if __name__ == "__main__":
    # 检查 d3dshot
    has_d3dshot = test_d3dshot_available()

    # 性能测试
    d3dshot_fps = 0
    mss_fps = 0

    if has_d3dshot:
        d3dshot_fps, _ = test_d3dshot_performance(duration=5)

    mss_fps, _ = test_mss_performance(duration=5)

    # 对比
    compare_performance(d3dshot_fps, mss_fps)

    # 测试 DXGI 后端
    test_dxgi_backend()

    # 显示安装说明
    if not has_d3dshot:
        show_installation_instructions()

    print("\n" + "="*70)
    print("建议")
    print("="*70)

    if has_d3dshot and d3dshot_fps > mss_fps:
        print("✅ 使用 d3dshot (DirectX) 进行捕获")
        print("   在 src/config.py 或 main.py 中:")
        print("   capture = FastDXGICapture(...)")
    else:
        print("⚠️  使用优化的 MSS 进行捕获")
        print("   如需更高性能，请安装 d3dshot:")
        print("   pip install d3dshot")
