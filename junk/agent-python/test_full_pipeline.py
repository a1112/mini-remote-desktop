#!/usr/bin/env python3
"""
完整流水线测试 - DXGI C++ 捕获 + GPU 编码 + RTP + 解码 + 显示

流水线:
  DXGI C++ (188 FPS) → h264_mf 编码 (GPU) → RTP 打包 → RTP 解包 → 解码 → 显示
"""
import sys
import time
import ctypes
import io
import numpy as np
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent / 'src'))

print("="*70)
print("完整流水线性能测试")
print("="*70)

# ============================================================================
# 1. DXGI C++ 捕获 (188 FPS)
# ============================================================================
print("\n[1/5] 加载 DXGI C++ 捕获器...")

dll_path = Path(__file__).parent / 'dxgi_capture.dll'
dxgi_dll = ctypes.CDLL(str(dll_path))

dxgi_dll.init_capture.argtypes = [ctypes.c_int]
dxgi_dll.init_capture.restype = ctypes.c_void_p

class FrameInfo(ctypes.Structure):
    _fields_ = [
        ("width", ctypes.c_int),
        ("height", ctypes.c_int),
        ("stride", ctypes.c_int),
        ("format", ctypes.c_ulong),
        ("timestamp", ctypes.c_ulonglong),
    ]

dxgi_dll.capture_frame.argtypes = [
    ctypes.c_void_p,
    ctypes.POINTER(ctypes.c_ubyte),
    ctypes.c_int,
    ctypes.POINTER(FrameInfo)
]
dxgi_dll.capture_frame.restype = ctypes.c_int

dxgi_dll.free_capture.argtypes = [ctypes.c_void_p]
dxgi_dll.free_capture.restype = None

dxgi_handle = dxgi_dll.init_capture(0)
if not dxgi_handle:
    print("  ❌ DXGI 初始化失败")
    sys.exit(1)

# 获取尺寸
buffer = (ctypes.c_ubyte * (2560 * 1440 * 4))()
info = FrameInfo()
dxgi_dll.capture_frame(dxgi_handle, buffer, 2560 * 1440 * 4, ctypes.byref(info))
width, height = info.width, info.height
print(f"  ✅ DXGI C++: {width}x{height}")

# ============================================================================
# 2. 硬件编码器 (h264_mf)
# ============================================================================
print("\n[2/5] 初始化硬件编码器...")

try:
    import av

    encode_output = io.BytesIO()
    encode_container = av.open(encode_output, 'w', format='h264')
    encode_stream = encode_container.add_stream('h264_mf', rate=60)
    encode_stream.width = width
    encode_stream.height = height
    encode_stream.bit_rate = 5_000_000
    encode_pts = 0

    print(f"  ✅ h264_mf 硬件编码器")
except ImportError:
    print("  ❌ PyAV 未安装")
    dxgi_dll.free_capture(dxgi_handle)
    sys.exit(1)

# ============================================================================
# 3. RTP 打包器
# ============================================================================
print("\n[3/5] 初始化 RTP 打包器...")

from webrtc.rtp import H264RTPPacketizer
packetizer = H264RTPPacketizer()
print(f"  ✅ RTP 打包器")

# ============================================================================
# 4. RTP 解包器
# ============================================================================
print("\n[4/5] 初始化 RTP 解包器...")

from webrtc.rtp import H264RTPDepacketizer
depacketizer = H264RTPDepacketizer()
print(f"  ✅ RTP 解包器")

# ============================================================================
# 5. 解码器
# ============================================================================
print("\n[5/5] 初始化解码器...")

import asyncio
from decoder.pyav_decoder import PyAVDecoder

async def init_decoder():
    decoder = PyAVDecoder()
    await decoder.initialize(width, height)
    return decoder

# 简单事件循环
loop = asyncio.new_event_loop()
asyncio.set_event_loop(loop)
decoder = loop.run_until_complete(init_decoder())
print(f"  ✅ 解码器")

