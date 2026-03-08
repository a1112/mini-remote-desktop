#!/usr/bin/env python3
"""
d3dshot 性能测试 - Python 3.10

使用 Python 3.10 运行此脚本以测试 d3dshot 性能。
"""
import time
import sys

print("="*70)
print("d3dshot 性能测试")
print("="*70)

try:
    import d3dshot

    print("\n✅ d3dshot 已安装")

    # 创建实例
    d3d = d3dshot.create()

    print(f"\n显示器信息:")
    for i, display in enumerate(d3d.displays):
        print(f"  显示器 {i}: {display.name}")
        print(f"    分辨率: {display.resolution}")

    # 配置捕获
    print(f"\n配置:")
    print(f"  捕获输出: {d3d.capture_output}")

    # 测试 1: 单帧捕获测试
    print(f"\n[测试 1] 单帧捕获")
    frame = d3d.capture()
    if frame is not None:
        print(f"  ✅ 成功捕获")
        print(f"  帧类型: {type(frame)}")
        print(f"  帧大小: {frame.size if hasattr(frame, 'size') else 'N/A'}")

        # 如果是 PIL Image
        if hasattr(frame, 'width'):
            print(f"  宽度: {frame.width}")
            print(f"  高度: {frame.height}")

    # 测试 2: 连续捕获性能
    print(f"\n[测试 2] 连续捕获 (5 秒)")
    print("测试中...")

    times = []
    frames = []
    start = time.time()

    while time.time() - start < 5:
        t0 = time.perf_counter()
        frame = d3d.capture()
        t1 = time.perf_counter()

        if frame is not None:
            times.append((t1 - t0) * 1000)
            frames.append(frame)

    elapsed = time.time() - start
    fps = len(times) / elapsed

    print(f"\n结果:")
    print(f"  捕获帧数: {len(times)}")
    print(f"  实际 FPS: {fps:.1f}")
    print(f"  平均延迟: {sum(times)/len(times):.1f} ms")
    print(f"  最快延迟: {min(times):.1f} ms")
    print(f"  最慢延迟: {max(times):.1f} ms")

    # 评级
    if fps >= 100:
        rating = "🚀🚀🚀 极快!"
    elif fps >= 60:
        rating = "🚀🚀 非常快"
    elif fps >= 40:
        rating = "🚀 快"
    elif fps >= 30:
        rating = "⚡⚡ 良好"
    elif fps >= 20:
        rating = "⚡ 一般"
    else:
        rating = "💻 慢"

    print(f"  评级: {rating}")

    # 对比 MSS
    print(f"\n[对比] MSS (Python 3.12): 30.3 FPS")
    print(f"      d3dshot (Python 3.10): {fps:.1f} FPS")

    improvement = (fps / 30.3 - 1) * 100
    if fps > 30.3:
        print(f"      提升: {improvement:.1f}%")
    else:
        print(f"      差距: {-improvement:.1f}%")

except ImportError as e:
    print(f"\n❌ d3dshot 未安装: {e}")
    print(f"\n安装方法:")
    print(f"  pip install d3dshot")
except Exception as e:
    print(f"\n❌ 错误: {e}")
    import traceback
    traceback.print_exc()
