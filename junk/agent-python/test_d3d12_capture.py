#!/usr/bin/env python3
"""
D3D12 混合捕获测试 - Python ctypes

测试 D3D11 捕获 + D3D12 输出
"""
import ctypes
import sys
import time
import numpy as np
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent / 'src'))

print("=" * 70)
print("D3D12 混合捕获测试")
print("=" * 70)

# 加载 DLL
dll_path = Path(__file__).parent / 'cpp_capture' / 'd3d12_hybrid_capture.dll'

if not dll_path.exists():
    print(f"❌ DLL 不存在: {dll_path}")
    print("请先运行 compile_d3d12.bat")
    sys.exit(1)

try:
    dll = ctypes.CDLL(str(dll_path))
    print(f"✅ 加载 DLL: {dll_path}")
except Exception as e:
    print(f"❌ 加载 DLL 失败: {e}")
    sys.exit(1)

# 设置函数签名
dll.init_hybrid_capture.argtypes = [ctypes.c_int, ctypes.c_int]
dll.init_hybrid_capture.restype = ctypes.c_void_p

class D3D12HybridFrame(ctypes.Structure):
    _fields_ = [
        ("width", ctypes.c_int),
        ("height", ctypes.c_int),
        ("stride", ctypes.c_int),
        ("format", ctypes.c_ulong),
        ("timestamp", ctypes.c_ulonglong),
        ("d3d11_resource", ctypes.c_void_p),
        ("d3d12_resource", ctypes.c_void_p),
    ]

dll.capture_hybrid_frame.argtypes = [
    ctypes.c_void_p,
    ctypes.POINTER(D3D12HybridFrame)
]
dll.capture_hybrid_frame.restype = ctypes.c_int

dll.copy_hybrid_frame_to_cpu.argtypes = [
    ctypes.c_void_p,
    ctypes.POINTER(ctypes.c_ubyte),
    ctypes.c_int
]
dll.copy_hybrid_frame_to_cpu.restype = ctypes.c_int

dll.get_hybrid_d3d12_device.argtypes = [ctypes.c_void_p]
dll.get_hybrid_d3d12_device.restype = ctypes.c_void_p

dll.get_hybrid_d3d12_queue.argtypes = [ctypes.c_void_p]
dll.get_hybrid_d3d12_queue.restype = ctypes.c_void_p

dll.free_hybrid_capture.argtypes = [ctypes.c_void_p]
dll.free_hybrid_capture.restype = None

# 初始化
print("\n[1/3] 初始化 D3D12 混合捕获...")
print("  尝试启用 D3D12 输出...")

handle = dll.init_hybrid_capture(0, 1)  # monitor_index=0, enable_d3d12=1

if not handle:
    print("  ⚠️  D3D12 初始化失败，回退到 D3D11...")
    handle = dll.init_hybrid_capture(0, 0)  # enable_d3d12=0

    if not handle:
        print("  ❌ 初始化完全失败")
        sys.exit(1)

print(f"  ✅ 句柄: {hex(handle)}")

# 检查 D3D12
d3d12_device = dll.get_hybrid_d3d12_device(handle)
d3d12_queue = dll.get_hybrid_d3d12_queue(handle)

if d3d12_device:
    print(f"  ✅ D3D12 设备: {hex(d3d12_device)}")
    print(f"  ✅ D3D12 队列: {hex(d3d12_queue)}")
else:
    print("  ℹ️  D3D12 不可用，使用 D3D11 路径")

# 性能测试
print("\n[2/3] 性能测试 (5秒)...")

frame_info = D3D12HybridFrame()
frames = []
times = []

# 预分配缓冲区
buffer = (ctypes.c_ubyte * (2560 * 1440 * 4))()
width, height = 0, 0

start = time.time()

while time.time() - start < 5:
    t0 = time.perf_counter()
    result = dll.capture_hybrid_frame(handle, ctypes.byref(frame_info))
    t1 = time.perf_counter()

    if result == 1:  # 成功
        width, height = frame_info.width, frame_info.height
        frames.append(frame_info)
        times.append((t1 - t0) * 1000)

        # 复制到 CPU
        # dll.copy_hybrid_frame_to_cpu(handle, buffer, len(buffer))
    elif result == -1:
        pass  # 暂无新帧

elapsed = time.time() - start
fps = len(frames) / elapsed if elapsed > 0 else 0

print(f"\n结果:")
print(f"  捕获帧数: {len(frames)}")
print(f"  分辨率: {width}x{height}")
print(f"  FPS: {fps:.1f}")

if frames:
    avg_time = sum(times) / len(times)
    min_time = min(times)
    max_time = max(times)
    p50 = sorted(times)[len(times)//2]

    print(f"  平均延迟: {avg_time:.2f} ms")
    print(f"  最快延迟: {min_time:.2f} ms")
    print(f"  最慢延迟: {max_time:.2f} ms")
    print(f"  P50 延迟: {p50:.2f} ms")

# CPU 复制测试
print("\n[3/3] CPU 复制测试 (10帧)...")

cpu_times = []
cpu_frames = 0

for i in range(20):  # 尝试 20 次获取 10 帧
    result = dll.capture_hybrid_frame(handle, ctypes.byref(frame_info))
    if result == 1:
        t0 = time.perf_counter()
        copy_result = dll.copy_hybrid_frame_to_cpu(handle, buffer, len(buffer))
        t1 = time.perf_counter()

        if copy_result == 1:
            cpu_frames += 1
            cpu_times.append((t1 - t0) * 1000)

        if cpu_frames >= 10:
            break
    time.sleep(0.01)

if cpu_times:
    avg_cpu = sum(cpu_times) / len(cpu_times)
    print(f"  成功复制 {cpu_frames} 帧")
    print(f"  平均复制延迟: {avg_cpu:.2f} ms")

# 清理
dll.free_hybrid_capture(handle)

# 对比
print("\n" + "=" * 70)
print("性能对比")
print("=" * 70)
print(f"""
  D3D12 Hybrid:    {fps:6.1f} FPS  ({avg_time if times else 0:.2f}ms)
  DXGI C++ D3D11:  {188:6.1f} FPS  (2.43ms)
  d3dshot (Py3.12): {86:6.1f} FPS  (11.62ms)
""")

# 评级
if fps >= 100:
    rating = "⭐⭐⭐ 优秀"
elif fps >= 50:
    rating = "⭐⭐ 良好"
else:
    rating = "⭐ 一般"

print(f"评级: {rating}")
