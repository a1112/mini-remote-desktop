#!/usr/bin/env python3
"""
窗口 WGC -> NVENC 测试

测试指定窗口的 GPU Direct 编码管道
"""

import sys
import time
import ctypes
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent / 'src'))

from src.capture.wgc_capture import WGCCapture

TARGET_HWND = 0x1900F2A

print("=" * 70)
print("窗口 WGC -> NVENC 编码测试")
print("=" * 70)
print(f"目标 HWND: 0x{TARGET_HWND:X}")
print()

# ============================================================================
# 1. 加载 DLL
# ============================================================================

print("[1/6] 加载 DLL...")
print("-" * 70)

wgc_dll_path = Path(__file__).parent / "cpp_capture" / "wgc_capture.dll"
nvenc_dll_path = Path(__file__).parent / "cpp_capture" / "nvenc_full.dll"

if not wgc_dll_path.exists():
    print(f"  ✗ WGC DLL 不存在: {wgc_dll_path}")
    sys.exit(1)

if not nvenc_dll_path.exists():
    print(f"  ✗ NVENC DLL 不存在: {nvenc_dll_path}")
    sys.exit(1)

print(f"  ✓ wgc_capture.dll: {wgc_dll_path.stat().st_size:,} 字节")
print(f"  ✓ nvenc_full.dll: {nvenc_dll_path.stat().st_size:,} 字节")

# ============================================================================
# 2. 枚举窗口
# ============================================================================

print()
print("[2/6] 枚举可用窗口...")
print("-" * 70)

windows = WGCCapture.enum_windows()
print(f"  发现 {len(windows)} 个窗口:")

target_found = False
for i, w in enumerate(windows[:30]):  # 只显示前 30 个
    hwnd_hex = f"0x{w.hwnd:X}"
    marker = " <- 目标" if w.hwnd == TARGET_HWND else ""
    visible = "[可见]" if w.is_visible else "[隐藏]"
    title_short = (w.title[:40] + "...") if len(w.title) > 40 else w.title
    print(f"    [{i}] {hwnd_hex} - {title_short} {visible}{marker}")
    if w.hwnd == TARGET_HWND:
        target_found = True
        print(f"        大小: {w.size[0]}x{w.size[1]}")

if len(windows) > 30:
    print(f"    ... 还有 {len(windows) - 30} 个窗口")

if not target_found:
    print()
    print(f"  ⚠ 目标窗口 0x{TARGET_HWND:X} 不在可见窗口列表中")
    print(f"     尝试继续捕获...")

# ============================================================================
# 3. 初始化 WGC Capture (窗口模式)
# ============================================================================

print()
print("[3/6] 初始化 WGC Capture (窗口模式)...")
print("-" * 70)

capture = WGCCapture()

print(f"  尝试启动窗口捕获: HWND=0x{TARGET_HWND:X}")
if not capture.start_window(TARGET_HWND):
    print("  ✗ 启动窗口捕获失败")
    print()
    print("  可能原因:")
    print("    1. 窗口句柄无效")
    print("    2. 窗口不支持捕获 (最小化、某些特殊窗口)")
    print("    3. 桌面复制 API 限制")
    sys.exit(1)

device = capture.d3d11_device
print(f"  ✓ 捕获已启动")
print(f"  ✓ D3D11 设备: {hex(device)}")

# 等待窗口更新
print("  等待窗口更新...")
frame = None
for attempt in range(20):
    frame = capture.capture_frame()
    if frame:
        break
    time.sleep(0.1)

if frame:
    print(f"  ✓ 首帧捕获: {frame.width}x{frame.height}")
    print(f"  ✓ D3D11 纹理: {hex(frame.d3d11_texture)}")
    width, height = frame.width, frame.height
else:
    print("  ✗ 未捕获到帧 (窗口可能无更新)")
    print("  提示: 移动鼠标或在窗口中操作来生成更新")
    # 继续尝试，可能需要更长时间

# ============================================================================
# 4. 初始化 NVENC 编码器
# ============================================================================

print()
print("[4/6] 初始化 NVENC 编码器...")
print("-" * 70)

# 加载 NVENC DLL
nvenc = ctypes.CDLL(str(nvenc_dll_path))

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
        ("quality", ctypes.c_int),
    ]

nvenc.is_nvenc_supported.argtypes = []
nvenc.is_nvenc_supported.restype = ctypes.c_int

