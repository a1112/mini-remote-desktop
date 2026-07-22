#!/usr/bin/env python3
"""
完整 NVENC 编码器测试

测试使用 NVENC SDK 13.0 的完整硬件编码功能
"""
import sys
import time
import ctypes
import threading
import queue
import io
import numpy as np
from pathlib import Path

print("=" * 70)
print("完整 NVENC 编码器测试 (SDK 13.0)")
print("=" * 70)

# ============================================================================
# 1. 加载 NVENC 完整编码器 DLL
# ============================================================================

print("\n[1/4] 加载 NVENC 编码器...")

nvenc_dll_path = Path(__file__).parent / 'cpp_capture' / 'nvenc_full.dll'

if not nvenc_dll_path.exists():
    print(f"  ❌ DLL 不存在: {nvenc_dll_path}")
    sys.exit(1)

try:
    nvenc_dll = ctypes.CDLL(str(nvenc_dll_path))
    print(f"  ✅ NVENC DLL 加载成功")
except Exception as e:
    print(f"  ❌ DLL 加载失败: {e}")
    sys.exit(1)

# 设置函数签名
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

# 设置函数签名
nvenc_dll.is_nvenc_supported.argtypes = []
nvenc_dll.is_nvenc_supported.restype = ctypes.c_int

nvenc_dll.is_cuda_d3d11_interop_supported.argtypes = []
nvenc_dll.is_cuda_d3d11_interop_supported.restype = ctypes.c_int

class NVENCVersion(ctypes.Structure):
    _fields_ = [("major", ctypes.c_int), ("minor", ctypes.c_int)]

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

# ============================================================================
# 2. 检查支持情况
# ============================================================================

print("\n[2/4] 检查支持情况...")

version = NVENCVersion()
nvenc_dll.get_nvenc_version(ctypes.byref(version))
print(f"  NVENC 版本: {version.major}.{version.minor}")

nvenc_sup = nvenc_dll.is_nvenc_supported()
print(f"  NVENC 支持: {'✅ 是' if nvenc_sup else '❌ 否'}")

cuda_sup = nvenc_dll.is_cuda_d3d11_interop_supported()
print(f"  CUDA-D3D11 互操作: {'✅ 是' if cuda_sup else '❌ 否'}")

if not nvenc_sup or not cuda_sup:
    print("\n  ⚠️  环境不满足要求，退出测试")
    sys.exit(1)

# ============================================================================
# 3. 创建模拟 D3D11 设备 (用于测试)
# ============================================================================

print("\n[3/4] 初始化编码器...")

# 创建一个模拟的 D3D11 设备指针 (实际上我们不使用 D3D11 路径进行编码测试)
# 直接测试 CPU 输入编码

config = NVENCEncodeConfig()
config.width = 1920
config.height = 1080
config.framerate = 60
config.bitrate = 5_000_000  # 5 Mbps
config.gop_size = 60
config.preset = 3  # fast
config.rc_mode = 2  # CBR

print(f"  配置: {config.width}x{config.height} @ {config.framerate}fps")
print(f"        {config.bitrate / 1000000:.1f} Mbps, GOP={config.gop_size}")

# 注意：由于我们没有真实的 D3D11 设备，这里创建一个虚拟指针
# 实际使用时应该从混合捕获 DLL 获取
d3d11_device = ctypes.c_void_p(1)  # 虚拟设备指针
d3d11_context = ctypes.c_void_p(2)  # 虚拟上下文指针

nvenc_handle = nvenc_dll.init_nvenc_encoder_d3d11(
    d3d11_device,
    d3d11_context,
    ctypes.byref(config)
)

if nvenc_handle:
    print(f"  ✅ NVENC 编码器初始化成功: {hex(nvenc_handle.value or nvenc_handle)}")
else:
    print(f"  ❌ NVENC 编码器初始化失败")
    print(f"     这可能是因为:")
    print(f"     - 传递了无效的 D3D11 设备指针")
    print(f"     - NVENC 驱动不支持当前 GPU")
    print(f"     - CUDA 初始化失败")

    # 尝试使用动态加载版本作为回退
    print(f"\n  尝试使用动态加载版本...")
    dynamic_dll_path = Path(__file__).parent / 'cpp_capture' / 'nvenc_d3d12_dynamic.dll'
    if dynamic_dll_path.exists():
        print(f"  ✅ 可以使用 nvenc_d3d12_dynamic.dll 作为回退")
    sys.exit(1)