# ============================================================================
# 流水线性能测试
# ============================================================================
print("\n" + "="*70)
print("流水线性能测试 (10秒)")
print("="*70)
print("\n测试中...")

import io
import cv2

cv2.namedWindow("Full Pipeline", cv2.WINDOW_NORMAL)

# 统计
capture_times = []
encode_times = []
decode_times = []
total_frames = 0
encoded_frames = 0
decoded_frames = 0
start_time = time.time()
last_stats = start_time

# 预创建解码函数
decode_async = decoder.decode
def decode_sync(data):
    return loop.run_until_complete(decode_async(data, time.time()))

try:
    while time.time() - start_time < 10:
        loop_start = time.perf_counter()

        # 1. 捕获 (DXGI C++)
        t0 = time.perf_counter()
        result = dxgi_dll.capture_frame(dxgi_handle, buffer, width * height * 4, ctypes.byref(info))
        t1 = time.perf_counter()

        if result != 1:
            continue  # 没有新帧

        # 转换为 numpy
        frame_bgra = np.ctypeslib.as_array(buffer)
        frame_bgra = frame_bgra.reshape((height, width, 4))
        frame_bgra = frame_bgra.copy()  # 复制数据
        frame_rgb = frame_bgra[:, :, :3][:, :, [2, 1, 0]]  # BGRA → RGB

        capture_times.append((t1 - t0) * 1000)

        # 2. 编码 (h264_mf GPU)
        av_frame = av.VideoFrame.from_ndarray(frame_rgb, format='rgb24')
        av_frame.pts = encode_pts
        encode_pts += 1

        start_pos = encode_output.tell()
        for packet in encode_stream.encode(av_frame):
            encode_container.mux(packet)
        end_pos = encode_output.tell()

        if end_pos > start_pos:
            encode_output.seek(start_pos)
            encoded_data = encode_output.read(end_pos - start_pos)
            encode_output.seek(end_pos)
            encoded_frames += 1

            # 定期重置缓冲区
            if end_pos > 1024 * 1024:
                encode_output = io.BytesIO()
                encode_container = av.open(encode_output, 'w', format='h264')
                encode_stream = encode_container.add_stream('h264_mf', rate=60)
                encode_stream.width = width
                encode_stream.height = height
                encode_stream.bit_rate = 5_000_000
                encode_pts = 0

            t2 = time.perf_counter()
            encode_times.append((t2 - t1) * 1000)

            # 3. RTP 打包
            rtp_packets = packetizer.packetize(encoded_data, int(total_frames * 1000 / 60), False)

            # 4. RTP 解包
            reassembled = b''
            for pkt in rtp_packets:
                data = depacketizer.depacketize(pkt)
                if data:
                    reassembled += data

            # 5. 解码
            if reassembled:
                decoded = decode_sync(reassembled)
                if decoded:
                    t3 = time.perf_counter()
                    decode_times.append((t3 - t2) * 1000)
                    decoded_frames += 1

                    # 显示
                    display_frame = decoded.data
                    if display_frame.shape[:2] != (height, width):
                        display_frame = cv2.resize(display_frame, (width, height))

                    # 绘制信息
                    overlay = display_frame.copy()
                    cv2.rectangle(overlay, (5, 5), (450, 200), (0, 0, 0), -1)
                    display_frame = cv2.addWeighted(overlay, 0.7, display_frame, 0.3, 0)

                    now = time.time()
                    capture_fps = total_frames / (now - start_time + 0.001)
                    pipeline_fps = decoded_frames / (now - start_time + 0.001)

                    # FPS 颜色
                    fps_color = (0, 200, 0) if pipeline_fps >= 60 else (0, 200, 200)

                    y = 35
                    cv2.putText(display_frame, f"Pipeline FPS: {pipeline_fps:.1f}",
                               (15, y), cv2.FONT_HERSHEY_SIMPLEX, 0.7, fps_color, 2)
                    y += 30
                    cv2.putText(display_frame, f"捕获: {capture_fps:.1f} FPS",
                               (15, y), cv2.FONT_HERSHEY_SIMPLEX, 0.5, (200, 255, 200), 1)
                    y += 25
                    cv2.putText(display_frame, f"编码: {encoded_frames} 帧",
                               (15, y), cv2.FONT_HERSHEY_SIMPLEX, 0.45, (255, 255, 255), 1)
                    y += 25
                    cv2.putText(display_frame, f"解码: {decoded_frames} 帧",
                               (15, y), cv2.FONT_HERSHEY_SIMPLEX, 0.45, (255, 255, 255), 1)
                    y += 25

                    if capture_times:
                        avg_capture = sum(capture_times[-60:]) / min(len(capture_times), 60)
                        cv2.putText(display_frame, f"捕获延迟: {avg_capture:.1f} ms",
                                   (15, y), cv2.FONT_HERSHEY_SIMPLEX, 0.45, (0, 255, 0), 1)
                    y += 25
                    if encode_times:
                        avg_encode = sum(encode_times[-60:]) / min(len(encode_times), 60)
                        cv2.putText(display_frame, f"编码延迟: {avg_encode:.1f} ms",
                                   (15, y), cv2.FONT_HERSHEY_SIMPLEX, 0.45, (0, 255, 0), 1)
                    y += 25
                    if decode_times:
                        avg_decode = sum(decode_times[-60:]) / min(len(decode_times), 60)
                        cv2.putText(display_frame, f"解码延迟: {avg_decode:.1f} ms",
                                   (15, y), cv2.FONT_HERSHEY_SIMPLEX, 0.45, (200, 200, 200), 1)

                    cv2.putText(display_frame, "🚀 DXGI + GPU",
                               (15, 195), cv2.FONT_HERSHEY_SIMPLEX, 0.6, (0, 255, 0), 2)

                    cv2.imshow("Full Pipeline", display_frame)

        total_frames += 1

        # 退出检查
        key = cv2.waitKey(1) & 0xFF
        if key == 27 or key == ord('q'):
            break

        # 帧率控制
        elapsed = time.perf_counter() - loop_start
        target = 1.0 / 60
        if elapsed < target:
            time.sleep(target - elapsed)

