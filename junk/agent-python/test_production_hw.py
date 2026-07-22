#!/usr/bin/env python3
"""
生产级测试 - 使用硬件编码器的完整流水线。

测试流程:
1. 捕获 (MSS)
2. 编码 (PyAVEncoder with hardware acceleration)
3. RTP 打包
4. 解码 (PyAVDecoder)
5. 显示
"""
import sys
import time
import asyncio
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent / 'src'))

import cv2
import numpy as np

from encoder.pyav_encoder import PyAVEncoder
from decoder.pyav_decoder import PyAVDecoder
from webrtc.rtp import H264RTPPacketizer, H264RTPDepacketizer


class ProductionPipeline:
    """
    完整的生产流水线。

    架构:
    捕获 → 编码(GPU) → RTP → 解码 → 显示
    """

    def __init__(self, width=1280, height=720, fps=30):
        self.width = width
        self.height = height
        self.fps = fps

        # 组件
        self.encoder = None
        self.decoder = None
        self.packetizer = H264RTPPacketizer()
        self.depacketizer = H264RTPDepacketizer()

        # 统计
        self.frame_count = 0
        self.encoded_count = 0
        self.rtp_count = 0
        self.decoded_count = 0
        self.start_time = 0

        # FPS
        self.current_fps = 0
        self.encode_latency = []
        self.decode_latency = []

    async def initialize(self, use_hardware=True):
        """初始化所有组件。"""
        print("="*70)
        print("生产级流水线初始化")
        print("="*70)

        # 编码器
        print("\n[1/3] 初始化编码器...")
        self.encoder = PyAVEncoder(
            width=self.width,
            height=self.height,
            fps=self.fps,
            bitrate_kbps=3000,
            hardware_accel=use_hardware,
        )
        if not await self.encoder.initialize():
            print("   ❌ 编码器初始化失败")
            return False
        encoder_type = "GPU" if use_hardware else "CPU"
        print(f"   ✅ 编码器初始化成功 ({encoder_type})")

        # 解码器
        print("\n[2/3] 初始化解码器...")
        self.decoder = PyAVDecoder()
        if not await self.decoder.initialize():
            print("   ❌ 解码器初始化失败")
            return False
        print(f"   ✅ 解码器初始化成功")

        # RTP
        print("\n[3/3] RTP 打包器就绪")
        print(f"   ✅ 打包器: {self.packetizer.__class__.__name__}")
        print(f"   ✅ 解包器: {self.depacketizer.__class__.__name__}")

        print("\n" + "="*70)
        print("流水线就绪!")
        print("="*70)
        return True

    def capture_frame(self):
        """捕获一帧 (使用 MSS)。"""
        try:
            import mss

            # 每次捕获时初始化 MSS (避免线程问题)
            sct = mss.mss()

            # 计算捕获区域
            import ctypes
            user32 = ctypes.windll.user32
            screen_w = user32.GetSystemMetrics(0)
            screen_h = user32.GetSystemMetrics(1)

            scale = min(self.width / screen_w, self.height / screen_h)
            capture_w = int(screen_w * scale)
            capture_h = int(screen_h * scale)

            monitor = {
                "left": (screen_w - capture_w) // 2,
                "top": (screen_h - capture_h) // 2,
                "width": capture_w,
                "height": capture_h,
            }

            screenshot = sct.grab(monitor)
            arr = np.frombuffer(screenshot.rgb, dtype=np.uint8)
            frame = arr.reshape((capture_h, capture_w, 3))

            if capture_w != self.width or capture_h != self.height:
                frame = cv2.resize(frame, (self.width, self.height),
                                  interpolation=cv2.INTER_LINEAR)

            return frame

        except Exception as e:
            print(f"[捕获] 错误: {e}")
            return None

    async def process_frame(self, frame):
        """处理一帧的完整流水线。"""
        if frame is None:
            return None

        t0 = time.perf_counter()

        # 1. 编码
        frame_bytes = frame.tobytes()
        encoded = await self.encoder.encode(
            frame_bytes, self.width, self.height, "RGB"
        )
        if encoded is None:
            return None

        t1 = time.perf_counter()
        self.encode_latency.append((t1 - t0) * 1000)
        if len(self.encode_latency) > 30:
            self.encode_latency.pop(0)

        # 2. RTP 打包
        timestamp_ms = int(self.frame_count * 1000 / self.fps)
        rtp_packets = self.packetizer.packetize(
            encoded.data, timestamp_ms, encoded.is_keyframe
        )
        self.rtp_count += len(rtp_packets)

        # 3. RTP 解包
        reassembled = b''
        for packet in rtp_packets:
            data = self.depacketizer.depacketize(packet)
            if data:
                reassembled += data

        t2 = time.perf_counter()

        # 4. 解码
        if reassembled:
            decoded = await self.decoder.decode(reassembled, encoded.timestamp)
            if decoded:
                t3 = time.perf_counter()
                self.decode_latency.append((t3 - t2) * 1000)
                if len(self.decode_latency) > 30:
                    self.decode_latency.pop(0)

                self.encoded_count += 1
                self.decoded_count += 1
                return decoded.data

        return None

    async def run(self, duration=30):
        """运行生产流水线。"""
        print("\n按 ESC 或 Q 退出")
        print("="*70)

        self.start_time = time.time()
        last_stats = self.start_time

        cv2.namedWindow("Production Pipeline", cv2.WINDOW_NORMAL)

        try:
            while time.time() - self.start_time < duration:
                loop_start = time.perf_counter()

                # 捕获
                frame = self.capture_frame()
                if frame is None:
                    continue
                self.frame_count += 1

                # 处理流水线
                decoded_frame = await self.process_frame(frame)

                # 更新 FPS
                now = time.time()
                if now - last_stats >= 0.5:
                    self.current_fps = self.frame_count / (now - self.start_time)
                    last_stats = now

                # 显示
                display_frame = decoded_frame if decoded_frame is not None else frame
                self._draw_stats(display_frame)

                cv2.imshow("Production Pipeline", display_frame)

                # 退出检查
                key = cv2.waitKey(1) & 0xFF
                if key == 27 or key == ord('q'):
                    break

                # 帧率控制
                elapsed = time.perf_counter() - loop_start
                target = 1.0 / 60
                if elapsed < target:
                    await asyncio.sleep(target - elapsed)

        finally:
            cv2.destroyAllWindows()

        # 最终统计
        self._print_stats()

    def _draw_stats(self, frame):
        """绘制统计信息。"""
        overlay = frame.copy()
        cv2.rectangle(overlay, (5, 5), (480, 250), (0, 0, 0), -1)
        frame = cv2.addWeighted(overlay, 0.7, frame, 0.3, 0)

        # FPS 颜色
        if self.current_fps >= 25:
            fps_color = (0, 200, 0)
        elif self.current_fps >= 15:
            fps_color = (0, 200, 200)
        else:
            fps_color = (0, 0, 255)

        y = 35
        cv2.putText(frame, f"FPS: {self.current_fps:.1f}",
                   (15, y), cv2.FONT_HERSHEY_SIMPLEX, 0.7, fps_color, 2)
        y += 30

        # 组件状态
        cv2.putText(frame, f"捕获: {self.frame_count} 帧",
                   (15, y), cv2.FONT_HERSHEY_SIMPLEX, 0.45, (200, 255, 200), 1)
        y += 25
        cv2.putText(frame, f"编码: {self.encoded_count} 帧",
                   (15, y), cv2.FONT_HERSHEY_SIMPLEX, 0.45, (200, 255, 200), 1)
        y += 25
        cv2.putText(frame, f"RTP: {self.rtp_count} 包",
                   (15, y), cv2.FONT_HERSHEY_SIMPLEX, 0.45, (200, 200, 255), 1)
        y += 25
        cv2.putText(frame, f"解码: {self.decoded_count} 帧",
                   (15, y), cv2.FONT_HERSHEY_SIMPLEX, 0.45, (200, 255, 200), 1)
        y += 30

        # 延迟
        if self.encode_latency:
            avg_encode = sum(self.encode_latency) / len(self.encode_latency)
            color = (0, 255, 0) if avg_encode < 10 else (0, 200, 200)
            cv2.putText(frame, f"编码延迟: {avg_encode:.1f} ms",
                       (15, y), cv2.FONT_HERSHEY_SIMPLEX, 0.45, color, 1)
            y += 25

        if self.decode_latency:
            avg_decode = sum(self.decode_latency) / len(self.decode_latency)
            cv2.putText(frame, f"解码延迟: {avg_decode:.1f} ms",
                       (15, y), cv2.FONT_HERSHEY_SIMPLEX, 0.45, (200, 200, 200), 1)
            y += 25

        # 硬件加速标记
        if self.encoder and self.encoder.hardware_accel:
            cv2.putText(frame, "🚀 GPU 硬件加速",
                       (15, 240), cv2.FONT_HERSHEY_SIMPLEX, 0.6, (0, 255, 0), 2)

    def _print_stats(self):
        """打印最终统计。"""
        total_time = time.time() - self.start_time

        print("\n" + "="*70)
        print("流水线统计")
        print("="*70)
        print(f"持续时间: {total_time:.1f}s")
        print(f"捕获帧数: {self.frame_count}")
        print(f"编码帧数: {self.encoded_count}")
        print(f"RTP 包数: {self.rtp_count}")
        print(f"解码帧数: {self.decoded_count}")
        print(f"\n性能指标:")
        print(f"  端到端 FPS: {self.frame_count / total_time:.1f}")

        if self.encode_latency:
            avg_encode = sum(self.encode_latency) / len(self.encode_latency)
            print(f"  平均编码延迟: {avg_encode:.1f} ms")
            print(f"  理论编码 FPS: {1000 / avg_encode:.1f}")

        if self.decode_latency:
            avg_decode = sum(self.decode_latency) / len(self.decode_latency)
            print(f"  平均解码延迟: {avg_decode:.1f} ms")

        # 评级
        fps = self.frame_count / total_time
        if fps >= 25:
            print(f"\n评级: ⭐⭐⭐ 优秀")
        elif fps >= 15:
            print(f"\n评级: ⭐⭐ 良好")
        else:
            print(f"\n评级: ⭐ 一般")

    async def close(self):
        """清理资源。"""
        if self.encoder:
            await self.encoder.close()
        if self.decoder:
            await self.decoder.close()


async def main():
    """主函数。"""
    import argparse

    parser = argparse.ArgumentParser()
    parser.add_argument("--duration", type=int, default=30)
    parser.add_argument("--width", type=int, default=1280)
    parser.add_argument("--height", type=int, default=720)
    parser.add_argument("--fps", type=int, default=30)
    parser.add_argument("--no-gpu", action="store_true", help="禁用 GPU 硬件加速")
    args = parser.parse_args()

    pipeline = ProductionPipeline(
        width=args.width,
        height=args.height,
        fps=args.fps
    )

    try:
        if await pipeline.initialize(use_hardware=not args.no_gpu):
            await pipeline.run(duration=args.duration)
    except KeyboardInterrupt:
        print("\n中断退出")
    finally:
        await pipeline.close()


if __name__ == "__main__":
    asyncio.run(main())
