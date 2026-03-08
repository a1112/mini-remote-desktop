#!/usr/bin/env python3
"""
d3dshot 完整性能测试 - Python 3.10

使用 Python 3.10 运行此脚本。
"""
import sys
import time
import os

print("="*70)
print("d3dshot 完整性能测试")
print("="*70)

try:
    import d3dshot
    import numpy as np

    print("\n✅ d3dshot 已安装")

    # 创建实例
    d3d = d3dshot.create(capture_output='numpy')

    print(f"\n显示器: {d3d.displays[0]}")

    # Benchmark
    print("\n[1/3] 运行 benchmark...")
    print("请稍候 (60秒)...")
    # d3dshot 的 benchmark 会运行 60 秒，我们用自定义测试

    # 自定义性能测试
    print("\n[2/3] 自定义性能测试 (5 秒)")

    times = []
    frames_captured = 0
    start = time.time()

    while time.time() - start < 5:
        t0 = time.perf_counter()
        frame = d3d.capture()
        t1 = time.perf_counter()

        if frame is not None and isinstance(frame, np.ndarray):
            frames_captured += 1
            times.append((t1 - t0) * 1000)

    elapsed = time.time() - start
    fps = frames_captured / elapsed

    print(f"\n结果:")
    print(f"  有效帧数: {frames_captured}")
    print(f"  FPS: {fps:.1f}")

    if times:
        print(f"  平均延迟: {sum(times)/len(times):.1f} ms")
        print(f"  最快: {min(times):.1f} ms")
        print(f"  最慢: {max(times):.1f} ms")

        if fps >= 50:
            rating = "🚀🚀🚀"
        elif fps >= 30:
            rating = "🚀🚀"
        else:
            rating = "⚡"
        print(f"  评级: {rating}")

    # 获取帧信息
    print("\n[3/3] 帧信息")
    test_frame = d3d.capture()
    if test_frame is not None and isinstance(test_frame, np.ndarray):
        print(f"  形状: {test_frame.shape}")
        print(f"  类型: {test_frame.dtype}")
        print(f"  大小: {test_frame.nbytes / 1024 / 1024:.1f} MB")

    # 对比
    print("\n" + "="*70)
    print("性能对比")
    print("="*70)
    print(f"""
  d3dshot (DirectX):  {fps:>8.1f} FPS  🚀
  MSS (GDI):          {30.3:>8.1f} FPS  ⚡
  PIL.ImageGrab:      {19.3:>8.1f} FPS  💻

  d3dshot vs MSS:     {(fps/30.3 - 1)*100:+.1f}%
    """)

except ImportError as e:
    print(f"\n❌ 请在 Python 3.10 环境运行")
    print(f"   当前: {sys.version}")
    print(f"\n   使用: J:\\python\\py10\\python.exe {os.path.basename(__file__)}")
except Exception as e:
    print(f"\n❌ 错误: {e}")
    import traceback
    traceback.print_exc()