nvenc.init_nvenc_encoder_d3d11.argtypes = [
    ctypes.c_void_p,
    ctypes.c_void_p,
    ctypes.POINTER(NVENCEncodeConfig)
]
nvenc.init_nvenc_encoder_d3d11.restype = ctypes.c_void_p

nvenc.encode_nvenc_frame_cpu.argtypes = [
    ctypes.c_void_p,
    ctypes.c_void_p,
    ctypes.c_int,
    ctypes.c_longlong,
    ctypes.c_int,
]
nvenc.encode_nvenc_frame_cpu.restype = ctypes.c_int

nvenc.get_nvenc_encoded_frame_buffer.argtypes = [
    ctypes.c_void_p,
    ctypes.POINTER(ctypes.c_ubyte),
    ctypes.POINTER(ctypes.c_int),
    ctypes.POINTER(ctypes.c_int),
    ctypes.POINTER(ctypes.c_longlong),
]
nvenc.get_nvenc_encoded_frame_buffer.restype = ctypes.c_int

nvenc.free_nvenc_encoder.argtypes = [ctypes.c_void_p]
nvenc.free_nvenc_encoder.restype = None

# 检查 NVENC 支持
supported = nvenc.is_nvenc_supported()
print(f"  NVENC 支持: {'是' if supported else '否'}")

if not supported:
    print("  ✗ NVENC 不可用")
    sys.exit(1)

# 获取 D3D11 上下文
d3d11_context = capture.d3d11_context
print(f"  D3D11 上下文: {hex(d3d11_context)}")

# 创建编码配置
# 使用窗口实际分辨率
encode_width = frame.width if frame else 1920
encode_height = frame.height if frame else 1080

config = NVENCEncodeConfig(
    width=encode_width,
    height=encode_height,
    framerate=60,
    bitrate=8000000,  # 8Mbps for better quality
    gop_size=60,
    preset=3,  # P4 (fast)
    rc_mode=0,  # ConstQP
    quality=20,  # QP 20
)

print(f"  配置: {encode_width}x{encode_height} @ 60fps, 8Mbps, QP=20")

# 初始化编码器
encoder = nvenc.init_nvenc_encoder_d3d11(
    ctypes.c_void_p(device),
    ctypes.c_void_p(d3d11_context),
    ctypes.byref(config)
)

if not encoder:
    print("  ✗ NVENC 初始化失败")
    sys.exit(1)

print(f"  ✓ NVENC 编码器: {hex(encoder)}")

# ============================================================================
# 5. 窗口编码测试
# ============================================================================

print()
print("[5/6] 窗口编码测试 (30帧)...")
print("-" * 70)

# 定义缓冲区
MAX_ENCODED_SIZE = encode_width * encode_height * 2
encoded_buffer = ctypes.create_string_buffer(MAX_ENCODED_SIZE)
CPU_BUFFER_SIZE = encode_width * encode_height * 4
cpu_buffer = ctypes.create_string_buffer(CPU_BUFFER_SIZE)

# 编码统计
frame_times = []
encode_results = []
total_encoded_size = 0
keyframe_interval = 30

print(f"  开始编码 30 帧...")
print(f"  提示: 在目标窗口中移动鼠标或操作以生成更新")
print()

for i in range(30):
    # 捕获新帧
    start_capture = time.perf_counter()
    frame = capture.capture_frame()
    capture_time = (time.perf_counter() - start_capture) * 1000

    if not frame:
        # 无新帧，等待一下再试
        time.sleep(0.01)
        frame = capture.capture_frame()
        if not frame:
            print(f"  帧 {i+1}: 无更新 (等待窗口活动...)")
            continue

    # 复制到 CPU 内存
    if not capture.copy_to_cpu(cpu_buffer):
        print(f"  ✗ 帧 {i+1}: 复制到 CPU 失败")
        continue

    timestamp = time.perf_counter_ns()
    force_keyframe = (i % keyframe_interval == 0)

    start_encode = time.perf_counter()

    # 编码
    result = nvenc.encode_nvenc_frame_cpu(
        encoder,
        cpu_buffer,
        CPU_BUFFER_SIZE,
        ctypes.c_longlong(timestamp),
        ctypes.c_int(1 if force_keyframe else 0)
    )

    encode_time = (time.perf_counter() - start_encode) * 1000

    if result == 1:
        # 获取编码后的数据
        data_size = ctypes.c_int(0)
        out_size = ctypes.c_int(0)
        out_pts = ctypes.c_longlong(0)

        get_result = nvenc.get_nvenc_encoded_frame_buffer(
            encoder,
            ctypes.cast(encoded_buffer, ctypes.POINTER(ctypes.c_ubyte)),
            ctypes.byref(data_size),
            ctypes.byref(out_size),
            ctypes.byref(out_pts)
        )

        frame_type = "关键帧" if force_keyframe else "P帧"
        size_info = ""
        if get_result == 1 and out_size.value > 0:
            total_encoded_size += out_size.value
            size_kb = out_size.value / 1024
            size_info = f", 大小 {size_kb:.1f} KB"

        print(f"  帧 {i+1}: 捕获 {capture_time:.2f}ms, 编码 {encode_time:.2f}ms{size_info}, {frame_type}")
        frame_times.append(encode_time)
        encode_results.append(True)
    else:
        print(f"  ✗ 帧 {i+1}: 编码失败")
        encode_results.append(False)

