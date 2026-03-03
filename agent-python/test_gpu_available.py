#!/usr/bin/env python3
"""
检查可用的 GPU 加速选项。
"""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent / 'src'))

def check_gpu_capture():
    """检查 GPU 捕获选项。"""
    print("="*60)
    print("GPU 捕获选项检查")
    print("="*60)

    # 1. d3dshot (DirectX)
    print("\n1. d3dshot (DirectX 捕获):")
    try:
        import d3dshot
        print("   ✅ d3dshot 已安装")
        print("   → 使用 DirectX 11/12 GPU 直接捕获")
        print("   → 比 GDI 快，零拷贝")
    except ImportError:
        print("   ❌ d3dshot 未安装")
        print("   → 安装: pip install d3dshot")
    except Exception as e:
        print(f"   ⚠️  d3dshot 可用但有警告: {e}")

    # 2. DXGI Desktop Duplication (需要 C++ 扩展)
    print("\n2. DXGI Desktop Duplication:")
    print("   ⚠️  需要 C++ Python 扩展")
    print("   → pip install pydirectx  (实验性)")
    print("   → 或使用 ctypes 绑定 (复杂)")

    # 3. D3D11/OpenGL 共享纹理
    print("\n3. D3D11/OpenGL 共享纹理:")
    print("   ⚠️  需要现代图形 API 绑定")
    print("   → pip install moderngl  (OpenGL)")
    print("   → pip install PyDirectX12  (D3D12)")


def check_hardware_encoder():
    """检查硬件编码器选项。"""
    print("\n" + "="*60)
    print("硬件编码器检查")
    print("="*60)

    try:
        import av

        print("\n可用的 H.264 编码器:")
        h264_codecs = []
        for codec in av.codecs_available:
            if '264' in codec.lower():
                try:
                    cc = av.CodecContext.create(codec, 'w')
                    h264_codecs.append(codec)
                    cc.close()
                except:
                    pass

        # 分类显示
        hw_encoders = []
        sw_encoders = []

        for codec in sorted(h264_codecs):
            if 'nvenc' in codec:
                hw_encoders.append(('🚀 NVENC', codec, 'NVIDIA GPU'))
            elif 'qsv' in codec:
                hw_encoders.append(('⚡ Quick Sync', codec, 'Intel GPU'))
            elif 'amf' in codec:
                hw_encoders.append(('🔥 AMF', codec, 'AMD GPU'))
            elif 'mf' in codec:
                hw_encoders.append(('📺 Media Foundation', codec, 'Windows'))
            elif 'x264' in codec:
                sw_encoders.append(('💻 libx264', codec, '软件'))
            else:
                hw_encoders.append(('• ' + codec, codec, '未知'))

        if hw_encoders:
            print("\n  硬件编码器:")
            for name, codec, desc in hw_encoders:
                print(f"    {name:<15} {codec:<20} ({desc})")

        if sw_encoders:
            print("\n  软件编码器:")
            for name, codec, desc in sw_encoders:
                print(f"    {name:<15} {codec:<20} ({desc})")

        return len(hw_encoders) > 0

    except ImportError:
        print("   ❌ PyAV 未安装")
        print("   → 安装: pip install av")
        return False


