#!/usr/bin/env python3
"""
异步流水线测试 - 捕获线程 + 编码线程

架构:
  主线程: 显示
  捕获线程: DXGI C++ (165 FPS)
  编码线程: h264_mf (239 FPS)

使用队列解耦，实现真正的并行处理。
"""
import sys
import time
import ctypes
import io
import threading
import queue
import asyncio
import numpy as np
from pathlib import Path
from concurrent.futures import ThreadPoolExecutor

sys.path.insert(0, str(Path(__file__).parent / 'src'))

print("="*70)
print("异步流水线测试 - 捕获线程 + 编码线程")
print("="*70)

# ============================================================================
# 配置
# ============================================================================
TARGET_WIDTH = 1920
# TARGET_HEIGHT = 1080
TARGET_FPS = 60
QUEUE_SIZE = 5  # 只保留最新帧

# ============================================================================
# 初始化 DXGI C++
# ============================================================================
print("\n[1/4] 初始化 DXGI C++ 捕获器...")

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
if not dxgi_handle:
    print("  ❌ DXGI 初始化失败")
    sys.exit(1)

# 获取尺寸
buffer = (ctypes.c_ubyte * (2560 * 1440 * 4))()
info = FrameInfo()
dxgi_dll.capture_frame(dxgi_handle, buffer, 2560 * 1440 * 4, ctypes.byref(info))
width, height = info.width, info.height
print(f"  ✅ DXGI C++: {width}x{height}")

# ============================================================================
# 初始化编码器
# ============================================================================
print("\n[2/4] 初始化硬件编码器...")

import av

encode_container = av.open(io.BytesIO(), 'w', format='h264')
encode_stream = encode_container.add_stream('h264_mf', rate=60)
encode_stream.width = 1920
encode_stream.height = 1080
encode_stream.bit_rate = 5_000_000
encode_pts = 0

# 编码锁（PyAV 不是线程安全）
encode_lock = threading.Lock()

print(f"  ✅ h264_mf 硬件编码器")

# ============================================================================
# 创建队列
# ============================================================================
print("\n[3/4] 创建队列...")

frame_queue = queue.Queue(maxsize=QUEUE_SIZE)
encoded_queue = queue.Queue(maxsize=QUEUE_SIZE)

print(f"  ✅ 队列大小: {QUEUE_SIZE}")

# ============================================================================
# 统计
# ============================================================================
print("\n[4/4] 启动测试...")

stats = {
    'captured': 0,
    'encoded': 0,
    'dropped': 0,
    'capture_times': [],
    'encode_times': [],
    'start_time': time.time(),
}

running = True
lock = threading.Lock()

# ============================================================================
# 捕获线程
# ============================================================================
def capture_thread_func():
    """高速捕获线程 - DXGI C++"""
    print("  [捕获线程] 启动")

    local_buffer = (ctypes.c_ubyte * (width * height * 4))()
    local_info = FrameInfo()

    while running:
        t0 = time.perf_counter()
        result = dxgi_dll.capture_frame(dxgi_handle, local_buffer, width * height * 4, ctypes.byref(local_info))
        t1 = time.perf_counter()

        if result == 1:
            # 转换为 numpy
            frame_bgra = np.ctypeslib.as_array(local_buffer)
            frame_bgra = frame_bgra.reshape((height, width, 4))

            # 调整大小
            if width != 1920 or height != 1080:
                import cv2
                frame_rgb = cv2.resize(frame_bgra, (1920, 1080))
                frame_rgb = frame_rgb[:, :, :3][:, :, [2, 1, 0]]
            else:
                frame_rgb = frame_bgra[:, :, :3][:, :, [2, 1, 0]]

            # 放入队列（非阻塞）
            try:
                frame_queue.put((frame_rgb, time.time()), block=False)
                with lock:
                    stats['captured'] += 1
                    stats['capture_times'].append((t1 - t0) * 1000)
            except queue.Full:
                with lock:
                    stats['dropped'] += 1

    print("  [捕获线程] 停止")

# ============================================================================
# 编码线程
# ============================================================================
def encode_thread_func():
    """编码线程 - h264_mf GPU"""
    print("  [编码线程] 启动")

    local_output = io.BytesIO()
    local_container = av.open(local_output, 'w', format='h264')
    local_stream = local_container.add_stream('h264_mf', rate=60)
    local_stream.width = 1920
    local_stream.height = 1080
    local_stream.bit_rate = 5_000_000
    local_pts = 0
    buffer_pos = 0
    max_buffer = 10 * 1024 * 1024

    while running:
        try:
            frame_rgb, timestamp = frame_queue.get(timeout=0.1)
        except queue.Empty:
            continue

        t0 = time.perf_counter()

        # 编码（需要锁）
        with encode_lock:
            av_frame = av.VideoFrame.from_ndarray(frame_rgb, format='rgb24')
            av_frame.pts = local_pts
            local_pts += 1

            start_pos = local_output.tell()
            for packet in local_stream.encode(av_frame):
                local_container.mux(packet)
            end_pos = local_output.tell()

            if end_pos > start_pos:
                encoded_data = local_output.read(end_pos - start_pos)
                local_output.seek(end_pos)

                # 放入编码队列
                try:
                    encoded_queue.put((encoded_data, timestamp), block=False)
                    with lock:
                        stats['encoded'] += 1
                        stats['encode_times'].append((time.perf_counter() - t0) * 1000)
                except queue.Full:
                    pass

                # 定期重置
                buffer_pos = end_pos
                if buffer_pos > max_buffer:
                    local_output = io.BytesIO()
                    local_container = av.open(local_output, 'w', format='h264')
                    local_stream = local_container.add_stream('h264_mf', rate=60)
                    local_stream.width = 1920
                    local_stream.height = 1080
                    local_stream.bit_rate = 5_000_000
                    local_pts = 0
                    buffer_pos = 0

    print("  [编码线程] 停止")

