"""
GPU Direct 简单测试 - 直接使用 ctypes 验证 WGC → D3D11 → CUDA → NVENC 零拷贝
"""

import ctypes
import ctypes.wintypes as wintypes
import time

# ============================================================================
# 加载 WGC Capture DLL
# ============================================================================

wgc_dll = ctypes.CDLL(r"J:\ProjectTest\远程探查\mini-remote-desktop\agent-python\cpp_capture\wgc_capture.dll")

# WGC 常量
WGC_TYPE_MONITOR = 0
WGC_TYPE_WINDOW = 1

class WgcFrame(ctypes.Structure):
    _fields_ = [
        ("width", ctypes.c_int),
        ("height", ctypes.c_int),
        ("d3d11_texture", ctypes.c_void_p),
        ("timestamp", ctypes.c_longlong),
        ("frame_id", ctypes.c_uint),
    ]

# WGC 函数
wgc_dll.wgc_create_session.restype = ctypes.c_void_p
wgc_dll.wgc_create_session.argtypes = [ctypes.c_int, ctypes.c_void_p]

wgc_dll.wgc_start.restype = ctypes.c_int
wgc_dll.wgc_start.argtypes = [ctypes.c_void_p]

wgc_dll.wgc_get_frame.restype = ctypes.c_int
wgc_dll.wgc_get_frame.argtypes = [ctypes.c_void_p, ctypes.POINTER(WgcFrame)]

wgc_dll.wgc_free_session.restype = None
wgc_dll.wgc_free_session.argtypes = [ctypes.c_void_p]

wgc_dll.wgc_get_d3d11_device.restype = ctypes.c_void_p
wgc_dll.wgc_get_d3d11_device.argtypes = [ctypes.c_void_p]

wgc_dll.wgc_get_d3d11_context.restype = ctypes.c_void_p
wgc_dll.wgc_get_d3d11_context.argtypes = [ctypes.c_void_p]

# ============================================================================
# 加载 NVENC Full DLL
# ============================================================================

nvenc_dll = ctypes.CDLL(r"J:\ProjectTest\远程探查\mini-remote-desktop\agent-python\cpp_capture\nvenc_full.dll")

# NVENC 常量
NVENC_H264 = 0
NVENC_HEVC = 1

class NVENCEncodeConfig(ctypes.Structure):
    _fields_ = [
        ("width", ctypes.c_int),
        ("height", ctypes.c_int),
        ("framerate", ctypes.c_int),
        ("bitrate", ctypes.c_int),
        ("gop_size", ctypes.c_int),
        ("preset", ctypes.c_int),           # 0=default, 1=slow, 2=medium, 3=fast, 4=fastest
        ("rc_mode", ctypes.c_int),          # 0=constqp, 1=vbr, 2=cbr, 3=cq
        ("quality", ctypes.c_int),          # 质量级别 (1-51)
    ]

class NVENCEncodedFrame(ctypes.Structure):
    _fields_ = [
        ("data", ctypes.POINTER(ctypes.c_uint8)),
        ("size", ctypes.c_int),
        ("key_frame", ctypes.c_int),
        ("timestamp", ctypes.c_longlong),
    ]

class NVENCZeroCopyStats(ctypes.Structure):
    _fields_ = [
        ("encode_calls", ctypes.c_ulonglong),
        ("encode_submit_success", ctypes.c_ulonglong),
        ("encode_submit_need_more_input", ctypes.c_ulonglong),
        ("encode_submit_fail", ctypes.c_ulonglong),
        ("slot_busy_skips", ctypes.c_ulonglong),
        ("map_failures", ctypes.c_ulonglong),
        ("lock_busy_count", ctypes.c_ulonglong),
        ("lock_retryable_count", ctypes.c_ulonglong),
        ("lock_failures", ctypes.c_ulonglong),
        ("bitstream_outputs", ctypes.c_ulonglong),
        ("unmap_count", ctypes.c_ulonglong),
        ("pending_peak", ctypes.c_uint),
        ("pending_current", ctypes.c_uint),
    ]

# NVENC 函数 (使用实际的导出函数名)
nvenc_dll.is_nvenc_supported.restype = ctypes.c_int
nvenc_dll.is_nvenc_supported.argtypes = []

