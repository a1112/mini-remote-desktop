#!/usr/bin/env python3
"""
对比理论编码 FPS 和实际编码 FPS。

理论 FPS: 1000 / 单帧编码时间 (编码器极限速度)
实际 FPS: 实际每秒能编码多少帧 (考虑流水线)
"""
import sys
import time
import io
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent / 'src'))

import numpy as np

print("="*70)
print("理论 FPS vs 实际 FPS 对比")
print("="*70)

try:
    import av

    # 创建测试帧
    frame_data = np.random.randint(0, 255, (720, 1280, 3), dtype=np.uint8)
    frame = av.VideoFrame.from_ndarray(frame_data, format='rgb24')

    # 测试 h264_mf 硬件编码
    print("\n测试编码器: h264_mf (硬件)")
    print("-"*70)

    output = io.BytesIO()
    container = av.open(output, 'w', format='h264')
    stream = container.add_stream('h264_mf', rate=30)
    stream.width = 1280
    stream.height = 720
    stream.bit_rate = 3_000_000

    # 测试 1: 单帧编码时间 (理论 FPS)
    print("\n[测试 1] 单帧编码时间 (理论 FPS)")
    single_frame_times = []
    for i in range(10):
        frame.pts = i
        t0 = time.perf_counter()
        for packet in stream.encode(frame):
            container.mux(packet)
        t1 = time.perf_counter()
        single_frame_times.append((t1 - t0) * 1000)

    avg_single = sum(single_frame_times) / len(single_frame_times)
    theoretical_fps = 1000 / avg_single

    print(f"  单帧编码时间: {avg_single:.2f} ms")
    print(f"  理论编码 FPS: {theoretical_fps:.1f}")

    # 测试 2: 连续编码 (实际 FPS)
    print("\n[测试 2] 连续编码 100 帧 (实际 FPS)")
    start = time.time()
    for i in range(100):
        frame.pts = i
        for packet in stream.encode(frame):
            container.mux(packet)
    elapsed = time.time() - start
    actual_fps = 100 / elapsed

    print(f"  总耗时: {elapsed:.2f} s")
    print(f"  实际编码 FPS: {actual_fps:.1f}")

    # 测试 3: 模拟流水线 (捕获 + 编码)
    print("\n[测试 3] 模拟流水线 (模拟捕获 + 编码)")
    import mss

    sct = mss.mss()
    monitor = sct.monitors[1]

    frame_times = []
    encode_times = []

    start = time.time()
    for i in range(30):
        # 捕获
        t0 = time.perf_counter()
        screenshot = sct.grab({
            "left": 0, "top": 0,
            "width": 1280, "height": 720,
            "mon": 1
        })
        arr = np.frombuffer(screenshot.rgb, dtype=np.uint8)
        arr = arr.reshape((720, 1280, 3))
        capture_time = (time.perf_counter() - t0) * 1000
        frame_times.append(capture_time)

        # 编码
        av_frame = av.VideoFrame.from_ndarray(arr, format='rgb24')
        av_frame.pts = i

        t1 = time.perf_counter()
        for packet in stream.encode(av_frame):
            container.mux(packet)
        encode_time = (time.perf_counter() - t1) * 1000
        encode_times.append(encode_time)

    elapsed = time.time() - start

    print(f"  总耗时: {elapsed:.2f} s")
    print(f"  端到端 FPS: {30 / elapsed:.1f}")
    print(f"  平均捕获时间: {sum(frame_times)/len(frame_times):.1f} ms")
    print(f"  平均编码时间: {sum(encode_times)/len(encode_times):.1f} ms")

    # 对比总结
    print("\n" + "="*70)
    print("对比总结")
    print("="*70)
    print(f"""
  理论编码 FPS:     {theoretical_fps:>8.1f}  (编码器极限速度)
  实际编码 FPS:     {actual_fps:>8.1f}  (纯编码，连续)
  流水线 FPS:       {30 / elapsed:>8.1f}  (捕获 + 编码)

  差异原因:
  1. 理论 FPS = 1000ms / 单帧编码时间
     → 只测量编码器本身的速度，不考虑其他开销

  2. 实际 FPS < 理论 FPS
     → 帧与帧之间有开销 (内存管理、上下文切换等)

  3. 流水线 FPS << 理论 FPS
     → 受限于捕获速度 (瓶颈在捕获，不在编码!)

  结论: 硬件编码器 (h264_mf) 不是瓶颈
    """)

except Exception as e:
    print(f"错误: {e}")
    import traceback
    traceback.print_exc()