# ============================================================================
# 启动线程
# ============================================================================
capture_thread = threading.Thread(target=capture_thread_func, daemon=True)
encode_thread = threading.Thread(target=encode_thread_func, daemon=True)

capture_thread.start()
encode_thread.start()

# 等待队列填充
time.sleep(1)

# ============================================================================
# 主循环 - 显示
# ============================================================================
print("\n" + "="*70)
print("异步流水线测试 (10秒)")
print("="*70)
print("按 ESC 或 Q 退出")
print("="*70)

import cv2

cv2.namedWindow("Async Pipeline", cv2.WINDOW_NORMAL)

last_stats_update = time.time()

try:
    while time.time() - stats['start_time'] < 10:
        loop_start = time.perf_counter()

        # 从编码队列获取已编码的帧（用于显示）
        # 同时从帧队列获取原始帧（用于显示捕获状态）
        display_frame = None

        # 优先显示原始帧
        try:
            frame_rgb, _ = frame_queue.get(timeout=0.05)
        except queue.Empty:
            pass

        # 更新统计
        now = time.time()
        if now - last_stats_update >= 0.2:
            elapsed = now - stats['start_time']
            capture_fps = stats['captured'] / elapsed if elapsed > 0 else 0
            encode_fps = stats['encoded'] / elapsed if elapsed > 0 else 0

            if frame_rgb is not None:
                # 绘制信息
                overlay = frame_rgb.copy()
                cv2.rectangle(overlay, (5, 5), (450, 180), (0, 0, 0), -1)
                frame_rgb = cv2.addWeighted(overlay, 0.7, frame_rgb, 0.3, 0)

                y = 35
                fps_color = (0, 200, 0) if capture_fps >= 50 else (0, 200, 200)

                cv2.putText(frame_rgb, f"捕获: {capture_fps:.1f} FPS",
                           (15, y), cv2.FONT_HERSHEY_SIMPLEX, 0.6, fps_color, 2)
                y += 30
                cv2.putText(frame_rgb, f"编码: {encode_fps:.1f} FPS",
                           (15, y), cv2.FONT_HERSHEY_SIMPLEX, 0.6, (100, 255, 100), 2)
                y += 30
                cv2.putText(frame_rgb, f"丢弃: {stats['dropped']} 帧",
                           (15, y), cv2.FONT_HERSHEY_SIMPLEX, 0.5, (255, 255, 255), 1)
                y += 30

                if stats['capture_times']:
                    avg_capture = sum(stats['capture_times']) / len(stats['capture_times'])
                    cv2.putText(frame_rgb, f"捕获延迟: {avg_capture:.1f} ms",
                               (15, y), cv2.FONT_HERSHEY_SIMPLEX, 0.45, (0, 255, 0), 1)
                y += 25
                if stats['encode_times']:
                    avg_encode = sum(stats['encode_times']) / len(stats['encode_times'])
                    cv2.putText(frame_rgb, f"编码延迟: {avg_encode:.1f} ms",
                               (15, y), cv2.FONT_HERSHEY_SIMPLEX, 0.45, (0, 255, 0), 1)

                cv2.putText(frame_rgb, "🚀 异步流水线",
                           (15, 170), cv2.FONT_HERSHEY_SIMPLEX, 0.6, (0, 255, 0), 2)

                cv2.imshow("Async Pipeline", frame_rgb)

            last_stats_update = now

        # 退出检查
        key = cv2.waitKey(1) & 0xFF
        if key == 27 or key == ord('q'):
            break

finally:
    running = False
    cv2.destroyAllWindows()

# 等待线程结束
capture_thread.join(timeout=2)
encode_thread.join(timeout=2)

# 清理
dxgi_dll.free_capture(dxgi_handle)

# ============================================================================
# 最终统计
# ============================================================================
total_time = time.time() - stats['start_time']

print("\n" + "="*70)
print("异步流水线统计")
print("="*70)
print(f"测试时长: {total_time:.1f}s")
print(f"捕获帧数: {stats['captured']}")
print(f"编码帧数: {stats['encoded']}")
print(f"丢弃帧数: {stats['dropped']}")

print(f"\n性能指标:")
print(f"  捕获 FPS: {stats['captured'] / total_time:.1f}")
print(f"  编码 FPS: {stats['encoded'] / total_time:.1f}")
print(f"  端到端 FPS: {stats['encoded'] / total_time:.1f}")

if stats['capture_times']:
    print(f"  平均捕获延迟: {sum(stats['capture_times'])/len(stats['capture_times']):.2f} ms")

if stats['encode_times']:
    print(f"  平均编码延迟: {sum(stats['encode_times'])/len(stats['encode_times']):.2f} ms")

# 评级
pipeline_fps = stats['encoded'] / total_time
if pipeline_fps >= 50:
    rating = "⭐⭐⭐ 优秀"
elif pipeline_fps >= 30:
    rating = "⭐⭐ 良好"
else:
    rating = "⭐ 一般"

print(f"\n评级: {rating}")

# 对比同步版本
print(f"\n对比:")
print(f"  同步流水线:  22.5 FPS")
print(f"  异步流水线:  {pipeline_fps:.1f} FPS")
print(f"  提升:        {(pipeline_fps/22.5 - 1)*100:+.1f}%")
