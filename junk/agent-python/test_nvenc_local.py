#!/usr/bin/env python3
"""
本机完整 NVENC 验证测试

使用真实的 D3D11 设备进行 NVENC 编码测试
"""
import sys
import time
import ctypes
import threading
import queue
import numpy as np
from pathlib import Path

print("=" * 70)
print("本机 NVENC 完整验证测试")
print("=" * 70)

# ============================================================================
# 1. 加载混合捕获 DLL (获取真实 D3D11 设备)
# ============================================================================

print("\n[1/5] 加载混合捕获 DLL...")

capture_dll_path = Path(__file__).parent / 'cpp_capture' / 'd3d12_hybrid_capture.dll'

if not capture_dll_path.exists():
    print(f"  ❌ 混合捕获 DLL 不存在: {capture_dll_path}")
    print(f"     请先编译 d3d12_hybrid_capture.dll")
    sys.exit(1)

try:
    capture_dll = ctypes.CDLL(str(capture_dll_path))
    print(f"  ✅ 混合捕获 DLL 加载成功")
except Exception as e:
    print(f"  ❌ 混合捕获 DLL 加载失败: {e}")
    sys.exit(1)

# 设置混合捕获函数签名
capture_dll.init_hybrid_capture.argtypes = [ctypes.c_int, ctypes.c_int]
capture_dll.init_hybrid_capture.restype = ctypes.c_void_p

class HybridFrame(ctypes.Structure):
    _fields_ = [
        ("width", ctypes.c_int),
        ("height", ctypes.c_int),
        ("stride", ctypes.c_int),
        ("format", ctypes.c_int),
        ("timestamp", ctypes.c_longlong),
        ("d3d11_resource", ctypes.c_void_p),
        ("d3d12_resource", ctypes.c_void_p),
    ]

capture_dll.capture_hybrid_frame.argtypes = [
    ctypes.c_void_p,
    ctypes.POINTER(HybridFrame)
]
capture_dll.capture_hybrid_frame.restype = ctypes.c_int

capture_dll.copy_hybrid_frame_to_cpu.argtypes = [
    ctypes.c_void_p,
    ctypes.POINTER(ctypes.c_ubyte),
    ctypes.c_int
]
capture_dll.copy_hybrid_frame_to_cpu.restype = ctypes.c_int

capture_dll.get_hybrid_d3d11_device.argtypes = [ctypes.c_void_p]
capture_dll.get_hybrid_d3d11_device.restype = ctypes.c_void_p

capture_dll.get_hybrid_d3d11_context.argtypes = [ctypes.c_void_p]
capture_dll.get_hybrid_d3d11_context.restype = ctypes.c_void_p

capture_dll.free_hybrid_capture.argtypes = [ctypes.c_void_p]
capture_dll.free_hybrid_capture.restype = None

# 初始化捕获 (使用 D3D11 模式)
print(f"  初始化捕获器 (D3D11 模式)...")
capture_handle = capture_dll.init_hybrid_capture(0, 0)  # 0 = D3D11 模式

if not capture_handle:
    print(f"  ❌ 捕获器初始化失败")
    sys.exit(1)

print(f"  ✅ 捕获器初始化成功")

# 获取 D3D11 设备和上下文
d3d11_device = capture_dll.get_hybrid_d3d11_device(capture_handle)
d3d11_context = capture_dll.get_hybrid_d3d11_context(capture_handle)

print(f"  ✅ D3D11 设备: 0x{d3d11_device or 0:X}")
print(f"  ✅ D3D11 上下文: 0x{d3d11_context or 0:X}")

# 获取屏幕尺寸
frame_info = HybridFrame()
result = capture_dll.capture_hybrid_frame(capture_handle, ctypes.byref(frame_info))
width, height = frame_info.width, frame_info.height
print(f"  ✅ 屏幕分辨率: {width}x{height}")

# ============================================================================
# 2. 加载 NVENC 编码器
# ============================================================================

print("\n[2/5] 加载 NVENC 编码器...")

nvenc_dll_path = Path(__file__).parent / 'cpp_capture' / 'nvenc_full.dll'

if not nvenc_dll_path.exists():
    print(f"  ❌ NVENC DLL 不存在: {nvenc_dll_path}")
    capture_dll.free_hybrid_capture(capture_handle)
    sys.exit(1)

try:
    nvenc_dll = ctypes.CDLL(str(nvenc_dll_path))
    print(f"  ✅ NVENC DLL 加载成功")
except Exception as e:
    print(f"  ❌ NVENC DLL 加载失败: {e}")
    capture_dll.free_hybrid_capture(capture_handle)
    sys.exit(1)

# 设置 NVENC 函数签名
class NVENCEncodeConfig(ctypes.Structure):
    _fields_ = [
        ("width", ctypes.c_int),
        ("height", ctypes.c_int),
        ("framerate", ctypes.c_int),
        ("bitrate", ctypes.c_int),
        ("gop_size", ctypes.c_int),
        ("preset", ctypes.c_int),
        ("rc_mode", ctypes.c_int),
    ]

