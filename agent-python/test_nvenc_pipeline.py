#!/usr/bin/env python3
"""
D3D12-NVENC 编码器测试

测试 D3D12 资源直接传递给 NVENC 编码器
需要: CUDA Toolkit, NVIDIA GPU, NVENC SDK
"""
import sys
import time
import ctypes
import threading
import queue
import io
import numpy as np
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent / 'src'))

print("=" * 70)
print("D3D12-NVENC 编码器测试")
print("=" * 70)

# ============================================================================
# 1. 检查依赖
# ============================================================================

print("\n[1/4] 检查依赖...")

# 检查 CUDA
try:
    cuda_libs = [
        ("nvcuvid", "CUDA Video Decoder"),
        ("cuda", "CUDA Runtime"),
        ("nvEncodeAPI64", "NVENC"),
    ]

    cuda_available = False
    for lib, name in cuda_libs:
        try:
            ctypes.CDLL(lib + ".dll")
            print(f"  ✅ {name}: {lib}.dll")
            cuda_available = True
        except:
            print(f"  ❌ {name}: {lib}.dll 未找到")

    if not cuda_available:
        print("\n  ⚠️  CUDA/NVENC 未安装，使用回退方案")

except Exception as e:
    print(f"  ❌ 检查失败: {e}")
    cuda_available = False

# ============================================================================
# 2. 加载 D3D12 混合捕获
# ============================================================================

print("\n[2/4] 加载 D3D12 混合捕获...")

capture_dll_path = Path(__file__).parent / 'cpp_capture' / 'd3d12_hybrid_capture.dll'
capture_dll = ctypes.CDLL(str(capture_dll_path))

# 设置函数签名
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

capture_dll.get_hybrid_d3d12_device.argtypes = [ctypes.c_void_p]
capture_dll.get_hybrid_d3d12_device.restype = ctypes.c_void_p

capture_dll.get_hybrid_d3d12_queue.argtypes = [ctypes.c_void_p]
capture_dll.get_hybrid_d3d12_queue.restype = ctypes.c_void_p

capture_dll.free_hybrid_capture.argtypes = [ctypes.c_void_p]
capture_dll.free_hybrid_capture.restype = None

# 初始化捕获
capture_handle = capture_dll.init_hybrid_capture(0, 1)
if not capture_handle:
    print("  ❌ 捕获器初始化失败")
    sys.exit(1)

d3d12_device = capture_dll.get_hybrid_d3d12_device(capture_handle)
d3d12_queue = capture_dll.get_hybrid_d3d12_queue(capture_handle)

print(f"  ✅ 捕获器初始化成功")
print(f"  ✅ D3D12 设备: {hex(d3d12_device)}")
print(f"  ✅ D3D12 队列: {hex(d3d12_queue)}")

# 获取尺寸
frame_info = HybridFrame()
result = capture_dll.capture_hybrid_frame(capture_handle, ctypes.byref(frame_info))
width, height = frame_info.width, frame_info.height
print(f"  ✅ 分辨率: {width}x{height}")

# ============================================================================
# 3. 尝试加载 NVENC 编码器
# ============================================================================

print("\n[3/4] 加载 NVENC 编码器...")

nvenc_dll_path = Path(__file__).parent / 'cpp_capture' / 'nvenc_d3d12_encoder.dll'
nvenc_dll = None
nvenc_handle = None

try:
    nvenc_dll = ctypes.CDLL(str(nvenc_dll_path))
    print(f"  ✅ NVENC DLL 加载成功")

    # 设置函数签名
    nvenc_dll.is_nvenc_supported.argtypes = []
    nvenc_dll.is_nvenc_supported.restype = ctypes.c_int

    nvenc_dll.init_nvenc_encoder.argtypes = [
        ctypes.c_void_p,
        ctypes.c_void_p,
        ctypes.c_void_p
    ]
    nvenc_dll.init_nvenc_encoder.restype = ctypes.c_void_p

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

    config = NVENCEncodeConfig()
    config.width = width
    config.height = height
    config.framerate = 60
    config.bitrate = 5_000_000
    config.gop_size = 60
    config.preset = 2  # medium
    config.rc_mode = 2  # CBR

    nvenc_handle = nvenc_dll.init_nvenc_encoder(
        d3d12_device,
        d3d12_queue,
        ctypes.byref(config)
    )

    if nvenc_handle:
        print(f"  ✅ NVENC 编码器初始化成功: {hex(nvenc_handle)}")
        nvenc_available = True
    else:
        print(f"  ⚠️  NVENC 编码器初始化失败，回退到 h264_mf")
        nvenc_available = False

except Exception as e:
    print(f"  ⚠️  NVENC 不可用: {e}")
    print(f"     回退到 h264_mf")
    nvenc_available = False

# ============================================================================
# 4. 编码测试
# ============================================================================

print("\n[4/4] 编码测试 (5秒)...")

# 使用 h264_mf (回退方案或作为基准)
import av

output = io.BytesIO()
container = av.open(output, 'w', format='h264')
stream = container.add_stream('h264_mf', rate=60)
stream.width = width
stream.height = height
stream.bit_rate = 5_000_000
pts = 0

# 预分配缓冲区
buffer = (ctypes.c_ubyte * (width * height * 4))()

# 统计
stats = {
    'captured': 0,
    'encoded': 0,
    'capture_times': [],
    'encode_times': [],
}

