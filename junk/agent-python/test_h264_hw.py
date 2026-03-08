#!/usr/bin/env python3
"""
GPU 硬件编码测试 - 正确初始化 NVENC/Media Foundation。
"""
import sys
import time
import io
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent / 'src'))

import cv2
import numpy as np


def test_h264_nvenc():
    """测试 NVENC 硬件编码。"""
    print("="*60)
    print("NVENC 硬件编码测试")
    print("="*60)

    try:
        import av

        # 创建测试帧
        frame_data = np.random.randint(0, 255, (1080, 1920, 3), dtype=np.uint8)
        frame = av.VideoFrame.from_ndarray(frame_data, format='rgb24')

        # 方法 1: 使用容器方式
        print("\n方法 1: 容器方式 (h264_nvenc)")
        try:
            output = io.BytesIO()
            container = av.open(output, 'w', format='h264')
            stream = container.add_stream('h264_nvenc', rate=30)
            stream.width = 1920
            stream.height = 1080
            stream.bit_rate = 5_000_000

            # NVENC 特定选项
            try:
                stream.options['preset'] = 'fast'
                stream.options['tune'] = 'll'
                stream.options['rc'] = 'cbr'
                stream.options['b'] = '5000k'
            except:
                pass  # 选项可能不支持

            print("   编码 10 帧...")
            times = []
            for i in range(10):
                frame.pts = i
                t0 = time.perf_counter()
                for packet in stream.encode(frame):
                    container.mux(packet)
                t1 = time.perf_counter()
                times.append((t1 - t0) * 1000)

            # Flush
            for packet in stream.encode():
                container.mux(packet)

            container.close()

            size = len(output.getvalue())
            avg = sum(times) / len(times)
            fps = 1000 / avg

            print(f"   ✅ 成功!")
            print(f"   输出大小: {size / 1024:.1f} KB")
            print(f"   平均编码时间: {avg:.1f} ms")
            print(f"   理论 FPS: {fps:.1f}")

            return True

        except Exception as e:
            print(f"   ❌ 容器方式失败: {e}")

    except ImportError:
        print("   ❌ PyAV 未安装")

    return False


def test_h264_mf():
    """测试 Media Foundation 硬件编码。"""
    print("\n" + "="*60)
    print("Media Foundation 硬件编码测试")
    print("="*60)

    try:
        import av

        frame_data = np.random.randint(0, 255, (720, 1280, 3), dtype=np.uint8)
        frame = av.VideoFrame.from_ndarray(frame_data, format='rgb24')

        print("\n方法: 容器方式 (h264_mf)")
        try:
            output = io.BytesIO()
            container = av.open(output, 'w', format='h264')
            stream = container.add_stream('h264_mf', rate=30)
            stream.width = 1280
            stream.height = 720
            stream.bit_rate = 3_000_000

            print("   编码 10 帧...")
            times = []
            for i in range(10):
                frame.pts = i
                t0 = time.perf_counter()
                for packet in stream.encode(frame):
                    container.mux(packet)
                t1 = time.perf_counter()
                times.append((t1 - t0) * 1000)

            # Flush
            for packet in stream.encode():
                container.mux(packet)

            container.close()

            size = len(output.getvalue())
            avg = sum(times) / len(times)
            fps = 1000 / avg

            print(f"   ✅ 成功!")
            print(f"   输出大小: {size / 1024:.1f} KB")
            print(f"   平均编码时间: {avg:.1f} ms")
            print(f"   理论 FPS: {fps:.1f}")

            return True

        except Exception as e:
            print(f"   ❌ 失败: {e}")

    except ImportError:
        print("   ❌ PyAV 未安装")

    return False


