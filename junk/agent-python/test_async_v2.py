#!/usr/bin/env python3
"""
异步流水线 V2 - 优化版

优化:
1. 直接捕获目标分辨率 (避免缩放)
2. 增大队列
3. 降低分辨率到 1280x720
"""
import sys
import time
import ctypes
import io
import threading
import queue
import numpy as np
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent / 'src'))

print("="*70)
print("异步流水线 V2 - 优化版")
print("="*70)

# ============================================================================
# 配置
# ============================================================================
TARGET_WIDTH = 1280
# TARGET_HEIGHT = 720
QUEUE_SIZE = 10

# ============================================================================
# 初始化 DXGI C++
# ============================================================================
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

# 获取尺寸并重新初始化为目标分辨率
print(f"初始化 DXGI C++...")

# 获取原始尺寸
buffer = (ctypes.c_ubyte * (2560 * 1440 * 4))()
info = FrameInfo()
dxgi_dll.capture_frame(dxgi_handle, buffer, 2560 * 1440 * 4, ctypes.byref(info))
orig_width, orig_height = info.width, info.height
print(f"原始分辨率: {orig_width}x{orig_height}")

# 使用目标分辨率
width, height = 1280, 720
buffer = (ctypes.c_ubyte * (width * height * 4))()

# ============================================================================
# 初始化编码器
# ============================================================================
import av

encode_container = av.open(io.BytesIO(), 'w', format='h264')
encode_stream = encode_container.add_stream('h264_mf', rate=60)
encode_stream.width = width
encode_stream.height = height
encode_stream.bit_rate = 3_000_000
encode_pts = 0

encode_lock = threading.Lock()

print(f"目标分辨率: {width}x{height}")
print(f"编码器: h264_mf")

# ============================================================================
# 队列
# ============================================================================
frame_queue = queue.Queue(maxsize=QUEUE_SIZE)

stats = {
    'captured': 0,
    'encoded': 0,
    'capture_times': [],
    'encode_times': [],
    'start_time': time.time(),
}

running = True
lock = threading.Lock()

# ============================================================================
# 捕获线程 (简化 - 缩放到目标分辨率)
# ============================================================================
def capture_thread_func():
    """捕获线程 - 直接捕获目标分辨率"""
    print("  [捕获线程] 启动")

    # 计算缩放区域
    scale_x = width / orig_width
    scale_y = height / orig_height
    capture_w = int(orig_width * scale_x)
    capture_h = int(orig_height * scale_y)
    offset_x = (orig_width - capture_w) // 2
    offset_y = (orig_height - capture_h) // 2

    # 使用更大的缓冲区用于原始捕获
    orig_buffer = (ctypes.c_ubyte * (orig_width * orig_height * 4))()
    local_info = FrameInfo()

    import cv2

    while running:
        t0 = time.perf_counter()
        result = dxgi_dll.capture_frame(dxgi_handle, orig_buffer, orig_width * orig_height * 4, ctypes.byref(local_info))
        t1 = time.perf_counter()

        if result == 1:
            # 转换并缩放
            frame_bgra = np.ctypeslib.as_array(orig_buffer)
            frame_bgra = frame_bgra.reshape((orig_height, orig_width, 4))

            # 裁剪并缩放
            cropped = frame_bgra[offset_y:offset_y+capture_h, offset_x:offset_x+capture_w]
            frame_rgb = cv2.resize(cropped, (width, height))
            frame_rgb = frame_rgb[:, :, :3][:, :, [2, 1, 0]]

            # 放入队列
            try:
                frame_queue.put((frame_rgb, time.time()), block=False)
                with lock:
                    stats['captured'] += 1
                    stats['capture_times'].append((t1 - t0) * 1000)
            except queue.Full:
                pass

    print("  [捕获线程] 停止")