running = True
encode_queue = queue.Queue(maxsize=5)

# 捕获线程
def capture_thread():
    global running
    print("  [捕获线程] 启动")

    while running:
        t0 = time.perf_counter()
        result = capture_dll.capture_hybrid_frame(capture_handle, ctypes.byref(frame_info))
        t1 = time.perf_counter()

        if result == 1:
            # 获取 D3D12 资源指针
            d3d12_resource = frame_info.d3d12_resource

            # 复制到 CPU (如果 NVENC 不可用)
            if not nvenc_available or True:  # 暂时总是复制到 CPU
                copy_result = capture_dll.copy_hybrid_frame_to_cpu(
                    capture_handle, buffer, len(buffer)
                )

                if copy_result == 1:
                    # 转换为 numpy
                    arr = np.ctypeslib.as_array(buffer)
                    arr = arr.reshape((height, width, 4))
                    frame_rgb = arr[:, :, :3][:, :, [2, 1, 0]]  # BGRA → RGB

                    try:
                        encode_queue.put((frame_rgb, time.time(), d3d12_resource), block=False)
                        stats['captured'] += 1
                        stats['capture_times'].append((t1 - t0) * 1000)
                    except queue.Full:
                        pass

    print("  [捕获线程] 停止")

# 编码线程
def encode_thread():
    global running
    print("  [编码线程] 启动")

    local_output = io.BytesIO()
    local_container = av.open(local_output, 'w', format='h264')
    local_stream = local_container.add_stream('h264_mf', rate=60)
    local_stream.width = width
    local_stream.height = height
    local_stream.bit_rate = 5_000_000
    local_pts = 0
    max_buffer = 10 * 1024 * 1024

    while running:
        try:
            frame_rgb, timestamp, d3d12_resource = encode_queue.get(timeout=0.1)
        except queue.Empty:
            continue

        t0 = time.perf_counter()

        # 优先使用 NVENC
        if nvenc_available and nvenc_handle:
            # NVENC 编码 (D3D12 直接传递)
            pass  # TODO: 实现 NVENC 调用
        else:
            # h264_mf 编码
            av_frame = av.VideoFrame.from_ndarray(frame_rgb, format='rgb24')
            av_frame.pts = local_pts
            local_pts += 1

            start_pos = local_output.tell()
            for packet in local_stream.encode(av_frame):
                local_container.mux(packet)
            end_pos = local_output.tell()

            if end_pos > start_pos:
                with threading.Lock():
                    stats['encoded'] += 1
                    stats['encode_times'].append((time.perf_counter() - t0) * 1000)

                # 重置缓冲区
                if end_pos > max_buffer:
                    local_output = io.BytesIO()
                    local_container = av.open(local_output, 'w', format='h264')
                    local_stream = local_container.add_stream('h264_mf', rate=60)
                    local_stream.width = width
                    local_stream.height = height
                    local_stream.bit_rate = 5_000_000
                    local_pts = 0

    print("  [编码线程] 停止")

# 启动线程
capture_thr = threading.Thread(target=capture_thread, daemon=True)
encode_thr = threading.Thread(target=encode_thread, daemon=True)

capture_thr.start()
encode_thr.start()

time.sleep(0.5)

# 主循环
start_time = time.time()

while time.time() - start_time < 5:
    time.sleep(0.5)
    elapsed = time.time() - start_time
    capture_fps = stats['captured'] / elapsed if elapsed > 0 else 0
    encode_fps = stats['encoded'] / elapsed if elapsed > 0 else 0

    print(f"  捕获: {stats['captured']:4d} 帧 @ {capture_fps:5.1f} FPS   "
          f"编码: {stats['encoded']:4d} 帧 @ {encode_fps:5.1f} FPS   "
          f"队列: {encode_queue.qsize():2d}/5   "
          f"NVENC: {'✅' if nvenc_available else '❌'}")

running = False
capture_thr.join(timeout=2)
encode_thr.join(timeout=2)

# ============================================================================
# 5. 统计结果
# ============================================================================

print("\n" + "=" * 70)
print("编码器测试结果")
print("=" * 70)

total_time = time.time() - start_time

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

print(f"\n可用组件:")
print(f"  D3D12 设备: ✅ {hex(d3d12_device)}")
print(f"  D3D12 队列: ✅ {hex(d3d12_queue)}")
print(f"  NVENC 编码器: {'✅ 可用' if nvenc_available else '❌ 不可用 (需要 CUDA/NVENC)'}")

# 清理
if nvenc_handle and nvenc_dll:
    nvenc_dll.free_nvenc_encoder(nvenc_handle)

capture_dll.free_hybrid_capture(capture_handle)

# 评级
pipeline_fps = stats['encoded'] / total_time if total_time > 0 else 0
if pipeline_fps >= 100:
    rating = "⭐⭐⭐ 优秀"
elif pipeline_fps >= 50:
    rating = "⭐⭐ 良好"
else:
    rating = "⭐ 一般"

print(f"\n评级: {rating}")

print(f"\n下一步:")
print(f"  1. 安装 CUDA Toolkit 11.0+")
print(f"  2. 安装 NVIDIA Video Codec SDK")
print(f"  3. 编译 nvenc_d3d12_encoder.dll")
print(f"  4. 启用零拷贝 D3D12 → NVENC 编码")
