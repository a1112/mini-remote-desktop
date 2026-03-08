"""
编码器性能对比测试
比较 NVENC、AMF (AMD)、QuickSync (Intel) 三种硬件编码器的性能
"""

import ctypes
import ctypes.wintypes as wintypes
import time
import sys

# ============================================================================
# 加载 WGC Capture DLL
# ============================================================================

wgc_dll = ctypes.CDLL(r"J:\ProjectTest\远程探查\mini-remote-desktop\agent-python\cpp_capture\wgc_capture.dll")

WGC_TYPE_MONITOR = 0

class WgcFrame(ctypes.Structure):
    _fields_ = [
        ("width", ctypes.c_int),
        ("height", ctypes.c_int),
        ("d3d11_texture", ctypes.c_void_p),
        ("timestamp", ctypes.c_longlong),
        ("frame_id", ctypes.c_uint),
    ]

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
# 编码器抽象类
# ============================================================================

class HardwareEncoder:
    """硬件编码器抽象基类"""

    def __init__(self, name, dll_path):
        self.name = name
        try:
            self.dll = ctypes.CDLL(dll_path)
            self.available = True
        except Exception as e:
            print(f"  {name}: 不可用 ({e})")
            self.available = False
            self.dll = None

    def is_supported(self):
        return self.available

    def init(self, d3d11_device, d3d11_context, width, height):
        pass

    def encode(self, texture_ptr, timestamp, force_keyframe=0):
        pass

    def get_frame(self):
        pass

    def release(self):
        pass


# ============================================================================
# NVENC 编码器
# ============================================================================

class NVENCEncoder(HardwareEncoder):
    def __init__(self):
        super().__init__("NVENC", r"J:\ProjectTest\远程探查\mini-remote-desktop\agent-python\cpp_capture\nvenc_full.dll")

        if self.available:
            # 配置函数
            self.dll.init_nvenc_encoder_d3d11.restype = ctypes.c_void_p
            self.dll.init_nvenc_encoder_d3d11.argtypes = [
                ctypes.c_void_p, ctypes.c_void_p, ctypes.c_void_p
            ]

            self.dll.encode_nvenc_frame_d3d11.restype = ctypes.c_int
            self.dll.encode_nvenc_frame_d3d11.argtypes = [
                ctypes.c_void_p, ctypes.c_void_p, ctypes.c_longlong, ctypes.c_int
            ]

            self.dll.get_nvenc_encoded_frame.restype = ctypes.c_int
            self.dll.get_nvenc_encoded_frame.argtypes = [ctypes.c_void_p, ctypes.c_void_p]

            self.dll.free_nvenc_encoder.restype = None
            self.dll.free_nvenc_encoder.argtypes = [ctypes.c_void_p]

            self.encoder = None

    def init(self, d3d11_device, d3d11_context, width, height):
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

        config = NVENCEncodeConfig(
            width=width, height=height,
            framerate=60, bitrate=8000000,
            gop_size=60, preset=2, rc_mode=3, quality=20
        )

        self.encoder = self.dll.init_nvenc_encoder_d3d11(
            d3d11_device, d3d11_context, ctypes.byref(config)
        )
        return self.encoder is not None

    def encode(self, texture_ptr, timestamp, force_keyframe=0):
        if self.encoder:
            return self.dll.encode_nvenc_frame_d3d11(
                self.encoder, texture_ptr, timestamp, force_keyframe
            )
        return 0

    def get_frame(self):
        class NVENCEncodedFrame(ctypes.Structure):
            _fields_ = [
                ("data", ctypes.POINTER(ctypes.c_uint8)),
                ("size", ctypes.c_int),
                ("key_frame", ctypes.c_int),
                ("timestamp", ctypes.c_longlong),
            ]

        frame = NVENCEncodedFrame()
        if self.dll.get_nvenc_encoded_frame(self.encoder, ctypes.byref(frame)):
            return frame
        return None

    def release(self):
        if self.encoder:
            self.dll.free_nvenc_encoder(self.encoder)
            self.encoder = None


# ============================================================================
# 性能测试函数
# ============================================================================

