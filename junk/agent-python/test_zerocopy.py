"""
测试 NVENC 零拷贝编码性能
对比 encode_nvenc_frame_d3d11 和 encode_nvenc_frame_d3d11_zerocopy
"""

import ctypes
import os
import time
import pytest

# 加载 DLL
wgc_dll = ctypes.CDLL(r"J:\ProjectTest\远程探查\mini-remote-desktop\agent-python\cpp_capture\wgc_capture.dll")
nvenc_dll = ctypes.CDLL(r"J:\ProjectTest\远程探查\mini-remote-desktop\agent-python\cpp_capture\nvenc_full.dll")

# WGC 常量
WGC_TYPE_MONITOR = 0

class WgcFrame(ctypes.Structure):
    _fields_ = [
        ("width", ctypes.c_int),
        ("height", ctypes.c_int),
        ("d3d11_texture", ctypes.c_void_p),
        ("timestamp", ctypes.c_longlong),
        ("frame_id", ctypes.c_uint),
    ]

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
        ("data", ctypes.POINTER(ctypes.c_uint8)),
        ("size", ctypes.c_int),
        ("key_frame", ctypes.c_int),
        ("timestamp", ctypes.c_longlong),
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

# NVENC 函数
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

nvenc_dll.get_nvenc_encoded_frame.restype = ctypes.c_int
nvenc_dll.get_nvenc_encoded_frame.argtypes = [ctypes.c_void_p, ctypes.POINTER(NVENCEncodedFrame)]

nvenc_dll.free_nvenc_encoder.restype = None
nvenc_dll.free_nvenc_encoder.argtypes = [ctypes.c_void_p]

# 检查零拷贝函数
try:
    nvenc_dll.encode_nvenc_frame_d3d11_zerocopy.restype = ctypes.c_int
    nvenc_dll.encode_nvenc_frame_d3d11_zerocopy.argtypes = [ctypes.c_void_p, ctypes.c_void_p, ctypes.c_longlong, ctypes.c_int]
    HAS_ZEROCOPY = True
    print("✓ 零拷贝函数可用")
except AttributeError:
    HAS_ZEROCOPY = False
    print("✗ 零拷贝函数不可用")

def run_encode_mode(use_zerocopy):
    """测试编码性能"""
    mode_name = "Zero-Copy" if use_zerocopy else "Normal"

    print(f"\n{'='*60}")
    print(f"测试 {mode_name} 模式")
    print(f"{'='*60}")

    # 创建 WGC 会话
    wgc_session = wgc_dll.wgc_create_session(WGC_TYPE_MONITOR, ctypes.c_void_p(0))
    if not wgc_session:
        print(f"✗ WGC 会话创建失败")
        return None

    if not wgc_dll.wgc_start(wgc_session):
        wgc_dll.wgc_free_session(wgc_session)
        print(f"✗ 启动捕获失败")
        return None

    d3d11_device = wgc_dll.wgc_get_d3d11_device(wgc_session)
    d3d11_context = wgc_dll.wgc_get_d3d11_context(wgc_session)

    # 获取初始帧
    frame = WgcFrame()
    for _ in range(20):
        if wgc_dll.wgc_get_frame(wgc_session, frame):
            break
        time.sleep(0.01)

    if not frame.d3d11_texture:
        wgc_dll.wgc_free_session(wgc_session)
        print(f"✗ 无法获取帧数据")
        return None

    # 创建 NVENC 编码器
    config = NVENCEncodeConfig(
        width=frame.width,
        height=frame.height,
        framerate=60,
        bitrate=8000000,
        gop_size=60,
        preset=2,
        rc_mode=3,
        quality=20,
    )

    init_func = nvenc_dll.init_nvenc_encoder_d3d11
    if use_zerocopy and HAS_ZEROCOPY_INIT:
        init_func = nvenc_dll.init_nvenc_encoder_d3d11_zerocopy
    nvenc_encoder = init_func(d3d11_device, d3d11_context, config)
    if not nvenc_encoder:
        wgc_dll.wgc_free_session(wgc_session)
        print(f"✗ NVENC 编码器创建失败")
        return None

    # 编码测试
    print(f"  帧  | 编码时间  | 状态")
    print(f"  {'-'*35}")

    encoded_frames = 0
    total_time = 0
    min_time = float('inf')
    max_time = 0
    force_keyframe = 1

    encode_func = nvenc_dll.encode_nvenc_frame_d3d11_zerocopy if use_zerocopy else nvenc_dll.encode_nvenc_frame_d3d11

    for i in range(50):
        frame = WgcFrame()
        if not wgc_dll.wgc_get_frame(wgc_session, frame):
            time.sleep(0.001)
            continue

        start = time.perf_counter()
        result = encode_func(nvenc_encoder, frame.d3d11_texture, frame.timestamp, force_keyframe)
        elapsed = (time.perf_counter() - start) * 1000  # ms

        if result:
            encoded_frames += 1
            total_time += elapsed
            min_time = min(min_time, elapsed)
            max_time = max(max_time, elapsed)
            force_keyframe = 0

            if encoded_frames <= 5 or encoded_frames % 10 == 0:
                status = "✓" if elapsed < 5 else "⚠" if elapsed < 15 else "✗"
                print(f"  {encoded_frames:3d} | {elapsed:7.3f}ms | {status}")

        if encoded_frames >= 30:
            break

    # 获取输出
    output_frames = 0
    for _ in range(100):
        enc_frame = NVENCEncodedFrame()
        if nvenc_dll.get_nvenc_encoded_frame(nvenc_encoder, enc_frame):
            if enc_frame.size > 0:
                output_frames += 1

    # 清理
    nvenc_dll.free_nvenc_encoder(nvenc_encoder)
    wgc_dll.wgc_free_session(wgc_session)

    if encoded_frames > 0:
        print(f"\n  统计:")
        print(f"  • 成功编码: {encoded_frames} 帧")
        print(f"  • 输出帧数: {output_frames}")
        print(f"  • 平均时间: {total_time/encoded_frames:.3f} ms")
        print(f"  • 最小时间: {min_time:.3f} ms")
        print(f"  • 最大时间: {max_time:.3f} ms")
        print(f"  • 理论 FPS: {1000/(total_time/encoded_frames):.1f}")

    return {
        "mode": mode_name,
        "frames": encoded_frames,
        "avg_time": total_time/encoded_frames if encoded_frames > 0 else 0,
        "min_time": min_time if encoded_frames > 0 else 0,
        "max_time": max_time if encoded_frames > 0 else 0,
    }