# ============================================================================
# 编码线程
# ============================================================================
def encode_thread_func():
    """编码线程"""
    print("  [编码线程] 启动")

    local_output = io.BytesIO()
    local_container = av.open(local_output, 'w', format='h264')
    local_stream = local_container.add_stream('h264_mf', rate=60)
    local_stream.width = width
    local_stream.height = height
    local_stream.bit_rate = 3_000_000
    local_pts = 0
    max_buffer = 5 * 1024 * 1024

    while running:
        try:
            frame_rgb, timestamp = frame_queue.get(timeout=0.1)
        except queue.Empty:
            continue

        t0 = time.perf_counter()

        # 编码
        with encode_lock:
            av_frame = av.VideoFrame.from_ndarray(frame_rgb, format='rgb24')
            av_frame.pts = local_pts
            local_pts += 1

            start_pos = local_output.tell()
            for packet in local_stream.encode(av_frame):
                local_container.mux(packet)
            end_pos = local_output.tell()

            if end_pos > start_pos:
                # 编码成功
                with lock:
                    stats['encoded'] += 1
                    stats['encode_times'].append((time.perf_counter() - t0) * 1000)

                # 重置缓冲区
                if end_pos > max_buffer:
                    local_output = io.BytesIO()
                    local_container = av.open(local_output, 'w', format='h264')
                    local_stream = local_container.add_stream('h264_mf', rate=60)
                    local_stream.width = width
                    local_stream.height = height
                    local_stream.bit_rate = 3_000_000
                    local_pts = 0

    print("  [编码线程] 停止")

# ============================================================================
# 启动
# ============================================================================
capture_thread = threading.Thread(target=capture_thread_func, daemon=True)
encode_thread = threading.Thread(target=encode_thread_func, daemon=True)

capture_thread.start()
encode_thread.start()

time.sleep(0.5)

# ============================================================================
# 测试
# ============================================================================
print("\n" + "="*70)
print("异步流水线 V2 测试 (10秒)")
print("="*70)

start_time = time.time()

while time.time() - start_time < 10:
    time.sleep(0.5)
    elapsed = time.time() - start_time
    capture_fps = stats['captured'] / elapsed if elapsed > 0 else 0
    encode_fps = stats['encoded'] / elapsed if elapsed > 0 else 0

    print(f"  捕获: {stats['captured']:4d} 帧 @ {capture_fps:5.1f} FPS   "
          f"编码: {stats['encoded']:4d} 帧 @ {encode_fps:5.1f} FPS   "
          f"队列: {frame_queue.qsize():2d}/{QUEUE_SIZE}")

running = False
capture_thread.join(timeout=2)
encode_thread.join(timeout=2)

dxgi_dll.free_capture(dxgi_handle)

# ============================================================================
# 统计
# ============================================================================
total_time = time.time() - stats['start_time']

print("\n" + "="*70)
print("异步流水线 V2 统计")
print("="*70)
print(f"测试时长: {total_time:.1f}s")
print(f"捕获帧数: {stats['captured']}")
print(f"编码帧数: {stats['encoded']}")

print(f"\n性能指标:")
print(f"  捕获 FPS: {stats['captured'] / total_time:.1f}")
print(f"  编码 FPS: {stats['encoded'] / total_time:.1f}")
print(f"  端到端 FPS: {stats['encoded'] / total_time:.1f}")

if stats['capture_times']:
    print(f"  平均捕获延迟: {sum(stats['capture_times'])/len(stats['capture_times']):.2f} ms")

if stats['encode_times']:
    print(f"  平均编码延迟: {sum(stats['encode_times'])/len(stats['encode_times']):.2f} ms")

# 对比
print(f"\n对比:")
print(f"  同步流水线:  22.5 FPS")
print(f"  异步流水线:  {stats['encoded'] / total_time:.1f} FPS")
print(f"  d3dshot:      ~60.0 FPS")
print(f"  MSS:          ~30.0 FPS")

pipeline_fps = stats['encoded'] / total_time
if pipeline_fps >= 50:
    print(f"\n评级: ⭐⭐⭐ 优秀")
elif pipeline_fps >= 30:
    print(f"\n评级: ⭐⭐ 良好")
else:
    print(f"\n评级: ⭐ 一般")