def test_encoder(encoder_class, name):
    """测试单个编码器的性能"""
    print(f"\n{'='*60}")
    print(f"测试 {name}")
    print(f"{'='*60}")

    # 创建 WGC 会话
    wgc_session = wgc_dll.wgc_create_session(WGC_TYPE_MONITOR, ctypes.c_void_p(0))
    if not wgc_session:
        print(f"  ✗ WGC 会话创建失败")
        return None

    if not wgc_dll.wgc_start(wgc_session):
        wgc_dll.wgc_free_session(wgc_session)
        print(f"  ✗ 启动捕获失败")
        return None

    # 获取 D3D11 设备
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
        print(f"  ✗ 无法获取帧数据")
        return None

    # 创建编码器
    encoder = encoder_class()
    if not encoder.is_supported():
        print(f"  ⚠ 编码器不可用")
        wgc_dll.wgc_free_session(wgc_session)
        return None

    if not encoder.init(d3d11_device, d3d11_context, frame.width, frame.height):
        print(f"  ✗ 编码器初始化失败")
        wgc_dll.wgc_free_session(wgc_session)
        return None

    print(f"  ✓ 分辨率: {frame.width}x{frame.height}")

    # 编码测试
    print(f"  帧  | 编码时间  | 累计帧数")
    print(f"  {'-'*40}")

    encoded_frames = 0
    total_time = 0
    min_time = float('inf')
    max_time = 0
    force_keyframe = 1

    for i in range(50):
        frame = WgcFrame()
        if not wgc_dll.wgc_get_frame(wgc_session, frame):
            time.sleep(0.001)
            continue

        start = time.perf_counter()
        result = encoder.encode(frame.d3d11_texture, frame.timestamp, force_keyframe)
        elapsed = (time.perf_counter() - start) * 1000  # ms

        if result:
            encoded_frames += 1
            total_time += elapsed
            min_time = min(min_time, elapsed)
            max_time = max(max_time, elapsed)
            force_keyframe = 0

            if encoded_frames <= 5 or encoded_frames % 10 == 0:
                status = "✓" if elapsed < 10 else "⚠"
                print(f"  {encoded_frames:3d} | {elapsed:7.3f}ms | {encoded_frames:3d}")

        if encoded_frames >= 30:
            break

    # 获取输出帧
    output_frames = 0
    total_size = 0
    for _ in range(50):
        enc_frame = encoder.get_frame()
        if enc_frame and enc_frame.size > 0:
            output_frames += 1
            total_size += enc_frame.size

    # 清理
    encoder.release()
    wgc_dll.wgc_free_session(wgc_session)

    if encoded_frames > 0:
        print(f"\n  统计:")
        print(f"  • 成功编码: {encoded_frames} 帧")
        print(f"  • 输出帧数: {output_frames}")
        if output_frames > 0:
            print(f"  • 平均帧大小: {total_size // output_frames:,} 字节")
        print(f"  • 平均时间: {total_time/encoded_frames:.3f} ms")
        print(f"  • 最小时间: {min_time:.3f} ms")
        print(f"  • 最大时间: {max_time:.3f} ms")
        print(f"  • 理论 FPS: {1000/(total_time/encoded_frames):.1f}")

        return {
            "name": name,
            "frames": encoded_frames,
            "avg_time": total_time / encoded_frames,
            "min_time": min_time,
            "max_time": max_time,
        }

    return None


def main():
    print("=" * 60)
    print("硬件编码器性能对比测试")
    print("=" * 60)
    print("\n测试编码器:")
    print("  1. NVENC (NVIDIA)")
    print("  2. AMF (AMD) - 开发中")
    print("  3. QuickSync (Intel) - 开发中")
    print()

    results = []

    # 测试 NVENC
    nvenc_result = test_encoder(NVENCEncoder, "NVENC (NVIDIA)")
    if nvenc_result:
        results.append(nvenc_result)

    # 对比结果
    if len(results) > 1:
        print(f"\n{'='*60}")
        print("性能对比")
        print(f"{'='*60}")
        print(f"  编码器  | 平均时间  | 理论 FPS")
        print(f"  {'-'*40}")
        for r in results:
            print(f"  {r['name']:8s} | {r['avg_time']:7.3f}ms | {1000/r['avg_time']:5.1f}")

    print(f"\n{'='*60}")
    print("结论:")
    print(f"{'='*60}")

    if results:
        fastest = min(results, key=lambda x: x['avg_time'])
        print(f"  • 最快编码器: {fastest['name']} ({fastest['avg_time']:.3f} ms)")
        print(f"  • 理论最大 FPS: {1000/fastest['avg_time']:.1f}")

        if fastest['avg_time'] < 5:
            print(f"  ✓ 达到高性能目标 (< 5ms)")
        elif fastest['avg_time'] < 10:
            print(f"  ⚠ 性能良好，但未达到最优")
        else:
            print(f"  ✗ 性能需要优化")
    else:
        print("  没有可用的编码器")


if __name__ == "__main__":
    try:
        main()
    except Exception as e:
        print(f"✗ 测试出错: {e}")
        import traceback
        traceback.print_exc()