nvenc_dll.is_cuda_d3d11_interop_supported.restype = ctypes.c_int
nvenc_dll.is_cuda_d3d11_interop_supported.argtypes = []

nvenc_dll.init_nvenc_encoder_d3d11.restype = ctypes.c_void_p
nvenc_dll.init_nvenc_encoder_d3d11.argtypes = [ctypes.c_void_p, ctypes.c_void_p, ctypes.POINTER(NVENCEncodeConfig)]

try:
    nvenc_dll.init_nvenc_encoder_d3d11_zerocopy.restype = ctypes.c_void_p
    nvenc_dll.init_nvenc_encoder_d3d11_zerocopy.argtypes = [ctypes.c_void_p, ctypes.c_void_p, ctypes.POINTER(NVENCEncodeConfig)]
    HAS_ZEROCOPY_INIT = True
except AttributeError:
    HAS_ZEROCOPY_INIT = False

nvenc_dll.encode_nvenc_frame_d3d11.restype = ctypes.c_int
nvenc_dll.encode_nvenc_frame_d3d11.argtypes = [ctypes.c_void_p, ctypes.c_void_p, ctypes.c_longlong, ctypes.c_int]

# 零拷贝编码函数 (仅在需要时加载)
try:
    nvenc_dll.encode_nvenc_frame_d3d11_zerocopy.restype = ctypes.c_int
    nvenc_dll.encode_nvenc_frame_d3d11_zerocopy.argtypes = [ctypes.c_void_p, ctypes.c_void_p, ctypes.c_longlong, ctypes.c_int]
    HAS_ZEROCOPY = True
except AttributeError:
    HAS_ZEROCOPY = False
    print("    ⚠ 零拷贝编码函数不可用")

nvenc_dll.get_nvenc_encoded_frame.restype = ctypes.c_int
nvenc_dll.get_nvenc_encoded_frame.argtypes = [ctypes.c_void_p, ctypes.POINTER(NVENCEncodedFrame)]

try:
    nvenc_dll.get_nvenc_zerocopy_stats.restype = ctypes.c_int
    nvenc_dll.get_nvenc_zerocopy_stats.argtypes = [ctypes.c_void_p, ctypes.POINTER(NVENCZeroCopyStats)]
    HAS_ZEROCOPY_STATS = True
except AttributeError:
    HAS_ZEROCOPY_STATS = False

nvenc_dll.free_nvenc_encoder.restype = None
nvenc_dll.free_nvenc_encoder.argtypes = [ctypes.c_void_p]

# ============================================================================
# GPU Direct 测试
# ============================================================================

def wait_for_next_frame(wgc_session, last_frame_id, last_timestamp, timeout_sec=0.05, poll_interval_sec=0.001):
    """等待下一帧（frame_id/timestamp 任一变化即视为新帧）"""
    deadline = time.perf_counter() + timeout_sec
    frame = WgcFrame()

    while time.perf_counter() < deadline:
        if wgc_dll.wgc_get_frame(wgc_session, frame):
            if frame.frame_id != last_frame_id or frame.timestamp != last_timestamp:
                return True, frame
        time.sleep(poll_interval_sec)

    return False, frame