def compare_software_vs_hardware():
    """对比软件和硬件编码性能。"""
    print("\n" + "="*60)
    print("软件 vs 硬件编码对比")
    print("="*60)

    try:
        import av

        # 测试帧
        frame_data = np.random.randint(0, 255, (720, 1280, 3), dtype=np.uint8)

        results = []

        encoders = [
            ('libx264 (软件)', 'libx264', {}),
            ('h264_mf (MF)', 'h264_mf', {}),
        ]

        # 尝试 NVENC
        try:
            output = io.BytesIO()
            container = av.open(output, 'w', format='h264')
            stream = container.add_stream('h264_nvenc', rate=30)
            stream.width = 1280
            stream.height = 720
            container.close()
            encoders.append(('h264_nvenc (NVENC)', 'h264_nvenc', {}))
        except:
            pass

        print(f"\n{'编码器':<20} {'编码时间':<12} {'FPS':<10}")
        print("-"*50)

        for name, codec_name, options in encoders:
            try:
                frame = av.VideoFrame.from_ndarray(frame_data, format='rgb24')

                output = io.BytesIO()
                container = av.open(output, 'w', format='h264')
                stream = container.add_stream(codec_name, rate=30)
                stream.width = 1280
                stream.height = 720
                stream.bit_rate = 3_000_000

                for key, value in options.items():
                    try:
                        stream.options[key] = value
                    except:
                        pass

                times = []
                for i in range(5):
                    frame.pts = i
                    t0 = time.perf_counter()
                    for packet in stream.encode(frame):
                        container.mux(packet)
                    t1 = time.perf_counter()
                    if t1 - t0 > 0:
                        times.append((t1 - t0) * 1000)

                # Flush
                for packet in stream.encode():
                    container.mux(packet)

                container.close()

                if times:
                    avg = sum(times) / len(times)
                    fps = 1000 / avg

                    marker = "🚀" if fps > 100 else "⚡" if fps > 50 else "💻"
                    print(f"{marker} {name:<20} {avg:8.1f} ms   {fps:6.1f}")

            except Exception as e:
                print(f"• {name:<20} 失败: {str(e)[:30]}")

    except ImportError:
        print("   ❌ PyAV 未安装")


def show_optimized_architecture():
    """显示优化后的架构。"""
    print("\n" + "="*60)
    print("推荐 GPU 加速架构")
    print("="*60)

    print("""
当前架构 (软件编码):
┌──────────┐    ┌──────────┐    ┌──────────┐
│ 捕获     │ →  │ numpy    │ →  │ libx264  │ → 网络
│ (GDI)    │    │ 转换     │    │ (CPU)    │
└──────────┘    └──────────┘    └──────────┘
   30ms          10ms            25ms
   └───────────────── 56ms (18 FPS) ──────┘

GPU 加速架构:
┌──────────┐    ┌──────────┐    ┌──────────┐
│ 捕获     │ →  │ GPU纹理   │ →  │ NVENC    │ → 网络
│ (d3dshot)│    │ (零拷贝)  │    │ (GPU)    │
└──────────┘    └──────────┘    └──────────┘
   5ms           0ms             5ms
   └────────────────── 10ms (100 FPS) ──────┘


实现步骤:
─────────────────────────────────────────────────────────────────────
1. 安装 d3dshot
   pip install d3dshot

2. 使用容器式 NVENC 编码
   container = av.open(output, 'w', format='h264')
   stream = container.add_stream('h264_nvenc', rate=30)

3. 异步架构
   - 捕获线程 (d3dshot)
   - 编码线程 (NVENC)
   - 推送线程 (WebRTC/RTSP)

预期性能:
  • 捕获: 60+ FPS @ 1080p
  • 编码: < 5ms @ 1080p
  • 端到端: 60 FPS @ 1080p
    """)


if __name__ == "__main__":
    # 测试硬件编码器
    nvenc_ok = test_h264_nvenc()
    mf_ok = test_h264_mf()

    if nvenc_ok or mf_ok:
        print("\n✅ 硬件编码可用!")
    else:
        print("\n❌ 硬件编码不可用，使用软件编码")

    # 对比
    compare_software_vs_hardware()

    # 显示优化架构
    show_optimized_architecture()
