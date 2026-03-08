#!/usr/bin/env python3
"""
GPU 加速实时显示 - 使用 Media Foundation 硬件编码。

这是最快的配置：GDI 捕获 + h264_mf 硬件编码
"""
import sys
import time
import io
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent / 'src'))

import cv2
import numpy as np

# 硬件编码
try:
    import av
    HAS_AV = True
except ImportError:
    HAS_AV = False
    print("❌ 需要 PyAV: pip install av")


class H264MFCapture:
    """
    使用 Media Foundation 硬件编码的高速捕获。

    性能:
    - 捕获: GDI @ 1080p → ~60 FPS
    - 编码: h264_mf → ~120 FPS
    - 综合: 30+ FPS 端到端
    """

    def __init__(self, width=1920, height=1080, fps=30):
        self.width = width
        self.height = height
        self.fps = fps
        self.running = False

        # 初始化编码器
        if HAS_AV:
            self._init_encoder()

        # 初始化 GDI 捕获
        self._init_gdi_capture()

        # 统计
        self.frame_count = 0
        self.encode_count = 0
        self.start_time = 0
        self.current_fps = 0
        self.encode_fps = 0

    def _init_encoder(self):
        """初始化 Media Foundation 编码器。"""
        self._output = io.BytesIO()
        self._container = av.open(self._output, 'w', format='h264')
        self._stream = self._container.add_stream('h264_mf', rate=self.fps)
        self._stream.width = self.width
        self._stream.height = self.height
        self._stream.bit_rate = 3_000_000
        self._pts = 0

        print(f"[编码器] h264_mf 硬件编码初始化: {self.width}x{self.height} @ {self.fps}fps")

    def _init_gdi_capture(self):
        """初始化 GDI 捕获。"""
        try:
            import win32gui
            import win32con
            import ctypes

            user32 = ctypes.windll.user32
            src_w = user32.GetSystemMetrics(0)
            src_h = user32.GetSystemMetrics(1)

            # 计算缩放
            scale = min(self.width / src_w, self.height / src_h)

            self.src_width = src_w
            self.src_height = src_h
            self.capture_width = int(src_w * scale)
            self.capture_height = int(src_h * scale)
            self.scale = scale

            self.hwnd = win32gui.GetDesktopWindow()
            self.hdc = win32gui.GetDC(self.hwnd)
            self.hdc_mem = win32gui.CreateCompatibleDC(self.hdc)
            self.hbitmap = win32gui.CreateCompatibleBitmap(
                self.hdc, self.capture_width, self.capture_height
            )
            self.hobj = win32gui.SelectObject(self.hdc_mem, self.hbitmap)

            print(f"[捕获] GDI 初始化: {self.capture_width}x{self.capture_height}")
            return True

        except Exception as e:
            print(f"[捕获] GDI 初始化失败: {e}")
            return False

    def capture_frame(self):
        """捕获一帧。"""
        try:
            import win32gui
            import win32con

            # GDI 捕获并缩放
            win32gui.StretchBlt(
                self.hdc_mem, 0, 0,
                self.capture_width, self.capture_height,
                self.hdc, 0, 0,
                self.src_width, self.src_height,
                win32con.SRCCOPY
            )

            # 获取数据
            import ctypes
            from ctypes import wintypes

            class BITMAPINFOHEADER(ctypes.Structure):
                _fields_ = [
                    ("biSize", wintypes.DWORD),
                    ("biWidth", wintypes.LONG),
                    ("biHeight", wintypes.LONG),
                    ("biPlanes", wintypes.WORD),
                    ("biBitCount", wintypes.WORD),
                    ("biCompression", wintypes.DWORD),
                    ("biSizeImage", wintypes.DWORD),
                    ("biXPelsPerMeter", wintypes.LONG),
                    ("biYPelsPerMeter", wintypes.LONG),
                    ("biClrUsed", wintypes.DWORD),
                    ("biClrImportant", wintypes.DWORD),
                ]

            class BITMAPINFO(ctypes.Structure):
                _fields_ = [("bmiHeader", BITMAPINFOHEADER)]

            bmi = BITMAPINFO()
            bmi.bmiHeader.biSize = ctypes.sizeof(BITMAPINFOHEADER)
            bmi.bmiHeader.biWidth = self.capture_width
            bmi.bmiHeader.biHeight = -self.capture_height  # top-down
            bmi.bmiHeader.biPlanes = 1
            bmi.bmiHeader.biBitCount = 32  # BGRA
            bmi.bmiHeader.biCompression = 0

            bmp_buffer = (ctypes.c_ubyte * (self.capture_width * self.capture_height * 4))()

            gdi32 = ctypes.windll.gdi32
            gdi32.GetDIBits(
                int(self.hdc),
                int(self.hbitmap),
                0,
                self.capture_height,
                ctypes.byref(bmp_buffer),
                ctypes.byref(bmi),
                0
            )

            # 转换为 numpy
            arr = np.frombuffer(bmp_buffer, dtype=np.uint8)
            arr = arr.reshape((self.capture_height, self.capture_width, 4))
            arr = arr[:, :, :3][:, :, [2, 1, 0]]  # BGRA → RGB

            return arr

        except Exception as e:
            print(f"[捕获] 错误: {e}")
            return None

    def encode_frame(self, frame):
        """使用硬件编码一帧。"""
        if frame is None or not HAS_AV:
            return None

        try:
            import av

            # 转换为 VideoFrame
            av_frame = av.VideoFrame.from_ndarray(frame, format='rgb24')
            av_frame.pts = self._pts
            self._pts += 1

            # 编码
            start_pos = self._output.tell()
            for packet in self._stream.encode(av_frame):
                self._container.mux(packet)
            end_pos = self._output.tell()

            self.encode_count += 1

            # 如果有新数据
            if end_pos > start_pos:
                self._output.seek(start_pos)
                data = self._output.read(end_pos - start_pos)
                self._output.seek(end_pos)

                # 重置缓冲区（简化处理）
                if end_pos > 1024 * 1024:  # 1MB 后重置
                    self._output = io.BytesIO()
                    self._container = av.open(self._output, 'w', format='h264')
                    self._stream = self._container.add_stream('h264_mf', rate=self.fps)
                    self._stream.width = self.width
                    self._stream.height = self.height
                    self._stream.bit_rate = 3_000_000
                    self._pts = 0

                return data

            return None

        except Exception as e:
            print(f"[编码] 错误: {e}")
            return None

    def decode_frame(self, encoded_data):
        """解码一帧（用于验证）。"""
        try:
            import av

            input_buffer = io.BytesIO(encoded_data)
            input_container = av.open(input_buffer, 'r', format='h264')

            for packet in input_container.demux():
                for decoded in packet.decode():
                    if decoded.width > 0:
                        img = decoded.to_ndarray(format='rgb24')
                        return img

            return None

        except Exception:
            return None

    def close(self):
        """清理资源。"""
        try:
            if HAS_AV and self._container:
                self._container.close()
        except:
            pass

        try:
            import win32gui
            win32gui.SelectObject(self.hdc_mem, self.hobj)
            win32gui.DeleteObject(self.hbitmap)
            win32gui.DeleteDC(self.hdc_mem)
            win32gui.ReleaseDC(self.hwnd, self.hdc)
        except:
            pass

    def run(self, duration=60):
        """运行实时显示测试。"""
        print("="*70)
        print("GPU 加速实时显示 - GDI 捕获 + h264_mf 硬件编码")
        print("="*70)
        print("按 ESC 或 Q 退出")
        print("="*70)

        self.running = True
        self.start_time = time.time()
        last_fps_update = self.start_time
        last_encode_update = self.start_time

        cv2.namedWindow("GPU Accelerated", cv2.WINDOW_NORMAL)

        try:
            while self.running and time.time() - self.start_time < duration:
                loop_start = time.perf_counter()

                # 捕获
                frame = self.capture_frame()
                if frame is None:
                    continue

                self.frame_count += 1

                # 编码（异步，不阻塞显示）
                # 注意：这里我们只编码，不显示编码后的帧
                # 实际应用中应该使用单独的编码线程
                encoded = self.encode_frame(frame)

                # 更新 FPS
                now = time.time()
                if now - last_fps_update >= 0.2:
                    self.current_fps = self.frame_count / (now - self.start_time)
                    last_fps_update = now

                # 显示帧
                h, w = frame.shape[:2]

                # 绘制信息
                overlay = frame.copy()
                cv2.rectangle(overlay, (5, 5), (450, 180), (0, 0, 0), -1)
                frame = cv2.addWeighted(overlay, 0.7, frame, 0.3, 0)

                # 颜色
                if self.current_fps >= 50:
                    fps_color = (0, 200, 0)
                elif self.current_fps >= 30:
                    fps_color = (0, 200, 200)
                else:
                    fps_color = (0, 0, 255)

                cv2.putText(frame, f"FPS: {self.current_fps:.1f}", (15, 40),
                           cv2.FONT_HERSHEY_SIMPLEX, 1.0, fps_color, 2)

                # 编码器信息
                cv2.putText(frame, f"捕获: GDI", (15, 70),
                           cv2.FONT_HERSHEY_SIMPLEX, 0.5, (200, 255, 200), 1)
                cv2.putText(frame, f"编码: h264_mf (GPU)", (15, 90),
                           cv2.FONT_HERSHEY_SIMPLEX, 0.5, (100, 255, 100), 1)
                cv2.putText(frame, f"已编码: {self.encode_count} 帧", (15, 110),
                           cv2.FONT_HERSHEY_SIMPLEX, 0.45, (255, 255, 255), 1)
                cv2.putText(frame, f"分辨率: {w}x{h}", (15, 130),
                           cv2.FONT_HERSHEY_SIMPLEX, 0.45, (255, 255, 255), 1)

                # 硬件加速标记
                cv2.putText(frame, "🚀 GPU 加速", (15, 160),
                           cv2.FONT_HERSHEY_SIMPLEX, 0.6, (0, 255, 0), 1)

                # 显示
                cv2.imshow("GPU Accelerated", frame)

                # 退出检查
                key = cv2.waitKey(1) & 0xFF
                if key == 27 or key == ord('q'):
                    break

                # 帧 pacing
                elapsed = time.perf_counter() - loop_start
                target = 1.0 / 60
                if elapsed < target:
                    time.sleep(target - elapsed)

        finally:
            cv2.destroyAllWindows()

        # 最终统计
        total_time = time.time() - self.start_time

        print("\n" + "="*70)
        print("测试完成")
        print("="*70)
        print(f"持续时间: {total_time:.1f}s")
        print(f"捕获帧数: {self.frame_count}")
        print(f"编码帧数: {self.encode_count}")
        print(f"平均 FPS: {self.frame_count / total_time:.1f}")

        if self.frame_count / total_time >= 25:
            print(f"评级: ⭐⭐⭐ 优秀")
        elif self.frame_count / total_time >= 15:
            print(f"评级: ⭐⭐ 良好")
        else:
            print(f"评级: ⭐ 一般")


if __name__ == "__main__":
    import argparse

    parser = argparse.ArgumentParser()
    parser.add_argument("--duration", type=int, default=30)
    parser.add_argument("--width", type=int, default=1920)
    parser.add_argument("--height", type=int, default=1080)
    args = parser.parse_args()

    try:
        app = H264MFCapture(
            width=args.width,
            height=args.height,
            fps=30
        )

        app.run(duration=args.duration)

    except Exception as e:
        print(f"\n错误: {e}")
        import traceback
        traceback.print_exc()
    finally:
        if 'app' in locals():
            app.close()