def test_gpu_direct_monitor():
    """测试完整 GPU Direct 流程: WGC → D3D11 → CUDA → NVENC"""

    print("=" * 60)
    print("GPU Direct 完整流程测试")
    print("=" * 60)
    print()

    # 1. 创建 WGC 捕获会话 (主显示器)
    print("[1/5] 创建 WGC 捕获会话...")
    monitor_index = 0
    wgc_session = wgc_dll.wgc_create_session(WGC_TYPE_MONITOR, ctypes.c_void_p(monitor_index))
    if not wgc_session:
        print("    ✗ WGC 会话创建失败")
        return False
    print("    ✓ WGC 会话创建成功")

    # 2. 启动捕获
    print("[2/5] 启动捕获...")
    if not wgc_dll.wgc_start(wgc_session):
        print("    ✗ 启动捕获失败")
        wgc_dll.wgc_free_session(wgc_session)
        return False
    print("    ✓ 捕获已启动")

    # 3. 获取 D3D11 设备和上下文
    print("[3/5] 获取 D3D11 设备...")
    d3d11_device = wgc_dll.wgc_get_d3d11_device(wgc_session)
    d3d11_context = wgc_dll.wgc_get_d3d11_context(wgc_session)
    if not d3d11_device or not d3d11_context:
        print("    ✗ 获取 D3D11 设备失败")
        wgc_dll.wgc_free_session(wgc_session)
        return False
    print(f"    ✓ D3D11 Device: 0x{d3d11_device:X}")
    print(f"    ✓ D3D11 Context: 0x{d3d11_context:X}")

    # 4. 获取一帧来获取分辨率
    frame = WgcFrame()
    retry_count = 0
    while not wgc_dll.wgc_get_frame(wgc_session, frame) and retry_count < 10:
        time.sleep(0.01)
        retry_count += 1

    if retry_count >= 10:
        print("    ✗ 获取初始帧失败")
        wgc_dll.wgc_free_session(wgc_session)
        return False

    width, height = frame.width, frame.height
    print(f"    ✓ 分辨率: {width}x{height}")

    # 5. 检查 NVENC 支持
    print("[4/5] 检查 NVENC 支持...")
    if not nvenc_dll.is_nvenc_supported():
        print("    ✗ NVENC 不支持")
        wgc_dll.wgc_free_session(wgc_session)
        return False
    print("    ✓ NVENC 支持")

    if not nvenc_dll.is_cuda_d3d11_interop_supported():
        print("    ✗ CUDA-D3D11 互操作不支持")
        wgc_dll.wgc_free_session(wgc_session)
        return False
    print("    ✓ CUDA-D3D11 互操作支持")

    # 6. 创建 NVENC 编码器
    print("[5/6] 创建 NVENC 编码器...")
    config = NVENCEncodeConfig(
        width=width,
        height=height,
        framerate=60,
        bitrate=8000000,
        gop_size=60,
        preset=2,      # Medium
        rc_mode=3,     # CQ
        quality=20,    # 高质量
    )

    nvenc_encoder = nvenc_dll.init_nvenc_encoder_d3d11(d3d11_device, d3d11_context, config)
    if not nvenc_encoder:
        print("    ✗ NVENC 编码器创建失败")
        wgc_dll.wgc_free_session(wgc_session)
        return False
    print("    ✓ NVENC 编码器创建成功")

    # 7. 测试 GPU Direct 编码循环
    print()
    print("[6/6] GPU Direct 编码测试 (50 帧)...")
    print("    流程: WGC → D3D11 Texture → CUDA → NV12 → NVENC")
    print("          (完全在 GPU 上，零 CPU 复制)")
    print()

    encoded_frames = 0
    total_encode_time = 0
    min_time = float('inf')
    max_time = 0

    # 强制关键帧
    force_keyframe = 1

    print("    帧数 | 编码时间  | 累计帧数 | 状态")
    print("    " + "-" * 45)

    for i in range(50):
        # 获取帧 (D3D11 Texture)
        frame = WgcFrame()
        if not wgc_dll.wgc_get_frame(wgc_session, frame):
            time.sleep(0.001)  # 1ms 等待
            continue

        # GPU Direct 编码: 直接从 D3D11 纹理编码
        start_time = time.perf_counter()

        result = nvenc_dll.encode_nvenc_frame_d3d11(
            nvenc_encoder,
            frame.d3d11_texture,
            frame.timestamp,
            force_keyframe
        )

        encode_time = (time.perf_counter() - start_time) * 1000  # ms

        if result:
            encoded_frames += 1
            total_encode_time += encode_time
            min_time = min(min_time, encode_time)
            max_time = max(max_time, encode_time)

            force_keyframe = 0  # 后续帧不强制关键帧

            if encoded_frames <= 5 or encoded_frames % 10 == 0:
                status = "✓" if encode_time < 5 else "⚠"
                print(f"    {i+1:4d} | {encode_time:6.3f}ms | {encoded_frames:8d} | {status}")

    # 获取编码输出帧
    output_frames = 0
    total_output_size = 0
    for _ in range(100):
        encoded_frame = NVENCEncodedFrame()
        if nvenc_dll.get_nvenc_encoded_frame(nvenc_encoder, encoded_frame):
            if encoded_frame.size > 0:
                output_frames += 1
                total_output_size += encoded_frame.size

    print()
    print("    " + "=" * 45)
    print(f"    ✓ 编码完成!")
    print()
    print("    统计结果:")
    print(f"    • 成功编码: {encoded_frames} / 50 帧")
    print(f"    • 输出帧数: {output_frames}")
    if output_frames > 0:
        print(f"    • 平均帧大小: {total_output_size // output_frames:,} 字节")
        print(f"    • 平均编码时间: {total_encode_time/encoded_frames:.3f} ms")
    print(f"    • 最小编码时间: {min_time:.3f} ms")
    print(f"    • 最大编码时间: {max_time:.3f} ms")
    if total_encode_time > 0:
        print(f"    • 理论最大 FPS: {1000/(total_encode_time/encoded_frames*1000):.1f}")

    # 清理
    nvenc_dll.free_nvenc_encoder(nvenc_encoder)
    wgc_dll.wgc_free_session(wgc_session)

    return encoded_frames > 0

