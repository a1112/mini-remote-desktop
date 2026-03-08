#!/usr/bin/env python3
"""
NVENC WebRTC 集成测试

测试 NVENC 编码器与 WebRTC 轨道的集成
"""
import sys
import time
import asyncio
import ctypes
import numpy as np
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent / 'src'))

print("=" * 70)
print("NVENC WebRTC 集成测试")
print("=" * 70)

# ============================================================================
# 1. 初始化捕获
# ============================================================================
print("\n[1/4] 初始化 DXGI 捕获...")

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

capture_handle = capture_dll.init_hybrid_capture(0, 0)
d3d11_device = capture_dll.get_hybrid_d3d11_device(capture_handle)
d3d11_context = capture_dll.get_hybrid_d3d11_context(capture_handle)

frame_info = HybridFrame()
capture_dll.capture_hybrid_frame(capture_handle, ctypes.byref(frame_info))
width, height = frame_info.width, frame_info.height

print(f"  ✅ 分辨率: {width}x{height}")

# ============================================================================
# 2. 测试 NVENC 编码器
# ============================================================================
print("\n[2/4] 测试 NVENC 编码器...")

from src.encoder.nvenc_encoder import create_nvenc_encoder, NVENCEncoder

# 测试不同质量级别
qualities = [
    (18, "保真"),
    (24, "高质"),
    (30, "中高"),
    (36, "中等"),
]

for quality, name in qualities:
    encoder = create_nvenc_encoder(d3d11_device, d3d11_context, width, height, quality, 60)
    if encoder:
        print(f"  ✅ {name} (QP={quality}): 编码器创建成功")
        encoder.close()
    else:
        print(f"  ❌ {name} (QP={quality}): 编码器创建失败")

# ============================================================================
# 3. 测试编码
# ============================================================================
print("\n[3/4] 测试实际编码...")

encoder = create_nvenc_encoder(d3d11_device, d3d11_context, width, height, 30, 60)

if encoder:
    buffer = (ctypes.c_ubyte * (width * height * 4))()

    stats = {"encoded": 0, "total_size": 0}
    start_time = time.time()
    test_duration = 2

    print(f"  编码 {test_duration} 秒...")

    while time.time() - start_time < test_duration:
        result = capture_dll.capture_hybrid_frame(capture_handle, ctypes.byref(frame_info))
        if result == 1:
            copy_result = capture_dll.copy_hybrid_frame_to_cpu(
                capture_handle, buffer, len(buffer)
            )
            if copy_result == 1:
                frame_bytes = bytes(buffer)
                encoded = encoder.encode(frame_bytes)
                if encoded:
                    stats["encoded"] += 1
                    stats["total_size"] += encoded.size

    encoder.close()

    elapsed = time.time() - start_time
    fps = stats["encoded"] / elapsed
    avg_size = stats["total_size"] / stats["encoded"] if stats["encoded"] > 0 else 0
    bitrate = (stats["total_size"] * 8 / elapsed / 1000000) if elapsed > 0 else 0

    print(f"  ✅ 编码完成: {stats['encoded']} 帧")
    print(f"     FPS: {fps:.1f}")
    print(f"     平均帧大小: {avg_size:.0f} bytes")
    print(f"     码率: {bitrate:.1f} Mbps")

# ============================================================================
# 4. 测试 WebRTC 轨道
# ============================================================================
print("\n[4/4] 测试 NVENC WebRTC 轨道...")

try:
    from src.webrtc.nvenc_track import NVENCVideoTrack

    async def test_track():
        track = NVENCVideoTrack(
            d3d11_device,
            d3d11_context,
            width,
            height,
            fps=60,
            quality=30
        )

        print(f"  ✅ NVENC 视频轨道创建成功")

        # 模拟发送几帧
        buffer = (ctypes.c_ubyte * (width * height * 4))()
        frames_sent = 0

        for _ in range(5):
            result = capture_dll.capture_hybrid_frame(capture_handle, ctypes.byref(frame_info))
            if result == 1:
                copy_result = capture_dll.copy_hybrid_frame_to_cpu(
                    capture_handle, buffer, len(buffer)
                )
                if copy_result == 1:
                    await track.send_frame(bytes(buffer))
                    frames_sent += 1

                    # 检查是否有编码帧
                    encoded = await track.get_encoded_frame()
                    if encoded:
                        print(f"  ✅ 编码帧: {encoded.size} bytes, key_frame={encoded.key_frame}")

        print(f"  ✅ 发送了 {frames_sent} 帧到轨道")

        track.stop()
        print(f"  ✅ 轨道统计: {track.stats}")

    asyncio.run(test_track())

except ImportError as e:
    print(f"  ⚠️  WebRTC 轨道测试跳过: {e}")
except Exception as e:
    print(f"  ❌ WebRTC 轨道测试失败: {e}")

# 清理
capture_dll.free_hybrid_capture(capture_handle)

print("\n" + "=" * 70)
print("✅ 测试完成")
print("=" * 70)