# ============================================================================
# 4. 编码测试
# ============================================================================

print("\n[4/4] 编码测试...")

# 创建测试帧数据 (BGRA 格式)
width = 1920
height = 1080
frame_size = width * height * 4
test_frame = np.zeros(frame_size, dtype=np.uint8)

# 生成简单的测试图案 (渐变色)
for y in range(height):
    for x in range(width):
        idx = (y * width + x) * 4
        test_frame[idx + 0] = int(255 * x / width)     # B
        test_frame[idx + 1] = int(255 * y / height)   # G
        test_frame[idx + 2] = int(255 * (x + y) / (width + height))  # R
        test_frame[idx + 3] = 255                     # A

# 编码多帧
print(f"  编码 10 帧测试数据...")

stats = {
    'encoded': 0,
    'total_size': 0,
    'key_frames': 0,
    'encode_times': [],
}

for i in range(10):
    t0 = time.perf_counter()

    # 编码帧
    result = nvenc_dll.encode_nvenc_frame_cpu(
        nvenc_handle,
        test_frame.ctypes.data_as(ctypes.POINTER(ctypes.c_ubyte)),
        frame_size,
        int(time.time() * 1000000),
        1 if i == 0 else 0  # 第一帧强制为关键帧
    )

    t1 = time.perf_counter()

    if result:
        # 尝试获取编码输出
        encoded_frame = NVENCEncodedFrame()
        if nvenc_dll.get_nvenc_encoded_frame(nvenc_handle, ctypes.byref(encoded_frame)):
            stats['encoded'] += 1
            stats['total_size'] += encoded_frame.size
            if encoded_frame.key_frame:
                stats['key_frames'] += 1
            stats['encode_times'].append((t1 - t0) * 1000)

            print(f"    帧 {i+1}: {encoded_frame.size} bytes, "
                  f"{'关键帧' if encoded_frame.key_frame else 'P帧'}, "
                  f"延迟: {((t1 - t0) * 1000):.2f} ms")
        else:
            print(f"    帧 {i+1}: 编码中...")
    else:
        print(f"    帧 {i+1}: 编码失败")

# 等待所有帧完成
print(f"\n  等待编码完成...")
time.sleep(1)

# 检查是否有更多输出
while True:
    encoded_frame = NVENCEncodedFrame()
    if nvenc_dll.get_nvenc_encoded_frame(nvenc_handle, ctypes.byref(encoded_frame)):
        stats['encoded'] += 1
        stats['total_size'] += encoded_frame.size
        if encoded_frame.key_frame:
            stats['key_frames'] += 1
        print(f"    延迟输出: {encoded_frame.size} bytes, "
              f"{'关键帧' if encoded_frame.key_frame else 'P帧'}")
    else:
        break

# 清理
nvenc_dll.free_nvenc_encoder(nvenc_handle)

# ============================================================================
# 5. 统计结果
# ============================================================================

print("\n" + "=" * 70)
print("编码测试结果")
print("=" * 70)

print(f"编码帧数: {stats['encoded']}")
print(f"总大小: {stats['total_size']} bytes ({stats['total_size'] / 1024:.1f} KB)")
print(f"关键帧数: {stats['key_frames']}")

if stats['encoded'] > 0:
    avg_size = stats['total_size'] / stats['encoded']
    print(f"平均帧大小: {avg_size:.0f} bytes")

    if stats['encode_times']:
        avg_time = sum(stats['encode_times']) / len(stats['encode_times'])
        print(f"平均编码延迟: {avg_time:.2f} ms")
        print(f"理论最大 FPS: {1000 / avg_time:.1f}")

    # 评级
    if stats['encoded'] >= 10:
        rating = "⭐⭐⭐ 优秀"
    elif stats['encoded'] >= 5:
        rating = "⭐⭐ 良好"
    else:
        rating = "⭐ 一般"

    print(f"\n评级: {rating}")
else:
    print("\n⚠️  没有成功编码任何帧")
    print("   这可能是因为:")
    print("   - D3D11 设备指针无效")
    print("   - NVENC 会话初始化不完整")
    print("   - 需要真实的 D3D11 设备")

print("\n下一步:")
print("  1. ✅ NVENC 完整编码器 DLL 已编译成功")
print("  2. 使用真实的 D3D11 设备进行完整测试")
print("  3. 集成到混合捕获流水线中")
