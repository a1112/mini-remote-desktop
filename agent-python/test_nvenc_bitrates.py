#!/usr/bin/env python3
"""
NVENC 多档码率/质量测试

测试不同码率和质量模式的编码效果
"""
import sys
import time
import ctypes
import numpy as np
from pathlib import Path

print("=" * 70)
print("NVENC 多档码率/质量对比测试")
print("=" * 70)

# ============================================================================
# 码率/质量配置
# ============================================================================

configs = [
    # (name, rc_mode, bitrate, quality, description)
    # rc_mode: 0=ConstQP(固定质量)
    # QP 值越低质量越高，码率也越高
    ("保真", 0, 0, 18, "QP=18，最佳质量 (~150Mbps)"),
    ("20M级", 0, 0, 24, "QP=24，高质量 (~80Mbps)"),
    ("10M级", 0, 0, 30, "QP=30，中高质量 (~50Mbps)"),
    ("5M级", 0, 0, 36, "QP=36，中等质量 (~25Mbps)"),
    ("2M级", 0, 0, 42, "QP=42，偏低质量 (~10Mbps)"),
    ("1M级", 0, 0, 48, "QP=48，低质量 (~3Mbps)"),
]

# ============================================================================
# 加载 DLL
# ============================================================================

print("\n[1/3] 初始化...")

# 混合捕获
capture_dll_path = Path(__file__).parent / 'cpp_capture' / 'd3d12_hybrid_capture.dll'
capture_dll = ctypes.CDLL(str(capture_dll_path))

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

capture_dll.init_hybrid_capture.argtypes = [ctypes.c_int, ctypes.c_int]
capture_dll.init_hybrid_capture.restype = ctypes.c_void_p
capture_dll.capture_hybrid_frame.argtypes = [ctypes.c_void_p, ctypes.POINTER(HybridFrame)]
capture_dll.capture_hybrid_frame.restype = ctypes.c_int
capture_dll.copy_hybrid_frame_to_cpu.argtypes = [ctypes.c_void_p, ctypes.POINTER(ctypes.c_ubyte), ctypes.c_int]
capture_dll.copy_hybrid_frame_to_cpu.restype = ctypes.c_int
capture_dll.get_hybrid_d3d11_device.argtypes = [ctypes.c_void_p]
capture_dll.get_hybrid_d3d11_device.restype = ctypes.c_void_p
capture_dll.get_hybrid_d3d11_context.argtypes = [ctypes.c_void_p]
capture_dll.get_hybrid_d3d11_context.restype = ctypes.c_void_p
capture_dll.free_hybrid_capture.argtypes = [ctypes.c_void_p]
capture_dll.free_hybrid_capture.restype = None

# 初始化捕获
capture_handle = capture_dll.init_hybrid_capture(0, 0)
d3d11_device = capture_dll.get_hybrid_d3d11_device(capture_handle)
d3d11_context = capture_dll.get_hybrid_d3d11_context(capture_handle)

frame_info = HybridFrame()
capture_dll.capture_hybrid_frame(capture_handle, ctypes.byref(frame_info))
width, height = frame_info.width, frame_info.height

# NVENC 编码器
nvenc_dll_path = Path(__file__).parent / 'cpp_capture' / 'nvenc_full.dll'
nvenc_dll = ctypes.CDLL(str(nvenc_dll_path))

class NVENCEncodeConfig(ctypes.Structure):
    _fields_ = [
        ("width", ctypes.c_int),
        ("height", ctypes.c_int),
        ("framerate", ctypes.c_int),
        ("bitrate", ctypes.c_int),
        ("gop_size", ctypes.c_int),
        ("preset", ctypes.c_int),
        ("rc_mode", ctypes.c_int),
        ("quality", ctypes.c_int),
    ]

class NVENCEncodedFrame(ctypes.Structure):
    _fields_ = [
        ("data", ctypes.POINTER(ctypes.c_ubyte)),
        ("size", ctypes.c_int),
        ("key_frame", ctypes.c_int),
        ("timestamp", ctypes.c_longlong),
    ]

nvenc_dll.init_nvenc_encoder_d3d11.argtypes = [ctypes.c_void_p, ctypes.c_void_p, ctypes.c_void_p]
nvenc_dll.init_nvenc_encoder_d3d11.restype = ctypes.c_void_p
nvenc_dll.encode_nvenc_frame_cpu.argtypes = [ctypes.c_void_p, ctypes.POINTER(ctypes.c_ubyte), ctypes.c_int, ctypes.c_longlong, ctypes.c_int]
nvenc_dll.encode_nvenc_frame_cpu.restype = ctypes.c_int
nvenc_dll.get_nvenc_encoded_frame.argtypes = [ctypes.c_void_p, ctypes.POINTER(NVENCEncodedFrame)]
nvenc_dll.get_nvenc_encoded_frame.restype = ctypes.c_int
nvenc_dll.free_nvenc_encoder.argtypes = [ctypes.c_void_p]
nvenc_dll.free_nvenc_encoder.restype = None

buffer = (ctypes.c_ubyte * (width * height * 4))()

print(f"  ✅ 分辨率: {width}x{height}")

# ============================================================================
# 测试函数
# ============================================================================

