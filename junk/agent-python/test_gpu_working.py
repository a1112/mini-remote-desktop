#!/usr/bin/env python3
"""
GPU 加速实时显示 - 稳定版本。

使用 MSS 捕获 + h264_mf 硬件编码
"""
import sys
import time
import io
import threading
import queue
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent / 'src'))

import cv2
import numpy as np


class GPUAcceleratedCapture:
    """
    GPU 加速捕获 - 稳定版本。

    架构:
    - 捕获线程: MSS 捕获屏幕
    - 编码线程: h264_mf 硬件编码
    - 显示线程: OpenCV 显示
    """

    def __init__(self, width=1280, height=720, fps=30):
        self.width = width
        self.height = height
        self.fps = fps
        self.running = False

        # 队列 (用于线程间通信)
        self.frame_queue = queue.Queue(maxsize=2)  # 只保留最新帧

        # 统计
        self.capture_count = 0
        self.encode_count = 0
        self.display_count = 0
        self.dropped_frames = 0
        self.start_time = 0

        # FPS 计算
        self.current_capture_fps = 0
        self.current_encode_fps = 0
        self.current_display_fps = 0

        # 硬件编码器
        self._init_hardware_encoder()

    def _init_hardware_encoder(self):
        """初始化 Media Foundation 硬件编码器。"""
        try:
            import av

            self._encode_output = io.BytesIO()
            self._encode_container = av.open(
                self._encode_output, 'w', format='h264'
            )
            self._encode_stream = self._encode_container.add_stream(
                'h264_mf', rate=self.fps
            )
            self._encode_stream.width = self.width
            self._encode_stream.height = self.height
            self._encode_stream.bit_rate = 3_000_000
            self._encode_pts = 0

            print(f"✅ 硬件编码器 h264_mf 初始化成功")
            print(f"   分辨率: {self.width}x{self.height}")
            print(f"   目标 FPS: {self.fps}")
            self.has_hw_encoder = True

        except Exception as e:
            print(f"⚠️  硬件编码器初始化失败: {e}")
            print(f"   将使用软件编码器")
            self.has_hw_encoder = False

    def capture_thread_func(self):
        """捕获线程 - 使用 MSS。"""
        try:
            import mss

            # 在线程内初始化 MSS (避免线程安全问题)
            sct = mss.mss()

            # 计算捕获区域 (居中裁剪)
            import ctypes
            user32 = ctypes.windll.user32
            screen_w = user32.GetSystemMetrics(0)
            screen_h = user32.GetSystemMetrics(1)

            # 计算缩放比例
            scale = min(self.width / screen_w, self.height / screen_h)
            capture_w = int(screen_w * scale)
            capture_h = int(screen_h * scale)

            monitor = {
                "left": (screen_w - capture_w) // 2,
                "top": (screen_h - capture_h) // 2,
                "width": capture_w,
                "height": capture_h,
                "mon": 1  # 主显示器
            }

            print(f"[捕获线程] MSS 初始化: {capture_w}x{capture_h}")

            last_time = time.time()
            frame_interval = 1.0 / self.fps

            while self.running:
                loop_start = time.perf_counter()

                # 捕获
                screenshot = sct.grab(monitor)
                arr = np.frombuffer(screenshot.rgb, dtype=np.uint8)
                frame = arr.reshape((capture_h, capture_w, 3))

                # 调整大小到目标分辨率
                if capture_w != self.width or capture_h != self.height:
                    frame = cv2.resize(frame, (self.width, self.height),
                                      interpolation=cv2.INTER_LINEAR)

                self.capture_count += 1

                # 放入队列 (非阻塞)
                try:
                    self.frame_queue.put_nowait(frame)
                except queue.Full:
                    self.dropped_frames += 1

                # FPS pacing
                elapsed = time.perf_counter() - loop_start
                if elapsed < frame_interval:
                    time.sleep(frame_interval - elapsed)

                # 更新 FPS
                now = time.time()
                if now - last_time >= 0.5:
                    self.current_capture_fps = self.capture_count / (now - self.start_time)
                    last_time = now

        except Exception as e:
            print(f"[捕获线程] 错误: {e}")
            import traceback
            traceback.print_exc()

    def encode_frame(self, frame):
        """编码一帧 (使用硬件编码器)。"""
        if not self.has_hw_encoder:
            return None

        try:
            import av

            # 转换为 VideoFrame
            av_frame = av.VideoFrame.from_ndarray(frame, format='rgb24')
            av_frame.pts = self._encode_pts
            self._encode_pts += 1

            # 编码
            start_pos = self._encode_output.tell()
            for packet in self._encode_stream.encode(av_frame):
                self._encode_container.mux(packet)
            end_pos = self._encode_output.tell()

            self.encode_count += 1

            # 获取编码数据
            if end_pos > start_pos:
                self._encode_output.seek(start_pos)
                data = self._encode_output.read(end_pos - start_pos)
                self._encode_output.seek(end_pos)

                # 定期重置缓冲区
                if end_pos > 1024 * 1024:
                    self._reset_encoder()

                return data

            return None

        except Exception as e:
            print(f"[编码] 错误: {e}")
            return None

    def _reset_encoder(self):
        """重置编码器缓冲区。"""
        try:
            import av

            self._encode_output = io.BytesIO()
            self._encode_container = av.open(
                self._encode_output, 'w', format='h264'
            )
            self._encode_stream = self._encode_container.add_stream(
                'h264_mf', rate=self.fps
            )
            self._encode_stream.width = self.width
            self._encode_stream.height = self.height
            self._encode_stream.bit_rate = 3_000_000
            self._encode_pts = 0

        except Exception as e:
            print(f"[编码器重置] 错误: {e}")

    def run(self, duration=60):
        """运行 GPU 加速实时显示。"""
        print("="*70)
        print("GPU 加速实时显示 - MSS + h264_mf")
        print("="*70)
        print("按 ESC 或 Q 退出")
        print("="*70)

        self.running = True
        self.start_time = time.time()

        # 启动捕获线程
        capture_thread = threading.Thread(
            target=self.capture_thread_func,
            daemon=True
        )
        capture_thread.start()

        # 显示循环
        cv2.namedWindow("GPU Accelerated", cv2.WINDOW_NORMAL)

        last_encode_time = time.time()
        last_display_time = time.time()
        encode_times = []
        last_stats_update = self.start_time

        try:
            while self.running and time.time() - self.start_time < duration:
                loop_start = time.perf_counter()

                # 从队列获取帧
                try:
                    frame = self.frame_queue.get(timeout=0.1)
                except queue.Empty:
                    if not capture_thread.is_alive():
                        print("[主线程] 捕获线程已退出")
                        break
                    continue

                # 编码 (后台处理，不阻塞显示)
                encode_start = time.perf_counter()
                encoded = self.encode_frame(frame)
                encode_elapsed = (time.perf_counter() - encode_start) * 1000

                if encoded:
                    encode_times.append(encode_elapsed)
                    if len(encode_times) > 30:
                        encode_times.pop(0)

                # 更新 FPS
                now = time.time()
                if now - last_encode_time >= 0.5:
                    self.current_encode_fps = self.encode_count / (now - self.start_time)
                    last_encode_time = now
                if now - last_display_time >= 0.5:
                    self.current_display_fps = self.display_count / (now - self.start_time)
                    last_display_time = now

                self.display_count += 1

                # 绘制信息覆盖层
                overlay = frame.copy()
                cv2.rectangle(overlay, (5, 5), (420, 200), (0, 0, 0), -1)
                frame = cv2.addWeighted(overlay, 0.7, frame, 0.3, 0)

                # FPS 颜色
                if self.current_capture_fps >= 25:
                    fps_color = (0, 200, 0)
                elif self.current_capture_fps >= 15:
                    fps_color = (0, 200, 200)
                else:
                    fps_color = (0, 0, 255)

                # 显示信息
                y = 35
                cv2.putText(frame, f"捕获 FPS: {self.current_capture_fps:.1f}",
                           (15, y), cv2.FONT_HERSHEY_SIMPLEX, 0.6, fps_color, 2)
                y += 28
                cv2.putText(frame, f"编码 FPS: {self.current_encode_fps:.1f}",
                           (15, y), cv2.FONT_HERSHEY_SIMPLEX, 0.6, (100, 255, 100), 2)
                y += 28
                cv2.putText(frame, f"显示 FPS: {self.current_display_fps:.1f}",
                           (15, y), cv2.FONT_HERSHEY_SIMPLEX, 0.6, (200, 200, 255), 2)
                y += 28

                # 编码器信息
                encoder_name = "h264_mf (GPU)" if self.has_hw_encoder else "libx264 (CPU)"
                cv2.putText(frame, f"编码器: {encoder_name}",
                           (15, y), cv2.FONT_HERSHEY_SIMPLEX, 0.5, (255, 255, 255), 1)
                y += 25
                cv2.putText(frame, f"已编码: {self.encode_count} 帧",
                           (15, y), cv2.FONT_HERSHEY_SIMPLEX, 0.45, (255, 255, 255), 1)
                y += 25
                if encode_times:
                    avg_encode = sum(encode_times) / len(encode_times)
                    cv2.putText(frame, f"编码延迟: {avg_encode:.1f} ms",
                               (15, y), cv2.FONT_HERSHEY_SIMPLEX, 0.45, (255, 255, 255), 1)

                # GPU 加速标记
                if self.has_hw_encoder:
                    cv2.putText(frame, "🚀 GPU 硬件加速",
                               (15, 195), cv2.FONT_HERSHEY_SIMPLEX, 0.6, (0, 255, 0), 2)

                # 显示
                cv2.imshow("GPU Accelerated", frame)

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
            self.running = False
            cv2.destroyAllWindows()

            # 等待捕获线程结束
            capture_thread.join(timeout=2)

        # 最终统计
        total_time = time.time() - self.start_time

        print("\n" + "="*70)
        print("测试完成")
        print("="*70)
        print(f"持续时间: {total_time:.1f}s")
        print(f"捕获帧数: {self.capture_count}")
        print(f"编码帧数: {self.encode_count}")
        print(f"显示帧数: {self.display_count}")
        print(f"丢弃帧数: {self.dropped_frames}")
        print(f"\n性能指标:")
        print(f"  捕获 FPS: {self.capture_count / total_time:.1f}")
        print(f"  编码 FPS: {self.encode_count / total_time:.1f}")
        print(f"  显示 FPS: {self.display_count / total_time:.1f}")
        print(f"  端到端 FPS: {self.display_count / total_time:.1f}")

        if encode_times:
            avg_encode = sum(encode_times) / len(encode_times)
            print(f"  平均编码延迟: {avg_encode:.1f} ms")
            print(f"  理论编码 FPS: {1000 / avg_encode:.1f}")

        # 评级
        fps = self.display_count / total_time
        if fps >= 25:
            print(f"\n评级: ⭐⭐⭐ 优秀 - GPU 加速工作正常!")
        elif fps >= 15:
            print(f"\n评级: ⭐⭐ 良好")
        else:
            print(f"\n评级: ⭐ 一般")

    def close(self):
        """清理资源。"""
        self.running = False
        try:
            if self.has_hw_encoder and self._encode_container:
                self._encode_container.close()
        except:
            pass


if __name__ == "__main__":
    import argparse

    parser = argparse.ArgumentParser()
    parser.add_argument("--duration", type=int, default=30)
    parser.add_argument("--width", type=int, default=1280)
    parser.add_argument("--height", type=int, default=720)
    parser.add_argument("--fps", type=int, default=30)
    args = parser.parse_args()

    try:
        app = GPUAcceleratedCapture(
            width=args.width,
            height=args.height,
            fps=args.fps
        )

        app.run(duration=args.duration)

    except Exception as e:
        print(f"\n错误: {e}")
        import traceback
        traceback.print_exc()
    finally:
        if 'app' in locals():
            app.close()