@pytest.mark.parametrize("use_zerocopy", [False, True])
def test_encode_mode(use_zerocopy):
    """Manual GPU benchmark; opt-in via RUN_GPU_BENCH=1."""
    if os.environ.get("RUN_GPU_BENCH") != "1":
        pytest.skip("manual GPU benchmark, set RUN_GPU_BENCH=1 to run")
    if use_zerocopy and not HAS_ZEROCOPY:
        pytest.skip("zerocopy entrypoint not available")
    result = run_encode_mode(use_zerocopy)
    assert result is not None
    assert result["frames"] > 0

if __name__ == "__main__":
    print("=" * 60)
    print("NVENC 编码性能对比测试")
    print("=" * 60)

    # 测试普通模式
    normal_result = run_encode_mode(use_zerocopy=False)

    # 测试零拷贝模式
    zerocopy_result = None
    if HAS_ZEROCOPY:
        time.sleep(1)  # 冷却时间
        zerocopy_result = run_encode_mode(use_zerocopy=True)

    # 对比结果
    print(f"\n{'='*60}")
    print("性能对比")
    print(f"{'='*60}")

    if normal_result and zerocopy_result and zerocopy_result["frames"] > 0:
        improvement = (normal_result["avg_time"] - zerocopy_result["avg_time"]) / normal_result["avg_time"] * 100
        speedup = normal_result["avg_time"] / zerocopy_result["avg_time"] if zerocopy_result["avg_time"] > 0 else 0

        print(f"\n  模式       | 平均时间  | 提升")
        print(f"  {'-'*40}")
        print(f"  普通       | {normal_result['avg_time']:7.3f}ms | -")
        print(f"  零拷贝     | {zerocopy_result['avg_time']:7.3f}ms | {improvement:+.1f}%")

        print(f"\n  加速比: {speedup:.2f}x")

        if zerocopy_result["avg_time"] < 5:
            print(f"  ✓ 零拷贝模式达到高性能目标 (< 5ms)")
        elif zerocopy_result["avg_time"] < normal_result["avg_time"]:
            print(f"  ✓ 零拷贝模式性能更好，但未达到最优")
        else:
            print(f"  ⚠ 零拷贝模式未改善性能 (可能未真正实现零拷贝)")
    elif HAS_ZEROCOPY and zerocopy_result and zerocopy_result["frames"] == 0:
        print("\n  ✗ 零拷贝模式未成功编码任何帧")
        print("  请检查 zerocopy 初始化或输入纹理格式兼容性")
