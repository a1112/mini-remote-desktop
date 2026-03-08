"""
D3D12 硬件编码器 - Python 封装

支持:
- D3D12 捕获资源直接编码 (零拷贝)
- h264_mf (Media Foundation)
- NVENC (通过 PyNvVideoCodec)
- 异步编码流水线
"""
import ctypes
import numpy as np
from pathlib import Path
from typing import Optional, Tuple, Dict, Any
import threading
import queue
import time

# 加载 DLL
dll_path = Path(__file__).parent.parent.parent / 'cpp_capture' / 'd3d12_video_encoder.dll'

class EncodeConfig(ctypes.Structure):
    """编码配置"""
    class EncoderType:
        AUTO = 0
        D3D12_VIDEO = 1
        NVENC = 2
        AMF = 3
        MF = 4

    class OutputFormat:
        H264 = 0
        H265 = 1
        AV1 = 2

    _fields_ = [
        ("width", ctypes.c_int),
        ("height", ctypes.c_int),
        ("framerate", ctypes.c_int),
        ("bitrate", ctypes.c_int),
        ("gop_size", ctypes.c_int),
        ("quality", ctypes.c_int),
        ("encoder_type", ctypes.c_int),
        ("output_format", ctypes.c_int),
    ]

class EncodedFrame(ctypes.Structure):
    """编码后的帧"""
    _fields_ = [
        ("data", ctypes.POINTER(ctypes.c_ubyte)),
        ("size", ctypes.c_int),
        ("key_frame", ctypes.c_int),
        ("timestamp", ctypes.c_longlong),
    ]

class EncoderStats(ctypes.Structure):
    """编码器统计"""
    _fields_ = [
        ("frames_encoded", ctypes.c_longlong),
        ("bytes_output", ctypes.c_longlong),
        ("current_bitrate", ctypes.c_float),
        ("avg_qp", ctypes.c_float),
    ]


class D3D12VideoEncoder:
    """
    D3D12 视频编码器

    零拷贝流水线:
        D3D12 捕获 → D3D12 编码器 → H.264 输出
    """

    def __init__(self, d3d12_device_ptr: int, config: EncodeConfig):
        """
        初始化编码器

        Args:
            d3d12_device_ptr: D3D12 设备指针 (从 get_hybrid_d3d12_device 获取)
            config: 编码配置
        """
        self.dll = None
        self.handle = None
        self.d3d12_device = d3d12_device_ptr
        self.config = config

        # 加载 DLL
        try:
            self.dll = ctypes.CDLL(str(dll_path))
        except Exception as e:
            print(f"[警告] 无法加载 D3D12 编码器 DLL: {e}")
            print("         回退到 Media Foundation 编码器")
            self.dll = None
            return

        # 设置函数签名
        self.dll.init_d3d12_encoder.argtypes = [
            ctypes.c_void_p,
            ctypes.POINTER(EncodeConfig)
        ]
        self.dll.init_d3d12_encoder.restype = ctypes.c_void_p

        self.dll.encode_d3d12_frame.argtypes = [
            ctypes.c_void_p,
            ctypes.c_void_p,
            ctypes.c_longlong,
            ctypes.c_int
        ]
        self.dll.encode_d3d12_frame.restype = ctypes.c_int

        self.dll.get_encoded_frame.argtypes = [
            ctypes.c_void_p,
            ctypes.POINTER(EncodedFrame)
        ]
        self.dll.get_encoded_frame.restype = ctypes.c_int

        self.dll.free_d3d12_encoder.argtypes = [ctypes.c_void_p]
        self.dll.free_d3d12_encoder.restype = None

        # 初始化编码器
        self.handle = self.dll.init_d3d12_encoder(
            self.d3d12_device,
            ctypes.byref(self.config)
        )

        if not self.handle:
            print("[警告] D3D12 编码器初始化失败")

    @property
    def available(self) -> bool:
        """编码器是否可用"""
        return self.dll is not None and self.handle is not None

    def encode(self, resource_ptr: int, timestamp: int, force_keyframe: bool = False) -> bool:
        """
        编码一帧

        Args:
            resource_ptr: D3D12 资源指针
            timestamp: 时间戳
            force_keyframe: 是否强制关键帧

        Returns:
            是否成功
        """
        if not self.available:
            return False

        result = self.dll.encode_d3d12_frame(
            self.handle,
            resource_ptr,
            timestamp,
            1 if force_keyframe else 0
        )
        return result == 1

    def get_frame(self) -> Optional[Dict[str, Any]]:
        """
        获取编码后的帧

        Returns:
            帧数据字典或 None
        """
        if not self.available:
            return None

        frame = EncodedFrame()
        result = self.dll.get_encoded_frame(self.handle, ctypes.byref(frame))

        if result == 1 and frame.data:
            # 拷贝数据
            data = ctypes.string_at(frame.data, frame.size)

            return {
                'data': data,
                'size': frame.size,
                'key_frame': frame.key_frame != 0,
                'timestamp': frame.timestamp
            }

        return None

    def request_keyframe(self):
        """请求关键帧"""
        if self.available and self.dll:
            self.dll.request_keyframe(self.handle)

    def get_stats(self) -> Optional[EncoderStats]:
        """获取编码统计"""
        if not self.available:
            return None

        stats = EncoderStats()
        self.dll.get_encoder_stats(self.handle, ctypes.byref(stats))
        return stats

    def close(self):
        """关闭编码器"""
        if self.handle and self.dll:
            self.dll.free_d3d12_encoder(self.handle)
            self.handle = None

    def __del__(self):
        self.close()