class NVENCEncodedFrame(ctypes.Structure):
    _fields_ = [
        ("data", ctypes.POINTER(ctypes.c_ubyte)),
        ("size", ctypes.c_int),
        ("key_frame", ctypes.c_int),
        ("timestamp", ctypes.c_longlong),
    ]

class NVENCVersion(ctypes.Structure):
    _fields_ = [("major", ctypes.c_int), ("minor", ctypes.c_int)]

nvenc_dll.is_nvenc_supported.argtypes = []
nvenc_dll.is_nvenc_supported.restype = ctypes.c_int

nvenc_dll.is_cuda_d3d11_interop_supported.argtypes = []
nvenc_dll.is_cuda_d3d11_interop_supported.restype = ctypes.c_int

nvenc_dll.get_nvenc_version.argtypes = [ctypes.POINTER(NVENCVersion)]
nvenc_dll.get_nvenc_version.restype = None

nvenc_dll.init_nvenc_encoder_d3d11.argtypes = [
    ctypes.c_void_p,
    ctypes.c_void_p,
    ctypes.c_void_p
]
nvenc_dll.init_nvenc_encoder_d3d11.restype = ctypes.c_void_p

nvenc_dll.encode_nvenc_frame_cpu.argtypes = [
    ctypes.c_void_p,
    ctypes.POINTER(ctypes.c_ubyte),
    ctypes.c_int,
    ctypes.c_longlong,
    ctypes.c_int
]
nvenc_dll.encode_nvenc_frame_cpu.restype = ctypes.c_int

nvenc_dll.get_nvenc_encoded_frame.argtypes = [
    ctypes.c_void_p,
    ctypes.POINTER(NVENCEncodedFrame)
]
nvenc_dll.get_nvenc_encoded_frame.restype = ctypes.c_int

nvenc_dll.free_nvenc_encoder.argtypes = [ctypes.c_void_p]
nvenc_dll.free_nvenc_encoder.restype = None

# 检查 NVENC 支持
version = NVENCVersion()
nvenc_dll.get_nvenc_version(ctypes.byref(version))
print(f"  NVENC 版本: {version.major}.{version.minor}")

nvenc_sup = nvenc_dll.is_nvenc_supported()
cuda_sup = nvenc_dll.is_cuda_d3d11_interop_supported()
print(f"  NVENC 支持: {'✅' if nvenc_sup else '❌'}")
print(f"  CUDA-D3D11: {'✅' if cuda_sup else '❌'}")

if not nvenc_sup:
    print(f"  ⚠️  NVENC 不可用，使用回退方案")
    capture_dll.free_hybrid_capture(capture_handle)
    sys.exit(1)

# ============================================================================
# 3. 初始化 NVENC 编码器
# ============================================================================

print("\n[3/5] 初始化 NVENC 编码器...")

config = NVENCEncodeConfig()
config.width = width
config.height = height
config.framerate = 60
config.bitrate = 5_000_000  # 5 Mbps
config.gop_size = 60
config.preset = 3  # fast
config.rc_mode = 2  # CBR

print(f"  配置: {config.width}x{config.height} @ {config.framerate}fps")
print(f"        码率: {config.bitrate / 1000000:.1f} Mbps")
print(f"        GOP: {config.gop_size}")

nvenc_handle = nvenc_dll.init_nvenc_encoder_d3d11(
    ctypes.c_void_p(d3d11_device),
    ctypes.c_void_p(d3d11_context),
    ctypes.byref(config)
)

if nvenc_handle:
    print(f"  ✅ NVENC 编码器初始化成功!")
    print(f"     句柄: 0x{nvenc_handle or 0:X}")
else:
    print(f"  ❌ NVENC 编码器初始化失败")
    print(f"     可能原因:")
    print(f"     - GPU 不支持 NVENC")
    print(f"     - CUDA-D3D11 互操作失败")
    print(f"     - NVENC 会话创建失败")
    capture_dll.free_hybrid_capture(capture_handle)
    sys.exit(1)

# ============================================================================
# 4. 编码测试
# ============================================================================

print("\n[4/5] 编码测试 (5秒)...")

# 预分配缓冲区
buffer = (ctypes.c_ubyte * (width * height * 4))()

# 统计
stats = {
    'captured': 0,
    'encoded': 0,
    'total_size': 0,
    'key_frames': 0,
    'capture_times': [],
    'encode_times': [],
}

running = True
encode_queue = queue.Queue(maxsize=10)

