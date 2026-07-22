#!/usr/bin/env python3
"""
异步架构演示 - 分离捕获和显示，类似 OBS 的架构。

关键改进：
1. 捕获线程独立运行 (可以达到 60 FPS)
2. 显示线程只取最新帧
3. 使用有界队列避免内存堆积
"""
import sys
import time
import threading
from pathlib import Path
from collections import deque

sys.path.insert(0, str(Path(__file__).parent / 'src'))

import cv2
import numpy as np
import mss


class AsyncCaptureDisplay:
    """
    异步捕获和显示架构 - 模拟 OBS 的多线程设计。

    架构:
      捕获线程 (60 FPS) → 队列 (max=2) → 显示线程 (30 FPS)
    """

    def __init__(self, backend="mss", target_fps=60):
        self.backend = backend
        self.target_fps = target_fps
        self.running = False

        # 帧队列 - 只保留最新帧
        self.frame_queue = deque(maxlen=2)

        # 统计
        self.capture_count = 0
        self.display_count = 0
        self.dropped_frames = 0

        # FPS 计算
        self.capture_fps = 0
        self.display_fps = 0
        self.last_capture_time = 0
        self.last_display_time = 0
        self.last_capture_count = 0
        self.last_display_count = 0

        # 目标分辨率
        self.capture_width = 1920
        self.capture_height = 1080

    def capture_thread_func(self):
        """高速捕获线程 - 类似 OBS 的捕获线程。"""
        print(f"[捕获线程] 启动，目标 {self.target_fps} FPS")

        # 在线程内初始化 MSS (避免跨线程问题)
        if self.backend == "mss":
            import mss
            sct = mss.mss()
            monitor = sct.monitors[1]
        else:
            raise ValueError(f"Unknown backend: {self.backend}")

        last_time = time.time()
        frame_interval = 1.0 / self.target_fps

        while self.running:
            loop_start = time.time()

            # 捕获
            if self.backend == "mss":
                monitor_region = {
                    "left": 0, "top": 0,
                    "width": self.capture_width,
                    "height": self.capture_height,
                    "mon": 1
                }
                screenshot = sct.grab(monitor_region)
                frame = np.frombuffer(screenshot.rgb, dtype=np.uint8)
                frame = frame.reshape((screenshot.height, screenshot.width, 3))

            # 放入队列 (非阻塞)
            if len(self.frame_queue) >= self.frame_queue.maxlen:
                self.dropped_frames += 1

            self.frame_queue.append(frame)
            self.capture_count += 1

            # 更新捕获 FPS
            now = time.time()
            if now - self.last_capture_time >= 0.5:
                elapsed = now - self.last_capture_time
                frames = self.capture_count - self.last_capture_count
                self.capture_fps = frames / elapsed
                self.last_capture_time = now
                self.last_capture_count = self.capture_count

            # 帧 pacing
            elapsed = time.time() - loop_start
            if elapsed < frame_interval:
                time.sleep(frame_interval - elapsed)

        print(f"[捕获线程] 停止")

    def display_thread_func(self):
        """显示线程 - 只显示可用的帧。"""
        print(f"[显示线程] 启动")

        cv2.namedWindow("Async Capture (OBS-style)", cv2.WINDOW_NORMAL)

        display_interval = 1.0 / 30  # 30 FPS 显示
        last_display = time.time()

        while self.running:
            loop_start = time.time()

            # 从队列获取帧 (阻塞等待)
            if len(self.frame_queue) > 0:
                frame = self.frame_queue.popleft()

                # 更新显示 FPS
                now = time.time()
                if now - self.last_display_time >= 0.5:
                    elapsed = now - self.last_display_time
                    frames = self.display_count - self.last_display_count
                    self.display_fps = frames / elapsed if elapsed > 0 else 0
                    self.last_display_time = now
                    self.last_display_count = self.display_count

                # 绘制统计信息
                h, w = frame.shape[:2]

                overlay = frame.copy()
                cv2.rectangle(overlay, (5, 5), (450, 180), (0, 0, 0), -1)
                frame = cv2.addWeighted(overlay, 0.7, frame, 0.3, 0)

                # 捕获 FPS (关键性能指标)
                cap_color = (0, 255, 0) if self.capture_fps >= 50 else (0, 255, 255)
                cv2.putText(frame, f"Capture FPS: {self.capture_fps:.1f}", (15, 40),
                           cv2.FONT_HERSHEY_SIMPLEX, 1.0, cap_color, 2)

                # 显示 FPS
                disp_color = (0, 255, 0) if self.display_fps >= 25 else (0, 200, 255)
                cv2.putText(frame, f"Display FPS: {self.display_fps:.1f}", (15, 70),
                           cv2.FONT_HERSHEY_SIMPLEX, 1.0, disp_color, 2)

                # 队列状态
                cv2.putText(frame, f"Queue: {len(self.frame_queue)}/2", (15, 100),
                           cv2.FONT_HERSHEY_SIMPLEX, 0.5, (200, 200, 200), 1)
                cv2.putText(frame, f"Dropped: {self.dropped_frames}", (15, 120),
                           cv2.FONT_HERSHEY_SIMPLEX, 0.5, (255, 100, 100), 1)

                # 总计
                cv2.putText(frame, f"Captured: {self.capture_count}", (15, 145),
                           cv2.FONT_HERSHEY_SIMPLEX, 0.45, (255, 255, 255), 1)
                cv2.putText(frame, f"Displayed: {self.display_count}", (15, 165),
                           cv2.FONT_HERSHEY_SIMPLEX, 0.45, (255, 255, 255), 1)

                # 显示
                cv2.imshow("Async Capture (OBS-style)", frame)

                self.display_count += 1

            # 处理窗口消息
            key = cv2.waitKey(1) & 0xFF
            if key == 27 or key == ord('q'):
                self.running = False
                break

            # 显示帧 pacing (30 FPS)
            elapsed = time.time() - loop_start
            if elapsed < display_interval:
                time.sleep(display_interval - elapsed)

        cv2.destroyAllWindows()
        print(f"[显示线程] 停止")

    def run(self, duration=60):
        """运行异步捕获-显示。"""
        print("="*70)
        print("异步架构演示 - 类似 OBS 的多线程设计")
        print("="*70)
        print(f"捕获目标: {self.target_fps} FPS")
        print(f"显示目标: 30 FPS")
        print(f"队列深度: 2 帧 (只保留最新)")
        print("="*70)

        self.running = True
        self.last_capture_time = time.time()
        self.last_display_time = time.time()

        # 启动线程
        capture_thread = threading.Thread(target=self.capture_thread_func, daemon=True)
        display_thread = threading.Thread(target=self.display_thread_func, daemon=False)

        capture_thread.start()
        time.sleep(0.1)  # 让捕获线程先启动
        display_thread.start()

        # 等待完成或用户退出
        display_thread.join()

        # 最终统计
        print("\n" + "="*70)
        print("测试完成")
        print("="*70)
        print(f"捕获帧数: {self.capture_count}")
        print(f"显示帧数: {self.display_count}")
        print(f"丢弃帧数: {self.dropped_frames}")
        print(f"捕获 FPS: {self.capture_fps:.1f}")
        print(f"显示 FPS: {self.display_fps:.1f}")
        print(f"效率: {self.display_count / self.capture_count * 100:.1f}%")

        if self.capture_fps >= 50:
            rating = "⭐⭐⭐ 优秀 (接近 OBS 捕获性能)"
        elif self.capture_fps >= 30:
            rating = "⭐⭐ 良好"
        else:
            rating = "⭐ 一般"
        print(f"评级: {rating}")


if __name__ == "__main__":
    import argparse

    parser = argparse.ArgumentParser()
    parser.add_argument("--duration", type=int, default=60)
    parser.add_argument("--fps", type=int, default=60)
    parser.add_argument("--backend", default="mss")
    args = parser.parse_args()

    try:
        app = AsyncCaptureDisplay(backend=args.backend, target_fps=args.fps)
        app.run(duration=args.duration)
    except KeyboardInterrupt:
        print("\n\n中断")