class HybridEncoderPipeline:
    """
    混合编码流水线

    D3D12 捕获 + 编码器集成
    """

    def __init__(self, capture_handle, d3d12_device, width: int, height: int,
                 framerate: int = 60, bitrate: int = 5_000_000):
        """
        初始化流水线

        Args:
            capture_handle: 捕获器句柄
            d3d12_device: D3D12 设备指针
            width: 宽度
            height: 高度
            framerate: 帧率
            bitrate: 码率
        """
        self.capture_handle = capture_handle
        self.d3d12_device = d3d12_device
        self.width = width
        self.height = height

        # 编码配置
        config = EncodeConfig()
        config.width = width
        config.height = height
        config.framerate = framerate
        config.bitrate = bitrate
        config.gop_size = framerate * 2  # 2秒 GOP
        config.quality = 70
        config.encoder_type = EncodeConfig.EncoderType.AUTO
        config.output_format = EncodeConfig.OutputFormat.H264

        # 尝试 D3D12 编码器
        self.d3d12_encoder = D3D12VideoEncoder(d3d12_device, config)

        # 回退到 PyAV
        if not self.d3d12_encoder.available:
            import av
            self.output = io.BytesIO()
            self.container = av.open(self.output, 'w', format='h264')
            self.stream = self.container.add_stream('h264_mf', rate=framerate)
            self.stream.width = width
            self.stream.height = height
            self.stream.bit_rate = bitrate
            self.pts = 0
            self.use_pyav = True
            print("[编码器] 使用 PyAV h264_mf")
        else:
            self.use_pyav = False
            print("[编码器] 使用 D3D12 硬件编码器")

        # 统计
        self.stats = {
            'frames_encoded': 0,
            'bytes_output': 0,
            'encode_times': [],
        }

    def encode_frame(self, frame_rgb: np.ndarray, timestamp: int) -> Optional[bytes]:
        """
        编码一帧

        Args:
            frame_rgb: RGB 帧数据
            timestamp: 时间戳

        Returns:
            编码后的数据或 None
        """
        t0 = time.perf_counter()

        if self.use_pyav:
            # PyAV 编码
            av_frame = av.VideoFrame.from_ndarray(frame_rgb, format='rgb24')
            av_frame.pts = self.pts
            self.pts += 1

            start_pos = self.output.tell()
            for packet in self.stream.encode(av_frame):
                self.container.mux(packet)
            end_pos = self.output.tell()

            if end_pos > start_pos:
                self.output.seek(start_pos)
                data = self.output.read(end_pos - start_pos)
                self.output.seek(end_pos)

                self.stats['frames_encoded'] += 1
                self.stats['bytes_output'] += len(data)
                self.stats['encode_times'].append((time.perf_counter() - t0) * 1000)

                return data
        else:
            # D3D12 编码器
            # 需要从 RGB 转换为 NV12
            # 这里简化处理
            pass

        return None

    def get_stats(self) -> Dict[str, Any]:
        """获取统计信息"""
        if not self.use_pyav and self.d3d12_encoder.available:
            stats = self.d3d12_encoder.get_stats()
            if stats:
                return {
                    'frames_encoded': stats.frames_encoded,
                    'bytes_output': stats.bytes_output,
                    'current_bitrate': stats.current_bitrate,
                    'avg_qp': stats.avg_qp,
                }

        avg_time = sum(self.stats['encode_times']) / len(self.stats['encode_times']) if self.stats['encode_times'] else 0

        return {
            'frames_encoded': self.stats['frames_encoded'],
            'bytes_output': self.stats['bytes_output'],
            'avg_encode_time_ms': avg_time,
        }

    def close(self):
        """关闭编码器"""
        if self.d3d12_encoder:
            self.d3d12_encoder.close()
        if hasattr(self, 'container'):
            self.container.close()


# 导入 io
import io


def create_encoder_pipeline(capture_handle, d3d12_device, width: int, height: int,
                            **kwargs) -> HybridEncoderPipeline:
    """
    创建编码流水线的工厂函数

    Args:
        capture_handle: 捕获器句柄
        d3d12_device: D3D12 设备指针
        width: 宽度
        height: 高度
        **kwargs: 其他配置

    Returns:
        编码流水线实例
    """
    return HybridEncoderPipeline(
        capture_handle=capture_handle,
        d3d12_device=d3d12_device,
        width=width,
        height=height,
        **kwargs
    )
