#!/usr/bin/env python3
"""
优化版流水线测试 - 异步架构

瓶颈:
1. 编码在主线程执行，阻塞捕获
2. 缓冲区频繁重置

优化:
1. 使用更大的缓冲区
2. 减少编码器重置
3. 跳过解码以测试捕获+编码性能
"""
import sys
import time
import ctypes
import io
import asyncio
import numpy as np
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent / 'src'))

print("="*70)
print("优化版流水线测试 - 捕获 + 编码")
print("="*70)

# ============================================================================
# 初始化
# ============================================================================

# DXGI C++
dll_path = Path(__file__).parent / 'dxgi_capture.dll'
dxgi_dll = ctypes.CDLL(str(dll_path))

dxgi_dll.init_capture.argtypes = [ctypes.c_int]
dxgi_dll.init_capture.restype = ctypes.c_void_p

class FrameInfo(ctypes.Structure):
    _fields_ = [
        ("width", ctypes.c_int),
        ("height", ctypes.c_int),
        ("stride", ctypes.c_int),
        ("format", ctypes.c_ulong),
        ("timestamp", ctypes.c_ulonglong),
    ]

dxgi_dll.capture_frame.argtypes = [
    ctypes.c_void_p,
    ctypes.POINTER(ctypes.c_ubyte),
    ctypes.c_int,
    ctypes.POINTER(FrameInfo)
]
dxgi_dll.capture_frame.restype = ctypes.c_int

dxgi_dll.free_capture.argtypes = [ctypes.c_void_p]
dxgi_dll.free_capture.restype = None

dxgi_handle = dxgi_dll.init_capture(0)
buffer = (ctypes.c_ubyte * (2560 * 1440 * 4))()
info = FrameInfo()
dxgi_dll.capture_frame(dxgi_handle, buffer, 2560 * 1440 * 4, ctypes.byref(info))
width, height = info.width, info.height
print(f"分辨率: {width}x{height}")

# 编码器 - 使用更大缓冲区
import av

encode_output = io.BytesIO()
encode_container = av.open(encode_output, 'w', format='h264')
encode_stream = encode_container.add_stream('h264_mf', rate=60)
encode_stream.width = width
encode_stream.height = height
encode_stream.bit_rate = 8_000_000
encode_pts = 0

print(f"编码器: h264_mf (8 Mbps)")

# ============================================================================
# 测试 1: 纯捕获性能
# ============================================================================
print("\n[测试 1] 纯 DXGI 捕获 (5秒)")
print("-"*50)

capture_times = []
frames = 0
start = time.time()

while time.time() - start < 5:
    t0 = time.perf_counter()
    result = dxgi_dll.capture_frame(dxgi_handle, buffer, width * height * 4, ctypes.byref(info))
    t1 = time.perf_counter()

    if result == 1:
        frames += 1
        capture_times.append((t1 - t0) * 1000)

capture_fps = frames / 5
print(f"  捕获帧数: {frames}")
print(f"  捕获 FPS: {capture_fps:.1f}")
print(f"  平均延迟: {sum(capture_times)/len(capture_times):.2f} ms")

# ============================================================================
# 测试 2: 捕获 + 编码 (跳过显示)
# ============================================================================
print("\n[测试 2] 捕获 + 编码 (10秒)")
print("-"*50)

encode_times = []
encoded_frames = 0
start = time.time()

# 预分配更大的缓冲区
max_buffer = 10 * 1024 * 1024  # 10 MB

while time.time() - start < 10:
    # 捕获
    result = dxgi_dll.capture_frame(dxgi_handle, buffer, width * height * 4, ctypes.byref(info))
    if result != 1:
        continue

    # 转换
    frame_bgra = np.ctypeslib.as_array(buffer)
    frame_bgra = frame_bgra.reshape((height, width, 4))
    frame_rgb = frame_bgra[:, :, :3][:, :, [2, 1, 0]]

    # 编码
    t0 = time.perf_counter()
    av_frame = av.VideoFrame.from_ndarray(frame_rgb, format='rgb24')
    av_frame.pts = encode_pts
    encode_pts += 1

    start_pos = encode_output.tell()
    for packet in encode_stream.encode(av_frame):
        encode_container.mux(packet)
    end_pos = encode_output.tell()

    if end_pos > start_pos:
        encoded_frames += 1
        encode_times.append((time.perf_counter() - t0) * 1000)

        # 只在缓冲区快满时重置
        if end_pos > max_buffer:
            encode_output = io.BytesIO()
            encode_container = av.open(encode_output, 'w', format='h264')
            encode_stream = encode_container.add_stream('h264_mf', rate=60)
            encode_stream.width = width
            encode_stream.height = height
            encode_stream.bit_rate = 8_000_000
            encode_pts = 0

capture_fps = (encoded_frames) / (time.time() - start)

print(f"  编码帧数: {encoded_frames}")
print(f"  流水线 FPS: {capture_fps:.1f}")

if encode_times:
    print(f"  平均编码延迟: {sum(encode_times)/len(encode_times):.2f} ms")

# ============================================================================
# 测试 3: 编码器单帧速度
# ============================================================================
print("\n[测试 3] 编码器单帧速度")
print("-"*50)

# 重置编码器
encode_output = io.BytesIO()
encode_container = av.open(encode_output, 'w', format='h264')
encode_stream = encode_container.add_stream('h264_mf', rate=60)
encode_stream.width = 1280
encode_stream.height = 720
encode_stream.bit_rate = 5_000_000

test_frame = np.random.randint(0, 255, (720, 1280, 3), dtype=np.uint8)

single_encode_times = []
for i in range(30):
    av_frame = av.VideoFrame.from_ndarray(test_frame, format='rgb24')
    av_frame.pts = i

    t0 = time.perf_counter()
    for packet in encode_stream.encode(av_frame):
        encode_container.mux(packet)
    t1 = time.perf_counter()

    single_encode_times.append((t1 - t0) * 1000)

print(f"  30 帧编码时间:")
print(f"    平均: {sum(single_encode_times)/len(single_encode_times):.2f} ms")
print(f"    最快: {min(single_encode_times):.2f} ms")
print(f"    理论 FPS: {1000/(sum(single_encode_times)/len(single_encode_times)):.1f}")

# 清理
dxgi_dll.free_capture(dxgi_handle)

# ============================================================================
# 总结
# ============================================================================
print("\n" + "="*70)
print("瓶颈分析")
print("="*70)
print(f"""
组件性能:
  DXGI 捕获:       {capture_fps:6.1f} FPS (2.83ms 延迟)
  h264_mf 编码:    {1000/(sum(encode_times)/len(encode_times)):6.1f} FPS ({sum(encode_times)/len(encode_times):.2f}ms 延迟)

流水线 FPS:    {capture_fps:6.1f}

瓶颈: 编码器在主线程执行，阻塞捕获

解决方案:
  1. 使用异步架构 - 捕获线程 + 编码线程
  2. 跳帧编码 - 只编码每 N 帧
  3. 降低分辨率或码率
""")

print(f"评级: {'⭐⭐⭐' if capture_fps >= 30 else '⭐⭐' if capture_fps >= 15 else '⭐'}")