def test_nvenc_performance():
    """测试 NVENC 性能。"""
    print("\n" + "="*60)
    print("NVENC 性能测试")
    print("="*60)

    try:
        import av
        import numpy as np
        import time

        # 创建测试帧
        test_frame = av.VideoFrame.from_ndarray(
            np.random.randint(0, 255, (1080, 1920, 3), dtype=np.uint8),
            format='rgb24'
        )

        # 测试 libx264
        print("\n1. 软件编码 (libx264):")
        try:
            sw_enc = av.CodecContext.create('libx264', 'w')
            sw_enc.width = 1920
            sw_enc.height = 1080
            sw_enc.framerate = 30
            sw_enc.bit_rate = 5_000_000
            sw_enc.options['preset'] = 'ultrafast'
            sw_enc.options['tune'] = 'zerolatency'
            sw_enc.open()

            times = []
            for _ in range(10):
                t0 = time.perf_counter()
                packets = list(sw_enc.encode(test_frame))
                t1 = time.perf_counter()
                if packets:
                    times.append((t1 - t0) * 1000)

            sw_enc.close()

            if times:
                avg = sum(times) / len(times)
                fps = 1000 / avg
                print(f"   平均编码时间: {avg:.1f} ms")
                print(f"   理论 FPS: {fps:.1f}")

        except Exception as e:
            print(f"   ❌ 失败: {e}")

        # 测试 NVENC
        print("\n2. 硬件编码 (h264_nvenc):")
        try:
            hw_enc = av.CodecContext.create('h264_nvenc', 'w')
            hw_enc.width = 1920
            hw_enc.height = 1080
            hw_enc.framerate = 30
            hw_enc.bit_rate = 5_000_000
            # NVENC 选项
            try:
                hw_enc.options['preset'] = 'fast'
                hw_enc.options['tune'] = 'll'  # low latency
                hw_enc.options['rc'] = 'cbr'
            except:
                pass
            hw_enc.open()

            times = []
            for _ in range(10):
                t0 = time.perf_counter()
                packets = list(hw_enc.encode(test_frame))
                t1 = time.perf_counter()
                if packets:
                    times.append((t1 - t0) * 1000)

            hw_enc.close()

            if times:
                avg = sum(times) / len(times)
                fps = 1000 / avg
                print(f"   平均编码时间: {avg:.1f} ms")
                print(f"   理论 FPS: {fps:.1f}")

                if times[0] < 10:
                    print(f"   ✅ 硬件加速工作正常!")
                else:
                    print(f"   ⚠️  可能使用软件回退")

        except Exception as e:
            print(f"   ❌ 失败: {e}")

    except ImportError:
        print("   ❌ PyAV 未安装")


def show_gpu_optimization_tips():
    """显示 GPU 优化建议。"""
    print("\n" + "="*60)
    print("GPU 优化建议")
    print("="*60)

    print("""
要实现 GPU 加速，需要以下组件:

1. GPU 捕获 (零拷贝)
─────────────────────────────────────────────────────────────────────
方案 A: d3dshot
  • pip install d3dshot
  • 使用 DirectX 11 捕获
  • 速度: 60+ FPS @ 1080p

方案 B: DXGI Desktop Duplication (需要 C++)
  • 零拷贝 GPU 纹理
  • 自动变化检测
  • 速度: 120+ FPS

方案 C: Python ModernGL + OpenGL
  • pip install moderngl PyOpenGL
  • 直接读取 GPU 缓冲
  • 复杂但可行


2. 硬件编码 (GPU)
─────────────────────────────────────────────────────────────────────
NVIDIA (NVENC):
  • 需要 GeForce GTX 600 系列或更高
  • 安装最新 NVIDIA 驱动
  • PyAV: codec = av.CodecContext.create('h264_nvenc', 'w')

Intel (Quick Sync):
  • 需要 Intel CPU (2代及以上)
  • 安装 Intel Media SDK 驱动
  • PyAV: codec = av.CodecContext.create('h264_qsv', 'w')

AMD (AMF):
  • 需要 AMD GPU
  • 安装 AMD 驱动
  • PyAV: codec = av.CodecContext.create('h264_amf', 'w')


3. 零拷贝传输
─────────────────────────────────────────────────────────────────────
# 方案 1: 使用 PyAV 的 GPU 纹理
import av
import ctypes

# 创建 D3D11 上下文并传递给编码器
# 这需要 C++ 扩展或复杂的 ctypes 绑定

# 方案 2: 使用 CUDA 直接访问
import cupy as cp  # 需要 GPU

# 在 GPU 上处理帧，避免 CPU 拷贝
frame_gpu = cp.asarray(frame)
encoded = encode_gpu(frame_gpu)


4. 推荐架构
─────────────────────────────────────────────────────────────────────
捕获线程 (GPU/DirectX) → GPU 纹理 → 硬件编码 → 网络
    ↓
不经过 CPU 内存!
    """)


if __name__ == "__main__":
    check_gpu_capture()
    has_hw = check_hardware_encoder()

    if has_hw:
        test_nvenc_performance()

    show_gpu_optimization_tips()