def test_config(config_name, rc_mode, bitrate, quality, description, test_duration=3):
    """测试单个配置"""
    print(f"\n{'='*60}")
    print(f"测试: {config_name} - {description}")
    print(f"{'-'*50}")

    # 配置编码器
    config = NVENCEncodeConfig()
    config.width = width
    config.height = height
    config.framerate = 60
    config.bitrate = bitrate
    config.gop_size = 60
    config.preset = 3  # fast
    config.rc_mode = rc_mode
    config.quality = quality

    nvenc_handle = nvenc_dll.init_nvenc_encoder_d3d11(
        ctypes.c_void_p(d3d11_device),
        ctypes.c_void_p(d3d11_context),
        ctypes.byref(config)
    )

    if not nvenc_handle:
        print(f"  ❌ 编码器初始化失败")
        return None

    # 编码测试
    stats = {'encoded': 0, 'total_size': 0, 'times': []}

    start_time = time.time()
    last_print = start_time

    while time.time() - start_time < test_duration:
        # 捕获一帧
        t0 = time.perf_counter()
        result = capture_dll.capture_hybrid_frame(capture_handle, ctypes.byref(frame_info))
        t1 = time.perf_counter()

        if result == 1:
            # 复制到 CPU
            copy_result = capture_dll.copy_hybrid_frame_to_cpu(
                capture_handle, buffer, len(buffer)
            )

            if copy_result == 1:
                # 转换为 numpy
                arr = np.ctypeslib.as_array(buffer)
                arr = arr.reshape((height, width, 4))
                frame_bytes = np.ascontiguousarray(arr).ctypes.data_as(ctypes.POINTER(ctypes.c_ubyte))

                # 编码
                t2 = time.perf_counter()
                nvenc_dll.encode_nvenc_frame_cpu(
                    nvenc_handle,
                    frame_bytes,
                    arr.nbytes,
                    int(time.time() * 1000000),
                    0
                )

                # 获取输出
                encoded_frame = NVENCEncodedFrame()
                if nvenc_dll.get_nvenc_encoded_frame(nvenc_handle, ctypes.byref(encoded_frame)):
                    stats['encoded'] += 1
                    stats['total_size'] += encoded_frame.size
                    stats['times'].append((time.perf_counter() - t2) * 1000)

        # 打印进度
        if time.time() - last_print >= 0.5:
            elapsed = time.time() - start_time
            fps = stats['encoded'] / elapsed if elapsed > 0 else 0
            bitrate_mbps = (stats['total_size'] * 8 / elapsed / 1000000) if elapsed > 0 else 0
            print(f"  进度: {elapsed:.1f}s / {test_duration}s   "
                  f"FPS: {fps:5.1f}   码率: {bitrate_mbps:5.1f} Mbps   "
                  f"帧数: {stats['encoded']:3d}", end='\r')
            last_print = time.time()

    # 清理
    nvenc_dll.free_nvenc_encoder(nvenc_handle)

    # 统计结果
    total_time = time.time() - start_time
    avg_bitrate = (stats['total_size'] * 8 / total_time / 1000000) if total_time > 0 else 0
    avg_size = stats['total_size'] / stats['encoded'] if stats['encoded'] > 0 else 0
    avg_fps = stats['encoded'] / total_time if total_time > 0 else 0
    avg_latency = sum(stats['times']) / len(stats['times']) if stats['times'] else 0

    print()  # 换行

    return {
        'name': config_name,
        'description': description,
        'fps': avg_fps,
        'bitrate': avg_bitrate,
        'avg_size': avg_size,
        'latency': avg_latency,
        'encoded': stats['encoded'],
    }

# ============================================================================
# 测试所有配置
# ============================================================================

print("\n[2/3] 测试所有配置...")

results = []
for config_name, rc_mode, bitrate, quality, description in configs:
    result = test_config(config_name, rc_mode, bitrate, quality, description)
    if result:
        results.append(result)

# ============================================================================
# 汇总对比
# ============================================================================

print("\n[3/3] 对比汇总")
print("=" * 70)
print(f"{'配置':<8} {'描述':<12} {'FPS':>6} {'实际码率':>10} {'帧大小':>10} {'延迟':>6}")
print("-" * 70)

for r in results:
    print(f"{r['name']:<8} {r['description']:<12} {r['fps']:>6.1f} {r['bitrate']:>10.1f} {r['avg_size']:>10.0f} {r['latency']:>6.1f}")

print()
print("质量说明:")
print("  保真 (QP=18): ⭐⭐⭐⭐⭐ 文字完美，细节丰富，适合专业工作")
print("  20M级 (QP=24): ⭐⭐⭐⭐⭐ 高质量，文字清晰，适合办公")
print("  10M级 (QP=30): ⭐⭐⭐⭐   良好质量，文字可读")
print("  5M级  (QP=36): ⭐⭐⭐    基本可用，轻微模糊")
print("  2M级  (QP=42): ⭐⭐     文字边缘模糊，勉强可读")
print("  1M级  (QP=48): ⭐       文字难以辨认，不推荐")

print()
print("说明:")
print("  - QP (Quantization Parameter): 值越低质量越高，码率也越高")
print("  - 2560x1440 分辨率下，NVENC 最小码率约为 10-30 Mbps")
print("  - 如需更低码率，建议降低分辨率或使用软件编码器")
print()
print("建议:")
print("  - 办公/开发: 使用 保真 或 20M级")
print("  - 常规使用: 10M级")
print("  - 低带宽: 5M级 或更低")

# 清理
capture_dll.free_hybrid_capture(capture_handle)

print("\n" + "=" * 70)
print("✅ 测试完成")
print("=" * 70)