finally:
    cv2.destroyAllWindows()
    dxgi_dll.free_capture(dxgi_handle)

# ============================================================================
# 最终统计
# ============================================================================
total_time = time.time() - start_time

print("\n" + "="*70)
print("流水线统计")
print("="*70)
print(f"测试时长: {total_time:.1f}s")
print(f"捕获帧数: {total_frames}")
print(f"编码帧数: {encoded_frames}")
print(f"解码帧数: {decoded_frames}")
print(f"\n性能指标:")
print(f"  捕获 FPS: {total_frames / total_time:.1f}")
print(f"  端到端 FPS: {decoded_frames / total_time:.1f}")

if capture_times:
    print(f"  平均捕获延迟: {sum(capture_times)/len(capture_times):.2f} ms")
    print(f"  理论捕获 FPS: {1000/(sum(capture_times)/len(capture_times)):.1f}")

if encode_times:
    print(f"  平均编码延迟: {sum(encode_times)/len(encode_times):.2f} ms")
    print(f"  理论编码 FPS: {1000/(sum(encode_times)/len(encode_times)):.1f}")

if decode_times:
    print(f"  平均解码延迟: {sum(decode_times)/len(decode_times):.2f} ms")
    print(f"  理论解码 FPS: {1000/(sum(decode_times)/len(decode_times)):.1f}")

# 评级
pipeline_fps = decoded_frames / total_time
if pipeline_fps >= 50:
    rating = "⭐⭐⭐ 优秀"
elif pipeline_fps >= 30:
    rating = "⭐⭐ 良好"
else:
    rating = "⭐ 一般"

print(f"\n评级: {rating}")