def test_zero_copy_monitor():
    """测试零拷贝编码: WGC → D3D11 → NVENC (无 CPU 复制)"""

    print("=" * 60)
    print("零拷贝编码测试 (WGC → D3D11 → NVENC)")
    print("=" * 60)
    print()

    # 1. 创建 WGC 捕获会话
    print("[1/5] 创建 WGC 捕获会话...")
    wgc_session = wgc_dll.wgc_create_session(WGC_TYPE_MONITOR, ctypes.c_void_p(0))
    if not wgc_session:
        print("    ✗ WGC 会话创建失败")
        return False
    print("    ✓ WGC 会话创建成功")

    # 2. 启动捕获
    print("[2/5] 启动捕获...")
    if not wgc_dll.wgc_start(wgc_session):
        print("    ✗ 启动捕获失败")
        wgc_dll.wgc_free_session(wgc_session)
        return False
    print("    ✓ 捕获已启动")

    # 3. 获取 D3D11 设备和上下文
    print("[3/5] 获取 D3D11 设备...")
    d3d11_device = wgc_dll.wgc_get_d3d11_device(wgc_session)
    d3d11_context = wgc_dll.wgc_get_d3d11_context(wgc_session)
    if not d3d11_device or not d3d11_context:
        print("    ✗ 获取 D3D11 设备失败")
        wgc_dll.wgc_free_session(wgc_session)
        return False
    print(f"    ✓ D3D11 Device: 0x{d3d11_device:X}")

    # 4. 获取初始帧获取分辨率
    frame = WgcFrame()
    retry_count = 0
    while not wgc_dll.wgc_get_frame(wgc_session, frame) and retry_count < 10:
        time.sleep(0.01)
        retry_count += 1

    if retry_count >= 10:
        print("    ✗ 获取初始帧失败")
        wgc_dll.wgc_free_session(wgc_session)
        return False

    width, height = frame.width, frame.height
    print(f"    ✓ 分辨率: {width}x{height}")

    # 5. 创建 NVENC 编码器
    print("[4/5] 创建 NVENC 编码器...")
    config = NVENCEncodeConfig(
        width=width,
        height=height,
        framerate=60,
        bitrate=8000000,
        gop_size=60,
        preset=2,
        rc_mode=3,
        quality=20,
    )

    if HAS_ZEROCOPY_INIT:
        nvenc_encoder = nvenc_dll.init_nvenc_encoder_d3d11_zerocopy(d3d11_device, d3d11_context, config)
    else:
        nvenc_encoder = nvenc_dll.init_nvenc_encoder_d3d11(d3d11_device, d3d11_context, config)
    if not nvenc_encoder:
        print("    ✗ NVENC 编码器创建失败")
        wgc_dll.wgc_free_session(wgc_session)
        return False
    print("    ✓ NVENC 编码器创建成功")

    # 6. 零拷贝编码测试
    print()
    print("[5/5] 零拷贝编码测试 (50 帧)...")
    print("    流程: WGC → D3D11 Texture → NVENC MapInputResource → NVENC")
    print("          (真正零拷贝，无 CPU 参与)")
    print()

    encoded_frames = 0
    total_encode_time = 0
    min_time = float('inf')
    max_time = 0
    force_keyframe = 1

    print("    帧数 | 编码时间  | 累计帧数 | 状态")
    print("    " + "-" * 45)

    target_frames = 50
    processed_frames = 0
    frame_timeouts = 0
    max_timeouts = 200
    last_frame_id = frame.frame_id
    last_timestamp = frame.timestamp

    while processed_frames < target_frames and frame_timeouts < max_timeouts:
        ok, frame = wait_for_next_frame(
            wgc_session,
            last_frame_id,
            last_timestamp,
            timeout_sec=0.05,
            poll_interval_sec=0.001,
        )
        if not ok:
            frame_timeouts += 1
            continue

        last_frame_id = frame.frame_id
        last_timestamp = frame.timestamp
        processed_frames += 1

        start_time = time.perf_counter()

        # 使用零拷贝编码
        result = nvenc_dll.encode_nvenc_frame_d3d11_zerocopy(
            nvenc_encoder,
            frame.d3d11_texture,
            frame.timestamp,
            force_keyframe
        )

        encode_time = (time.perf_counter() - start_time) * 1000

        if result:
            encoded_frames += 1
            total_encode_time += encode_time
            min_time = min(min_time, encode_time)
            max_time = max(max_time, encode_time)
            force_keyframe = 0

            if encoded_frames <= 5 or encoded_frames % 10 == 0:
                status = "✓" if encode_time < 5 else "⚠"
                print(f"    {processed_frames:4d} | {encode_time:6.3f}ms | {encoded_frames:8d} | {status}")

    # 获取编码输出
    output_frames = 0
    total_output_size = 0
    for _ in range(100):
        encoded_frame = NVENCEncodedFrame()
        if nvenc_dll.get_nvenc_encoded_frame(nvenc_encoder, encoded_frame):
            if encoded_frame.size > 0:
                output_frames += 1
                total_output_size += encoded_frame.size

    print()
    print("    " + "=" * 45)
    print(f"    ✓ 编码完成!")
    print()
    print("    统计结果:")
    print(f"    • 成功编码: {encoded_frames} / {processed_frames} 帧")
    if frame_timeouts > 0:
        print(f"    • 等待新帧超时: {frame_timeouts}")
    print(f"    • 输出帧数: {output_frames}")
    if output_frames > 0:
        print(f"    • 平均帧大小: {total_output_size // output_frames:,} 字节")
    if encoded_frames > 0:
        print(f"    • 平均编码时间: {total_encode_time/encoded_frames:.3f} ms")
        print(f"    • 最小编码时间: {min_time:.3f} ms")
        print(f"    • 最大编码时间: {max_time:.3f} ms")
        print(f"    • 理论最大 FPS: {1000/(total_encode_time/encoded_frames):.1f}")
    if HAS_ZEROCOPY_STATS:
        zstats = NVENCZeroCopyStats()
        if nvenc_dll.get_nvenc_zerocopy_stats(nvenc_encoder, zstats):
            print("    • Zerocopy统计:")
            print(f"      - 提交调用: {zstats.encode_calls}")
            print(f"      - 提交成功: {zstats.encode_submit_success}")
            print(f"      - 需要更多输入: {zstats.encode_submit_need_more_input}")
            print(f"      - 提交失败: {zstats.encode_submit_fail}")
            print(f"      - 槽位忙跳过: {zstats.slot_busy_skips}")
            print(f"      - Map失败: {zstats.map_failures}")
            print(f"      - Lock busy: {zstats.lock_busy_count}")
            print(f"      - Lock可重试: {zstats.lock_retryable_count}")
            print(f"      - Lock失败: {zstats.lock_failures}")
            print(f"      - 产出位流: {zstats.bitstream_outputs}")
            print(f"      - Unmap次数: {zstats.unmap_count}")
            print(f"      - pending峰值/当前: {zstats.pending_peak}/{zstats.pending_current}")

    # 清理
    nvenc_dll.free_nvenc_encoder(nvenc_encoder)
    wgc_dll.wgc_free_session(wgc_session)

    return encoded_frames > 0

if __name__ == "__main__":
    import sys
    test_mode = sys.argv[1] if len(sys.argv) > 1 else "normal"

    try:
        if test_mode == "zerocopy":
            success = test_zero_copy_monitor()
        else:
            success = test_gpu_direct_monitor()

        print()
        if success:
            print("✓ 测试成功!")
        else:
            print("✗ 测试失败")
    except Exception as e:
        print(f"✗ 测试出错: {e}")
        import traceback
        traceback.print_exc()