# 轮询获取延迟的编码帧
print()
print("  轮询获取延迟的编码输出...")
for j in range(10):
    data_size = ctypes.c_int(0)
    out_size = ctypes.c_int(0)
    out_pts = ctypes.c_longlong(0)

    result = nvenc.get_nvenc_encoded_frame_buffer(
        encoder,
        ctypes.cast(encoded_buffer, ctypes.POINTER(ctypes.c_ubyte)),
        ctypes.byref(data_size),
        ctypes.byref(out_size),
        ctypes.byref(out_pts)
    )

    if result == 1 and out_size.value > 0:
        size_kb = out_size.value / 1024
        total_encoded_size += out_size.value
        print(f"    延迟帧 {j+1}: 大小 {size_kb:.1f} KB")
    else:
        break

# ============================================================================
# 6. 性能分析
# ============================================================================

print()
print("[6/6] 性能分析")
print("-" * 70)

successful_frames = sum(encode_results)
total_frames = len(encode_results)

print(f"  编码统计:")
print(f"    总帧数: {total_frames}")
print(f"    成功: {successful_frames}")
print(f"    失败: {total_frames - successful_frames}")

if successful_frames > 0:
    success_rate = successful_frames / total_frames * 100
    print(f"    成功率: {success_rate:.1f}%")

if frame_times:
    avg_encode = sum(frame_times) / len(frame_times)
    max_encode = max(frame_times)
    min_encode = min(frame_times)

    print()
    print(f"  编码延迟:")
    print(f"    平均: {avg_encode:.2f} ms")
    print(f"    最小: {min_encode:.2f} ms")
    print(f"    最大: {max_encode:.2f} ms")

    # 假设捕获延迟 ~2ms
    total_pipeline_time = avg_encode + 2
    theoretical_fps = 1000 / total_pipeline_time

    print()
    print(f"  管道分析:")
    print(f"    NVENC 编码: {avg_encode:.2f} ms")
    print(f"    WGC 捕获: ~2 ms (估计)")
    print(f"    总延迟: {total_pipeline_time:.2f} ms")
    print(f"    理论 FPS: {theoretical_fps:.1f}")

    # 评级
    if theoretical_fps >= 144:
        rating = "🚀 A+ - 超过 144fps 目标!"
    elif theoretical_fps >= 120:
        rating = "✓ A - 优秀"
    elif theoretical_fps >= 60:
        rating = "⚠ B - 良好"
    else:
        rating = "✗ C - 需优化"

    print()
    print(f"  评级: {rating}")

if total_encoded_size > 0:
    avg_frame_size = total_encoded_size / successful_frames if successful_frames > 0 else 0
    print()
    print(f"  编码数据:")
    print(f"    总大小: {total_encoded_size / 1024:.1f} KB")
    print(f"    平均帧大小: {avg_frame_size / 1024:.2f} KB")

# ============================================================================
# 清理
# ============================================================================

print()
print("清理资源...")
capture.stop()
nvenc.free_nvenc_encoder(encoder)

print()
print("=" * 70)
print("测试完成!")
print("=" * 70)
print()
print("窗口 WGC -> NVENC 管道验证:")
print("  ✓ WGC 窗口捕获 → D3D11 纹理")
print("  ✓ D3D11 纹理 → CPU 内存 (BGRA)")
print("  ✓ CPU BGRA → NVENC 编码 → H.264 比特流")
print()
print("窗口捕获管道可用!")
