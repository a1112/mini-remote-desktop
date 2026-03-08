"""
窗口捕获 + GPU Direct 编码测试
"""

import ctypes
import ctypes.wintypes as wintypes
import time

# 加载 DLL
wgc_dll = ctypes.CDLL(r"J:\ProjectTest\远程探查\mini-remote-desktop\agent-python\cpp_capture\wgc_capture.dll")
nvenc_dll = ctypes.CDLL(r"J:\ProjectTest\远程探查\mini-remote-desktop\agent-python\cpp_capture\nvenc_full.dll")

# WGC 常量和结构
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

class WgcWindowInfo(ctypes.Structure):
    _fields_ = [
        ("hwnd", ctypes.c_void_p),
        ("title", ctypes.c_wchar * 256),
        ("is_visible", ctypes.c_int),
        ("rect", wintypes.RECT),
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
wgc_dll.wgc_enum_windows.restype = ctypes.c_int
wgc_dll.wgc_enum_windows.argtypes = [ctypes.POINTER(WgcWindowInfo), ctypes.c_int]

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

nvenc_dll.encode_nvenc_frame_d3d11.restype = ctypes.c_int
nvenc_dll.encode_nvenc_frame_d3d11.argtypes = [ctypes.c_void_p, ctypes.c_void_p, ctypes.c_longlong, ctypes.c_int]

nvenc_dll.get_nvenc_encoded_frame.restype = ctypes.c_int
nvenc_dll.get_nvenc_encoded_frame.argtypes = [ctypes.c_void_p, ctypes.POINTER(NVENCEncodedFrame)]

nvenc_dll.free_nvenc_encoder.restype = None
nvenc_dll.free_nvenc_encoder.argtypes = [ctypes.c_void_p]

def test_window_capture(hwnd_hex=None):
    """测试窗口捕获"""

    print("=" * 70)
    print("窗口捕获 + GPU Direct 编码测试")
    print("=" * 70)
    print()

    # 枚举窗口
    print("[1/4] 枚举窗口...")
    max_windows = 100
    windows = (WgcWindowInfo * max_windows)()
    count = wgc_dll.wgc_enum_windows(windows, max_windows)

    print(f"    找到 {count} 个窗口:")
    target_hwnd = None

    for i in range(min(count, 10)):  # 显示前 10 个
        hwnd = windows[i].hwnd
        title = windows[i].title
        rect = windows[i].rect
        visible = "可见" if windows[i].is_visible else "隐藏"
        print(f"    [{i}] HWND=0x{hwnd:X} | {title} | {rect.left},{rect.top}-{rect.right},{rect.bottom} | {visible}")

        # 查找特定窗口
        if hwnd_hex and hwnd == int(hwnd_hex, 16):
            target_hwnd = hwnd
        # 自动选择游戏窗口或可视化窗口
        elif not target_hwnd and visible and rect.right - rect.left > 500:
            target_hwnd = hwnd

    if not target_hwnd:
        if hwnd_hex:
            print(f"    ✗ 未找到 HWND={hwnd_hex} 的窗口")
            return False
        # 使用第一个可见窗口
        for i in range(count):
            if windows[i].is_visible:
                target_hwnd = windows[i].hwnd
                break

    if not target_hwnd:
        print("    ✗ 没有可用的窗口")
        return False

    print(f"    选择窗口: HWND=0x{target_hwnd:X}")
    print()

    # 创建 WGC 会话
    print("[2/4] 创建 WGC 窗口捕获会话...")
    wgc_session = wgc_dll.wgc_create_session(WGC_TYPE_WINDOW, ctypes.c_void_p(target_hwnd))
    if not wgc_session:
        print("    ✗ WGC 会话创建失败")
        return False
    print("    ✓ WGC 会话创建成功")

    # 启动捕获
    if not wgc_dll.wgc_start(wgc_session):
        print("    ✗ 启动捕获失败")
        wgc_dll.wgc_free_session(wgc_session)
        return False
    print("    ✓ 捕获已启动")

    # 获取 D3D11 设备和上下文
    d3d11_device = wgc_dll.wgc_get_d3d11_device(wgc_session)
    d3d11_context = wgc_dll.wgc_get_d3d11_context(wgc_session)

    # 获取初始帧
    frame = WgcFrame()
    for _ in range(20):
        if wgc_dll.wgc_get_frame(wgc_session, frame):
            break
        time.sleep(0.01)

    if not frame.d3d11_texture:
        print("    ✗ 无法获取帧数据")
        wgc_dll.wgc_free_session(wgc_session)
        return False

    print(f"    ✓ 分辨率: {frame.width}x{frame.height}")
    print()

    # 创建 NVENC 编码器
    print("[3/4] 创建 NVENC 编码器...")
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

    nvenc_encoder = nvenc_dll.init_nvenc_encoder_d3d11(d3d11_device, d3d11_context, config)
    if not nvenc_encoder:
        print("    ✗ NVENC 编码器创建失败")
        wgc_dll.wgc_free_session(wgc_session)
        return False
    print("    ✓ NVENC 编码器创建成功")
    print()

    # 编码测试
    print("[4/4] 窗口捕获编码测试 (30 帧)...")
    print("    帧数 | 编码时间  | 累计帧数 | 状态")
    print("    " + "-" * 50)

    encoded_frames = 0
    total_encode_time = 0
    min_time = float('inf')
    max_time = 0
    force_keyframe = 1

    for i in range(50):
        frame = WgcFrame()
        if not wgc_dll.wgc_get_frame(wgc_session, frame):
            time.sleep(0.001)
            continue

        start_time = time.perf_counter()
        result = nvenc_dll.encode_nvenc_frame_d3d11(
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

            if encoded_frames <= 5 or encoded_frames % 5 == 0:
                status = "✓" if encode_time < 10 else "⚠"
                print(f"    {encoded_frames:3d} | {encode_time:7.3f}ms | {total_encode_time/encoded_frames:7.3f}ms | {status}")

        if encoded_frames >= 30:
            break

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
    print("    " + "=" * 50)
    print(f"    ✓ 测试完成!")
    print()
    print("    统计结果:")
    print(f"    • 成功编码: {encoded_frames} 帧")
    print(f"    • 输出帧数: {output_frames}")
    if output_frames > 0:
        print(f"    • 平均帧大小: {total_output_size // output_frames:,} 字节")
    if encoded_frames > 0:
        print(f"    • 平均编码时间: {total_encode_time/encoded_frames:.3f} ms")
        print(f"    • 最小编码时间: {min_time:.3f} ms")
        print(f"    • 最大编码时间: {max_time:.3f} ms")
        print(f"    • 实际 FPS: {1000/(total_encode_time/encoded_frames):.1f}")

    # 清理
    nvenc_dll.free_nvenc_encoder(nvenc_encoder)
    wgc_dll.wgc_free_session(wgc_session)

    return encoded_frames > 0

if __name__ == "__main__":
    import sys
    hwnd = sys.argv[1] if len(sys.argv) > 1 else None

    try:
        success = test_window_capture(hwnd)
        print()
        if success:
            print("✓ 窗口捕获测试成功!")
        else:
            print("✗ 窗口捕获测试失败")
    except Exception as e:
        print(f"✗ 测试出错: {e}")
        import traceback
        traceback.print_exc()