# 捕获线程
def capture_thread():
    global running
    print("  [捕获线程] 启动")

    local_buffer = (ctypes.c_ubyte * (width * height * 4))()

    while running:
        t0 = time.perf_counter()
        result = capture_dll.capture_hybrid_frame(capture_handle, ctypes.byref(frame_info))
        t1 = time.perf_counter()

        if result == 1:
            # 复制到 CPU
            copy_result = capture_dll.copy_hybrid_frame_to_cpu(
                capture_handle, local_buffer, len(local_buffer)
            )

            if copy_result == 1:
                # 转换为 numpy (避免拷贝)
                arr = np.ctypeslib.as_array(local_buffer)
                arr = arr.reshape((height, width, 4))

                try:
                    encode_queue.put((arr.copy(), time.time()), block=False)
                    stats['captured'] += 1
                    stats['capture_times'].append((t1 - t0) * 1000)
                except queue.Full:
                    pass

    print("  [捕获线程] 停止")

# 编码线程
def encode_thread():
    global running
    print("  [编码线程] 启动")

    frame_count = 0

    while running:
        try:
            frame_data, timestamp = encode_queue.get(timeout=0.1)
        except queue.Empty:
            continue

        t0 = time.perf_counter()

        # 转换为连续字节数组
        frame_data_contiguous = np.ascontiguousarray(frame_data)
        frame_bytes = frame_data_contiguous.ctypes.data_as(ctypes.POINTER(ctypes.c_ubyte))

        # 编码
        force_keyframe = (frame_count == 0)
        result = nvenc_dll.encode_nvenc_frame_cpu(
            nvenc_handle,
            frame_bytes,
            frame_data_contiguous.nbytes,
            int(timestamp * 1000000),
            1 if force_keyframe else 0
        )

        # 尝试获取输出
        encoded_frame = NVENCEncodedFrame()
        if nvenc_dll.get_nvenc_encoded_frame(nvenc_handle, ctypes.byref(encoded_frame)):
            stats['encoded'] += 1
            stats['total_size'] += encoded_frame.size
            if encoded_frame.key_frame:
                stats['key_frames'] += 1
            stats['encode_times'].append((time.perf_counter() - t0) * 1000)

        frame_count += 1

    print("  [编码线程] 停止")

# 启动线程
capture_thr = threading.Thread(target=capture_thread, daemon=True)
encode_thr = threading.Thread(target=encode_thread, daemon=True)

capture_thr.start()
encode_thr.start()

time.sleep(0.5)

# 主循环
start_time = time.time()
test_duration = 5

while time.time() - start_time < test_duration:
    time.sleep(0.5)
    elapsed = time.time() - start_time
    capture_fps = stats['captured'] / elapsed if elapsed > 0 else 0
    encode_fps = stats['encoded'] / elapsed if elapsed > 0 else 0

    print(f"  捕获: {stats['captured']:4d} 帧 @ {capture_fps:5.1f} FPS   "
          f"编码: {stats['encoded']:4d} 帧 @ {encode_fps:5.1f} FPS   "
          f"队列: {encode_queue.qsize():2d}/10")

running = False
capture_thr.join(timeout=2)
encode_thr.join(timeout=2)

# ============================================================================
# 5. 统计结果
# ============================================================================

print("\n[5/5] 测试结果...")

total_time = time.time() - start_time

print("\n" + "=" * 70)
print("测试结果")
print("=" * 70)

print(f"测试时长: {total_time:.1f}s")
print(f"捕获帧数: {stats['captured']}")
print(f"编码帧数: {stats['encoded']}")

if stats['captured'] > 0:
    print(f"\n捕获性能:")
    print(f"  捕获 FPS: {stats['captured'] / total_time:.1f}")
    if stats['capture_times']:
        print(f"  平均延迟: {sum(stats['capture_times'])/len(stats['capture_times']):.2f} ms")

if stats['encoded'] > 0:
    print(f"\n编码性能:")
    print(f"  编码 FPS: {stats['encoded'] / total_time:.1f}")
    if stats['encode_times']:
        print(f"  平均延迟: {sum(stats['encode_times'])/len(stats['encode_times']):.2f} ms")
    print(f"  总大小: {stats['total_size'] / 1024:.1f} KB")
    print(f"  平均帧大小: {stats['total_size'] / stats['encoded']:.0f} bytes")
    print(f"  关键帧数: {stats['key_frames']}")

    # 码率
    bitrate = (stats['total_size'] * 8 / total_time) / 1000000
    print(f"  实际码率: {bitrate:.2f} Mbps")

print(f"\n端到端性能:")
print(f"  端到端 FPS: {stats['encoded'] / total_time:.1f}")

# 评级
pipeline_fps = stats['encoded'] / total_time if total_time > 0 else 0
if pipeline_fps >= 50:
    rating = "⭐⭐⭐ 优秀 - 实时编码达标"
elif pipeline_fps >= 30:
    rating = "⭐⭐ 良好 - 基本实时"
elif pipeline_fps >= 15:
    rating = "⭐ 一般 - 有延迟"
else:
    rating = "❌ 需要优化"

print(f"\n评级: {rating}")

# 清理
nvenc_dll.free_nvenc_encoder(nvenc_handle)
capture_dll.free_hybrid_capture(capture_handle)

print("\n" + "=" * 70)
print("✅ 测试完成")
print("=" * 70)
